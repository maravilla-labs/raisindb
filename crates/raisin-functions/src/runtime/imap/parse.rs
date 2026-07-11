// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Pure parsing helpers for the IMAP binding: flag/envelope stringification and
//! RFC822 body extraction via `mail-parser`. Kept free of any IO so it is
//! trivially testable.

use async_imap::imap_proto::{Address, Envelope};
use async_imap::types::{Fetch, Flag};
use mail_parser::MessageParser;
use serde_json::{Map, Value};

use super::{MessageDetail, MessageSummary};

/// Render an IMAP flag to its canonical wire string (`\Seen`, custom, ...).
pub fn flag_to_string(flag: &Flag<'_>) -> String {
    match flag {
        Flag::Seen => "\\Seen".to_string(),
        Flag::Answered => "\\Answered".to_string(),
        Flag::Flagged => "\\Flagged".to_string(),
        Flag::Deleted => "\\Deleted".to_string(),
        Flag::Draft => "\\Draft".to_string(),
        Flag::Recent => "\\Recent".to_string(),
        Flag::MayCreate => "\\*".to_string(),
        Flag::Custom(s) => s.to_string(),
    }
}

fn bytes_to_string(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Format an IMAP `ENVELOPE` address list as `Name <mailbox@host>` entries.
fn format_addresses(addrs: &Option<Vec<Address<'_>>>) -> String {
    let Some(list) = addrs else {
        return String::new();
    };
    let parts: Vec<String> = list
        .iter()
        .map(|a| {
            let mailbox = a
                .mailbox
                .as_deref()
                .map(bytes_to_string)
                .unwrap_or_default();
            let host = a.host.as_deref().map(bytes_to_string).unwrap_or_default();
            let email = if host.is_empty() {
                mailbox
            } else {
                format!("{mailbox}@{host}")
            };
            match a.name.as_deref().map(bytes_to_string) {
                Some(name) if !name.is_empty() => format!("{name} <{email}>"),
                _ => email,
            }
        })
        .collect();
    parts.join(", ")
}

fn opt_bytes(v: &Option<std::borrow::Cow<'_, [u8]>>) -> String {
    v.as_deref().map(bytes_to_string).unwrap_or_default()
}

/// Build a lightweight [`MessageSummary`] from a fetch carrying `ENVELOPE` +
/// `FLAGS`. `snippet` is left empty here — full body lives in `fetch_message`.
pub fn summary_from_fetch(fetch: &Fetch) -> Option<MessageSummary> {
    let uid = fetch.uid?;
    let flags: Vec<String> = fetch.flags().map(|f| flag_to_string(&f)).collect();

    let (from, to, subject, date, message_id, in_reply_to) = match fetch.envelope() {
        Some(env) => (
            format_addresses(&env.from),
            format_addresses(&env.to),
            opt_bytes(&env.subject),
            opt_bytes(&env.date),
            opt_bytes(&env.message_id),
            opt_bytes(&env.in_reply_to),
        ),
        None => Default::default(),
    };

    let mut headers = Map::new();
    insert_str(&mut headers, "from", &from);
    insert_str(&mut headers, "to", &to);
    insert_str(&mut headers, "subject", &subject);
    insert_str(&mut headers, "date", &date);
    insert_str(&mut headers, "message-id", &message_id);

    let thread_id = if in_reply_to.is_empty() {
        None
    } else {
        Some(in_reply_to)
    };

    Some(MessageSummary {
        uid,
        headers,
        from,
        to,
        subject,
        date,
        snippet: String::new(),
        flags,
        message_id,
        thread_id,
    })
}

fn insert_str(map: &mut Map<String, Value>, key: &str, val: &str) {
    if !val.is_empty() {
        map.insert(key.to_string(), Value::String(val.to_string()));
    }
}

fn format_ml_address(addr: &mail_parser::Address<'_>) -> String {
    addr.iter()
        .map(|a| match (a.name(), a.address()) {
            (Some(n), Some(e)) if !n.is_empty() => format!("{n} <{e}>"),
            (_, Some(e)) => e.to_string(),
            (Some(n), None) => n.to_string(),
            _ => String::new(),
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parse a full RFC822 message (`BODY[]`) into a [`MessageDetail`].
pub fn detail_from_raw(uid: u32, flags: Vec<String>, raw: &[u8]) -> MessageDetail {
    let parsed = MessageParser::default().parse(raw);

    let mut from = String::new();
    let mut to = String::new();
    let mut subject = String::new();
    let mut date = String::new();
    let mut message_id = String::new();
    let mut text = String::new();
    let mut html = None;
    let mut snippet = String::new();

    if let Some(msg) = parsed {
        from = msg.from().map(format_ml_address).unwrap_or_default();
        to = msg.to().map(format_ml_address).unwrap_or_default();
        subject = msg.subject().unwrap_or_default().to_string();
        date = msg.date().map(|d| d.to_rfc3339()).unwrap_or_default();
        message_id = msg.message_id().unwrap_or_default().to_string();
        text = msg.body_text(0).map(|c| c.into_owned()).unwrap_or_default();
        html = msg.body_html(0).map(|c| c.into_owned());
        snippet = msg
            .body_preview(200)
            .map(|c| c.into_owned())
            .unwrap_or_default();
    }

    let mut headers = Map::new();
    insert_str(&mut headers, "from", &from);
    insert_str(&mut headers, "to", &to);
    insert_str(&mut headers, "subject", &subject);
    insert_str(&mut headers, "date", &date);
    insert_str(&mut headers, "message-id", &message_id);

    MessageDetail {
        uid,
        headers,
        from,
        to,
        subject,
        date,
        text,
        html,
        snippet,
        flags,
        message_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_render_to_wire_strings() {
        assert_eq!(flag_to_string(&Flag::Seen), "\\Seen");
        assert_eq!(
            flag_to_string(&Flag::Custom(std::borrow::Cow::Borrowed("$Label1"))),
            "$Label1"
        );
    }

    #[test]
    fn detail_parses_headers_and_body() {
        let raw = b"From: Alice <alice@example.org>\r\n\
                    To: Bob <bob@example.org>\r\n\
                    Subject: Hello\r\n\
                    Message-ID: <abc@example.org>\r\n\
                    \r\n\
                    Hello body text.\r\n";
        let d = detail_from_raw(7, vec!["\\Seen".to_string()], raw);
        assert_eq!(d.uid, 7);
        assert_eq!(d.subject, "Hello");
        assert!(d.from.contains("alice@example.org"));
        assert!(d.to.contains("bob@example.org"));
        assert!(d.text.contains("Hello body text"));
        assert_eq!(d.message_id, "abc@example.org");
    }
}
