// SPDX-License-Identifier: BSL-1.1

//! Anonymous visitor chat: `visitor_chat_start` and `visitor_chat_send`.
//!
//! See [`crate::visitor`] for the model. In short:
//!
//! * `visitor_chat_start {agent, session_token?, conversation_id?}` checks
//!   that the connection is anonymous, that the agent allows visitors (and
//!   this page's origin), binds the connection to a visitor session (a new
//!   one, or the one `session_token` proves), picks or mints a conversation,
//!   and starts forwarding that conversation's events to THIS connection.
//!   Nothing is written yet: the session home appears with the first message.
//! * `visitor_chat_send {conversation_id, text}` applies the limits (message
//!   size, per-session and per-IP rate, conversation length, the token budget,
//!   one turn at a time) and then writes the message into the session's outbox
//!   AS THE SYSTEM. From there the ordinary pipeline delivers it to the agent
//!   and the reply back into the session's conversation.
//!
//! A visitor never writes a node itself (row-level security refuses it), so
//! these checks cannot be bypassed by writing an outbox node directly.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use raisin_core::NodeService;
use raisin_models::auth::visitor::{
    visitor_home_path, visitor_participant_id, VISITOR_ROOT, VISITOR_SESSION_NODE_TYPE,
    VISITOR_WORKSPACE,
};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use serde::Deserialize;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{EventMessage, RequestEnvelope, ResponseEnvelope},
    visitor::{session, AnonymousChatConfig, VisitorBinding},
};

/// Visitor chats run on the main branch, where agents and triggers live.
const BRANCH: &str = "main";
const AGENT_WORKSPACE: &str = "functions";
const MINUTE: Duration = Duration::from_secs(60);

#[derive(Debug, Deserialize)]
struct StartPayload {
    /// `/agents/<name>` or `agent:<name>`.
    agent: String,
    /// The secret a previous start returned, to resume that session.
    #[serde(default)]
    session_token: Option<String>,
    /// A conversation of that session to continue.
    #[serde(default)]
    conversation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SendPayload {
    conversation_id: String,
    text: String,
    /// Optional client-side id, echoed back (idempotency is the caller's).
    #[serde(default)]
    client_id: Option<String>,
}

fn refuse(request_id: String, code: &str, message: impl Into<String>) -> ResponseEnvelope {
    ResponseEnvelope::error(request_id, code.to_string(), message.into())
}

/// `/agents/<name>` for `/agents/<name>` or `agent:<name>`; `None` otherwise.
pub(crate) fn agent_path_of(agent: &str) -> Option<String> {
    let name = agent
        .strip_prefix("agent:")
        .or_else(|| agent.strip_prefix("/agents/"))?;
    let ok = !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        && !name.starts_with('.');
    ok.then(|| format!("/agents/{name}"))
}

fn node_service<S, B>(
    state: &Arc<WsState<S, B>>,
    tenant: &str,
    repo: &str,
    workspace: &str,
) -> NodeService<S>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    NodeService::new_with_context(
        state.storage.clone(),
        tenant.to_string(),
        repo.to_string(),
        BRANCH.to_string(),
        workspace.to_string(),
    )
    .with_auth(AuthContext::system_as("visitor-chat"))
}

async fn load_node<S: Storage>(
    storage: &Arc<S>,
    tenant: &str,
    repo: &str,
    workspace: &str,
    path: &str,
) -> Option<Node> {
    storage
        .nodes()
        .get_by_path(
            StorageScope::new(tenant, repo, BRANCH, workspace),
            path,
            None,
        )
        .await
        .ok()
        .flatten()
}

