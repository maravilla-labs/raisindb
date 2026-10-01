// SPDX-License-Identifier: BSL-1.1

//! An agent's anonymous-chat settings, read from its `raisin:AIAgent` node.
//!
//! Off unless the agent says `allow_anonymous: true` or
//! `anonymous: { enabled: true, ... }`. Every limit has a default, so the flag
//! alone is a complete, bounded configuration.

use std::collections::HashMap;
use std::time::Duration;

use raisin_models::nodes::properties::PropertyValue;

/// The limits one agent applies to anonymous visitors.
#[derive(Debug, Clone, PartialEq)]
pub struct AnonymousChatConfig {
    /// Page origins allowed to open a visitor chat. Empty: any origin.
    pub allowed_origins: Vec<String>,
    /// Visitor messages per conversation.
    pub max_messages: u32,
    /// Characters per visitor message.
    pub max_message_chars: usize,
    /// Token budget per conversation (agent side `total_tokens_used`).
    pub max_conversation_tokens: u64,
    /// Conversations per visitor session.
    pub max_conversations: u32,
    /// Messages per session per minute.
    pub rate_per_session_per_minute: u32,
    /// Session starts + messages per client IP per minute.
    pub rate_per_ip_per_minute: u32,
    /// Turns a session may have in flight at once.
    pub max_concurrent_turns: u32,
    /// How long a turn holds its slot before it is presumed hung.
    pub turn_lease: Duration,
    /// Inactivity after which a session and its conversations are purged.
    pub session_ttl: Duration,
    /// Tokens ALL of this agent's visitor conversations may use per UTC day,
    /// as the pipeline records them (`raisin:AICostRecord`). `None`: no daily
    /// budget. The hard stop on what anonymous chat may cost.
    pub max_daily_tokens: Option<u64>,
    /// Visitor messages per client IP per 24 hours, persisted. Generous: a
    /// school or an office sends many visitors through one address.
    pub max_messages_per_ip_per_day: u32,
    /// New visitor sessions per client IP per hour, persisted. `None`: off.
    pub max_new_sessions_per_ip_per_hour: Option<u32>,
}

impl Default for AnonymousChatConfig {
    fn default() -> Self {
        Self {
            allowed_origins: Vec::new(),
            max_messages: 20,
            max_message_chars: 2000,
            max_conversation_tokens: 50_000,
            max_conversations: 5,
            rate_per_session_per_minute: 6,
            rate_per_ip_per_minute: 30,
            max_concurrent_turns: 1,
            turn_lease: Duration::from_secs(120),
            session_ttl: Duration::from_secs(24 * 3600),
            max_daily_tokens: None,
            max_messages_per_ip_per_day: 500,
            max_new_sessions_per_ip_per_hour: None,
        }
    }
}

fn number(v: Option<&PropertyValue>) -> Option<f64> {
    match v? {
        PropertyValue::Integer(i) => Some(*i as f64),
        PropertyValue::Float(f) => Some(*f),
        PropertyValue::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n: &f64| n.is_finite() && *n > 0.0)
}

