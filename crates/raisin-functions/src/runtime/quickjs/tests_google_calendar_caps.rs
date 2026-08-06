// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! google-calendar adapter: capability declaration and the conference-link
//! fallback. Shares the harness with [`super::tests_google_calendar_adapter`].

use super::tests_google_calendar_adapter::{call_adapter, list_input, mount, ok};
use serde_json::json;

/// conferenceData is the current field; hangoutLink is the legacy convenience
/// one. Both were discarded, so a synced meeting had no join link at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_conference_link_falls_back_to_conference_data() {
    let ev = json!({
        "id": "E2",
        "summary": "Sync",
        "status": "confirmed",
        "conferenceData": {
            "entryPoints": [
                { "entryPointType": "phone", "uri": "tel:+41000" },
                { "entryPointType": "video", "uri": "https://meet.example/abc" },
            ],
        },
        "start": { "dateTime": "2026-08-11T12:00:00Z" },
        "end": { "dateTime": "2026-08-11T13:00:00Z" },
    });
    let run = call_adapter(list_input(false), vec![ok(vec![ev], json!({}))]).await;
    assert_eq!(
        run.output.expect("output")["items"][0]["metadata"]["online_meeting_url"],
        json!("https://meet.example/abc")
    );
}

/// Push stays on. The capabilities object declared `supports_webhooks` TWICE
/// with contradicting values; JS last-wins made the answer `true`, so every
/// Google calendar mount's push worked purely by key order and any reformat
/// would have silently disabled it with no test to catch it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capabilities_declare_webhooks_exactly_once() {
    let caps = call_adapter(
        json!({ "operation": "capabilities", "mount": mount(false) }),
        vec![],
    )
    .await
    .output
    .expect("capabilities");
    assert_eq!(caps["supports_webhooks"], json!(true));
    assert_eq!(caps["supports_push"], json!(true));
    // Writes landed later; WHAT is declared is pinned in
    // `tests_google_calendar_write`. Kept here only so this test does not
    // silently stop covering the capability object it exists to guard.
    assert_eq!(caps["can_write"], json!(true));
}