/// The agent's anonymous settings; `Err` with the refusal to send.
async fn agent_config<S: Storage>(
    storage: &Arc<S>,
    tenant: &str,
    repo: &str,
    agent_path: &str,
    origin: Option<&str>,
) -> Result<AnonymousChatConfig, (&'static str, String)> {
    let agent = load_node(storage, tenant, repo, AGENT_WORKSPACE, agent_path)
        .await
        .filter(|n| n.node_type == "raisin:AIAgent");
    // "Not found" and "not allowed" answer alike: an anonymous caller learns
    // nothing about which agents exist.
    let cfg = agent
        .and_then(|a| AnonymousChatConfig::from_agent_properties(&a.properties))
        .ok_or((
            "ANONYMOUS_NOT_ALLOWED",
            "this agent does not accept anonymous visitors".to_string(),
        ))?;
    if !cfg.origin_allowed(origin) {
        return Err((
            "ORIGIN_NOT_ALLOWED",
            "this page's origin may not chat with this agent".to_string(),
        ));
    }
    Ok(cfg)
}

fn request_scope(
    request: &RequestEnvelope,
    conn: &ConnectionState,
) -> Result<(String, String), WsError> {
    let repo = request
        .context
        .repository
        .clone()
        .or_else(|| conn.repository.clone())
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    Ok((conn.tenant_id.clone(), repo))
}

fn is_anonymous(conn: &ConnectionState) -> bool {
    conn.is_anonymous()
        && conn
            .auth_context()
            .is_some_and(|a| a.is_anonymous_principal())
}

