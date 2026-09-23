// SPDX-License-Identifier: BSL-1.1

//! `GET /api/agent-runs/{repo}/{run}/stream` — the durable event log as SSE.
//!
//! Replay first, then live. The SSE id of every event is its `seq`, so a
//! client that reconnects with `Last-Event-ID` (or `?after_seq=`) resumes
//! exactly where it stopped: the log is the truth and the stream is only a
//! window on it, so a slow client can never lose an event.
//!
//! Live delivery is woken by the in-process commit notifier, and ALSO polled
//! every couple of seconds: a commit made on another cluster node reaches
//! this node through replication, which does not ring the local notifier.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Extension;
use futures::Stream;
use raisin_agent_runtime::api;
use raisin_agent_runtime::ids::RunId;
use raisin_models::auth::AuthContext;

use super::agent_runs::{caller, host, map_err, scope_of, BranchQuery};
use crate::error::ApiError;
use crate::middleware::TenantInfo;

const PAGE: usize = 500;
const POLL: Duration = Duration::from_secs(2);

/// Stream a run's events.
pub async fn stream_run(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    headers: HeaderMap,
    auth: Option<Extension<AuthContext>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    let host = host()?;
    let run = RunId(run);
    // Authorize once up front (a 403 must be an HTTP status, not a stream).
    api::get(&host, &scope, &run, &caller)
        .await
        .map_err(map_err)?;
    let from_header = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let mut last = from_header.or(q.after_seq).unwrap_or(0);
    let mut notify = host.service().subscribe(&run);

    let stream = async_stream::stream! {
        loop {
            let page = match api::events(&host, &scope, &run, &caller, last, PAGE).await {
                Ok(page) => page,
                Err(e) => {
                    yield Ok(Event::default().event("error").data(e.message));
                    break;
                }
            };
            let full = page.len() == PAGE;
            if page.is_empty() {
                // Resumed after the end: nothing will ever follow.
                let done = api::get(&host, &scope, &run, &caller)
                    .await
                    .map(|v| v.status.is_terminal() && v.run.domain.as_ref().is_none_or(|d| d.finalized))
                    .unwrap_or(true);
                if done {
                    yield Ok(Event::default().event("end").data("{}"));
                    break;
                }
            }
            let mut ended = false;
            for ev in page {
                last = ev.seq.0;
                ended |= matches!(ev.kind, raisin_agent_runtime::events::RunEventKind::Terminal { .. });
                let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                yield Ok(Event::default().id(ev.seq.0.to_string()).event("run-event").data(data));
            }
            if full {
                continue;
            }
            if ended {
                // The terminal event is out. A finalize may still append a
                // few domain events; give it one poll, then close.
                tokio::time::sleep(POLL).await;
                if let Ok(rest) = api::events(&host, &scope, &run, &caller, last, PAGE).await {
                    for ev in rest {
                        let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                        yield Ok(Event::default().id(ev.seq.0.to_string()).event("run-event").data(data));
                    }
                }
                yield Ok(Event::default().event("end").data("{}"));
                break;
            }
            tokio::select! {
                _ = notify.changed() => {}
                _ = tokio::time::sleep(POLL) => {}
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
