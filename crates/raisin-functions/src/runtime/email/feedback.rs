// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Provider delivery feedback: what a webhook payload means, independent of
//! transport.
//!
//! `raisin.email.send` only proves a provider ACCEPTED a message. What happens
//! afterwards (delivered, deferred, bounced, blocked, complained) arrives later
//! as a webhook. This module is the pure half of receiving it: verifying a
//! Svix signature, normalising Resend and Brevo payloads into one vocabulary,
//! and ranking statuses so a late event can never move a message backwards.
//! The HTTP handler and the `raisin:EmailMessage` / `raisin:EmailEvent` writes
//! live in `raisin-transport-http`; keeping this side free of both is what makes
//! it testable with recorded payloads.

use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Parent of the `raisin:EmailMessage` records (workspace `raisin:system`).
pub const MESSAGES_PATH: &str = "/email/messages";
/// Parent of the `raisin:EmailEvent` records (workspace `raisin:system`).
pub const EVENTS_PATH: &str = "/email/events";

/// Replay window for a signed webhook, in seconds.
pub const SIGNATURE_TOLERANCE_SECS: i64 = 300;

/// The node name a provider message id is stored under.
///
/// ONE function for the writer (`raisin.email.send`) and the reader (the
/// webhook handler): if they sanitised differently, every event would look like
/// it belonged to an unknown message and no error would say so. Ids are
/// `<..@..>`-shaped for Brevo and SMTP, so `/`, `<`, `>` and `@` must not
/// survive into a path.
pub fn message_node_name(message_id: &str) -> String {
    message_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// One provider event, normalised.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackEvent {
    /// Stable across redeliveries of the same event; names the stored node.
    pub dedupe_key: String,
    /// The provider's id for the message this is about.
    pub message_id: String,
    /// `accepted | deferred | delivered | soft_bounce | hard_bounce | blocked |
    /// complained | failed | opened | clicked | unsubscribed`.
    pub kind: &'static str,
    pub email: Option<String>,
    pub occurred_at: Option<String>,
    pub detail: Option<String>,
}

/// How far along a message is. `None` means the event is informational
/// (an open, a click) and never changes `status`.
///
/// Monotonic on purpose: providers retry and reorder deliveries, and a late
/// `deferred` must not overwrite `delivered`. A hard bounce or complaint can
/// still follow a delivery (asynchronous bounce), so those rank above it.
pub fn status_rank(kind: &str) -> Option<u8> {
    match kind {
        "accepted" => Some(0),
        "deferred" => Some(1),
        "soft_bounce" => Some(2),
        "delivered" => Some(3),
        "hard_bounce" | "blocked" | "complained" | "failed" => Some(4),
        _ => None,
    }
}

fn key(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u32).to_be_bytes());
        h.update(p.as_bytes());
    }
    hex::encode(h.finalize())[..32].to_string()
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty())
}

/// A provider timestamp as RFC3339 UTC, or `None` when it cannot be read
/// WITHOUT GUESSING.
///
/// `raisin:EmailEvent.occurred_at` is declared `Date`, and a `Date` write
/// refuses a string that is not RFC3339. Providers do not all send RFC3339:
/// Brevo's `date` is `2023-03-10 09:40:24` with no offset, and Resend has used
/// both `2024-02-22T23:41:12.126Z` and a Postgres-style
/// `2024-02-22 23:41:12.126+00`. Storing those verbatim would make the event
/// write fail and the webhook be retried until the provider gives up, so the
/// event and the status change it carries would be lost over a timestamp.
///
/// Accepted: RFC3339; a date-time with a numeric offset and a space separator;
/// epoch seconds or milliseconds (number or digit string). A date-time without
/// an offset is NOT accepted: its zone is unknown, and a wrong instant is worse
/// than none (`received_at` still says when the event arrived).
pub fn normalize_timestamp(v: &Value) -> Option<String> {
    use chrono::{DateTime, TimeZone, Utc};
    let from_epoch = |n: i64| {
        // Below 10^11 is seconds (until the year 5138), above it milliseconds.
        let dt = if n.abs() < 100_000_000_000 {
            Utc.timestamp_opt(n, 0).single()
        } else {
            Utc.timestamp_millis_opt(n).single()
        };
        dt.map(|d| d.to_rfc3339())
    };
    match v {
        Value::Number(n) => n.as_i64().and_then(from_epoch),
        Value::String(raw) => {
            let t = raw.trim();
            if t.is_empty() {
                return None;
            }
            if t.chars().all(|c| c.is_ascii_digit()) {
                return t.parse::<i64>().ok().and_then(from_epoch);
            }
            if let Ok(dt) = DateTime::parse_from_rfc3339(t) {
                return Some(dt.with_timezone(&Utc).to_rfc3339());
            }
            // `2024-02-22 23:41:12.126+00` / `+0000` / `+00:00`.
            for fmt in [
                "%Y-%m-%d %H:%M:%S%.f%#z",
                "%Y-%m-%d %H:%M:%S%.f%:z",
                "%Y-%m-%d %H:%M:%S%.f%z",
            ] {
                if let Ok(dt) = DateTime::parse_from_str(t, fmt) {
                    return Some(dt.with_timezone(&Utc).to_rfc3339());
                }
            }
            None
        }
        _ => None,
    }
}