/// `visitor_chat_start`
pub async fn handle_visitor_chat_start<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let id = request.request_id.clone();
    let payload: StartPayload = serde_json::from_value(request.payload.clone())?;

    let (tenant, repo, origin, ip, anonymous, bound) = {
        let conn = connection_state.read();
        let (tenant, repo) = request_scope(&request, &conn)?;
        (
            tenant,
            repo,
            conn.origin().map(str::to_string),
            conn.client_ip().map(str::to_string),
            is_anonymous(&conn),
            conn.visitor().cloned(),
        )
    };
    if !anonymous {
        return Ok(Some(refuse(
            id,
            "NOT_ANONYMOUS",
            "signed-in users chat through their own conversations (conversations.create)",
        )));
    }
    let Some(agent_path) = agent_path_of(payload.agent.trim()) else {
        return Ok(Some(refuse(
            id,
            "INVALID_AGENT",
            "agent must be /agents/<name>",
        )));
    };
    let cfg = match agent_config(
        &state.storage,
        &tenant,
        &repo,
        &agent_path,
        origin.as_deref(),
    )
    .await
    {
        Ok(cfg) => cfg,
        Err((code, msg)) => return Ok(Some(refuse(id, code, msg))),
    };

    let limits = &state.visitor_limits;
    let now = Instant::now();
    let ip_key = format!("ip:{tenant}:{}", ip.as_deref().unwrap_or("unattributed"));
    if !limits.hit(&ip_key, cfg.rate_per_ip_per_minute, MINUTE, now) {
        return Ok(Some(refuse(
            id,
            "RATE_LIMITED",
            "too many requests, try again shortly",
        )));
    }

    // The session: the one this connection is already bound to, the one the
    // presented secret proves, or a new one.
    let presented = payload
        .session_token
        .as_deref()
        .and_then(|t| session::key_of(t).map(|k| (t.to_string(), k)));
    let (secret, key) = match (&bound, presented) {
        (Some(b), Some((secret, key))) if b.key == key => (Some(secret), key),
        (Some(b), None) => (None, b.key.clone()),
        (Some(_), Some(_)) => {
            return Ok(Some(refuse(
                id,
                "SESSION_MISMATCH",
                "this connection already belongs to another visitor session",
            )))
        }
        (None, Some((secret, key))) => (Some(secret), key),
        (None, None) => {
            let secret = session::new_secret();
            let key = session::key_of(&secret).expect("a minted secret is well formed");
            (Some(secret), key)
        }
    };
    let home = visitor_home_path(&key);

    // A resumed session keeps its agent; an expired one is gone.
    let home_node = load_node(&state.storage, &tenant, &repo, VISITOR_WORKSPACE, &home).await;
    if let Some(node) = &home_node {
        if let Some(PropertyValue::String(bound_agent)) = node.properties.get("agent_path") {
            if *bound_agent != agent_path {
                return Ok(Some(refuse(
                    id,
                    "SESSION_MISMATCH",
                    "this visitor session belongs to another agent",
                )));
            }
        }
        if expired(node) {
            return Ok(Some(refuse(
                id,
                "SESSION_EXPIRED",
                "this visitor session has expired",
            )));
        }
    }

    // The conversation: one of this session's, or a new one.
    let chats = format!("{home}/inbox/chats");
    let requested = payload
        .conversation_id
        .as_deref()
        .filter(|c| session::is_conversation_id(c));
    let known_here = bound
        .as_ref()
        .map(|b| b.conversations.clone())
        .unwrap_or_default();
    let conversation_id = match requested {
        Some(c)
            if known_here.contains(c)
                || load_node(
                    &state.storage,
                    &tenant,
                    &repo,
                    VISITOR_WORKSPACE,
                    &format!("{chats}/{c}"),
                )
                .await
                .is_some() =>
        {
            c.to_string()
        }
        Some(_) => {
            return Ok(Some(refuse(
                id,
                "UNKNOWN_CONVERSATION",
                "no such conversation in this visitor session",
            )))
        }
        None => {
            let existing = match home_node {
                Some(_) => node_service(state, &tenant, &repo, VISITOR_WORKSPACE)
                    .list_children(&chats)
                    .await
                    .map(|c| c.len())
                    .unwrap_or(0),
                None => 0,
            };
            if existing + known_here.len() >= cfg.max_conversations as usize {
                return Ok(Some(refuse(
                    id,
                    "CONVERSATION_LIMIT",
                    "this visitor session has reached its conversation limit",
                )));
            }
            session::new_conversation_id()
        }
    };

    // Bind the connection: its auth context now reads this home (and only it).
    let start_forwarding = {
        let mut conn = connection_state.write();
        let mut binding = conn.visitor().cloned().unwrap_or_else(|| VisitorBinding {
            key: key.clone(),
            home: home.clone(),
            agent_path: agent_path.clone(),
            conversations: HashSet::new(),
            forwarding: HashSet::new(),
        });
        binding.conversations.insert(conversation_id.clone());
        let start = binding.forwarding.insert(conversation_id.clone());
        conn.bind_visitor(binding);
        start
    };
    let subscription_id = format!("visitor-chat:{conversation_id}");
    if start_forwarding {
        spawn_forwarder(
            Arc::clone(&state.visitor_limits),
            connection_state,
            key.clone(),
            conversation_id.clone(),
            subscription_id.clone(),
        );
    }

    Ok(Some(ResponseEnvelope::success(
        id,
        serde_json::json!({
            "session_token": secret,
            "session_home": home,
            "conversation_id": conversation_id,
            "conversation_path": format!("{chats}/{conversation_id}"),
            "conversation_workspace": VISITOR_WORKSPACE,
            "channel": format!("chat:{conversation_id}"),
            "subscription_id": subscription_id,
            "agent": agent_path,
            "limits": {
                "max_messages": cfg.max_messages,
                "max_message_chars": cfg.max_message_chars,
            },
        }),
    )))
}

fn expired(home: &Node) -> bool {
    match home.properties.get("expires_at") {
        Some(PropertyValue::String(s)) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|at| at <= chrono::Utc::now())
            .unwrap_or(false),
        Some(PropertyValue::Date(d)) => d.to_string() <= chrono::Utc::now().to_rfc3339(),
        _ => false,
    }
}

