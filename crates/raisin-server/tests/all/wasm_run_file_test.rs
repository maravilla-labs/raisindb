//! End-to-end WebAssembly function test against a real server process.
//!
//! Proves the two things only a running server can show:
//!
//! 1. `POST /api/files/{repo}/run` executes an uploaded `.wasm` **component**
//!    by `node_id` and streams the ordinary SSE events. That is the server half
//!    of the CLI dev loop, so the event shape is asserted, not just the output.
//! 2. ONE uploaded artifact serves TWO `raisin:Function` nodes — the second
//!    reaches it through a parent-relative `entry_file`
//!    (`../wasm-echo/main.wasm:reverse`) and runs the *other* handler — and the
//!    server compiles it exactly once.
//!
//! Run with:
//!   cargo test -p raisin-server --test all wasm_run_file_test -- --ignored --nocapture

#[allow(unused_imports)]
use crate::helpers;
use std::time::Duration;

use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::{multipart, Client};
use serde_json::{json, Value};

const REPO: &str = "wasmfn";
const TENANT: &str = "default";
const ADMIN_USER: &str = "admin";
const ADMIN_PASS: &str = "Admin12345!@#";
const PORT: u16 = 8112;

/// The same component `raisin-functions`' unit tests run, built from
/// `fixtures/wasm-guests/echo` (see that workspace's README). It registers two
/// handlers, `default` and `reverse`, which is what makes it the right fixture
/// for the one-artifact-two-functions half of this test.
const ECHO_WASM: &[u8] =
    include_bytes!("../../../raisin-functions/src/runtime/wasm/fixtures/echo.wasm");

// ---------------------------------------------------------------------------
// SSE
// ---------------------------------------------------------------------------

/// Pull the `data:` payload of the first event of `name` out of an SSE body.
///
/// The whole body is read first: `run_file` closes the stream after `done`, so
/// there is nothing to stream incrementally, and a text scan keeps the assertion
/// on the wire format itself (a renamed event fails this test, which is the
/// point — the CLI parses these names).
fn sse_event(body: &str, name: &str) -> Option<Value> {
    let mut current_event: Option<&str> = None;
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            current_event = Some(rest.trim());
        } else if let Some(rest) = line.strip_prefix("data:") {
            if current_event == Some(name) {
                return serde_json::from_str(rest.trim()).ok();
            }
        }
    }
    None
}

/// Run a file by node id and return the whole SSE body.
async fn run_file(
    client: &Client,
    base_url: &str,
    token: &str,
    node_id: &str,
    handler: &str,
    input: Value,
) -> String {
    let response = client
        .post(format!("{}/api/files/{}/run", base_url, REPO))
        .bearer_auth(token)
        .json(&json!({
            "node_id": node_id,
            "handler": handler,
            "input": input,
        }))
        .send()
        .await
        .expect("run_file request failed");

    assert!(
        response.status().is_success(),
        "run_file returned {}",
        response.status()
    );
    response.text().await.expect("run_file body was not text")
}