/// The first of `keys` on `v` that normalises to a timestamp.
fn first_timestamp(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|k| v.get(*k))
        .find_map(normalize_timestamp)
}

/// Brevo transactional webhook: one object, or an array of them.
pub fn parse_brevo(body: &Value) -> Vec<FeedbackEvent> {
    let items: Vec<&Value> = match body {
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) => vec![body],
        _ => vec![],
    };
    items
        .into_iter()
        .filter_map(|e| {
            let name = s(e, "event")?;
            let message_id = s(e, "message-id")?;
            let kind = match name {
                "request" => "accepted",
                "delivered" => "delivered",
                "deferred" => "deferred",
                "soft_bounce" => "soft_bounce",
                "hard_bounce" | "invalid_email" => "hard_bounce",
                "blocked" => "blocked",
                "spam" | "complaint" => "complained",
                "error" => "failed",
                "opened" | "unique_opened" | "proxy_open" => "opened",
                "click" => "clicked",
                "unsubscribed" => "unsubscribed",
                _ => return None,
            };
            let email = s(e, "email");
            // Epoch fields first: they carry their own zone. Brevo's `date`
            // has none and is only used if it happens to carry an offset.
            let when = first_timestamp(e, &["ts_epoch", "ts_event", "ts", "date"]);
            Some(FeedbackEvent {
                dedupe_key: key(&[
                    "brevo",
                    message_id,
                    name,
                    email.unwrap_or(""),
                    &e.get("ts_event").map(|v| v.to_string()).unwrap_or_default(),
                    &e.get("ts_epoch").map(|v| v.to_string()).unwrap_or_default(),
                ]),
                message_id: message_id.to_string(),
                kind,
                email: email.map(str::to_string),
                occurred_at: when,
                detail: s(e, "reason").map(str::to_string),
            })
        })
        .collect()
}

/// Resend webhook. `delivery_id` is the `svix-id` header: it is the same on
/// every redelivery of one event, which is what makes it a dedupe key.
pub fn parse_resend(body: &Value, delivery_id: &str) -> Vec<FeedbackEvent> {
    let Some(ty) = s(body, "type") else {
        return vec![];
    };
    let data = body.get("data").unwrap_or(&Value::Null);
    let Some(message_id) = s(data, "email_id") else {
        return vec![];
    };
    let transient = data
        .pointer("/bounce/type")
        .and_then(Value::as_str)
        .is_some_and(|t| t.eq_ignore_ascii_case("transient"));
    let kind = match ty {
        "email.sent" => "accepted",
        "email.delivered" => "delivered",
        "email.delivery_delayed" => "deferred",
        "email.bounced" if transient => "soft_bounce",
        "email.bounced" => "hard_bounce",
        "email.complained" => "complained",
        "email.failed" => "failed",
        "email.suppressed" => "blocked",
        "email.opened" => "opened",
        "email.clicked" => "clicked",
        _ => return vec![],
    };
    let email = data
        .get("to")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str);
    let detail = data
        .pointer("/bounce/message")
        .or_else(|| data.get("reason"))
        .and_then(Value::as_str);
    vec![FeedbackEvent {
        dedupe_key: key(&["resend", delivery_id, message_id, ty]),
        message_id: message_id.to_string(),
        kind,
        email: email.map(str::to_string),
        occurred_at: first_timestamp(body, &["created_at"]),
        detail: detail.map(str::to_string),
    }]
}

