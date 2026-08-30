// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Pure parsing helpers for the IMAP binding: flag/envelope stringification and
//! RFC822 body extraction via `mail-parser`. Kept free of any IO so it is
//! trivially testable.

use async_imap::imap_proto::{Address, Envelope};
use async_imap::types::{Fetch, Flag};
use mail_parser::{DateTime, MessageParser};
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

/// Normalise an RFC 2822 / RFC 5322 date header into the canonical node shape:
/// **RFC 3339, always in UTC** (`2026-09-02T07:00:00Z`).
///
/// WHY THIS EXISTS — do not "simplify" it away by passing the header through.
/// The mount contract is that a mapper normalises the provider's shape; every
/// other property here obeys it, and `date` used to be the one that did not.
/// The IMAP `ENVELOPE` carries the *raw* header (`"Wed, 2 Sep 2026 09:00:00
/// +0200"`) while the body path already emitted RFC 3339, so one mailbox wrote
/// one field in two shapes. Studio sorts and pages on `properties->>'date'` as
/// a **string**, so the raw form made an inbox order by WEEKDAY NAME — "Wed"
/// above a newer "Fri" — and the keyset `nextCursor` became `"Wed, 2 Sep…"`,
/// after which "load more" silently skipped every Thu/Fri/Sat/Sun message.
/// RFC 3339 is fixed-width and lexically ordered, so string sort == time sort
/// and a keyset pager is sound — the same reason the calendar mapper keeps
/// `start_local` fixed-width.
///
/// WHY UTC RATHER THAN THE ORIGINAL OFFSET: both are fixed-width, but only UTC
/// makes the LEXICAL order the CHRONOLOGICAL one. `09:00:00+02:00` (07:00Z)
/// must sort before `08:00:00Z`, and as strings it does not. Preserving the
/// offset would leave the sort subtly wrong across timezones, which is the bug
/// we are fixing. `mail-parser`'s `to_rfc3339` keeps the sender's offset, so we
/// re-anchor through the timestamp; the sender's original string is never lost,
/// it stays verbatim in `headers.date`.
///
/// An absent or unparsable header yields an EMPTY string, deliberately: a
/// message with no trustworthy time must still be listable, and empty sorts to
/// one edge of the mailbox rather than into the middle as if it had a real
/// time. Inventing a time (now, epoch) would be a wrong date; failing the fetch
/// would make one malformed message hide a whole page of good ones.
pub fn normalise_date(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    // RFC 2822 first, because that is what a `Date:` header and an IMAP
    // ENVELOPE actually carry. The RFC 3339 fallback makes this function
    // IDEMPOTENT, which is not a nicety: `normalise_date` is the obvious tool
    // for a backfill over dates already stored, and a mailbox mid-migration
    // holds both shapes — without it, re-normalising would blank every value
    // that was already canonical. It also rescues the malformed sender that
    // spells its `Date:` header in ISO 8601, which the RFC 2822 parser rejects
    // outright.
    DateTime::parse_rfc822(raw)
        .or_else(|| DateTime::parse_rfc3339(raw))
        .filter(|dt| dt.is_valid())
        .map(|dt| to_rfc3339_utc(&dt))
        .unwrap_or_default()
}

/// Render a parsed datetime as RFC 3339 in UTC (`…Z`), fixed width.
fn to_rfc3339_utc(dt: &DateTime) -> String {
    DateTime::from_timestamp(dt.to_timestamp()).to_rfc3339()
}

