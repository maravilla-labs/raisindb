//! The job handler wrapper: the periodic tick's entry point.

use std::sync::Arc;

use raisin_error::Result;
use raisin_locks::LockManagerHandle;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use serde_json::{json, Value};

use super::job::run_occurrence_rebuild;
use crate::RocksDBStorage;

/// Rebuilds the derived recurrence-occurrence projection.
pub struct CalendarExpandHandler {
    storage: Arc<RocksDBStorage>,
    /// `None` when the locks subsystem is disabled. The rebuild still runs —
    /// it is idempotent — but its once-per-cluster guarantee degrades to
    /// once-per-process, which on a multi-node cluster means every node
    /// rebuilds and the losers' writes are simply redundant.
    lock_manager: Option<LockManagerHandle>,
    instance_id: String,
}

impl CalendarExpandHandler {
    pub fn new(storage: Arc<RocksDBStorage>, lock_manager: Option<LockManagerHandle>) -> Self {
        Self {
            storage,
            lock_manager,
            instance_id: nanoid::nanoid!(8),
        }
    }

    /// Dispatch a `CalendarOccurrenceRebuild` job.
    pub async fn handle(&self, job: &JobInfo, _context: &JobContext) -> Result<Option<Value>> {
        let JobType::CalendarOccurrenceRebuild { tenant_id, repo_id } = &job.job_type else {
            return Ok(None);
        };
        let summary = run_occurrence_rebuild(
            &self.storage,
            self.lock_manager.as_ref(),
            &self.instance_id,
            tenant_id.clone(),
            repo_id.clone(),
        )
        .await?;
        // Returned as the job result rather than only logged: a projection that
        // silently stopped producing occurrences is otherwise invisible until
        // someone notices a query is short.
        Ok(Some(json!({
            "workspaces_scanned": summary.workspaces_scanned,
            "masters": summary.masters,
            "written": summary.written,
            "pruned": summary.pruned,
            "unchanged": summary.unchanged,
            "suppressed": summary.suppressed,
            "failed_masters": summary.failed_masters,
            "truncated_masters": summary.truncated_masters,
            "skipped_leased": summary.skipped_leased,
        })))
    }
}
