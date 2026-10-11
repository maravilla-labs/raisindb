//! The build record mirror (E2): progress, log and result written onto a
//! caller-named node, so a subscriber watches an ordinary node instead of the
//! job system.
//!
//! The caller (Studio) creates the record — `settings:/packages/<key>/<run>`
//! — and names it in the request as `record: {workspace, path}`. The engine
//! then writes onto it, through the same transaction recipe as the job
//! activity node (`jobs/activity/publisher.rs`), so every write emits a node
//! event:
//!
//! - **Progress writes are bookkeeping.** `phase`, `estimate`, `counters`,
//!   `current`, `log_tail`/`log_seq` and `heartbeat_at` are operational
//!   metadata; the bookkeeping marker keeps them out of trigger evaluation
//!   and the per-revision snapshot job. They are coalesced to at most one per
//!   [`MIN_WRITE_INTERVAL`], written only when something moved, plus a
//!   heartbeat at least every [`HEARTBEAT_INTERVAL`] while the build runs.
//! - **The terminal write is NOT bookkeeping.** `status: completed | failed |
//!   cancelled` must fire the `node_event` trigger that finalizes the build;
//!   a terminal write carrying the marker would never reach it.
//!
//! The mirror only ever MERGES the properties it owns into the node it reads
//! in the same transaction. In particular it never writes `status` on a
//! progress write except the one `running` transition, so a concurrent
//! `cancel_requested` written by the caller is never overwritten — it is
//! read back, and reported to the build as a cancel request.
//!
//! Fields written (the record's node type must declare them): `status`,
//! `phase`, `job_id`, `started_at`, `heartbeat_at`, `finished_at`,
//! `estimate` (Object), `counters` (Object), `current`, `log_tail` (Array),
//! `log_seq`, `error` (Object), `size_bytes`, `sha256`, `fidelity` (Object),
//! and the Resources `artifact`, `manifest`, `inventory`, `log`.

use crate::RocksDBStorage;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::Resource;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::{JobId, JobRegistry};
use raisin_storage::transactional::TransactionalStorage;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Identity stamped on record writes.
pub(super) const MIRROR_ACTOR: &str = "package-build";
/// Coalescing floor for progress writes.
pub(super) const MIN_WRITE_INTERVAL: Duration = Duration::from_secs(1);
/// A progress write happens at least this often while the build runs.
pub(super) const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// Lines kept in `log_tail`.
pub(super) const LOG_RING: usize = 200;
/// Lines kept for the full `log` Resource.
pub(super) const LOG_FULL_CAP: usize = 20_000;

/// The caller-named record.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RecordRef {
    pub workspace: String,
    pub path: String,
}

/// What a write is, which decides its bookkeeping marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WriteKind {
    /// The first write: `status: running`, `started_at`, `job_id`.
    Started,
    /// Phase change, estimate, counters, log, heartbeat.
    Progress,
    /// `completed` / `failed` / `cancelled`.
    Terminal,
}

impl WriteKind {
    /// Progress is engine bookkeeping; the terminal write must fire triggers.
    pub(super) fn bookkeeping(self) -> bool {
        !matches!(self, WriteKind::Terminal)
    }
}

/// What the record said when it was read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecordSignal {
    Continue,
    /// The caller wrote `status: cancel_requested`.
    CancelRequested,
    /// The record is gone (deleted mid-build).
    Missing,
}

#[derive(Debug, Clone)]
pub(super) struct LogLine {
    pub seq: u64,
    pub ts: chrono::DateTime<chrono::Utc>,
    pub level: &'static str,
    pub msg: String,
}