fn strings(v: Option<&PropertyValue>) -> Vec<String> {
    match v {
        Some(PropertyValue::Array(values)) => values
            .iter()
            .filter_map(|v| match v {
                PropertyValue::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn normalize_origin(origin: &str) -> String {
    origin.trim().trim_end_matches('/').to_ascii_lowercase()
}

impl AnonymousChatConfig {
    /// The agent's settings, or `None` when it does not accept anonymous
    /// visitors. The flag is required: an agent without it refuses them.
    pub fn from_agent_properties(props: &HashMap<String, PropertyValue>) -> Option<Self> {
        let block = match props.get("anonymous") {
            Some(PropertyValue::Object(block)) => Some(block),
            _ => None,
        };
        let enabled = matches!(
            props.get("allow_anonymous"),
            Some(PropertyValue::Boolean(true))
        ) || block
            .is_some_and(|b| matches!(b.get("enabled"), Some(PropertyValue::Boolean(true))));
        if !enabled {
            return None;
        }
        let mut cfg = Self::default();
        let Some(b) = block else {
            return Some(cfg);
        };
        cfg.allowed_origins = strings(b.get("allowed_origins"))
            .iter()
            .map(|o| normalize_origin(o))
            .collect();
        let int = |key: &str, cap: f64| number(b.get(key)).map(|n| n.min(cap));
        if let Some(n) = int("max_messages", 10_000.0) {
            cfg.max_messages = n as u32;
        }
        if let Some(n) = int("max_message_chars", 100_000.0) {
            cfg.max_message_chars = n as usize;
        }
        if let Some(n) = int("max_conversation_tokens", 1e9) {
            cfg.max_conversation_tokens = n as u64;
        }
        if let Some(n) = int("max_conversations", 1_000.0) {
            cfg.max_conversations = n as u32;
        }
        if let Some(n) = int("rate_per_session_per_minute", 10_000.0) {
            cfg.rate_per_session_per_minute = n as u32;
        }
        if let Some(n) = int("rate_per_ip_per_minute", 100_000.0) {
            cfg.rate_per_ip_per_minute = n as u32;
        }
        if let Some(n) = int("max_concurrent_turns", 100.0) {
            cfg.max_concurrent_turns = n as u32;
        }
        if let Some(n) = int("turn_lease_seconds", 3600.0) {
            cfg.turn_lease = Duration::from_secs(n as u64);
        }
        if let Some(n) = int("session_ttl_hours", 24.0 * 90.0) {
            cfg.session_ttl = Duration::from_secs_f64(n * 3600.0);
        }
        if let Some(n) = int("max_daily_tokens", 1e12) {
            cfg.max_daily_tokens = Some(n as u64);
        }
        if let Some(n) = int("max_messages_per_ip_per_day", 1e7) {
            cfg.max_messages_per_ip_per_day = n as u32;
        }
        if let Some(n) = int("max_new_sessions_per_ip_per_hour", 1e6) {
            cfg.max_new_sessions_per_ip_per_hour = Some(n as u32);
        }
        Some(cfg)
    }

    /// Whether a page at `origin` may open a visitor chat. With an allow-list,
    /// a missing `Origin` header is refused.
    pub fn origin_allowed(&self, origin: Option<&str>) -> bool {
        if self.allowed_origins.is_empty() {
            return true;
        }
        origin
            .map(normalize_origin)
            .is_some_and(|o| self.allowed_origins.iter().any(|a| *a == o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(v: serde_json::Value) -> HashMap<String, PropertyValue> {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), PropertyValue::from_json(v)))
            .collect()
    }

    #[test]
    fn an_agent_without_the_flag_refuses_visitors() {
        assert!(
            AnonymousChatConfig::from_agent_properties(&props(serde_json::json!({}))).is_none()
        );
        assert!(
            AnonymousChatConfig::from_agent_properties(&props(serde_json::json!({
                "allow_anonymous": false,
                "anonymous": { "enabled": false, "max_messages": 3 }
            })))
            .is_none()
        );
        // Limits alone are not the flag.
        assert!(
            AnonymousChatConfig::from_agent_properties(&props(serde_json::json!({
                "anonymous": { "max_messages": 3 }
            })))
            .is_none()
        );
    }

    #[test]
    fn the_shorthand_flag_enables_the_defaults() {
        let cfg = AnonymousChatConfig::from_agent_properties(&props(serde_json::json!({
            "allow_anonymous": true
        })))
        .unwrap();
        assert_eq!(cfg, AnonymousChatConfig::default());
    }

    #[test]
    fn the_block_sets_limits() {
        let cfg = AnonymousChatConfig::from_agent_properties(&props(serde_json::json!({
            "anonymous": {
                "enabled": true,
                "allowed_origins": ["https://www.Example.com/"],
                "max_messages": 3,
                "max_conversation_tokens": 1000,
                "rate_per_session_per_minute": 2,
                "turn_lease_seconds": 30,
                "session_ttl_hours": 1
            }
        })))
        .unwrap();
        assert_eq!(cfg.max_messages, 3);
        assert_eq!(cfg.max_conversation_tokens, 1000);
        assert_eq!(cfg.rate_per_session_per_minute, 2);
        assert_eq!(cfg.turn_lease, Duration::from_secs(30));
        assert_eq!(cfg.session_ttl, Duration::from_secs(3600));
        assert!(cfg.origin_allowed(Some("https://www.example.com")));
        assert!(!cfg.origin_allowed(Some("https://evil.example")));
        assert!(
            !cfg.origin_allowed(None),
            "an allow-list refuses a missing Origin"
        );
    }

    #[test]
    fn nonsense_values_keep_the_defaults() {
        let cfg = AnonymousChatConfig::from_agent_properties(&props(serde_json::json!({
            "anonymous": { "enabled": true, "max_messages": -5, "max_message_chars": "lots" }
        })))
        .unwrap();
        assert_eq!(cfg.max_messages, 20);
        assert_eq!(cfg.max_message_chars, 2000);
    }

    #[test]
    fn daily_limits_default_to_a_per_ip_cap_and_no_budget() {
        let cfg = AnonymousChatConfig::from_agent_properties(&props(serde_json::json!({
            "allow_anonymous": true
        })))
        .unwrap();
        assert_eq!(cfg.max_daily_tokens, None);
        assert_eq!(cfg.max_messages_per_ip_per_day, 500);
        assert_eq!(cfg.max_new_sessions_per_ip_per_hour, None);

        let cfg = AnonymousChatConfig::from_agent_properties(&props(serde_json::json!({
            "anonymous": {
                "enabled": true,
                "max_daily_tokens": 2_000_000,
                "max_messages_per_ip_per_day": 300,
                "max_new_sessions_per_ip_per_hour": 40
            }
        })))
        .unwrap();
        assert_eq!(cfg.max_daily_tokens, Some(2_000_000));
        assert_eq!(cfg.max_messages_per_ip_per_day, 300);
        assert_eq!(cfg.max_new_sessions_per_ip_per_hour, Some(40));
    }
}
