// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The ms-graph OUTBOX mapper, executed as the real file in QuickJS.
//!
//! The engine's own suite mocks the mapper entirely, so it would pass no matter
//! what this file emits. What is only observable here is the thing that actually
//! reaches a recipient: which addresses go in the To line, which body wins when
//! a command carries both, and — most importantly — which commands the mapper
//! REFUSES to translate. A mapper that guesses on this path sends someone an
//! email.

use super::tests_ms_graph_adapter::function_files;
use super::*;
use crate::api::MockFunctionApi;
use serde_json::{json, Value};

fn mapper_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../builtin-packages/ms-graph-adapter/content/functions/mappers/ms-graph-outbox")
}

fn mapper_source() -> String {
    let path = mapper_dir().join("index.js");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read mapper at {}: {e}", path.display()))
}

/// The mapper's sibling MODULES, read from disk exactly as `adapter_files`
/// does. See `function_files` for where that stops short of the engine's
/// `load_sibling_files`, and why the gap can only ever hide files from the
/// test rather than invent them.
///
/// This harness used to pass `HashMap::new()`, which supplied LESS than
/// production: an `import` the engine resolves fine failed here and nowhere
/// else, so the harness silently forbade ever splitting a mapper into more
/// than one file. An empty map stays legitimate for a single-file mapper.
fn mapper_files() -> HashMap<String, String> {
    function_files(&mapper_dir())
}

/// A mapper is pure and I/O-free, so it gets no scripted HTTP at all — and if it
/// ever reached for the network, `http_calls` below would catch it.
async fn call_mapper(input: Value) -> (Option<Value>, Vec<Value>) {
    let runtime = QuickJsRuntime::new();
    let api = Arc::new(MockFunctionApi::new(json!({})));
    let context = ExecutionContext::new("t1", "r1", "main", "tester").with_input(input);
    let metadata = FunctionMetadata::javascript("ms_graph_outbox");
    let result = runtime
        .execute(
            &FunctionCode::from(mapper_source()),
            "handler",
            context,
            &metadata,
            api.clone() as Arc<dyn crate::api::FunctionApi>,
            mapper_files(),
        )
        .await
        .expect("runtime execute");
    if let Some(err) = result.error {
        panic!("mapper threw: {}", err.message);
    }
    (result.output, api.http_calls())
}

/// The `to_external` input the engine's `submit.rs::map_command` sends.
async fn to_external(properties: Value) -> Option<Value> {
    let (out, http) = call_mapper(json!({
        "operation": "to_external",
        "node": { "properties": properties },
        "mount": { "mount_id": "m1", "config": {} },
        "fields": Value::Null,
    }))
    .await;
    assert!(http.is_empty(), "a mapper must perform no I/O");
    out.filter(|v| !v.is_null())
}

/// The probe the engine runs once per run. Without it the mount is reported
/// read-only, and an outbox that cannot say it writes is an outbox that never
/// resolves to `submit`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_mapper_declares_the_write_direction_and_refuses_the_read_one() {
    let (caps, _) = call_mapper(json!({ "operation": "mapper_capabilities", "mount": {} })).await;
    assert_eq!(caps.expect("capabilities")["to_external"], json!(true));

    // An outbox IMPORTS nothing. Null is the documented "skip this item", and
    // it is deliberate rather than incidental: a mount that also mapped inbound
    // would materialize sent mail as fresh commands.
    for op in ["to_node", "anything_else"] {
        let (out, _) = call_mapper(json!({
            "operation": op,
            "external_item": { "external_id": "X1", "name": "x" },
        }))
        .await;
        assert!(
            matches!(out, None | Some(Value::Null)),
            "{op} produced {out:?}"
        );
    }
}