/// Verify a Svix-signed webhook (what Resend uses).
///
/// `secret` is the `whsec_...` string. The signed content is
/// `{id}.{timestamp}.{raw body}`: the RAW bytes, which is why this cannot run
/// behind a handler that has already parsed the body into JSON.
pub fn verify_svix(
    secret: &str,
    id: &str,
    timestamp: &str,
    body: &[u8],
    signature_header: &str,
    now_secs: i64,
) -> Result<(), &'static str> {
    let ts: i64 = timestamp.parse().map_err(|_| "bad timestamp")?;
    if (now_secs - ts).abs() > SIGNATURE_TOLERANCE_SECS {
        return Err("timestamp outside tolerance");
    }
    let raw = secret.strip_prefix("whsec_").unwrap_or(secret);
    let key = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|_| "bad signing secret")?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| "bad signing secret")?;
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = mac.finalize().into_bytes();

    // `v1,<b64> v1,<b64>`: several are sent during secret rotation.
    for part in signature_header.split_whitespace() {
        let Some(sig) = part.strip_prefix("v1,") else {
            continue;
        };
        if let Ok(given) = base64::engine::general_purpose::STANDARD.decode(sig) {
            // `verify_slice` is constant-time.
            let mut m = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| "bad signing secret")?;
            m.update(id.as_bytes());
            m.update(b".");
            m.update(timestamp.as_bytes());
            m.update(b".");
            m.update(body);
            if given.len() == expected.len() && m.verify_slice(&given).is_ok() {
                return Ok(());
            }
        }
    }
    Err("signature mismatch")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn node_name_has_no_path_characters() {
        assert_eq!(message_node_name("<a.b@smtp.x>"), "_a_b_smtp_x_");
    }

    #[test]
    fn brevo_events_normalise() {
        let ev = parse_brevo(&json!({
            "event": "hard_bounce", "email": "x@y.z", "message-id": "<1@b>",
            "ts_event": 1700000000, "reason": "mailbox full"
        }));
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, "hard_bounce");
        assert_eq!(ev[0].detail.as_deref(), Some("mailbox full"));
        // Redelivery of the same payload must name the same node.
        let again = parse_brevo(&json!({
            "event": "hard_bounce", "email": "x@y.z", "message-id": "<1@b>",
            "ts_event": 1700000000, "reason": "mailbox full"
        }));
        assert_eq!(ev[0].dedupe_key, again[0].dedupe_key);
        assert!(parse_brevo(&json!({"event": "nonsense", "message-id": "m"})).is_empty());
    }

    #[test]
    fn resend_transient_bounce_is_soft() {
        let body = json!({
            "type": "email.bounced", "created_at": "2026-01-01T00:00:00Z",
            "data": {"email_id": "abc", "to": ["a@b.c"], "bounce": {"type": "Transient"}}
        });
        assert_eq!(parse_resend(&body, "msg_1")[0].kind, "soft_bounce");
        assert!(parse_resend(&json!({"type": "email.received"}), "x").is_empty());
    }

    #[test]
    fn occurred_at_is_rfc3339_or_absent() {
        let ev = parse_brevo(&json!({
            "event": "delivered", "message-id": "<1@b>",
            "date": "2023-03-10 09:40:24", "ts_event": 1700000000
        }));
        assert_eq!(
            ev[0].occurred_at.as_deref(),
            Some("2023-11-14T22:13:20+00:00")
        );
        // ts_epoch is milliseconds and wins.
        let ev = parse_brevo(&json!({
            "event": "delivered", "message-id": "<1@b>",
            "ts_epoch": 1700000000123_i64, "ts_event": 1700000000
        }));
        assert_eq!(
            ev[0].occurred_at.as_deref(),
            Some("2023-11-14T22:13:20.123+00:00")
        );
        // A zone-less date alone is not guessed at.
        let ev = parse_brevo(&json!({
            "event": "delivered", "message-id": "<1@b>", "date": "2023-03-10 09:40:24"
        }));
        assert_eq!(ev[0].occurred_at, None);

        let resend = |created_at: &str| {
            parse_resend(
                &json!({ "type": "email.delivered", "created_at": created_at,
                         "data": {"email_id": "abc", "to": ["a@b.c"]} }),
                "msg_1",
            )[0]
            .occurred_at
            .clone()
        };
        assert_eq!(
            resend("2024-02-22T23:41:12.126Z").as_deref(),
            Some("2024-02-22T23:41:12.126+00:00")
        );
        assert_eq!(
            resend("2024-02-22 23:41:12.126+00").as_deref(),
            Some("2024-02-22T23:41:12.126+00:00")
        );
        assert_eq!(resend("yesterday"), None);
    }

    #[test]
    fn every_normalised_timestamp_is_rfc3339() {
        for v in [
            json!(1700000000),
            json!("1700000000"),
            json!(1700000000123_i64),
            json!("2026-01-01T00:00:00Z"),
            json!("2026-01-01T01:00:00+01:00"),
            json!("2026-01-01 00:00:00+0000"),
        ] {
            let out = normalize_timestamp(&v).unwrap_or_else(|| panic!("{v}"));
            assert!(
                chrono::DateTime::parse_from_rfc3339(&out).is_ok(),
                "{v} -> {out}"
            );
        }
        for v in [
            json!(""),
            json!("2026-01-01 00:00:00"),
            json!(null),
            json!(true),
        ] {
            assert_eq!(normalize_timestamp(&v), None, "{v}");
        }
    }

    #[test]
    fn status_never_moves_backwards() {
        assert!(status_rank("deferred") < status_rank("delivered"));
        assert!(status_rank("delivered") < status_rank("hard_bounce"));
        assert_eq!(status_rank("opened"), None);
    }

    #[test]
    fn svix_roundtrip_and_tamper() {
        let secret = "whsec_dGVzdC1rZXk=";
        let (id, ts, body) = ("msg_1", "1700000000", br#"{"a":1}"#);
        let key = base64::engine::general_purpose::STANDARD
            .decode("dGVzdC1rZXk=")
            .unwrap();
        let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
        mac.update(format!("{id}.{ts}.").as_bytes());
        mac.update(body);
        let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        let header = format!("v1,{sig}");
        assert!(verify_svix(secret, id, ts, body, &header, 1700000100).is_ok());
        assert!(verify_svix(secret, id, ts, b"{}", &header, 1700000100).is_err());
        assert!(verify_svix(secret, id, ts, body, &header, 1700009999).is_err());
    }
}
