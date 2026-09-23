// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `NodeAgentRunStore`: the `AgentRunStore` contract over ordinary nodes,
//! persisted and replicated exactly the way flow instances are.
//!
//! A commit is ONE node transaction (the record, its events, the control ack,
//! idempotency marks, checkpoint, domain state, results and every index move),
//! marked as engine bookkeeping so it skips trigger evaluation, snapshots and
//! embeddings while staying durable, versioned, replicated and observable.
//! Critical sections are per run and per subject ([`RunLocks`]: the flow
//! instance pairing of an in-process keyed mutex and, when the locks subsystem
//! is configured, a `raisin-locks` lease), always taken subject first.
//!
//! Replication is the node path's: a peer receives each commit as ordinary node
//! operations. As with flow instances, the lease serializes writers across the
//! cluster; a writer on a node whose replica has not yet received the previous
//! commit reads an older version and gets a `VersionConflict`, which every
//! caller already retries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use raisin_agent_runtime::checkpoint::RunCheckpoint;
use raisin_agent_runtime::control::ControlAck;
use raisin_agent_runtime::events::{RunEvent, RunEventKind};
use raisin_agent_runtime::ids::{validate_key_part, RunId, RunScope, Seq, Version};
use raisin_agent_runtime::record::{check_invariants, AgentRunRecord};
use raisin_agent_runtime::state::RunStatus;
use raisin_agent_runtime::store::{
    check_control_dedup, verify_commit, AgentRunStore, CommitOutcome, CommitRequest, CreateOutcome,
    StoreError,
};
use raisin_locks::LockManagerHandle;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_storage::scope::StorageScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{ListOptions, NodeRepository, RepositoryManagementRepository, Storage};
use serde_json::Value;

use super::layout::{self as l, json, RUN_WORKSPACE};
use super::lock::{LockKey, RunLocks};
use super::store_writes::{handback_owed, Writes};
use crate::RocksDBStorage;

/// Branch used when a repository names no default.
const FALLBACK_BRANCH: &str = "main";

/// The node-backed run store.
pub struct NodeAgentRunStore {
    storage: Arc<RocksDBStorage>,
    run_locks: RunLocks,
    subject_locks: RunLocks,
    home: Mutex<HashMap<(String, String), String>>,
}

fn lock_key(scope: &RunScope, id: &str) -> LockKey {
    (
        scope.tenant_id.clone(),
        scope.repo_id.clone(),
        id.to_owned(),
    )
}

fn ids_of(nodes: Vec<Node>, limit: usize) -> Vec<RunId> {
    nodes
        .into_iter()
        .take(limit)
        .map(|n| RunId(n.name))
        .collect()
}

impl NodeAgentRunStore {
    /// A store whose commit sections also take a distributed lease from
    /// `locks` when the locks subsystem is configured.
    pub fn open(
        storage: Arc<RocksDBStorage>,
        locks: Option<LockManagerHandle>,
        node_id: String,
    ) -> Self {
        Self {
            storage,
            run_locks: RunLocks::new("run", locks.clone(), node_id.clone()),
            subject_locks: RunLocks::new("subject", locks, node_id),
            home: Mutex::new(HashMap::new()),
        }
    }

