// SPDX-License-Identifier: BSL-1.1

//! SSE streaming for conversation events.
//!
//! # Who may listen
//!
//! Conversation events (streamed answer text, tool-call names and results)
//! travel on a process-wide broadcaster keyed by a bare channel string. This
//! endpoint used to hand that stream to ANY caller who named the channel:
//! no identity, no read check, and the `path` a client sent was ignored.
//! Knowing (or guessing, or reading from a log) `chat:<id>` was enough to
//! follow someone else's conversation.
//!
//! Now a caller names the CONVERSATION it is listening to (`path`, plus
//! `workspace`, default `raisin:access_control`) and must be able to READ that
//! `raisin:Conversation` node under its own row-level security. The channel
//! must be one that conversation actually streams on. Administrators and the
//! system (who may read every conversation anyway) may still subscribe by
//! channel alone.

use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    Extension, Json,
};
use futures::stream::Stream;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Duration;

/// Where conversations live unless the caller says otherwise.
const DEFAULT_CONVERSATION_WORKSPACE: &str = "raisin:access_control";

#[derive(Deserialize)]
pub struct ConversationEventsQuery {
    pub channel: String,
    /// Path of the `raisin:Conversation` node being followed.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Deserialize)]
pub struct ConversationEventsBody {
    pub channel: String,
    /// Path of the `raisin:Conversation` node being followed.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

/// Stream conversation events for a specific conversation via SSE (GET).
///
/// GET /api/conversations/{repo}/events?channel={stream_channel}&path={conversation path}
///
/// Streams real-time events from an AI conversation:
/// - `text_chunk`: Streaming text from AI response
/// - `thought_chunk`: AI thinking/reasoning text
/// - `tool_call_started`: A tool call has begun
/// - `tool_call_completed`: A tool call has finished
/// - `message_saved`: A message was persisted
/// - `done`: The conversation turn is complete (stream closes)
///
/// The caller must be able to read the conversation at `path`; see the module
/// docs.
pub async fn stream_conversation_events(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    tenant: Option<Extension<TenantInfo>>,
    auth: Option<Extension<AuthContext>>,
    Query(query): Query<ConversationEventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let target = StreamTarget {
        channel: query.channel,
        path: query.path,
        workspace: query.workspace,
        branch: query.branch,
    };
    authorize_request(&state, &repo, tenant, auth, &target).await?;
    Ok(stream_conversation_events_inner(repo, target.channel, state.shutdown_signal()).await)
}

/// Stream conversation events for a specific conversation via SSE (POST).
///
/// POST /api/conversations/{repo}/events
/// Body: { "channel": "...", "path": "...", "workspace"?: "..." }
///
/// Identical to the GET variant but avoids exposing the conversation path
/// in URL parameters / server logs.
pub async fn stream_conversation_events_post(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    tenant: Option<Extension<TenantInfo>>,
    auth: Option<Extension<AuthContext>>,
    Json(body): Json<ConversationEventsBody>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let target = StreamTarget {
        channel: body.channel,
        path: body.path,
        workspace: body.workspace,
        branch: body.branch,
    };
    authorize_request(&state, &repo, tenant, auth, &target).await?;
    Ok(stream_conversation_events_inner(repo, target.channel, state.shutdown_signal()).await)
}

struct StreamTarget {
    channel: String,
    path: Option<String>,
    workspace: Option<String>,
    branch: Option<String>,
}

/// Why a subscription was refused. Coarse on the wire: "not found" and "not
/// yours" answer the same, so the endpoint cannot be used to probe which
/// conversations exist.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StreamDenied {
    /// No `path` was given and the caller is not an administrator.
    PathRequired,
    /// The conversation does not exist, or the caller cannot read it.
    NotReadable,
    /// The node is not a conversation, or it does not stream on this channel.
    WrongChannel,
}

fn is_admin(auth: &AuthContext) -> bool {
    auth.is_system || auth.permissions().is_some_and(|p| p.is_system_admin)
}

/// The channels a conversation node streams on: its `stream_channel`, and the
/// `chat:<conversation_id>` form every producer falls back to.
fn channels_of(node: &Node) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(PropertyValue::String(c)) = node.properties.get("stream_channel") {
        if !c.is_empty() {
            out.push(c.clone());
        }
    }
    let id = match node.properties.get("conversation_id") {
        Some(PropertyValue::String(id)) if !id.is_empty() => id.clone(),
        _ => node.name.clone(),
    };
    if !id.is_empty() {
        out.push(format!("chat:{id}"));
    }
    out
}

