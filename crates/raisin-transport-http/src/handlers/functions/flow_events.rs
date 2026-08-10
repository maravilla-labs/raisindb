// SPDX-License-Identifier: BSL-1.1

//! SSE streaming for flow instance execution events.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::Stream;
use std::convert::Infallible;
use std::time::Duration;

/// Stream flow execution events for a specific flow instance via SSE.
///
/// This endpoint streams real-time step-level events for flow visualization:
/// - `step_started`: A step has started execution
/// - `step_completed`: A step has completed successfully
/// - `step_failed`: A step has failed
/// - `flow_waiting`: Flow is waiting for external input
/// - `flow_completed`: Flow has completed successfully
/// - `flow_failed`: Flow has failed
///
/// The stream ends by itself once the flow reaches a terminal state — but a
/// flow parked on a human task can stay live for days, and an open SSE response
/// is an open connection that `axum::serve`'s graceful shutdown waits for. So
/// it also ends on the server shutdown signal.
pub async fn stream_flow_events(
    State(state): State<AppState>,
    Path((repo, instance_id)): Path<(String, String)>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let shutdown = state.shutdown_signal();
    tracing::debug!(
        repo = %repo,
        instance_id = %instance_id,
        "Client subscribed to flow events SSE"
    );

    let broadcaster = raisin_storage::jobs::global_flow_broadcaster();
    // Subscription, not a bare `subscribe`: this stream ends on a terminal
    // event, a lagging client, or shutdown, and the guard's Drop is what
    // returns the instance's broadcast channel — and the up-to-100 retained
    // step-output payloads in its ring — to the pool.
    let mut subscription =
        raisin_storage::jobs::FlowEventSubscription::new(broadcaster, &instance_id);

    let stream = async_stream::stream! {
        loop {
            match subscription.receiver().recv().await {
                Ok(event) => {
                    let event_type = event.event_type();

                    let data = serde_json::to_string(&event)
                        .unwrap_or_else(|_| "{}".to_string());

                    tracing::debug!(
                        instance_id = %instance_id,
                        event_type = %event_type,
                        data_len = data.len(),
                        "SSE yielding flow event"
                    );

                    yield Ok(Event::default()
                        .event("flow-event")
                        .data(data));

                    match &event {
                        raisin_storage::jobs::FlowEvent::FlowCompleted { .. }
                        | raisin_storage::jobs::FlowEvent::FlowFailed { .. } => {
                            tracing::debug!(
                                instance_id = %instance_id,
                                "Flow execution finished, closing SSE stream"
                            );
                            break;
                        }
                        _ => {}
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        instance_id = %instance_id,
                        lagged = n,
                        "SSE client lagged behind, some events were dropped"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::debug!(
                        instance_id = %instance_id,
                        "Flow event channel closed"
                    );
                    break;
                }
            }
        }
    };

    let stream = futures::StreamExt::take_until(stream, shutdown);

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