    /// The branch a repository's runs live on: its default branch (runs are
    /// keyed by repository; a run's own branch is in its record).
    async fn home(&self, scope: &RunScope) -> Result<String, StoreError> {
        let key = (scope.tenant_id.clone(), scope.repo_id.clone());
        if let Some(b) = self.home.lock().expect("home poisoned").get(&key) {
            return Ok(b.clone());
        }
        let branch = self
            .storage
            .repository_management()
            .get_repository(&scope.tenant_id, &scope.repo_id)
            .await?
            .map(|r| r.config.default_branch)
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| FALLBACK_BRANCH.to_string());
        self.home
            .lock()
            .expect("home poisoned")
            .insert(key, branch.clone());
        Ok(branch)
    }

    pub(super) async fn node_at(
        &self,
        scope: &RunScope,
        path: &str,
    ) -> Result<Option<Node>, StoreError> {
        let branch = self.home(scope).await?;
        let s = StorageScope::new(&scope.tenant_id, &scope.repo_id, &branch, RUN_WORKSPACE);
        Ok(self.storage.nodes().get_by_path(s, path, None).await?)
    }

    async fn children(&self, scope: &RunScope, path: &str) -> Result<Vec<Node>, StoreError> {
        let branch = self.home(scope).await?;
        let s = StorageScope::new(&scope.tenant_id, &scope.repo_id, &branch, RUN_WORKSPACE);
        match self
            .storage
            .nodes()
            .list_children(s, path, ListOptions::default())
            .await
        {
            Ok(nodes) => Ok(nodes),
            // A folder that was never written has no children.
            Err(raisin_error::Error::NotFound(_)) => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    async fn body_at<T: serde::de::DeserializeOwned>(
        &self,
        scope: &RunScope,
        path: &str,
        key: &str,
    ) -> Result<Option<T>, StoreError> {
        match self.node_at(scope, path).await? {
            Some(n) => l::body(&n, key).map(Some),
            None => Ok(None),
        }
    }

    /// Open a bookkeeping transaction on the runs' home branch.
    async fn begin(
        &self,
        scope: &RunScope,
        message: &str,
    ) -> Result<Box<dyn TransactionalContext>, StoreError> {
        let branch = self.home(scope).await?;
        let ctx = self.storage.begin_context().await?;
        ctx.set_tenant_repo(&scope.tenant_id, &scope.repo_id)?;
        ctx.set_branch(&branch)?;
        ctx.set_message(message)?;
        ctx.set_actor("system")?;
        ctx.set_auth_context(AuthContext::system())?;
        ctx.set_is_system(true)?;
        ctx.set_bookkeeping(true)?;
        ctx.set_validate_schema(false)?;
        Ok(ctx)
    }

    /// Apply `w` in ONE transaction.
    async fn write(&self, scope: &RunScope, message: &str, w: Writes) -> Result<(), StoreError> {
        let ctx = self.begin(scope, message).await?;
        for path in &w.deletes {
            if let Some(n) = ctx.get_node_by_path(RUN_WORKSPACE, path).await? {
                ctx.delete_node(RUN_WORKSPACE, &n.id).await?;
            }
        }
        // Parents first, so a record exists before its tables and an ancestor
        // is only ever auto-created where a folder is allowed.
        let mut puts: Vec<_> = w.puts.iter().collect();
        puts.sort_by_key(|(path, _)| l::depth(path));
        for (path, props) in puts {
            let props: Vec<(&str, String)> = props.iter().map(|(k, v)| (*k, v.clone())).collect();
            ctx.upsert_deep_node(
                RUN_WORKSPACE,
                &l::node(scope, path, &props),
                l::ancestor_type(path),
            )
            .await?;
        }
        ctx.commit().await?;
        Ok(())
    }

    async fn status_of(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<Option<RunStatus>, StoreError> {
        Ok(self.load(scope, run).await?.map(|r| r.state.status()))
    }
}

#[async_trait]
impl AgentRunStore for NodeAgentRunStore {
    async fn create(
        &self,
        rec: AgentRunRecord,
        first: RunEventKind,
        create_key: Option<&str>,
    ) -> Result<CreateOutcome, StoreError> {
        let scope = rec.scope.clone();
        scope.validate()?;
        let subject_key = rec.subject.key()?;
        if let Some(k) = create_key {
            validate_key_part(k)?;
        }
        check_invariants(None, &rec)?;
        l::record(&rec.run_id)?;
        let subj_path = l::subject(&subject_key);

        // Lock order: subject, then run.
        let _subject = self
            .subject_locks
            .lock(lock_key(&scope, &subject_key))
            .await?;
        let live = match self.node_at(&scope, &subj_path).await? {
            Some(n) => l::prop(&n, "live_run_id")
                .filter(|s| !s.is_empty())
                .map(|s| RunId(s.into())),
            None => None,
        };
        if let Some(existing) = live {
            let _run = self
                .run_locks
                .lock(lock_key(&scope, existing.as_str()))
                .await?;
            if let Some(status) = self.status_of(&scope, &existing).await? {
                if !status.is_terminal() {
                    return Ok(CreateOutcome::Existing {
                        run_id: existing,
                        status,
                    });
                }
            }
        }
        if let Some(k) = create_key {
            if let Some(n) = self.node_at(&scope, &l::create_key(k)).await? {
                let existing = RunId(l::prop(&n, "run_id").unwrap_or_default().to_string());
                let status = self
                    .status_of(&scope, &existing)
                    .await?
                    .unwrap_or(RunStatus::Queued);
                return Ok(CreateOutcome::Existing {
                    run_id: existing,
                    status,
                });
            }
        }
        let mut rec = rec;
        rec.version = Version(1);
        rec.last_seq = Seq(1);
        let run = rec.run_id.clone();
        let event = RunEvent {
            run_id: run.clone(),
            seq: Seq(1),
            at_ms: rec.created_at_ms,
            turn: None,
            op_id: None,
            kind: first,
        };
        let mut w = Writes::default();
        w.record(&rec)?;
        w.put(l::event(&run, 1)?, vec![("event", json(&event)?)]);
        w.put(l::status_entry(rec.state.status(), &run)?, vec![]);
        w.put(
            subj_path,
            vec![
                ("subject_key", subject_key.clone()),
                ("live_run_id", run.to_string()),
            ],
        );
        w.put(
            l::subject_run(&subject_key, &run)?,
            vec![("created_at_ms", rec.created_at_ms.to_string())],
        );
        if let Some(k) = create_key {
            w.put(
                l::create_key(k),
                vec![("key", k.to_string()), ("run_id", run.to_string())],
            );
        }
        self.write(&scope, &format!("agent run {run} created"), w)
            .await?;
        Ok(CreateOutcome::Created {
            run_id: run,
            seq: Seq(1),
        })
    }

    async fn load(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<Option<AgentRunRecord>, StoreError> {
        let rec: Option<AgentRunRecord> = self.body_at(scope, &l::record(run)?, "record").await?;
        // Runs are keyed by repository: another tenant/repo never sees it.
        Ok(
            rec.filter(|r| {
                r.scope.tenant_id == scope.tenant_id && r.scope.repo_id == scope.repo_id
            }),
        )
    }

    async fn commit(&self, req: CommitRequest) -> Result<CommitOutcome, StoreError> {
        let scope = req.scope.clone();
        let run = req.run_id.clone();
        let _run = self.run_locks.lock(lock_key(&scope, run.as_str())).await?;

        let stored = self.load(&scope, &run).await?.ok_or(StoreError::NotFound)?;
        let events = verify_commit(&stored, &req)?;
        if let Some((id, _, _)) = &req.control {
            let existing: Option<(ControlAck, String)> = self
                .body_at(&scope, &l::keyed(&run, "ctl", id.as_str())?, "ack")
                .await?;
            check_control_dedup(existing.as_ref(), &req)?;
        }
        if let Some((rev, _)) = &req.domain_state {
            if self
                .node_at(&scope, &l::domain_state(&run, *rev)?)
                .await?
                .is_some()
            {
                return Err(StoreError::Malformed(format!(
                    "domain state {rev} is write-once"
                )));
            }
        }

        let mut w = Writes::default();
        w.record(&req.record)?;
        for ev in &events {
            w.put(l::event(&run, ev.seq.0)?, vec![("event", json(ev)?)]);
        }
        if let Some((id, ack, digest)) = &req.control {
            w.put(
                l::keyed(&run, "ctl", id.as_str())?,
                vec![("id", id.to_string()), ("ack", json(&(ack, digest))?)],
            );
        }
        for (key, seq) in &req.idem {
            w.put(
                l::keyed(&run, "idem", key)?,
                vec![("key", key.clone()), ("seq", seq.0.to_string())],
            );
        }
        if let Some(c) = &req.checkpoint {
            w.put(
                l::checkpoint(&run, c.checkpoint_no)?,
                vec![("checkpoint", json(c)?)],
            );
        }
        if let Some((rev, value)) = &req.domain_state {
            w.put(l::domain_state(&run, *rev)?, vec![("state", json(value)?)]);
        }
        for (r, bytes) in &req.results {
            use base64::Engine as _;
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            w.put(
                l::keyed(&run, "res", &r.key)?,
                vec![("key", r.key.clone()), ("bytes_b64", b64)],
            );
        }
        let (from, to) = (stored.state.status(), req.record.state.status());
        if from != to {
            w.delete(l::status_entry(from, &run)?);
            w.put(l::status_entry(to, &run)?, vec![]);
        }
        if to.is_terminal() && !from.is_terminal() {
            let key = stored.subject.key()?;
            let subj = self.node_at(&scope, &l::subject(&key)).await?;
            if subj.as_ref().and_then(|n| l::prop(n, "live_run_id")) == Some(run.as_str()) {
                w.put(
                    l::subject(&key),
                    vec![("subject_key", key), ("live_run_id", String::new())],
                );
            }
        }
        // The finalize-owed worklist: a terminal domain run stays in it until
        // its reducer has seen the end.
        let owed = to.is_terminal() && req.record.domain.as_ref().is_some_and(|d| !d.finalized);
        let was_owed = from.is_terminal() && stored.domain.as_ref().is_some_and(|d| !d.finalized);
        w.worklist(l::owed(l::FINALIZE, &run)?, was_owed, owed);
        // The hand-back-owed worklist: a terminal run stays in it until what
        // waits for it (a parent's mailbox, a flow) has been told.
        let hb = handback_owed(&req.record);
        let was_hb = handback_owed(&stored);
        w.worklist(l::owed(l::HANDBACK, &run)?, was_hb, hb);
        self.write(
            &scope,
            &format!("agent run {run} v{}", req.record.version.0),
            w,
        )
        .await?;
        Ok(CommitOutcome {
            version: req.record.version,
            first_seq: Seq(stored.last_seq.0 + 1),
            last_seq: req.record.last_seq,
        })
    }

    async fn read_events(
        &self,
        scope: &RunScope,
        run: &RunId,
        after: Seq,
        limit: usize,
    ) -> Result<Vec<RunEvent>, StoreError> {
        let Some(rec) = self.load(scope, run).await? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        let mut seq = after.0.saturating_add(1);
        while seq <= rec.last_seq.0 && out.len() < limit {
            if let Some(ev) = self.body_at(scope, &l::event(run, seq)?, "event").await? {
                out.push(ev);
            }
            seq += 1;
        }
        Ok(out)
    }

    async fn control_ack(
        &self,
        scope: &RunScope,
        run: &RunId,
        id: &str,
    ) -> Result<Option<(ControlAck, String)>, StoreError> {
        self.body_at(scope, &l::keyed(run, "ctl", id)?, "ack").await
    }

    async fn idem_seen(
        &self,
        scope: &RunScope,
        run: &RunId,
        key: &str,
    ) -> Result<Option<Seq>, StoreError> {
        let Some(n) = self.node_at(scope, &l::keyed(run, "idem", key)?).await? else {
            return Ok(None);
        };
        l::prop(&n, "seq")
            .and_then(|s| s.parse().ok())
            .map(|s| Some(Seq(s)))
            .ok_or_else(|| StoreError::Backend(format!("{}: bad seq", n.path)))
    }

    async fn latest_checkpoint(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<Option<RunCheckpoint>, StoreError> {
        let mut nodes = self.children(scope, &l::table(run, "ckpt")?).await?;
        nodes.sort_by(|a, b| a.name.cmp(&b.name));
        match nodes.pop() {
            Some(n) => l::body(&n, "checkpoint").map(Some),
            None => Ok(None),
        }
    }

    async fn read_checkpoint(
        &self,
        scope: &RunScope,
        run: &RunId,
        n: u32,
    ) -> Result<Option<RunCheckpoint>, StoreError> {
        self.body_at(scope, &l::checkpoint(run, n)?, "checkpoint")
            .await
    }

    async fn domain_state(
        &self,
        scope: &RunScope,
        run: &RunId,
        rev: u64,
    ) -> Result<Option<Value>, StoreError> {
        self.body_at(scope, &l::domain_state(run, rev)?, "state")
            .await
    }

    async fn read_result(
        &self,
        scope: &RunScope,
        run: &RunId,
        key: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        use base64::Engine as _;
        let Some(n) = self.node_at(scope, &l::keyed(run, "res", key)?).await? else {
            return Ok(None);
        };
        let raw = l::prop(&n, "bytes_b64").unwrap_or_default();
        base64::engine::general_purpose::STANDARD
            .decode(raw)
            .map(Some)
            .map_err(|e| StoreError::Backend(format!("{}: {e}", n.path)))
    }

    async fn scan_subject(
        &self,
        scope: &RunScope,
        subject_key: &str,
        limit: usize,
    ) -> Result<Vec<RunId>, StoreError> {
        let mut nodes = self.children(scope, &l::subject(subject_key)).await?;
        nodes.sort_by_key(|n| {
            l::prop(n, "created_at_ms")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        });
        Ok(ids_of(nodes, limit))
    }

    async fn scan_status(
        &self,
        scope: &RunScope,
        status: RunStatus,
        limit: usize,
    ) -> Result<Vec<RunId>, StoreError> {
        Ok(ids_of(
            self.children(scope, &l::status_folder(status)).await?,
            limit,
        ))
    }

    async fn scan_handback_owed(
        &self,
        scope: &RunScope,
        limit: usize,
    ) -> Result<Vec<RunId>, StoreError> {
        Ok(ids_of(
            self.children(scope, &l::owed_folder(l::HANDBACK)).await?,
            limit,
        ))
    }

    async fn scan_unfinalized(
        &self,
        scope: &RunScope,
        limit: usize,
    ) -> Result<Vec<RunId>, StoreError> {
        Ok(ids_of(
            self.children(scope, &l::owed_folder(l::FINALIZE)).await?,
            limit,
        ))
    }
}