/// A send becomes a Graph `sendMail` body, with recipients in Graph's shape and
/// the HTML body winning over the plain-text one.
///
/// HTML wins because Graph takes ONE contentType: sending the text half of a
/// message someone composed in HTML is a silent downgrade nobody sees until the
/// recipient does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_becomes_a_graph_message_with_html_winning() {
    let out = to_external(json!({
        "action": "send",
        "to": ["a@b.com", { "address": "c@d.com", "name": "C D" }],
        "cc": ["e@f.com"],
        "bcc": ["g@h.com"],
        "subject": "hello",
        "body_html": "<p>hi</p>",
        "body_text": "hi",
        "importance": "high",
    }))
    .await
    .expect("a complete send must map");

    assert_eq!(out["payload"]["action"], json!("send"));
    // No `external_id`: a fresh send addresses no existing provider object.
    assert!(out.get("external_id").is_none() || out["external_id"].is_null());

    let message = &out["payload"]["body"]["message"];
    assert_eq!(message["subject"], json!("hello"));
    assert_eq!(message["body"]["contentType"], json!("HTML"));
    assert_eq!(message["body"]["content"], json!("<p>hi</p>"));
    assert_eq!(
        message["toRecipients"],
        json!([
            { "emailAddress": { "address": "a@b.com" } },
            { "emailAddress": { "address": "c@d.com", "name": "C D" } },
        ]),
        "both a bare address and an object form must be accepted: a compose UI \
         produces one and an import produces the other"
    );
    assert_eq!(
        message["ccRecipients"][0]["emailAddress"]["address"],
        json!("e@f.com")
    );
    assert_eq!(
        message["bccRecipients"][0]["emailAddress"]["address"],
        json!("g@h.com")
    );
    assert_eq!(message["importance"], json!("high"));
    // Stated, not inherited from a Graph default that could change: a sent
    // message that is not filed is one the user cannot find afterwards.
    assert_eq!(out["payload"]["body"]["saveToSentItems"], json!(true));
}

/// A reply carries the provider's own message id and nothing else has to know
/// about threading — the provider quotes, threads and addresses it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reply_addresses_the_provider_message_it_answers() {
    let out = to_external(json!({
        "action": "reply",
        "in_reply_to_external_id": "AAMkTARGET=",
        "body_text": "sounds good",
    }))
    .await
    .expect("a reply must map");
    assert_eq!(out["payload"]["action"], json!("reply"));
    assert_eq!(out["external_id"], json!("AAMkTARGET="));
    assert_eq!(
        out["payload"]["body"]["message"]["body"]["contentType"],
        json!("Text")
    );
    // No recipients: a reply INHERITS them, which is the whole reason the
    // provider's own reply action is used instead of composing one here.
    assert!(out["payload"]["body"]["message"]
        .get("toRecipients")
        .is_none());
}

/// An RSVP maps through the same mapper, because it is the same protocol.
///
/// `send_response` is emitted EXPLICITLY, both ways. Notifying the organizer is
/// the externally-visible half of an RSVP, so which way it goes must be readable
/// on the request rather than inherited from a provider default.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_rsvp_maps_through_the_same_mapper_and_states_send_response() {
    let out = to_external(json!({
        "action": "tentative",
        "target_external_id": "EVT1",
        "comment": "might be late",
    }))
    .await
    .expect("an rsvp must map");
    assert_eq!(out["payload"]["action"], json!("tentative"));
    assert_eq!(out["external_id"], json!("EVT1"));
    assert_eq!(out["payload"]["body"]["comment"], json!("might be late"));
    assert_eq!(out["payload"]["body"]["sendResponse"], json!(true));

    let quiet = to_external(json!({
        "action": "decline",
        "target_external_id": "EVT1",
        "send_response": false,
    }))
    .await
    .expect("a silent decline must map");
    assert_eq!(quiet["payload"]["body"]["sendResponse"], json!(false));
}

/// Every command the mapper cannot translate answers NULL rather than a guess.
///
/// Null is not a failure here — it is the answer the engine turns into a
/// terminal `failed` with a stated reason. A guess, by contrast, is an email to
/// nobody, an RSVP to nothing, or a forward with no recipient, and the engine
/// cannot tell any of them from a real command.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_untranslatable_command_answers_null_rather_than_guessing() {
    let cases = [
        ("no action at all", json!({ "subject": "orphan" })),
        (
            "an action this mapper does not know",
            json!({ "action": "publish", "to": ["a@b.com"] }),
        ),
        (
            "a send with nowhere to send it",
            json!({ "action": "send", "subject": "hi", "body_text": "hi" }),
        ),
        (
            "a reply with no message to answer",
            json!({ "action": "reply", "body_text": "hi" }),
        ),
        (
            "a forward with no recipient",
            json!({ "action": "forward", "in_reply_to_external_id": "AAMk1" }),
        ),
        (
            "an rsvp with no event",
            json!({ "action": "accept", "comment": "yes" }),
        ),
    ];
    for (label, properties) in cases {
        assert!(
            to_external(properties).await.is_none(),
            "{label} was translated instead of refused"
        );
    }
}
