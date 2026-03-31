// SPDX-License-Identifier: BSL-1.1

//! SSE streaming for conversation events.

use axum::{
    extract::{Path, Query},
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures::stream::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Duration;

#[derive(Deserialize)]
pub struct ConversationEventsQuery {
    pub channel: String,
}

#[derive(Deserialize)]
pub struct ConversationEventsBody {
    pub channel: String,
}

/// Stream conversation events for a specific conversation via SSE (GET).
///
/// GET /api/conversations/{repo}/events?channel={stream_channel}
///
/// Streams real-time events from an AI conversation:
/// - `text_chunk`: Streaming text from AI response
/// - `thought_chunk`: AI thinking/reasoning text
/// - `tool_call_started`: A tool call has begun
/// - `tool_call_completed`: A tool call has finished
/// - `message_saved`: A message was persisted
/// - `done`: The conversation turn is complete (stream closes)
pub async fn stream_conversation_events(
    Path(repo): Path<String>,
    Query(query): Query<ConversationEventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    stream_conversation_events_inner(repo, query.channel).await
}

/// Stream conversation events for a specific conversation via SSE (POST).
///
/// POST /api/conversations/{repo}/events
/// Body: { "channel": "..." }
///
/// Identical to the GET variant but avoids exposing the conversation path
/// in URL parameters / server logs.
pub async fn stream_conversation_events_post(
    Path(repo): Path<String>,
    Json(body): Json<ConversationEventsBody>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    stream_conversation_events_inner(repo, body.channel).await
}

/// Shared SSE implementation for both GET and POST variants.
async fn stream_conversation_events_inner(
    repo: String,
    channel: String,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let subscription_key = channel;

    tracing::debug!(
        repo = %repo,
        subscription_key = %subscription_key,
        "Client subscribed to conversation events SSE"
    );

    let broadcaster = raisin_storage::jobs::global_conversation_broadcaster();
    let mut receiver = broadcaster.subscribe(&subscription_key);

    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let event_type = event.event_type();

                    let data = serde_json::to_string(&event)
                        .unwrap_or_else(|_| "{}".to_string());

                    tracing::debug!(
                        subscription_key = %subscription_key,
                        event_type = %event_type,
                        data_len = data.len(),
                        "SSE yielding conversation event"
                    );

                    yield Ok(Event::default()
                        .event("conversation-event")
                        .data(data));

                    if matches!(&event, raisin_storage::jobs::ConversationEvent::Done { .. }) {
                        tracing::debug!(
                            subscription_key = %subscription_key,
                            "Conversation turn done, closing SSE stream"
                        );
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        subscription_key = %subscription_key,
                        lagged = n,
                        "SSE client lagged behind, some events were dropped"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::debug!(
                        subscription_key = %subscription_key,
                        "Conversation event channel closed"
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
