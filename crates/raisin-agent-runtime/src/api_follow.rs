// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Finding a run by what it is about, and following a run's log live — the
//! two reads a UI needs to attach to "the run of this conversation" and stay
//! on it. Transport-neutral: the WebSocket channel and HTTP both use them.

use std::time::Duration;

use serde::Serialize;

use crate::api::{may_access, ApiError, Caller, RunView, R};
use crate::events::{RunEvent, RunEventKind};
use crate::host::AgentRunHost;
use crate::ids::{RunId, RunScope, Seq, SubjectRef};
use crate::service::ServiceError;

/// Page size of a follow.
const PAGE: usize = 500;
/// How often a follower re-reads when no local commit told it to: a commit
/// on ANOTHER node reaches this one by replication, not by the in-process
/// notification.
pub const FOLLOW_POLL: Duration = Duration::from_secs(2);

/// Every run about any of `subjects` that the caller may see, NEWEST first
/// (the live one, if any, is the first). A subject is keyed by its node id when
/// it has one, else its path; pass both spellings to find runs created either
/// way.
pub async fn by_subject(
    host: &AgentRunHost,
    scope: &RunScope,
    caller: &Caller,
    subjects: &[SubjectRef],
    limit: usize,
) -> R<Vec<RunView>> {
    let store = host.service().store();
    let mut ids: Vec<RunId> = Vec::new();
    for s in subjects {
        let key = s
            .key()
            .map_err(|e| ApiError::new(400, "invalid_subject", e.to_string()))?;
        for id in store
            .scan_subject(scope, &key, 10_000)
            .await
            .map_err(ServiceError::from)?
        {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let mut out = Vec::new();
    for id in ids {
        if let Some(mut rec) = store.load(scope, &id).await.map_err(ServiceError::from)? {
            if may_access(&rec, caller) {
                rec.control_capability_hash = None;
                out.push(RunView {
                    status: rec.state.status(),
                    run: rec,
                    projection: None,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        (!a.status.is_terminal(), a.run.created_at_ms)
            .cmp(&(!b.status.is_terminal(), b.run.created_at_ms))
            .reverse()
    });
    out.truncate(limit);
    Ok(out)
}

/// What a follower receives.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FollowItem {
    /// One durable event, in seq order.
    Event(RunEvent),
    /// The run ended and its log is complete: nothing will follow.
    End {
        /// The last seq sent.
        last_seq: u64,
    },
}

/// Whether nothing more will ever be appended to the run's log.
async fn settled(host: &AgentRunHost, scope: &RunScope, run: &RunId, caller: &Caller) -> bool {
    crate::api::get(host, scope, run, caller)
        .await
        .map(|v| v.status.is_terminal() && v.run.domain.as_ref().is_none_or(|d| d.finalized))
        .unwrap_or(true)
}

/// Replay a run's events after `after_seq`, then follow it live until it
/// ends, `emit` answers `false` (the receiver is gone), or `stop` fires.
///
/// Gap-free and resumable: every event is read from the durable log, the
/// in-process notification and a [`FOLLOW_POLL`] are only reasons to reread.
pub async fn follow<F>(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    after_seq: u64,
    mut stop: tokio::sync::oneshot::Receiver<()>,
    mut emit: F,
) -> R<()>
where
    F: FnMut(FollowItem) -> bool,
{
    crate::api::load_for(host, scope, run, caller).await?;
    let mut notify = host.service().subscribe(run);
    let mut last = after_seq;
    let mut ended = false;
    loop {
        let page = host
            .service()
            .read_events(scope, run, Seq(last), PAGE)
            .await
            .map_err(ApiError::from)?;
        let full = page.len() == PAGE;
        for ev in page {
            last = ev.seq.0;
            ended |= matches!(ev.kind, RunEventKind::Terminal { .. });
            if !emit(FollowItem::Event(ev)) {
                return Ok(());
            }
        }
        if full {
            continue;
        }
        if (ended || last == after_seq) && settled(host, scope, run, caller).await {
            // Once more: a finalize may have appended in between.
            let rest = host
                .service()
                .read_events(scope, run, Seq(last), PAGE)
                .await
                .map_err(ApiError::from)?;
            for ev in rest {
                last = ev.seq.0;
                if !emit(FollowItem::Event(ev)) {
                    return Ok(());
                }
            }
            emit(FollowItem::End { last_seq: last });
            return Ok(());
        }
        tokio::select! {
            _ = notify.changed() => {}
            _ = tokio::time::sleep(FOLLOW_POLL) => {}
            _ = &mut stop => return Ok(()),
        }
    }
}