/// `visitor_chat_send`
pub async fn handle_visitor_chat_send<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let id = request.request_id.clone();
    let payload: SendPayload = serde_json::from_value(request.payload.clone())?;

    let (tenant, repo, origin, ip, binding) = {
        let conn = connection_state.read();
        let (tenant, repo) = request_scope(&request, &conn)?;
        (
            tenant,
            repo,
            conn.origin().map(str::to_string),
            conn.client_ip().map(str::to_string),
            conn.visitor().cloned(),
        )
    };
    let Some(binding) = binding else {
        return Ok(Some(refuse(id, "NO_SESSION", "start a visitor chat first")));
    };
    if !binding.conversations.contains(&payload.conversation_id) {
        return Ok(Some(refuse(
            id,
            "UNKNOWN_CONVERSATION",
            "no such conversation in this visitor session",
        )));
    }
    // Re-read the agent: turning the flag off takes effect on the next message.
    let cfg = match agent_config(
        &state.storage,
        &tenant,
        &repo,
        &binding.agent_path,
        origin.as_deref(),
    )
    .await
    {
        Ok(cfg) => cfg,
        Err((code, msg)) => return Ok(Some(refuse(id, code, msg))),
    };

    let text = payload.text.trim();
    if let Err((code, msg)) = check_text(&cfg, text) {
        return Ok(Some(refuse(id, code, msg)));
    }

    let limits = &state.visitor_limits;
    let now = Instant::now();
    let session_key = format!("session:{tenant}:{}", binding.key);
    let ip_key = format!("ip:{tenant}:{}", ip.as_deref().unwrap_or("unattributed"));
    if !limits.hit(&session_key, cfg.rate_per_session_per_minute, MINUTE, now)
        || !limits.hit(&ip_key, cfg.rate_per_ip_per_minute, MINUTE, now)
    {
        return Ok(Some(refuse(
            id,
            "RATE_LIMITED",
            "too many messages, try again shortly",
        )));
    }

    // One writer per session: the counters live on the home node.
    let lock = limits.session_lock(&session_key);
    let _guard = lock.lock().await;

    let agent_name = binding
        .agent_path
        .trim_start_matches("/agents/")
        .to_string();
    let conversation_id = payload.conversation_id.clone();

    // The token budget, measured where the pipeline records it: the agent's
    // copy of the conversation.
    let agent_chat = load_node(
        &state.storage,
        &tenant,
        &repo,
        "ai",
        &format!("/agents/{agent_name}/inbox/chats/{conversation_id}"),
    )
    .await;
    let used = agent_chat
        .as_ref()
        .and_then(|n| match n.properties.get("total_tokens_used") {
            Some(PropertyValue::Integer(i)) => Some(*i as f64),
            Some(PropertyValue::Float(f)) => Some(*f),
            _ => None,
        })
        .unwrap_or(0.0);

    let home_node = load_node(
        &state.storage,
        &tenant,
        &repo,
        VISITOR_WORKSPACE,
        &binding.home,
    )
    .await;
    if home_node.as_ref().is_some_and(expired) {
        return Ok(Some(refuse(
            id,
            "SESSION_EXPIRED",
            "this visitor session has expired",
        )));
    }
    let mut counts = message_counts(home_node.as_ref());
    let sent = counts.get(&conversation_id).copied().unwrap_or(0);
    if let Err((code, msg)) = check_budget(&cfg, used, sent) {
        return Ok(Some(refuse(id, code, msg)));
    }

    if !limits.acquire_turn(
        &session_key,
        &conversation_id,
        cfg.max_concurrent_turns,
        cfg.turn_lease,
        now,
    ) {
        return Ok(Some(refuse(id, "BUSY", "the assistant is still answering")));
    }

    counts.insert(conversation_id.clone(), sent + 1);
    let written = write_message(
        state,
        &tenant,
        &repo,
        &binding,
        home_node,
        &cfg,
        origin.as_deref(),
        &agent_name,
        &conversation_id,
        text,
        counts,
    )
    .await;
    let message_id = match written {
        Ok(message_id) => message_id,
        Err(e) => {
            limits.release_turn(&session_key, &conversation_id);
            tracing::warn!(error = %e, "visitor chat: could not write the message");
            return Ok(Some(refuse(
                id,
                "UNAVAILABLE",
                "the message could not be sent",
            )));
        }
    };

    Ok(Some(ResponseEnvelope::success(
        id,
        serde_json::json!({
            "accepted": true,
            "message_id": message_id,
            "client_id": payload.client_id,
            "conversation_id": conversation_id,
            "messages_left": (cfg.max_messages as i64 - (sent + 1)).max(0),
        }),
    )))
}

