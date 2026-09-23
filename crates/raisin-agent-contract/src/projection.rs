// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The generic plan projection: core-owned shape, domain-owned content.
//!
//! [`effective_projection`] is what makes "no stopped run displays an active
//! task" true BY CONSTRUCTION: whatever a domain last stored, an `in_progress`
//! item survives only while the run is actually running.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::reducer::RunStatus;

/// A plan projection.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PlanProjection {
    /// Items, in display order.
    #[serde(default)]
    pub items: Vec<PlanItem>,
    /// Summary line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// One item of a plan projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    /// Stable key.
    pub key: String,
    /// Display title.
    pub title: String,
    /// Status.
    pub status: ItemStatus,
    /// Domain-owned detail, opaque to core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

/// Status of one plan item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    /// Not started.
    Pending,
    /// Being worked on — only ever shown while the run is running.
    InProgress,
    /// Waiting for a human or an external result.
    Waiting,
    /// Done.
    Completed,
    /// Cannot proceed.
    Blocked,
    /// Failed.
    Failed,
    /// The run was stopped.
    Stopped,
    /// The run is paused.
    Paused,
}

impl PlanProjection {
    /// Parse a projection value; `None` for `null`, `Err` for a wrong shape.
    pub fn parse(value: &Value) -> Result<Option<Self>, serde_json::Error> {
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value.clone()).map(Some)
    }
}

/// The projection a reader should display for a run in `run_status`.
///
/// `in_progress` survives only when the run is `running`; otherwise it becomes
/// `waiting` (waiting), `paused` (paused or queued), `stopped` (stopped or
/// cancelling), `failed` (failed) or `blocked` (completed with the item still
/// open). Other statuses are left as the domain stored them.
pub fn effective_projection(
    projection: Option<&PlanProjection>,
    run_status: RunStatus,
) -> Option<PlanProjection> {
    let mut projection = projection?.clone();
    if run_status == RunStatus::Running {
        return Some(projection);
    }
    let replacement = match run_status {
        RunStatus::Running => ItemStatus::InProgress,
        RunStatus::Waiting => ItemStatus::Waiting,
        RunStatus::Paused | RunStatus::Queued => ItemStatus::Paused,
        RunStatus::Stopped | RunStatus::Cancelling => ItemStatus::Stopped,
        RunStatus::Failed => ItemStatus::Failed,
        RunStatus::Completed => ItemStatus::Blocked,
    };
    for item in &mut projection.items {
        if item.status == ItemStatus::InProgress {
            item.status = replacement;
        }
    }
    Some(projection)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj() -> PlanProjection {
        PlanProjection {
            items: vec![
                PlanItem {
                    key: "a".into(),
                    title: "A".into(),
                    status: ItemStatus::InProgress,
                    detail: None,
                },
                PlanItem {
                    key: "b".into(),
                    title: "B".into(),
                    status: ItemStatus::Completed,
                    detail: None,
                },
            ],
            summary: None,
        }
    }

    #[test]
    fn in_progress_only_survives_running() {
        let all = [
            RunStatus::Queued,
            RunStatus::Running,
            RunStatus::Waiting,
            RunStatus::Paused,
            RunStatus::Cancelling,
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Stopped,
        ];
        for status in all {
            let p = effective_projection(Some(&proj()), status).unwrap();
            let any_active = p.items.iter().any(|i| i.status == ItemStatus::InProgress);
            assert_eq!(any_active, status == RunStatus::Running, "{status:?}");
            assert_eq!(p.items[1].status, ItemStatus::Completed);
        }
        let stopped = effective_projection(Some(&proj()), RunStatus::Stopped).unwrap();
        assert_eq!(stopped.items[0].status, ItemStatus::Stopped);
        let done = effective_projection(Some(&proj()), RunStatus::Completed).unwrap();
        assert_eq!(done.items[0].status, ItemStatus::Blocked);
    }

    #[test]
    fn none_stays_none() {
        assert!(effective_projection(None, RunStatus::Stopped).is_none());
    }
}