/// The whole decision, free of I/O so it can be tested directly.
///
/// `conversation` is the node at the requested path as STORED (or `None` when
/// there is none); row-level security is applied here, with `auth`, so a node
/// the caller cannot read is treated exactly like a missing one.
pub(crate) fn authorize_conversation_stream(
    auth: Option<&AuthContext>,
    channel: &str,
    path_given: bool,
    conversation: Option<Node>,
    scope: &PermissionScope,
) -> Result<(), StreamDenied> {
    if auth.is_some_and(is_admin) {
        return Ok(());
    }
    if !path_given {
        return Err(StreamDenied::PathRequired);
    }
    // No auth context at all is not "internal" on a public endpoint: deny.
    let auth = auth.ok_or(StreamDenied::NotReadable)?;
    let node = conversation.ok_or(StreamDenied::NotReadable)?;
    let node = raisin_core::services::rls_filter::filter_node(node, auth, scope)
        .ok_or(StreamDenied::NotReadable)?;
    if node.node_type != "raisin:Conversation" {
        return Err(StreamDenied::WrongChannel);
    }
    if !channels_of(&node).iter().any(|c| c == channel) {
        return Err(StreamDenied::WrongChannel);
    }
    Ok(())
}

async fn authorize_request(
    state: &AppState,
    repo: &str,
    tenant: Option<Extension<TenantInfo>>,
    auth: Option<Extension<AuthContext>>,
    target: &StreamTarget,
) -> Result<(), ApiError> {
    use raisin_storage::{NodeRepository, Storage};

    let auth = auth.map(|Extension(a)| a);
    let tenant_id = tenant
        .map(|Extension(t)| t.tenant_id)
        .unwrap_or_else(|| "default".to_string());
    let workspace = target
        .workspace
        .as_deref()
        .filter(|w| !w.is_empty())
        .unwrap_or(DEFAULT_CONVERSATION_WORKSPACE);
    let branch = target
        .branch
        .as_deref()
        .filter(|b| !b.is_empty())
        .unwrap_or("main");
    let path = target.path.as_deref().filter(|p| p.starts_with('/'));

    let conversation = match (path, auth.as_ref().is_some_and(is_admin)) {
        (Some(path), false) => state
            .storage
            .nodes()
            .get_by_path(
                raisin_storage::scope::StorageScope::new(&tenant_id, repo, branch, workspace),
                path,
                None,
            )
            .await
            .ok()
            .flatten(),
        _ => None,
    };

    let scope = PermissionScope::new(workspace, branch);
    authorize_conversation_stream(
        auth.as_ref(),
        &target.channel,
        path.is_some(),
        conversation,
        &scope,
    )
    .map_err(|denied| {
        tracing::debug!(
            repo = %repo,
            channel = %target.channel,
            reason = ?denied,
            "Conversation event subscription refused"
        );
        match denied {
            StreamDenied::PathRequired => ApiError::new(
                StatusCode::BAD_REQUEST,
                "CONVERSATION_PATH_REQUIRED",
                "name the conversation (path) whose events you want to follow",
            ),
            StreamDenied::NotReadable | StreamDenied::WrongChannel => ApiError::new(
                StatusCode::FORBIDDEN,
                "FORBIDDEN",
                "not allowed to follow this conversation",
            ),
        }
    })
}