/// A visitor message's own admissibility: not empty, not too long.
pub(crate) fn check_text(
    cfg: &AnonymousChatConfig,
    text: &str,
) -> Result<(), (&'static str, String)> {
    if text.trim().is_empty() {
        return Err(("EMPTY_MESSAGE", "the message is empty".to_string()));
    }
    if text.chars().count() > cfg.max_message_chars {
        return Err((
            "TOO_LONG",
            format!(
                "messages are limited to {} characters",
                cfg.max_message_chars
            ),
        ));
    }
    Ok(())
}

/// Whether a conversation may take another visitor message: its token budget
/// (`used`, the agent side's `total_tokens_used`) and its length (`sent`
/// visitor messages so far).
pub(crate) fn check_budget(
    cfg: &AnonymousChatConfig,
    used: f64,
    sent: i64,
) -> Result<(), (&'static str, String)> {
    if used >= cfg.max_conversation_tokens as f64 {
        return Err((
            "LIMIT_REACHED",
            "this conversation has used its budget; start a new one".to_string(),
        ));
    }
    if sent >= cfg.max_messages as i64 {
        return Err((
            "TOO_MANY_MESSAGES",
            "this conversation has reached its length limit; start a new one".to_string(),
        ));
    }
    Ok(())
}

fn message_counts(home: Option<&Node>) -> HashMap<String, i64> {
    match home.and_then(|h| h.properties.get("message_counts")) {
        Some(PropertyValue::Object(map)) => map
            .iter()
            .filter_map(|(k, v)| match v {
                PropertyValue::Integer(i) => Some((k.clone(), *i)),
                PropertyValue::Float(f) => Some((k.clone(), *f as i64)),
                _ => None,
            })
            .collect(),
        _ => HashMap::new(),
    }
}

fn new_node(name: &str, node_type: &str, props: HashMap<String, PropertyValue>) -> Node {
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        properties: props,
        workspace: Some(VISITOR_WORKSPACE.to_string()),
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        version: 1,
        ..Default::default()
    }
}

fn s(v: impl Into<String>) -> PropertyValue {
    PropertyValue::String(v.into())
}