/// Build a lightweight [`MessageSummary`] from a fetch carrying `ENVELOPE` +
/// `FLAGS`. `snippet` is left empty here — full body lives in `fetch_message`.
pub fn summary_from_fetch(fetch: &Fetch) -> Option<MessageSummary> {
    let uid = fetch.uid?;
    let flags: Vec<String> = fetch.flags().map(|f| flag_to_string(&f)).collect();

    let (from, to, subject, date_raw, message_id, in_reply_to) = match fetch.envelope() {
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

    // Canonical node shape: RFC 3339 UTC. The RAW header string keeps its home
    // in `headers.date`, unchanged, so nothing is lost and a mail client's
    // original wording stays inspectable.
    let date = normalise_date(&date_raw);

    let mut headers = Map::new();
    insert_str(&mut headers, "from", &from);
    insert_str(&mut headers, "to", &to);
    insert_str(&mut headers, "subject", &subject);
    insert_str(&mut headers, "date", &date_raw);
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
    let mut date_raw = String::new();
    let mut message_id = String::new();
    let mut text = String::new();
    let mut html = None;
    let mut snippet = String::new();

    if let Some(msg) = parsed {
        from = msg.from().map(format_ml_address).unwrap_or_default();
        to = msg.to().map(format_ml_address).unwrap_or_default();
        subject = msg.subject().unwrap_or_default().to_string();
        // Same canonical shape as the envelope/listing path (see
        // `normalise_date`): RFC 3339 in UTC, not the sender's offset, so the
        // string order is the time order.
        date = msg.date().map(to_rfc3339_utc).unwrap_or_default();
        date_raw = msg
            .header_raw("Date")
            .map(|h| h.trim().to_string())
            .unwrap_or_default();
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
    insert_str(&mut headers, "date", &date_raw);
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

    fn envelope_summary(date: &[u8]) -> (String, Map<String, Value>) {
        // `summary_from_fetch` needs a real `Fetch`, which cannot be built
        // outside `async-imap`; the date handling it delegates to is exercised
        // directly here, together with the raw-header rule it applies.
        let raw = String::from_utf8_lossy(date).into_owned();
        let mut headers = Map::new();
        insert_str(&mut headers, "date", &raw);
        (normalise_date(&raw), headers)
    }

    #[test]
    fn envelope_date_is_normalised_to_rfc3339_utc() {
        assert_eq!(
            normalise_date("Wed, 2 Sep 2026 09:00:00 +0200"),
            "2026-09-02T07:00:00Z"
        );
    }

    #[test]
    fn single_digit_day_still_yields_fixed_width() {
        // The bug this guards: "2 Sep" is 1 char narrower than "12 Sep", so the
        // raw header is NOT fixed width and cannot be sorted as a string.
        let a = normalise_date("Wed, 2 Sep 2026 09:00:00 +0200");
        let b = normalise_date("Sat, 12 Sep 2026 09:00:00 +0200");
        assert_eq!(a.len(), b.len());
        assert_eq!(a.len(), "2026-09-02T07:00:00Z".len());
        assert!(a < b);
    }

    #[test]
    fn lexical_order_matches_chronological_order_across_zones() {
        // Wednesday must sort BELOW the newer Friday. As raw headers the
        // weekday names ordered them "Fri" < "Wed" and the inbox was wrong.
        let wed = normalise_date("Wed, 2 Sep 2026 09:00:00 +0200");
        let fri = normalise_date("Fri, 4 Sep 2026 01:00:00 -0700");
        assert!(wed < fri, "{wed} should sort before {fri}");

        // And the reason we re-anchor to UTC rather than keeping the sender's
        // offset: 09:00+02:00 is 07:00Z, which is EARLIER than 08:00Z even
        // though the offset-preserving strings compare the other way.
        let plus_two = normalise_date("Wed, 2 Sep 2026 09:00:00 +0200");
        let utc = normalise_date("Wed, 2 Sep 2026 08:00:00 +0000");
        assert!(plus_two < utc, "{plus_two} should sort before {utc}");
    }

    #[test]
    fn obsolete_zones_are_accepted() {
        assert_eq!(
            normalise_date("Wed, 2 Sep 2026 09:00:00 GMT"),
            "2026-09-02T09:00:00Z"
        );
        // "-0000" means "UTC, but the sender declined to state a local zone".
        assert_eq!(
            normalise_date("Wed, 2 Sep 2026 09:00:00 -0000"),
            "2026-09-02T09:00:00Z"
        );
        assert_eq!(
            normalise_date("2 Sep 2026 09:00:00 +0200"),
            "2026-09-02T07:00:00Z"
        );
    }

    #[test]
    fn normalising_an_already_canonical_value_is_a_no_op() {
        // Idempotence, and it is load-bearing: a backfill over stored dates
        // meets a mailbox holding BOTH shapes, and must not blank the half that
        // is already right.
        let once = normalise_date("Wed, 2 Sep 2026 09:00:00 +0200");
        assert_eq!(normalise_date(&once), once);
        // An offset-carrying RFC 3339 value is re-anchored, not passed through.
        assert_eq!(
            normalise_date("2026-09-02T09:00:00+02:00"),
            "2026-09-02T07:00:00Z"
        );
        assert_eq!(
            normalise_date("2026-09-02T07:00:00Z"),
            "2026-09-02T07:00:00Z"
        );
    }

    #[test]
    fn garbage_and_absent_dates_become_empty_not_wrong() {
        // Must not panic, must not invent a time, must not land mid-inbox.
        for raw in ["", "   ", "not a date at all", "Wed, 32 Xxx 20 25:99:99"] {
            assert_eq!(normalise_date(raw), "", "input: {raw:?}");
        }
    }

    #[test]
    fn raw_header_is_kept_verbatim_in_headers_date() {
        let (date, headers) = envelope_summary(b"Wed, 2 Sep 2026 09:00:00 +0200");
        assert_eq!(date, "2026-09-02T07:00:00Z");
        assert_eq!(
            headers.get("date").and_then(Value::as_str),
            Some("Wed, 2 Sep 2026 09:00:00 +0200"),
            "the sender's original string must stay inspectable"
        );
    }

    #[test]
    fn detail_date_matches_the_envelope_shape_and_keeps_raw_header() {
        let raw = b"From: Alice <alice@example.org>\r\n\
                    Subject: Hello\r\n\
                    Date: Wed, 2 Sep 2026 09:00:00 +0200\r\n\
                    \r\n\
                    Body.\r\n";
        let d = detail_from_raw(9, vec![], raw);
        assert_eq!(d.date, "2026-09-02T07:00:00Z");
        assert_eq!(d.date, normalise_date("Wed, 2 Sep 2026 09:00:00 +0200"));
        assert_eq!(
            d.headers.get("date").and_then(Value::as_str),
            Some("Wed, 2 Sep 2026 09:00:00 +0200")
        );
    }

    #[test]
    fn detail_without_date_header_is_empty_not_missing() {
        let raw = b"From: Alice <alice@example.org>\r\n\
                    Subject: Hello\r\n\
                    \r\n\
                    Body.\r\n";
        let d = detail_from_raw(9, vec![], raw);
        assert_eq!(d.date, "");
        assert!(d.headers.get("date").is_none());
    }
}
