// SPDX-License-Identifier: BSL-1.1

//! SSE streaming for flow instance execution events.

use axum::{
    extract::Path,
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
pub async fn stream_flow_events(
    Path((repo, instance_id)): Path<(String, String)>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    tracing::debug!(
        repo = %repo,
        instance_id = %instance_id,
        "Client subscribed to flow events SSE"
    );

    let broadcaster = raisin_storage::jobs::global_flow_broadcaster();
    let mut receiver = broadcaster.subscribe(&instance_id);

    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
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

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