/// Create the session home on first use, move its expiry forward, and write
/// the message into its outbox — all as the system.
#[allow(clippy::too_many_arguments)]
async fn write_message<S, B>(
    state: &Arc<WsState<S, B>>,
    tenant: &str,
    repo: &str,
    binding: &VisitorBinding,
    home_node: Option<Node>,
    cfg: &AnonymousChatConfig,
    origin: Option<&str>,
    agent_name: &str,
    conversation_id: &str,
    text: &str,
    counts: HashMap<String, i64>,
) -> Result<String, raisin_error::Error>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let svc = node_service(state, tenant, repo, VISITOR_WORKSPACE);
    let now = chrono::Utc::now();
    let ttl =
        chrono::Duration::from_std(cfg.session_ttl).unwrap_or_else(|_| chrono::Duration::hours(24));
    let expires_at = (now + ttl).to_rfc3339();
    let counts = PropertyValue::Object(
        counts
            .into_iter()
            .map(|(k, v)| (k, PropertyValue::Integer(v)))
            .collect(),
    );

    if home_node.is_none() {
        if svc.get_by_path(VISITOR_ROOT).await?.is_none() {
            let root = new_node(
                VISITOR_ROOT.trim_start_matches('/'),
                "raisin:AclFolder",
                HashMap::from([("title".to_string(), s("Visitors"))]),
            );
            if let Err(e) = svc.add_node("/", root).await {
                // Two first messages at once: the other one created it.
                if svc.get_by_path(VISITOR_ROOT).await?.is_none() {
                    return Err(e);
                }
            }
        }
        let mut props = HashMap::from([
            ("agent_path".to_string(), s(binding.agent_path.clone())),
            ("display_name".to_string(), s("Visitor")),
            ("created_at".to_string(), s(now.to_rfc3339())),
            ("last_activity_at".to_string(), s(now.to_rfc3339())),
            ("expires_at".to_string(), s(expires_at.clone())),
            ("message_counts".to_string(), counts.clone()),
        ]);
        if let Some(o) = origin {
            props.insert("origin".to_string(), s(o));
        }
        svc.add_node(
            VISITOR_ROOT,
            new_node(&binding.key, VISITOR_SESSION_NODE_TYPE, props),
        )
        .await?;
        svc.add_node(
            &binding.home,
            new_node(
                "outbox",
                "raisin:Folder",
                HashMap::from([("title".to_string(), s("Outbox"))]),
            ),
        )
        .await?;
    } else {
        svc.update_property_by_path(&binding.home, "last_activity_at", s(now.to_rfc3339()))
            .await?;
        svc.update_property_by_path(&binding.home, "expires_at", s(expires_at))
            .await?;
        svc.update_property_by_path(&binding.home, "message_counts", counts)
            .await?;
        let outbox = format!("{}/outbox", binding.home);
        if svc.get_by_path(&outbox).await?.is_none() {
            svc.add_node(
                &binding.home,
                new_node(
                    "outbox",
                    "raisin:Folder",
                    HashMap::from([("title".to_string(), s("Outbox"))]),
                ),
            )
            .await?;
        }
    }

    // The same shape a signed-in user's client writes: the pipeline
    // (process-chat → handle-chat) takes it from here.
    let message_id = uuid::Uuid::new_v4().to_string();
    let sender_id = visitor_participant_id(&binding.key);
    let body = PropertyValue::Object(HashMap::from([
        ("content".to_string(), s(text)),
        ("message_text".to_string(), s(text)),
        ("thread_id".to_string(), s(conversation_id)),
    ]));
    let props = HashMap::from([
        ("role".to_string(), s("user")),
        ("message_type".to_string(), s("chat")),
        ("status".to_string(), s("pending")),
        ("sender_id".to_string(), s(sender_id)),
        ("sender_path".to_string(), s(binding.home.clone())),
        ("recipient_id".to_string(), s(format!("agent:{agent_name}"))),
        ("subject".to_string(), s("Chat")),
        ("body".to_string(), body),
        ("conversation_id".to_string(), s(conversation_id)),
        ("client_id".to_string(), s(message_id.clone())),
        ("created_at".to_string(), s(now.to_rfc3339())),
    ]);
    svc.add_node(
        &format!("{}/outbox", binding.home),
        new_node(&format!("msg-{message_id}"), "raisin:Message", props),
    )
    .await?;
    Ok(message_id)
}

