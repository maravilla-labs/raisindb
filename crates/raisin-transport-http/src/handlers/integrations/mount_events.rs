// SPDX-License-Identifier: BSL-1.1

//! Live sync progress for one virtual mount, over SSE.
//!
//! The console's mount page used to get this from a WebSocket subscription to
//! `node:updated` on the mount's config node. Two problems with that, both
//! fixed here:
//!
//! - **It carried no information.** A node event says "something changed"; the
//!   page then re-fetched and diffed to work out what. This stream carries the
//!   counts, the phase and the queue depth directly.
//! - **A WebSocket cannot send an `Authorization` header.** That page was the
//!   only WS client in the console and the only one with an auth problem. SSE
//!   over `fetch` carries the ordinary bearer token, so it authenticates exactly
//!   like every other request the console makes.
//!
//! Mirrors [`flow_events`](crate::handlers::functions::flow_events), which
//! already proved the shape: one broadcaster, whichever transports want it.

use std::convert::Infallible;

use axum::{
    extract::{Path, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::Stream;

use crate::state::AppState;

/// `GET /api/integrations/{repo}/mounts/{mount_id}/events`
///
/// A sync is idle most of the time, so the stream is mostly silence — hence the
/// keep-alive, which also stops intermediaries closing an idle connection.
///
/// It ends on the server shutdown signal. An open SSE response is an open
/// connection that `axum::serve`'s graceful drain waits for, and a mount stream
/// left to its own devices would hold shutdown open indefinitely.
pub async fn stream_mount_events(
    State(state): State<AppState>,
    Path((repo, mount_id)): Path<(String, String)>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let shutdown = state.shutdown_signal();
    tracing::debug!(%repo, %mount_id, "client subscribed to mount sync events");

    let broadcaster = raisin_storage::jobs::global_mount_broadcaster();
    let mut receiver = broadcaster.subscribe(&mount_id);

    let stream = async_stream::stream! {
        // Released when this generator is dropped — which happens whether the
        // client disconnects, the server shuts down, or the loop exits. Doing it
        // at the end of the loop instead would leak on the common case: a page
        // the operator simply navigated away from never runs to completion.
        let _guard = ChannelGuard(mount_id.clone());

        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
                    yield Ok(Event::default().event("mount-sync").data(data));
                }
                // Lagging is harmless and must NOT close the stream. Every
                // payload is cumulative, so the next event carries the current
                // totals; a subscriber that missed ten is no worse off than one
                // that missed one.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::debug!(mount_id = %mount_id, missed, "mount SSE lagged; continuing");
                    continue;
                }
                // The channel went away. Do NOT break: a mount that is not
                // currently syncing has no channel, and the page must stay
                // connected so it sees the next run begin.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    receiver =
                        raisin_storage::jobs::global_mount_broadcaster().subscribe(&mount_id);
                    continue;
                }
            }
        }
    };

    Sse::new(futures::StreamExt::take_until(stream, shutdown)).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// Releases a mount's broadcast channel when the stream that used it goes away.
///
/// Without this the broadcaster's map grows by one entry per mount ever viewed
/// and never shrinks. Small, but unbounded, and this process runs for months.
struct ChannelGuard(String);

impl Drop for ChannelGuard {
    fn drop(&mut self) {
        raisin_storage::jobs::global_mount_broadcaster().release_if_idle(&self.0);
    }
}