/// Per-workspace counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct WsCount {
    pub items: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Estimate {
    pub items: u64,
    pub bytes: u64,
    pub per_ws: BTreeMap<String, WsCount>,
    /// The count hit its time budget; the real total is larger.
    pub partial: bool,
    pub revised_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Counters {
    pub items_written: u64,
    pub bytes_written: u64,
    pub files: u64,
    pub node_types: u64,
    pub warnings: u64,
    pub skipped: u64,
    pub bytes_per_s: u64,
    pub per_ws: BTreeMap<String, WsCount>,
}

/// The mirror's state, independent of storage, so coalescing is testable.
pub(super) struct MirrorState {
    pub phase: &'static str,
    pub estimate: Option<Estimate>,
    pub counters: Counters,
    pub current: Option<String>,
    log_tail: VecDeque<LogLine>,
    log_full: Vec<String>,
    log_dropped: u64,
    pub log_seq: u64,
    mirror_seq: u64,
    last_write: Option<Instant>,
    dirty: bool,
}

impl MirrorState {
    pub(super) fn new() -> Self {
        Self {
            phase: "queued",
            estimate: None,
            counters: Counters::default(),
            current: None,
            log_tail: VecDeque::with_capacity(LOG_RING),
            log_full: Vec::new(),
            log_dropped: 0,
            log_seq: 0,
            mirror_seq: 0,
            last_write: None,
            dirty: false,
        }
    }

    pub(super) fn log(&mut self, level: &'static str, msg: String) {
        self.log_seq += 1;
        let line = LogLine {
            seq: self.log_seq,
            ts: chrono::Utc::now(),
            level,
            msg,
        };
        if self.log_full.len() < LOG_FULL_CAP {
            self.log_full.push(format!(
                "{} {:5} {}",
                line.ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                line.level,
                line.msg
            ));
        } else {
            self.log_dropped += 1;
        }
        if self.log_tail.len() == LOG_RING {
            self.log_tail.pop_front();
        }
        self.log_tail.push_back(line);
        self.dirty = true;
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Should a progress write happen now? Something moved and the
    /// coalescing floor passed, or the heartbeat is due.
    pub(super) fn progress_due(&self, now: Instant) -> bool {
        match self.last_write {
            None => true,
            Some(at) => {
                let since = now.saturating_duration_since(at);
                (self.dirty && since >= MIN_WRITE_INTERVAL) || since >= HEARTBEAT_INTERVAL
            }
        }
    }

    pub(super) fn wrote(&mut self, now: Instant) {
        self.last_write = Some(now);
        self.dirty = false;
        self.mirror_seq += 1;
    }

    /// The full log, for the `log` Resource.
    pub(super) fn full_log(&self) -> String {
        let mut out = self.log_full.join("\n");
        if self.log_dropped > 0 {
            out.push_str(&format!(
                "\n… {} later lines omitted ({} kept)",
                self.log_dropped, LOG_FULL_CAP
            ));
        }
        out.push('\n');
        out
    }

    /// The progress properties, as of now.
    pub(super) fn progress_properties(&self) -> HashMap<String, PropertyValue> {
        let mut p = HashMap::new();
        p.insert("phase".into(), PropertyValue::String(self.phase.into()));
        p.insert("heartbeat_at".into(), now_date());
        if let Some(est) = &self.estimate {
            p.insert("estimate".into(), estimate_value(est));
        }
        let mut counters = counters_map(&self.counters);
        // Monotonic per build, so an observer can discard an out-of-order
        // event (record writes replicate like any node write, R6).
        counters.insert(
            "mirror_seq".into(),
            PropertyValue::Integer(self.mirror_seq as i64 + 1),
        );
        p.insert("counters".into(), PropertyValue::Object(counters));
        if let Some(cur) = &self.current {
            p.insert("current".into(), PropertyValue::String(cur.clone()));
        }
        p.insert(
            "log_tail".into(),
            PropertyValue::Array(
                self.log_tail
                    .iter()
                    .map(|l| {
                        let mut m = HashMap::new();
                        m.insert("seq".into(), PropertyValue::Integer(l.seq as i64));
                        m.insert(
                            "ts".into(),
                            PropertyValue::String(
                                l.ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                            ),
                        );
                        m.insert("level".into(), PropertyValue::String(l.level.into()));
                        m.insert("msg".into(), PropertyValue::String(l.msg.clone()));
                        PropertyValue::Object(m)
                    })
                    .collect(),
            ),
        );
        p.insert(
            "log_seq".into(),
            PropertyValue::Integer(self.log_seq as i64),
        );
        p
    }
}

pub(super) fn now_date() -> PropertyValue {
    PropertyValue::Date(chrono::Utc::now().into())
}

fn ws_map(per_ws: &BTreeMap<String, WsCount>) -> PropertyValue {
    PropertyValue::Object(
        per_ws
            .iter()
            .map(|(ws, c)| {
                let mut m = HashMap::new();
                m.insert("items".into(), PropertyValue::Integer(c.items as i64));
                m.insert("bytes".into(), PropertyValue::Integer(c.bytes as i64));
                (ws.clone(), PropertyValue::Object(m))
            })
            .collect(),
    )
}

pub(super) fn estimate_value(est: &Estimate) -> PropertyValue {
    let mut m = HashMap::new();
    m.insert("items".into(), PropertyValue::Integer(est.items as i64));
    m.insert("bytes".into(), PropertyValue::Integer(est.bytes as i64));
    m.insert("per_ws".into(), ws_map(&est.per_ws));
    m.insert("partial".into(), PropertyValue::Boolean(est.partial));
    if let Some(at) = est.revised_at {
        m.insert("revised_at".into(), PropertyValue::String(at.to_rfc3339()));
    }
    PropertyValue::Object(m)
}

fn counters_map(c: &Counters) -> HashMap<String, PropertyValue> {
    let mut m = HashMap::new();
    let int = |v: u64| PropertyValue::Integer(v as i64);
    m.insert("items_written".into(), int(c.items_written));
    m.insert("bytes_written".into(), int(c.bytes_written));
    m.insert("files".into(), int(c.files));
    m.insert("node_types".into(), int(c.node_types));
    m.insert("warnings".into(), int(c.warnings));
    m.insert("skipped".into(), int(c.skipped));
    m.insert("bytes_per_s".into(), int(c.bytes_per_s));
    m.insert("per_ws".into(), ws_map(&c.per_ws));
    m
}

/// A stored blob as a Resource envelope `{key, size, sha256, mime}`.
pub(super) fn blob_resource(
    stored: &raisin_binary::StoredObject,
    sha256: &str,
    name: &str,
    mime: &str,
) -> PropertyValue {
    let mut metadata = HashMap::new();
    metadata.insert(
        "storage_key".into(),
        PropertyValue::String(stored.key.clone()),
    );
    metadata.insert("sha256".into(), PropertyValue::String(sha256.to_string()));
    PropertyValue::Resource(Resource {
        uuid: nanoid::nanoid!(),
        name: Some(name.to_string()),
        size: Some(stored.size),
        mime_type: Some(mime.to_string()),
        url: Some(stored.key.clone()),
        metadata: Some(metadata),
        is_loaded: Some(true),
        is_external: Some(false),
        created_at: stored.created_at.into(),
        updated_at: stored.updated_at.into(),
    })
}

/// Writes the [`MirrorState`] onto the record. Without a record it only
/// feeds the job log, so a build started without one behaves as before.
pub(super) struct Mirror {
    storage: Arc<RocksDBStorage>,
    registry: Arc<JobRegistry>,
    job_id: JobId,
    tenant: String,
    repo: String,
    branch: String,
    record: Option<RecordRef>,
    pub state: MirrorState,
    started: bool,
}

impl Mirror {
    pub(super) fn new(
        storage: Arc<RocksDBStorage>,
        registry: Arc<JobRegistry>,
        job_id: JobId,
        tenant: &str,
        repo: &str,
        branch: &str,
        record: Option<RecordRef>,
    ) -> Self {
        Self {
            storage,
            registry,
            job_id,
            tenant: tenant.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
            record,
            state: MirrorState::new(),
            started: false,
        }
    }

    pub(super) fn record(&self) -> Option<&RecordRef> {
        self.record.as_ref()
    }

    /// Does the record say THIS job is already running it — i.e. is this a
    /// re-run of an attempt that was interrupted?
    pub(super) async fn was_running_as(&self, job_id: &JobId) -> bool {
        use raisin_storage::{NodeRepository, Storage, StorageScope};
        let Some(record) = &self.record else {
            return false;
        };
        let scope = StorageScope::new(&self.tenant, &self.repo, &self.branch, &record.workspace);
        let Ok(Some(node)) = self
            .storage
            .nodes()
            .get_by_path(scope, &record.path, None)
            .await
        else {
            return false;
        };
        let is = |k: &str, v: &str| matches!(node.properties.get(k), Some(PropertyValue::String(s)) if s == v);
        is("job_id", job_id.0.as_str())
            && (is("status", "running") || is("status", "cancel_requested"))
    }

    /// A log line: to the job's live log (E5) and to the record's ring.
    /// Lines carry paths, counts and codes — never property values.
    pub(super) async fn log(&mut self, level: &'static str, msg: impl Into<String>) {
        let msg = msg.into();
        self.registry.emit_log(&self.job_id, level, &msg).await;
        match level {
            "error" => tracing::error!(job_id = %self.job_id, "{msg}"),
            "warn" => tracing::warn!(job_id = %self.job_id, "{msg}"),
            _ => tracing::info!(job_id = %self.job_id, "{msg}"),
        }
        self.state.log(level, msg);
    }

    /// Enter a phase; written at once (a transition is never coalesced away).
    pub(super) async fn phase(&mut self, phase: &'static str) -> RecordSignal {
        self.state.phase = phase;
        self.state.mark_dirty();
        self.flush().await
    }

    /// Write progress if it is due. Cheap to call per node.
    pub(super) async fn tick(&mut self) -> RecordSignal {
        if self.record.is_none() || !self.state.progress_due(Instant::now()) {
            return RecordSignal::Continue;
        }
        self.flush().await
    }

    /// Write progress now.
    pub(super) async fn flush(&mut self) -> RecordSignal {
        if self.record.is_none() {
            self.state.wrote(Instant::now());
            return RecordSignal::Continue;
        }
        let mut props = self.state.progress_properties();
        let kind = if self.started {
            WriteKind::Progress
        } else {
            props.insert("status".into(), PropertyValue::String("running".into()));
            props.insert("started_at".into(), now_date());
            props.insert(
                "job_id".into(),
                PropertyValue::String(self.job_id.to_string()),
            );
            WriteKind::Started
        };
        let signal = self.write(kind, props).await;
        if !matches!(signal, Err(_)) {
            self.started = true;
        }
        self.state.wrote(Instant::now());
        signal.unwrap_or(RecordSignal::Continue)
    }

    /// The terminal write: progress plus the outcome, WITHOUT the
    /// bookkeeping marker. Retried once, because a lost terminal write leaves
    /// the record `running` until the caller's sweeper calls it lost.
    pub(super) async fn terminal(
        &mut self,
        status: &str,
        mut extra: HashMap<String, PropertyValue>,
    ) {
        if self.record.is_none() {
            return;
        }
        let mut props = self.state.progress_properties();
        props.insert("status".into(), PropertyValue::String(status.into()));
        props.insert("finished_at".into(), now_date());
        if !self.started {
            props.insert("started_at".into(), now_date());
            props.insert(
                "job_id".into(),
                PropertyValue::String(self.job_id.to_string()),
            );
        }
        props.extend(extra.drain());
        for attempt in 0..2 {
            match self.write(WriteKind::Terminal, props.clone()).await {
                Ok(_) => return,
                Err(e) => tracing::warn!(
                    job_id = %self.job_id,
                    attempt,
                    error = %e,
                    "package build: terminal record write failed"
                ),
            }
        }
    }

    async fn write(
        &self,
        kind: WriteKind,
        props: HashMap<String, PropertyValue>,
    ) -> raisin_error::Result<RecordSignal> {
        let Some(record) = &self.record else {
            return Ok(RecordSignal::Continue);
        };
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(&self.tenant, &self.repo)?;
        tx.set_branch(&self.branch)?;
        tx.set_actor(MIRROR_ACTOR)?;
        tx.set_auth_context(AuthContext::system_as(MIRROR_ACTOR))?;
        tx.set_message(match kind {
            WriteKind::Terminal => "package build finished",
            _ => "package build progress",
        })?;
        tx.set_bookkeeping(kind.bookkeeping())?;

        let Some(mut node) = tx.get_node_by_path(&record.workspace, &record.path).await? else {
            let _ = tx.rollback().await;
            return Ok(RecordSignal::Missing);
        };
        let cancel_requested = matches!(
            node.properties.get("status"),
            Some(PropertyValue::String(s)) if s == "cancel_requested"
        );
        let mut props = props;
        if cancel_requested && kind != WriteKind::Terminal {
            // Never overwrite the caller's request with `running`.
            props.remove("status");
        }
        node.properties.extend(props);
        tx.put_node(&record.workspace, &node).await?;
        tx.commit().await?;
        Ok(if cancel_requested {
            RecordSignal::CancelRequested
        } else {
            RecordSignal::Continue
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_terminal_write_fires_triggers() {
        assert!(WriteKind::Started.bookkeeping());
        assert!(WriteKind::Progress.bookkeeping());
        assert!(!WriteKind::Terminal.bookkeeping());
    }

    #[test]
    fn progress_is_coalesced_and_heartbeats() {
        let t0 = Instant::now();
        let mut s = MirrorState::new();
        assert!(s.progress_due(t0), "the first write is always due");
        s.wrote(t0);
        // Nothing moved: no write until the heartbeat.
        assert!(!s.progress_due(t0 + Duration::from_secs(5)));
        assert!(s.progress_due(t0 + HEARTBEAT_INTERVAL));
        // Something moved: written once the floor passes, not before.
        s.mark_dirty();
        assert!(!s.progress_due(t0 + Duration::from_millis(500)));
        assert!(s.progress_due(t0 + MIN_WRITE_INTERVAL));
        s.wrote(t0 + MIN_WRITE_INTERVAL);
        assert!(!s.progress_due(t0 + MIN_WRITE_INTERVAL + Duration::from_millis(10)));
    }

    #[test]
    fn the_log_ring_keeps_the_last_200_and_counts_on() {
        let mut s = MirrorState::new();
        for i in 0..450 {
            s.log("info", format!("line {i}"));
        }
        assert_eq!(s.log_seq, 450);
        let props = s.progress_properties();
        let Some(PropertyValue::Array(tail)) = props.get("log_tail") else {
            panic!("log_tail missing");
        };
        assert_eq!(tail.len(), LOG_RING);
        let first_seq = match &tail[0] {
            PropertyValue::Object(m) => m.get("seq").cloned(),
            _ => None,
        };
        assert_eq!(first_seq, Some(PropertyValue::Integer(251)));
        assert_eq!(props.get("log_seq"), Some(&PropertyValue::Integer(450)));
        assert!(s.full_log().contains("line 0\n"));
    }

    #[test]
    fn mirror_seq_increases_with_every_write() {
        let mut s = MirrorState::new();
        let seq = |s: &MirrorState| match s.progress_properties().get("counters") {
            Some(PropertyValue::Object(m)) => m.get("mirror_seq").cloned(),
            _ => None,
        };
        assert_eq!(seq(&s), Some(PropertyValue::Integer(1)));
        s.wrote(Instant::now());
        assert_eq!(seq(&s), Some(PropertyValue::Integer(2)));
    }
}
