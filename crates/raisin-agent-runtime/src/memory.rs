// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! In-memory store, for tests and single-process embedding.
//!
//! One `std` mutex makes every operation its own critical section; nothing is
//! held across an await. Fail points let tests fail a commit (to prove a failed
//! commit assigns no seq) or observe one.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::checkpoint::RunCheckpoint;
use crate::control::ControlAck;
use crate::events::{RunEvent, RunEventKind};
use crate::ids::{RunId, RunScope, Seq, Version};
use crate::record::{check_invariants, AgentRunRecord};
use crate::state::RunStatus;
use crate::store::{
    check_control_dedup, verify_commit, AgentRunStore, CommitOutcome, CommitRequest, CreateOutcome,
    StoreError,
};

type ScopeKey = (String, String);

#[derive(Default)]
struct RunData {
    rec: Option<AgentRunRecord>,
    events: Vec<RunEvent>,
    ctl: HashMap<String, (ControlAck, String)>,
    idem: HashMap<String, Seq>,
    ckpts: BTreeMap<u32, RunCheckpoint>,
    dom: BTreeMap<u64, Value>,
    res: HashMap<String, Vec<u8>>,
}

#[derive(Default)]
struct Inner {
    runs: HashMap<(ScopeKey, RunId), RunData>,
    subj: HashMap<(ScopeKey, String), RunId>,
    subj_runs: HashMap<(ScopeKey, String), Vec<RunId>>,
    ckey: HashMap<(ScopeKey, String), RunId>,
}

/// A predicate that fails a commit.
pub type CommitFailPoint = Arc<dyn Fn(&CommitRequest) -> bool + Send + Sync>;

/// In-memory [`AgentRunStore`].
pub struct InMemoryAgentRunStore {
    inner: Mutex<Inner>,
    fail_commit: Mutex<Option<CommitFailPoint>>,
}

impl Default for InMemoryAgentRunStore {
    fn default() -> Self {
        Self::new()
    }
}

fn scope_key(scope: &RunScope) -> ScopeKey {
    (scope.tenant_id.clone(), scope.repo_id.clone())
}

impl InMemoryAgentRunStore {
    /// An empty store.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            fail_commit: Mutex::new(None),
        }
    }

    /// Fail every commit matching `pred` with a backend error (before writing).
    pub fn fail_commits_when(&self, pred: Option<CommitFailPoint>) {
        *self.fail_commit.lock().expect("fail point poisoned") = pred;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("memory store poisoned")
    }
}