/// Shared SSE implementation for both GET and POST variants.
///
/// `shutdown` ends the stream when the server starts shutting down: the turn
/// normally ends it on `Done`, but a stalled model turn would otherwise leave an
/// open connection that `axum::serve`'s graceful shutdown waits on forever.
async fn stream_conversation_events_inner(
    repo: String,
    channel: String,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let subscription_key = channel;

    tracing::debug!(
        repo = %repo,
        subscription_key = %subscription_key,
        "Client subscribed to conversation events SSE"
    );

    let broadcaster = raisin_storage::jobs::global_conversation_broadcaster();
    // Subscription, not a bare `subscribe`: the guard's Drop returns the
    // channel when this stream ends. A conversation's ring retains its last
    // 100 events — streamed text chunks and tool-call result JSON — so a
    // released channel is the difference between a bounded map and one that
    // pins hundreds of KB per finished conversation forever.
    let mut subscription =
        raisin_storage::jobs::ConversationEventSubscription::new(broadcaster, &subscription_key);

    let stream = async_stream::stream! {
        loop {
            match subscription.receiver().recv().await {
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

    let stream = futures::StreamExt::take_until(stream, shutdown);

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};

    const WS: &str = "raisin:access_control";

    /// A logged-in user as `authenticated_user` shapes them: read anything
    /// under their own home, nothing else.
    fn user(id: &str, home: &str) -> AuthContext {
        let own_home = Permission::new("users/**", vec![Operation::Read])
            .with_workspace(WS)
            .with_condition("node.path.startsWith(auth.home)");
        let mut resolved = ResolvedPermissions::empty(id);
        resolved.permissions = vec![own_home];
        AuthContext::for_user(id)
            .with_home(home)
            .with_permissions(resolved)
    }

    fn anonymous() -> AuthContext {
        let public = Permission::new("**", vec![Operation::Read]).with_workspace("launchpad");
        AuthContext::anonymous_user("anon-node")
            .with_permissions(ResolvedPermissions::anonymous(vec![public]))
    }

    fn conversation(path: &str, id: &str) -> Node {
        let mut node = Node {
            id: format!("node-{id}"),
            name: id.to_string(),
            path: path.to_string(),
            node_type: "raisin:Conversation".to_string(),
            ..Default::default()
        };
        node.properties.insert(
            "conversation_id".into(),
            PropertyValue::String(id.to_string()),
        );
        node.properties.insert(
            "stream_channel".into(),
            PropertyValue::String(format!("chat:{id}")),
        );
        node
    }

    fn scope() -> PermissionScope {
        PermissionScope::new(WS, "main")
    }

    const ALICE_CHAT: &str = "/users/alice/inbox/chats/chat-1";

    #[test]
    fn the_owner_may_follow_their_conversation() {
        let alice = user("alice", "/users/alice");
        let res = authorize_conversation_stream(
            Some(&alice),
            "chat:chat-1",
            true,
            Some(conversation(ALICE_CHAT, "chat-1")),
            &scope(),
        );
        assert_eq!(res, Ok(()));
    }

    /// The leak this closes: another user who names the channel (and even
    /// the right path) no longer receives the stream.
    #[test]
    fn another_user_may_not_follow_it() {
        let bob = user("bob", "/users/bob");
        let res = authorize_conversation_stream(
            Some(&bob),
            "chat:chat-1",
            true,
            Some(conversation(ALICE_CHAT, "chat-1")),
            &scope(),
        );
        assert_eq!(res, Err(StreamDenied::NotReadable));
    }

    #[test]
    fn an_anonymous_caller_may_not_follow_it() {
        let res = authorize_conversation_stream(
            Some(&anonymous()),
            "chat:chat-1",
            true,
            Some(conversation(ALICE_CHAT, "chat-1")),
            &scope(),
        );
        assert_eq!(res, Err(StreamDenied::NotReadable));

        // ...and no auth context at all is no better.
        let res = authorize_conversation_stream(
            None,
            "chat:chat-1",
            true,
            Some(conversation(ALICE_CHAT, "chat-1")),
            &scope(),
        );
        assert_eq!(res, Err(StreamDenied::NotReadable));
    }

    /// Owning ONE conversation does not open every channel: the channel must
    /// be the one the named conversation streams on.
    #[test]
    fn a_readable_conversation_does_not_unlock_another_channel() {
        let alice = user("alice", "/users/alice");
        let res = authorize_conversation_stream(
            Some(&alice),
            "chat:someone-elses",
            true,
            Some(conversation(ALICE_CHAT, "chat-1")),
            &scope(),
        );
        assert_eq!(res, Err(StreamDenied::WrongChannel));
    }

    #[test]
    fn a_node_that_is_not_a_conversation_is_refused() {
        let alice = user("alice", "/users/alice");
        let mut node = conversation("/users/alice/profile", "profile");
        node.node_type = "raisin:Profile".into();
        let res =
            authorize_conversation_stream(Some(&alice), "chat:profile", true, Some(node), &scope());
        assert_eq!(res, Err(StreamDenied::WrongChannel));
    }

    #[test]
    fn a_missing_conversation_is_refused() {
        let alice = user("alice", "/users/alice");
        let res = authorize_conversation_stream(Some(&alice), "chat:x", true, None, &scope());
        assert_eq!(res, Err(StreamDenied::NotReadable));
    }

    #[test]
    fn a_non_admin_must_name_the_conversation() {
        let alice = user("alice", "/users/alice");
        let res = authorize_conversation_stream(Some(&alice), "chat:chat-1", false, None, &scope());
        assert_eq!(res, Err(StreamDenied::PathRequired));
    }

    /// The admin console's agent test chat subscribes by channel alone; the
    /// system may read every conversation anyway.
    #[test]
    fn the_system_may_subscribe_by_channel_alone() {
        let res = authorize_conversation_stream(
            Some(&AuthContext::system()),
            "chat:chat-1",
            false,
            None,
            &scope(),
        );
        assert_eq!(res, Ok(()));
    }
}