/// Stream one conversation's events to the ONE connection that owns it.
///
/// The conversation id is server-minted and bound to this connection's
/// session; no other connection starts a forwarder for it, so a reply cannot
/// reach anyone else. A `done` (or `waiting`) frees the session's turn slot.
pub(crate) fn spawn_forwarder(
    limits: Arc<crate::visitor::VisitorLimits>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    session_key: String,
    conversation_id: String,
    subscription_id: String,
) -> tokio::task::JoinHandle<()> {
    let conn = Arc::clone(connection_state);
    let (tenant, cancel) = {
        let c = connection_state.read();
        (c.tenant_id.clone(), c.visitor_cancel_token())
    };
    let lease_key = format!("session:{tenant}:{session_key}");
    let channel = format!("chat:{conversation_id}");

    tokio::spawn(async move {
        let broadcaster = raisin_storage::jobs::global_conversation_broadcaster();
        let mut subscription =
            raisin_storage::jobs::ConversationEventSubscription::new(broadcaster, &channel);
        loop {
            let received = tokio::select! {
                _ = cancel.cancelled() => break,
                r = subscription.receiver().recv() => r,
            };
            match received {
                Ok(event) => {
                    let ends_turn = matches!(
                        event,
                        raisin_storage::jobs::ConversationEvent::Done { .. }
                            | raisin_storage::jobs::ConversationEvent::Waiting { .. }
                    );
                    if ends_turn {
                        limits.release_turn(&lease_key, &conversation_id);
                    }
                    // Backend log lines are for operators, not for visitors.
                    if matches!(event, raisin_storage::jobs::ConversationEvent::Log { .. }) {
                        continue;
                    }
                    let mut data = serde_json::to_value(&event).unwrap_or_default();
                    if let Some(obj) = data.as_object_mut() {
                        obj.insert("conversationId".into(), conversation_id.clone().into());
                    }
                    let msg = EventMessage::new(
                        subscription_id.clone(),
                        event.event_type().to_string(),
                        data,
                    );
                    if conn.read().send_event(msg).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "visitor chat forwarder lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_agent_paths_name_agents() {
        assert_eq!(
            agent_path_of("/agents/site").as_deref(),
            Some("/agents/site")
        );
        assert_eq!(agent_path_of("agent:site").as_deref(), Some("/agents/site"));
        assert_eq!(agent_path_of("/agents/../users/x"), None);
        assert_eq!(agent_path_of("/agents/a/b"), None);
        assert_eq!(agent_path_of("/agents/"), None);
        assert_eq!(agent_path_of("/users/alice"), None);
    }

    fn cfg() -> AnonymousChatConfig {
        AnonymousChatConfig {
            max_messages: 2,
            max_message_chars: 10,
            max_conversation_tokens: 1000,
            ..AnonymousChatConfig::default()
        }
    }

    #[test]
    fn a_message_must_have_text_and_fit() {
        assert!(check_text(&cfg(), "hello").is_ok());
        assert_eq!(check_text(&cfg(), "   ").unwrap_err().0, "EMPTY_MESSAGE");
        assert_eq!(
            check_text(&cfg(), "eleven chars").unwrap_err().0,
            "TOO_LONG"
        );
    }

    /// The token budget applies per conversation, measured on the agent's
    /// copy (`total_tokens_used`, maintained by the pipeline).
    #[test]
    fn the_token_budget_closes_a_conversation() {
        assert!(check_budget(&cfg(), 999.0, 0).is_ok());
        assert_eq!(
            check_budget(&cfg(), 1000.0, 0).unwrap_err().0,
            "LIMIT_REACHED"
        );
    }

    #[test]
    fn a_conversation_has_a_maximum_length() {
        assert!(check_budget(&cfg(), 0.0, 1).is_ok());
        assert_eq!(
            check_budget(&cfg(), 0.0, 2).unwrap_err().0,
            "TOO_MANY_MESSAGES"
        );
    }

    fn visitor_connection(
        key: &str,
        conversation: &str,
    ) -> (
        Arc<RwLock<ConnectionState>>,
        tokio::sync::mpsc::UnboundedReceiver<EventMessage>,
    ) {
        let mut conn = ConnectionState::new("tenant-a".into(), Some("site".into()), 4, 100);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        conn.set_event_channel(tx);
        conn.set_auth_context(AuthContext::anonymous_user("anon"));
        conn.bind_visitor(VisitorBinding {
            key: key.into(),
            home: visitor_home_path(key),
            agent_path: "/agents/site".into(),
            conversations: HashSet::from([conversation.to_string()]),
            forwarding: HashSet::from([conversation.to_string()]),
        });
        (Arc::new(RwLock::new(conn)), rx)
    }

    fn text(t: &str) -> raisin_storage::jobs::ConversationEvent {
        raisin_storage::jobs::ConversationEvent::TextChunk {
            text: t.into(),
            timestamp: "2026-09-30T00:00:00Z".into(),
        }
    }

    fn done(conversation_path: &str) -> raisin_storage::jobs::ConversationEvent {
        raisin_storage::jobs::ConversationEvent::Done {
            conversation_path: conversation_path.into(),
            content: Some("hi".into()),
            role: Some("assistant".into()),
            sender_display_name: None,
            finish_reason: Some("stop".into()),
            timestamp: "2026-09-30T00:00:00Z".into(),
        }
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    /// Replies reach only the session that owns the conversation: two
    /// visitors chatting with the same agent at once each receive exactly
    /// their own conversation's events.
    #[tokio::test]
    async fn replies_reach_only_the_owning_session() {
        let limits = Arc::new(crate::visitor::VisitorLimits::new());
        let conv_a = session::new_conversation_id();
        let conv_b = session::new_conversation_id();
        let (conn_a, mut rx_a) = visitor_connection(&"a".repeat(32), &conv_a);
        let (conn_b, mut rx_b) = visitor_connection(&"b".repeat(32), &conv_b);
        spawn_forwarder(
            Arc::clone(&limits),
            &conn_a,
            "a".repeat(32),
            conv_a.clone(),
            format!("visitor-chat:{conv_a}"),
        );
        spawn_forwarder(
            Arc::clone(&limits),
            &conn_b,
            "b".repeat(32),
            conv_b.clone(),
            format!("visitor-chat:{conv_b}"),
        );
        settle().await;

        let broadcaster = raisin_storage::jobs::global_conversation_broadcaster();
        broadcaster.emit(&format!("chat:{conv_a}"), text("for A"));
        settle().await;

        let got = rx_a.try_recv().expect("A receives its own reply");
        assert_eq!(got.subscription_id, format!("visitor-chat:{conv_a}"));
        assert_eq!(got.payload["text"], "for A");
        assert_eq!(got.payload["conversationId"], conv_a.as_str());
        assert!(rx_b.try_recv().is_err(), "B never sees A's conversation");

        broadcaster.emit(&format!("chat:{conv_b}"), text("for B"));
        settle().await;
        assert_eq!(rx_b.try_recv().unwrap().payload["text"], "for B");
        assert!(rx_a.try_recv().is_err(), "A never sees B's conversation");
    }

    /// `done` frees the session's turn slot, so the next message is accepted
    /// without waiting for the lease to run out.
    #[tokio::test]
    async fn done_frees_the_turn_slot() {
        let limits = Arc::new(crate::visitor::VisitorLimits::new());
        let key = "c".repeat(32);
        let conv = session::new_conversation_id();
        let (conn, mut rx) = visitor_connection(&key, &conv);
        let lease_key = format!("session:tenant-a:{key}");
        let lease = Duration::from_secs(120);
        assert!(limits.acquire_turn(&lease_key, &conv, 1, lease, Instant::now()));
        spawn_forwarder(
            Arc::clone(&limits),
            &conn,
            key.clone(),
            conv.clone(),
            format!("visitor-chat:{conv}"),
        );
        settle().await;
        assert!(!limits.acquire_turn(&lease_key, &conv, 1, lease, Instant::now()));

        raisin_storage::jobs::global_conversation_broadcaster()
            .emit(&format!("chat:{conv}"), done("/agents/site/inbox/chats/x"));
        settle().await;
        assert_eq!(rx.try_recv().unwrap().event_type, "done");
        assert!(
            limits.acquire_turn(&lease_key, &conv, 1, lease, Instant::now()),
            "done released the slot"
        );
    }

    /// A disconnect stops the forwarder even when the conversation is idle.
    #[tokio::test]
    async fn a_disconnect_stops_the_forwarder() {
        let limits = Arc::new(crate::visitor::VisitorLimits::new());
        let key = "d".repeat(32);
        let conv = session::new_conversation_id();
        let (conn, _rx) = visitor_connection(&key, &conv);
        let task = spawn_forwarder(limits, &conn, key, conv.clone(), "s".into());
        settle().await;
        conn.read().cleanup();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("the forwarder ends on disconnect")
            .unwrap();
    }

    /// Binding a session is what lets a connection read its home: the auth
    /// context carries the home, and only that one.
    #[test]
    fn binding_a_session_scopes_the_auth_context_to_its_home() {
        let key = "e".repeat(32);
        let (conn, _rx) = visitor_connection(&key, &session::new_conversation_id());
        let conn = conn.read();
        let auth = conn.auth_context().unwrap();
        assert_eq!(
            auth.visitor_home.as_deref(),
            Some(visitor_home_path(&key).as_str())
        );
        assert!(auth.home.is_none(), "a visitor has no user home");
        assert!(auth.is_anonymous_principal());
    }
}
