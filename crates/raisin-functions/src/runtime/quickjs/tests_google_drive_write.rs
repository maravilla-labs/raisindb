// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The google-drive adapter's WRITE path, executed as the real file in QuickJS.
//!
//! Drive is the adapter the mirror mode was designed against, and it had no
//! tests at all. The defect that prompted these: `delete` accepted the engine's
//! `etag` — the concurrency base captured from the node's MVCC pre-image — and
//! IGNORED it, so a file someone else edited after the last sync was deleted
//! anyway. Nothing reported it: the engine's blast-radius rails were satisfied
//! (one node, one delete), so the only evidence was the missing file.

use super::*;
use crate::api::MockFunctionApi;
use serde_json::{json, Value};

fn adapter_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../builtin-packages/google-drive-adapter/content/functions/adapters/google-drive/index.js",
    );
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read adapter at {}: {e}", path.display()))
}

struct Run {
    output: Option<Value>,
    error: Option<String>,
    calls: Vec<Value>,
}

async fn call_adapter(input: Value, responses: Vec<Value>) -> Run {
    let runtime = QuickJsRuntime::new();
    let api = Arc::new(MockFunctionApi::new(json!({})).with_http_responses(responses));
    let context = ExecutionContext::new("t1", "r1", "main", "tester").with_input(input);
    let metadata = FunctionMetadata::javascript("google_drive_adapter");
    let result = runtime
        .execute(
            &adapter_source(),
            "handler",
            context,
            &metadata,
            api.clone() as Arc<dyn crate::api::FunctionApi>,
            HashMap::new(),
        )
        .await
        .expect("runtime execute");
    Run {
        output: result.output,
        error: result.error.map(|e| e.message),
        calls: api.http_calls(),
    }
}

fn input(op: &str, params: Value) -> Value {
    json!({
        "operation": op,
        "credential": { "access_token": "TOKEN123" },
        "mount": { "mount_id": "m1", "remote_root": "FOLDER1", "sync_config": {} },
        "params": params,
    })
}

fn ok(body: Value) -> Value {
    json!({ "status": 200, "headers": {}, "body": body })
}

fn not_found() -> Value {
    json!({ "status": 404, "headers": {}, "body": { "error": { "message": "File not found" } } })
}

// ---------------------------------------------------------------------------

/// THE HEADLINE. A delete must consult the concurrency base the engine sent.
///
/// Drive has no conditional request, so the check is a read-then-compare against
/// the file's `version`. It does not close the race — a change landing between
/// the GET and the DELETE is not seen — but it catches the case it exists for:
/// the file changed since the mount last read it, so the local delete was
/// decided against a stale view of it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delete_refuses_when_the_file_changed_since_the_mount_last_saw_it() {
    let run = call_adapter(
        input(
            "delete",
            json!({ "item_id": "F1", "policy": "purge", "etag": "7" }),
        ),
        vec![ok(json!({ "version": "9" }))],
    )
    .await;
    let err = run.error.expect("a changed file must not be deleted");
    assert!(err.contains("[code=conflict]"), "{err}");
    assert_eq!(
        run.calls.len(),
        1,
        "the version probe ran and nothing else did: {:#?}",
        run.calls
    );
    assert!(
        !run.calls.iter().any(|c| c["method"] == json!("DELETE")),
        "a DELETE was issued despite the conflict: {:#?}",
        run.calls
    );
}

/// The same check on the update path, which is where it already lived. Both
/// writes now go through ONE implementation — two writes with two different
/// answers to "has this changed?" is the drift this codebase pays for most.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_update_refuses_on_the_same_mismatch() {
    let run = call_adapter(
        input(
            "update",
            json!({ "item_id": "F1", "payload": { "name": "new.txt" }, "etag": "7" }),
        ),
        vec![ok(json!({ "version": "9" }))],
    )
    .await;
    let err = run.error.expect("a changed file must not be patched");
    assert!(err.contains("[code=conflict]"), "{err}");
    assert!(!run.calls.iter().any(|c| c["method"] == json!("PATCH")));
}