#[async_trait]
impl AgentRunStore for InMemoryAgentRunStore {
    async fn create(
        &self,
        rec: AgentRunRecord,
        first: RunEventKind,
        create_key: Option<&str>,
    ) -> Result<CreateOutcome, StoreError> {
        rec.scope.validate()?;
        crate::ids::validate_key_part(rec.run_id.as_str())?;
        let subject_key = rec.subject.key()?;
        if let Some(k) = create_key {
            crate::ids::validate_key_part(k)?;
        }
        check_invariants(None, &rec)?;
        let sk = scope_key(&rec.scope);
        let mut inner = self.lock();
        if let Some(existing) = inner.subj.get(&(sk.clone(), subject_key.clone())).cloned() {
            let status = inner
                .runs
                .get(&(sk.clone(), existing.clone()))
                .and_then(|d| d.rec.as_ref())
                .map(|r| r.state.status());
            match status {
                Some(s) if !s.is_terminal() => {
                    return Ok(CreateOutcome::Existing {
                        run_id: existing,
                        status: s,
                    })
                }
                _ => {
                    inner.subj.remove(&(sk.clone(), subject_key.clone()));
                }
            }
        }
        if let Some(k) = create_key {
            if let Some(existing) = inner.ckey.get(&(sk.clone(), k.to_owned())).cloned() {
                let status = inner.runs[&(sk.clone(), existing.clone())]
                    .rec
                    .as_ref()
                    .map(|r| r.state.status())
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
        let run_id = rec.run_id.clone();
        let event = RunEvent {
            run_id: run_id.clone(),
            seq: Seq(1),
            at_ms: rec.created_at_ms,
            turn: None,
            op_id: None,
            kind: first,
        };
        let data = RunData {
            rec: Some(rec),
            events: vec![event],
            ..RunData::default()
        };
        inner.runs.insert((sk.clone(), run_id.clone()), data);
        inner
            .subj_runs
            .entry((sk.clone(), subject_key.clone()))
            .or_default()
            .push(run_id.clone());
        inner.subj.insert((sk.clone(), subject_key), run_id.clone());
        if let Some(k) = create_key {
            inner.ckey.insert((sk, k.to_owned()), run_id.clone());
        }
        Ok(CreateOutcome::Created {
            run_id,
            seq: Seq(1),
        })
    }

    async fn load(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<Option<AgentRunRecord>, StoreError> {
        Ok(self
            .lock()
            .runs
            .get(&(scope_key(scope), run.clone()))
            .and_then(|d| d.rec.clone()))
    }

    async fn commit(&self, req: CommitRequest) -> Result<CommitOutcome, StoreError> {
        let fail = self
            .fail_commit
            .lock()
            .expect("fail point poisoned")
            .clone();
        let sk = scope_key(&req.scope);
        let mut inner = self.lock();
        let data = inner
            .runs
            .get(&(sk.clone(), req.run_id.clone()))
            .ok_or(StoreError::NotFound)?;
        let stored = data.rec.as_ref().ok_or(StoreError::NotFound)?;
        let events = verify_commit(stored, &req)?;
        if let Some((id, _, _)) = &req.control {
            check_control_dedup(data.ctl.get(id.as_str()), &req)?;
        }
        if let Some((rev, _)) = &req.domain_state {
            if data.dom.contains_key(rev) {
                return Err(StoreError::Malformed(format!(
                    "domain state {rev} is write-once"
                )));
            }
        }
        if let Some(pred) = fail {
            if pred(&req) {
                return Err(StoreError::Backend("fail point".into()));
            }
        }
        let was_terminal = stored.state.is_terminal();
        let subject_key = stored.subject.key()?;
        let first_seq = Seq(stored.last_seq.0 + 1);
        let outcome = CommitOutcome {
            version: req.record.version,
            first_seq,
            last_seq: req.record.last_seq,
        };
        let now_terminal = req.record.state.is_terminal();
        let run_id = req.run_id.clone();
        let data = inner
            .runs
            .get_mut(&(sk.clone(), req.run_id.clone()))
            .expect("checked above");
        data.events.extend(events);
        if let Some((id, ack, digest)) = req.control {
            data.ctl.insert(id.0, (ack, digest));
        }
        for (key, seq) in req.idem {
            data.idem.insert(key, seq);
        }
        if let Some(c) = req.checkpoint {
            data.ckpts.insert(c.checkpoint_no, c);
        }
        if let Some((rev, v)) = req.domain_state {
            data.dom.insert(rev, v);
        }
        for (r, bytes) in req.results {
            data.res.insert(r.key, bytes);
        }
        data.rec = Some(req.record);
        if now_terminal
            && !was_terminal
            && inner.subj.get(&(sk.clone(), subject_key.clone())) == Some(&run_id)
        {
            inner.subj.remove(&(sk, subject_key));
        }
        Ok(outcome)
    }

    async fn read_events(
        &self,
        scope: &RunScope,
        run: &RunId,
        after: Seq,
        limit: usize,
    ) -> Result<Vec<RunEvent>, StoreError> {
        let inner = self.lock();
        let Some(d) = inner.runs.get(&(scope_key(scope), run.clone())) else {
            return Ok(Vec::new());
        };
        Ok(d.events
            .iter()
            .filter(|e| e.seq > after)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn control_ack(
        &self,
        scope: &RunScope,
        run: &RunId,
        id: &str,
    ) -> Result<Option<(ControlAck, String)>, StoreError> {
        Ok(self
            .lock()
            .runs
            .get(&(scope_key(scope), run.clone()))
            .and_then(|d| d.ctl.get(id).cloned()))
    }

    async fn idem_seen(
        &self,
        scope: &RunScope,
        run: &RunId,
        key: &str,
    ) -> Result<Option<Seq>, StoreError> {
        Ok(self
            .lock()
            .runs
            .get(&(scope_key(scope), run.clone()))
            .and_then(|d| d.idem.get(key).copied()))
    }

    async fn latest_checkpoint(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<Option<RunCheckpoint>, StoreError> {
        Ok(self
            .lock()
            .runs
            .get(&(scope_key(scope), run.clone()))
            .and_then(|d| d.ckpts.values().next_back().cloned()))
    }

    async fn read_checkpoint(
        &self,
        scope: &RunScope,
        run: &RunId,
        n: u32,
    ) -> Result<Option<RunCheckpoint>, StoreError> {
        Ok(self
            .lock()
            .runs
            .get(&(scope_key(scope), run.clone()))
            .and_then(|d| d.ckpts.get(&n).cloned()))
    }

    async fn domain_state(
        &self,
        scope: &RunScope,
        run: &RunId,
        rev: u64,
    ) -> Result<Option<Value>, StoreError> {
        Ok(self
            .lock()
            .runs
            .get(&(scope_key(scope), run.clone()))
            .and_then(|d| d.dom.get(&rev).cloned()))
    }

    async fn read_result(
        &self,
        scope: &RunScope,
        run: &RunId,
        key: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self
            .lock()
            .runs
            .get(&(scope_key(scope), run.clone()))
            .and_then(|d| d.res.get(key).cloned()))
    }

    async fn scan_subject(
        &self,
        scope: &RunScope,
        subject_key: &str,
        limit: usize,
    ) -> Result<Vec<RunId>, StoreError> {
        let inner = self.lock();
        let mut ids = inner
            .subj_runs
            .get(&(scope_key(scope), subject_key.to_owned()))
            .cloned()
            .unwrap_or_default();
        ids.truncate(limit);
        Ok(ids)
    }

    async fn scan_status(
        &self,
        scope: &RunScope,
        status: RunStatus,
        limit: usize,
    ) -> Result<Vec<RunId>, StoreError> {
        let sk = scope_key(scope);
        let inner = self.lock();
        let mut ids: Vec<RunId> = inner
            .runs
            .iter()
            .filter(|((s, _), d)| {
                s == &sk && d.rec.as_ref().is_some_and(|r| r.state.status() == status)
            })
            .map(|((_, id), _)| id.clone())
            .collect();
        ids.sort();
        ids.truncate(limit);
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::conformance_suite;

    #[tokio::test]
    async fn in_memory_store_passes_conformance() {
        conformance_suite(|| Arc::new(InMemoryAgentRunStore::default()) as Arc<dyn AgentRunStore>)
            .await;
    }
}