/// How many times the server compiled a wasm component since boot.
///
/// `compile_count()` lives in the SERVER process, so an out-of-process test
/// cannot call it; the cache logs one line per compile at debug, which is the
/// same event. The server is started with that target at `debug` for exactly
/// this assertion.
fn compiles_logged() -> usize {
    let path = std::env::var("RAISIN_TEST_SERVER_LOG")
        .unwrap_or_else(|_| format!("/tmp/raisin-test-server-{}.log", PORT));
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .matches("Compiled wasm component")
        .count()
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

async fn create_node(client: &Client, base_url: &str, token: &str, parent: &str, node: Value) {
    let response = client
        .post(format!(
            "{}/api/repository/{}/main/head/functions/{}",
            base_url, REPO, parent
        ))
        .bearer_auth(token)
        .json(&json!({ "node": node }))
        .send()
        .await
        .expect("create node request failed");
    assert!(
        response.status().is_success(),
        "create node failed: {}",
        response.text().await.unwrap_or_default()
    );
}

/// The `raisin:Function` node shape a wasm function has: no source, an
/// `entry_file` naming the artifact and (optionally) the handler inside it.
fn function_node(name: &str, entry_file: &str) -> Value {
    json!({
        "name": name,
        "node_type": "raisin:Function",
        "properties": {
            "name": name,
            "title": name,
            "language": "wasm",
            "entry_file": entry_file,
            "execution_mode": "both",
            "enabled": true,
        }
    })
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // Boots a real server process
async fn wasm_component_runs_by_node_id_and_one_artifact_serves_two_functions() {
    println!("\n🧪 wasm run-file + shared artifact\n");

    let config = ServerConfig::new(PORT).with_rust_log("info,raisin_functions=debug");
    let server = ServerHandle::start(config)
        .await
        .expect("Failed to start server");

    // The admin user is created asynchronously during startup - retry auth.
    let mut token = None;
    for _ in 0..30 {
        match authenticate(&server.base_url, TENANT, ADMIN_USER, ADMIN_PASS).await {
            Ok(t) => {
                token = Some(t);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    let token = token.expect("Failed to authenticate within 15s");
    let client = Client::new();

    let repo_response = client
        .post(format!("{}/api/repositories", server.base_url))
        .bearer_auth(&token)
        .json(&json!({ "repo_id": REPO, "description": "wasm function e2e" }))
        .send()
        .await
        .expect("create repo request failed");
    assert!(
        repo_response.status().is_success(),
        "repo creation failed: {}",
        repo_response.text().await.unwrap_or_default()
    );

    // `/lib` is SEEDED by the functions workspace definition
    // (raisin-core/global_workspaces/functions.yaml:20), so creating it
    // normally conflicts — asserting on the create succeeding is what made an
    // earlier version of this test fail for 60s against a healthy server. The
    // create stays as a best-effort fallback; the precondition that actually
    // matters is that the folder is READABLE. Node types initialize with the
    // repo, so the retry still covers that race.
    let mut folder_ready = false;
    for _ in 0..60 {
        let existing = client
            .get(format!(
                "{}/api/repository/{}/main/head/functions/lib",
                server.base_url, REPO
            ))
            .bearer_auth(&token)
            .send()
            .await
            .expect("read folder request failed");
        if existing.status().is_success() {
            folder_ready = true;
            break;
        }
        let _ = client
            .post(format!(
                "{}/api/repository/{}/main/head/functions/",
                server.base_url, REPO
            ))
            .bearer_auth(&token)
            .json(&json!({
                "node": { "name": "lib", "node_type": "raisin:Folder", "properties": {} }
            }))
            .send()
            .await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(folder_ready, "/lib never became readable");

    // Two Function nodes, ONE artifact. The second points at the first's
    // `main.wasm` through a parent-relative entry_file and selects the other
    // handler — the storage-dedup path the name-routed export exists for.
    create_node(
        &client,
        &server.base_url,
        &token,
        "lib",
        function_node("wasm-echo", "main.wasm"),
    )
    .await;
    create_node(
        &client,
        &server.base_url,
        &token,
        "lib",
        function_node("wasm-echo-alt", "../wasm-echo/main.wasm:reverse"),
    )
    .await;
    println!("✅ Function nodes created");

    // Upload the component under the first function. This is the only place the
    // bytes exist: `wasm-echo-alt` never gets its own copy.
    let form = multipart::Form::new().part(
        "file",
        multipart::Part::bytes(ECHO_WASM.to_vec())
            .file_name("main.wasm")
            .mime_str("application/wasm")
            .unwrap(),
    );
    let upload = client
        .post(format!(
            "{}/api/repository/{}/main/head/functions/lib/wasm-echo/main.wasm?override_existing=true",
            server.base_url, REPO
        ))
        .bearer_auth(&token)
        .multipart(form)
        .send()
        .await
        .expect("upload request failed");
    assert!(
        upload.status().is_success(),
        "artifact upload failed: {}",
        upload.text().await.unwrap_or_default()
    );
    println!("✅ Artifact uploaded ({} bytes)", ECHO_WASM.len());

    let asset: Value = client
        .get(format!(
            "{}/api/repository/{}/main/head/functions/lib/wasm-echo/main.wasm",
            server.base_url, REPO
        ))
        .bearer_auth(&token)
        .send()
        .await
        .expect("asset lookup failed")
        .json()
        .await
        .expect("asset response was not JSON");
    let asset_id = asset["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no asset id in {asset}"))
        .to_string();

    // 1. Run the component by node id.
    let body = run_file(
        &client,
        &server.base_url,
        &token,
        &asset_id,
        "default",
        json!({ "name": "Ada" }),
    )
    .await;

    // The SSE contract the CLI dev loop parses.
    let started = sse_event(&body, "started").expect("no started event");
    assert_eq!(started["file_name"], "main.wasm");
    assert!(sse_event(&body, "done").is_some(), "no done event");

    let result = sse_event(&body, "result").expect("no result event");
    assert_eq!(result["success"], true, "run failed: {result}");
    assert_eq!(
        result["result"]["handler"], "default",
        "wrong handler ran: {result}"
    );
    assert_eq!(result["result"]["echo"]["name"], "Ada");
    println!("✅ default handler ran through run-file");

    // 2. The other handler, same artifact, same node id — the handler name is
    //    data, not a second upload.
    let body = run_file(
        &client,
        &server.base_url,
        &token,
        &asset_id,
        "reverse",
        json!({ "name": "Ada" }),
    )
    .await;
    let result = sse_event(&body, "result").expect("no result event");
    assert_eq!(result["success"], true, "reverse run failed: {result}");
    assert_eq!(result["result"]["handler"], "reverse");

    // 3. An unknown handler is the GUEST's error, naming what it registered.
    //    The host must never keep an allow-list of handler names.
    let body = run_file(
        &client,
        &server.base_url,
        &token,
        &asset_id,
        "nope",
        json!({}),
    )
    .await;
    let result = sse_event(&body, "result").expect("no result event");
    assert_eq!(result["success"], false, "unknown handler should fail");
    let error = result["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("nope") && error.contains("reverse"),
        "the guest should name its registered handlers: {error}"
    );
    println!("✅ unknown handler reported by the guest");

    // 4. The SECOND function node, invoked by name, resolves
    //    `../wasm-echo/main.wasm:reverse` to the artifact uploaded above.
    let response = client
        .post(format!(
            "{}/api/functions/{}/wasm-echo-alt/invoke",
            server.base_url, REPO
        ))
        .bearer_auth(&token)
        .json(&json!({ "input": { "name": "Grace" }, "sync": true }))
        .send()
        .await
        .expect("invoke request failed");
    assert!(
        response.status().is_success(),
        "invoke failed: {}",
        response.text().await.unwrap_or_default()
    );
    let invoked: Value = response.json().await.expect("invoke body was not JSON");
    assert!(
        invoked["error"].is_null(),
        "invoke reported an error: {invoked}"
    );
    assert_eq!(
        invoked["result"]["handler"], "reverse",
        "the shared artifact ran the wrong handler: {invoked}"
    );
    assert_eq!(invoked["result"]["echo"]["name"], "Grace");
    println!("✅ second function ran the other handler from the SAME artifact");

    // 5. Four executions of one artifact, one compile. A second compile here
    //    would mean the cache is keyed by something other than the bytes.
    let compiles = compiles_logged();
    assert_eq!(
        compiles, 1,
        "expected exactly one compile of the shared artifact, saw {compiles}"
    );
    println!("✅ compiled once for four executions");

    drop(server);
}