/// A matching version lets the write through — and `trash` really trashes
/// (`trashed: true`, reversible from the Drive UI for 30 days) rather than
/// issuing the irreversible DELETE. An adapter that treated the two the same
/// would make `supports_trash` a lie.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_matching_version_lets_the_delete_through_under_its_policy() {
    let run = call_adapter(
        input(
            "delete",
            json!({ "item_id": "F1", "policy": "trash", "etag": "7" }),
        ),
        vec![ok(json!({ "version": "7" })), ok(json!({ "id": "F1" }))],
    )
    .await;
    assert!(run.error.is_none(), "{:?}", run.error);
    let write = &run.calls[1];
    assert_eq!(write["method"], json!("PATCH"));
    assert_eq!(write["options"]["body"]["trashed"], json!(true));
    let out = run.output.expect("output");
    assert_eq!(out["deleted"], json!(true));
    assert_eq!(out["trashed"], json!(true));

    let run = call_adapter(
        input(
            "delete",
            json!({ "item_id": "F1", "policy": "purge", "etag": "7" }),
        ),
        vec![ok(json!({ "version": "7" })), ok(json!({}))],
    )
    .await;
    assert_eq!(run.calls[1]["method"], json!("DELETE"));
    assert_eq!(run.output.expect("output")["deleted"], json!(true));
}

/// A delete with NO etag still works: the engine sends one only when the
/// pre-image carried it, and demanding it would break every mount that predates
/// the field. No probe is issued in that case — a needless GET per delete on a
/// mount syncing thousands of files.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delete_without_an_etag_issues_no_probe() {
    let run = call_adapter(
        input("delete", json!({ "item_id": "F1", "policy": "purge" })),
        vec![ok(json!({}))],
    )
    .await;
    assert_eq!(run.calls.len(), 1, "{:#?}", run.calls);
    assert_eq!(run.calls[0]["method"], json!("DELETE"));
    assert_eq!(run.output.expect("output")["deleted"], json!(true));
}

/// A file that is already gone is SUCCESS for a delete and SETTLES an update.
///
/// The probe is where this now surfaces, and it was previously a plain Error
/// there — i.e. `Transient`, i.e. retried on every drain forever against an id
/// that can never come back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_vanished_file_completes_a_delete_and_settles_an_update() {
    let run = call_adapter(
        input(
            "delete",
            json!({ "item_id": "F1", "policy": "purge", "etag": "7" }),
        ),
        vec![not_found()],
    )
    .await;
    assert!(run.error.is_none(), "{:?}", run.error);
    assert_eq!(run.output.expect("output")["deleted"], json!(true));

    let run = call_adapter(
        input(
            "update",
            json!({ "item_id": "F1", "payload": { "name": "x" }, "etag": "7" }),
        ),
        vec![not_found()],
    )
    .await;
    assert!(run.error.is_none(), "{:?}", run.error);
    assert_eq!(run.output, Some(Value::Null));
}

/// An empty PATCH still bumps the file's `version`, which invalidates every
/// stored etag and makes the next delta re-deliver the file — on a mirror that
/// is one revision per file per drain, forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_update_is_refused_before_it_reaches_drive() {
    let run = call_adapter(
        input("update", json!({ "item_id": "F1", "payload": {} })),
        vec![],
    )
    .await;
    let err = run.error.expect("must throw");
    assert!(err.contains("[code=config_error]"), "{err}");
    assert!(run.calls.is_empty(), "an empty PATCH reached Drive");
}

/// `detach` is the default and it means "push nothing" — the engine never calls
/// `delete` for it. What the adapter must not do is claim a policy it cannot
/// serve: Drive HAS a real trash, so `supports_trash` is true, which is what
/// lets a mount choose `trash` at all. Without it the engine refuses the policy
/// rather than quietly promoting it to a permanent delete.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drive_declares_the_full_mirror_surface_and_a_safe_default() {
    let caps = call_adapter(json!({ "operation": "capabilities", "mount": {} }), vec![])
        .await
        .output
        .expect("capabilities");
    assert_eq!(caps["can_create"], json!(true));
    assert_eq!(caps["can_update"], json!(true));
    assert_eq!(caps["can_delete"], json!(true));
    assert_eq!(caps["supports_trash"], json!(true));
    // A mount is frequently a read-mostly view of a SHARED folder, and a node
    // deleted to tidy a workspace must not bin a colleague's file.
    assert_eq!(caps["default_delete_policy"], json!("detach"));
    assert_eq!(caps["mutable_fields"], json!(["title"]));
}
