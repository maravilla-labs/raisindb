// End-to-end workflow test via the REST API:
// create a raisin:Flow -> run it -> flow waits on a human task ->
// complete the task via the inbox API -> flow resumes and completes.
//
// Run with:
//   cargo test --package raisin-server --test flow_e2e_test -- --ignored --nocapture

mod helpers;

use std::time::Duration;

use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::Client;
use serde_json::{json, Value};

const REPO: &str = "workspace";
const TENANT: &str = "default";
const ADMIN_USER: &str = "admin";
const ADMIN_PASS: &str = "Admin12345!@#";

async fn poll_instance_status(
    client: &Client,
    base_url: &str,
    token: &str,
    instance_id: &str,
    want: &str,
    timeout: Duration,
) -> Value {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let response = client
            .get(format!(
                "{}/api/flows/{}/instances/{}",
                base_url, REPO, instance_id
            ))
            .bearer_auth(token)
            .send()
            .await
            .expect("instance status request failed");

        if response.status().is_success() {
            let body: Value = response.json().await.expect("invalid status JSON");
            if body["status"] == want {
                return body;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "Timed out waiting for instance {} to reach '{}'. Last: {}",
                    instance_id, want, body
                );
            }
        } else if std::time::Instant::now() > deadline {
            panic!(
                "Timed out waiting for instance {} (status endpoint: {})",
                instance_id,
                response.status()
            );
        }

        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[tokio::test]
#[ignore] // Run with --include-ignored (boots a real server)
async fn test_human_in_the_loop_flow_via_rest_api() {
    println!("\n🧪 Testing human-in-the-loop flow via REST API\n");

    let config = ServerConfig::new(8095);
    let server = ServerHandle::start(config)
        .await
        .expect("Failed to start server");
    println!("✅ Server started");

    // The admin user is created asynchronously during startup - retry auth
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
    println!("✅ Authenticated");

    // ------------------------------------------------------------------
    // 0. Create the repository (initializes node types incl. raisin:Flow)
    // ------------------------------------------------------------------
    let client = Client::new();
    let repo_response = client
        .post(format!("{}/api/repositories", server.base_url))
        .bearer_auth(&token)
        .json(&json!({ "repo_id": REPO, "description": "flow e2e test repo" }))
        .send()
        .await
        .expect("create repo request failed");
    assert!(
        repo_response.status().is_success(),
        "repo creation failed: {}",
        repo_response.text().await.unwrap_or_default()
    );
    println!("✅ Repository created");

    // ------------------------------------------------------------------
    // 1. Create the flow definition (human approval with template title)
    // ------------------------------------------------------------------
    let workflow_data = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "approve" },
            {
                "id": "approve",
                "step_type": "human_task",
                "properties": {
                    "task_type": "approval",
                    "title": "Approve order {{ input.order_id }}",
                    "description": "Please review this order",
                    "assignee": "/users/admin",
                    "priority": 4,
                    "options": [
                        {"value": "approve", "label": "Approve", "style": "success"},
                        {"value": "reject", "label": "Reject", "style": "danger"}
                    ]
                },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    // The functions workspace only allows folders at root - ensure /flows
    // exists (node types initialize with the repo; retry covers the race)
    let mut created = false;
    let mut last_err = String::new();
    for _ in 0..60 {
        // Folder may already exist from workspace init - ignore failures
        let _ = client
            .post(format!(
                "{}/api/repository/{}/main/head/functions/",
                server.base_url, REPO
            ))
            .bearer_auth(&token)
            .json(&json!({
                "node": { "name": "flows", "node_type": "raisin:Folder", "properties": {} }
            }))
            .send()
            .await;

        let response = client
            .post(format!(
                "{}/api/repository/{}/main/head/functions/flows",
                server.base_url, REPO
            ))
            .bearer_auth(&token)
            .json(&json!({
                "node": {
                    "name": "e2e-approval-flow",
                    "node_type": "raisin:Flow",
                    "properties": {
                        "name": "e2e-approval-flow",
                        "title": "E2E Approval Flow",
                        "enabled": true,
                        "workflow_data": workflow_data,
                    }
                }
            }))
            .send()
            .await
            .expect("create flow request failed");

        if response.status().is_success() {
            created = true;
            break;
        }
        last_err = response.text().await.unwrap_or_default();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(created, "Failed to create flow node: {}", last_err);
    println!("✅ Flow node created");

    // ------------------------------------------------------------------
    // 2. Run the flow
    // ------------------------------------------------------------------
    let run_response = client
        .post(format!("{}/api/flows/{}/run", server.base_url, REPO))
        .bearer_auth(&token)
        .json(&json!({
            "flow_path": "/flows/e2e-approval-flow",
            "input": { "order_id": "ORD-42" }
        }))
        .send()
        .await
        .expect("run request failed");
    assert!(
        run_response.status().is_success(),
        "run failed: {}",
        run_response.text().await.unwrap_or_default()
    );
    let run_body: Value = run_response.json().await.unwrap();
    let instance_id = run_body["instance_id"].as_str().unwrap().to_string();
    println!("✅ Flow started: {}", instance_id);

    // ------------------------------------------------------------------
    // 3. The flow pauses on the human task
    // ------------------------------------------------------------------
    let status = poll_instance_status(
        &client,
        &server.base_url,
        &token,
        &instance_id,
        "waiting",
        Duration::from_secs(30),
    )
    .await;
    println!("✅ Flow is waiting: {}", status["status"]);

    // ------------------------------------------------------------------
    // 4. The task shows up in the inbox with the resolved template title
    // ------------------------------------------------------------------
    let inbox_response = client
        .get(format!(
            "{}/api/inbox/{}?assignee=/users/admin&status=pending",
            server.base_url, REPO
        ))
        .bearer_auth(&token)
        .send()
        .await
        .expect("inbox request failed");
    assert!(
        inbox_response.status().is_success(),
        "inbox list failed: {}",
        inbox_response.text().await.unwrap_or_default()
    );
    let inbox: Value = inbox_response.json().await.unwrap();
    let tasks = inbox["tasks"].as_array().expect("tasks array");
    let task = tasks
        .iter()
        .find(|t| t["flow_instance_id"] == json!(instance_id))
        .expect("task for this flow instance in the inbox");

    assert_eq!(task["title"], json!("Approve order ORD-42"));
    assert_eq!(task["task_type"], json!("approval"));
    assert_eq!(task["status"], json!("pending"));
    let task_id = task["id"].as_str().expect("task id").to_string();
    println!("✅ Inbox task found: {} ({})", task["title"], task_id);

    // ------------------------------------------------------------------
    // 5. Complete the task -> the flow resumes
    // ------------------------------------------------------------------
    let complete_response = client
        .post(format!(
            "{}/api/inbox/{}/tasks/{}/complete",
            server.base_url, REPO, task_id
        ))
        .bearer_auth(&token)
        .json(&json!({
            "response": { "action": "approve", "comment": "looks good" }
        }))
        .send()
        .await
        .expect("complete request failed");
    assert!(
        complete_response.status().is_success(),
        "task completion failed: {}",
        complete_response.text().await.unwrap_or_default()
    );
    let completion: Value = complete_response.json().await.unwrap();
    assert_eq!(completion["status"], json!("completed"));
    println!("✅ Task completed");

    // Double completion must be rejected (idempotency)
    let second = client
        .post(format!(
            "{}/api/inbox/{}/tasks/{}/complete",
            server.base_url, REPO, task_id
        ))
        .bearer_auth(&token)
        .json(&json!({ "response": { "action": "approve" } }))
        .send()
        .await
        .expect("second complete request failed");
    assert!(
        !second.status().is_success(),
        "double completion must be rejected"
    );
    println!("✅ Double completion rejected");

    // ------------------------------------------------------------------
    // 6. Flow completes with the response visible
    // ------------------------------------------------------------------
    let final_status = poll_instance_status(
        &client,
        &server.base_url,
        &token,
        &instance_id,
        "completed",
        Duration::from_secs(30),
    )
    .await;
    println!("✅ Flow completed");

    let response_action = &final_status["variables"]["__human_response"]["action"];
    assert_eq!(
        response_action,
        &json!("approve"),
        "human response must be in flow variables: {}",
        final_status["variables"]
    );
    // completed_by records the authenticated principal (superadmins
    // resolve to "system")
    assert!(
        final_status["variables"]["__human_response"]["completed_by"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "completed_by must be recorded: {}",
        final_status["variables"]["__human_response"]
    );

    println!("\n🎉 Human-in-the-loop flow E2E passed\n");
}

// ============================================================================
// Node-event trigger -> workflow execution
// ============================================================================

/// A raisin:Trigger with a `function_flow` reference must start the
/// referenced flow when a matching node event occurs - the primary way
/// workflows are launched in production.
#[tokio::test]
#[ignore] // Run with --include-ignored (boots a real server)
async fn test_node_event_trigger_starts_flow() {
    println!("\n🧪 Testing node-event trigger -> workflow\n");

    let config = ServerConfig::new(8096);
    let server = ServerHandle::start(config)
        .await
        .expect("Failed to start server");

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
    let token = token.expect("auth");
    let client = Client::new();

    let repo_response = client
        .post(format!("{}/api/repositories", server.base_url))
        .bearer_auth(&token)
        .json(&json!({ "repo_id": REPO }))
        .send()
        .await
        .expect("create repo");
    assert!(repo_response.status().is_success());
    println!("✅ Repository created");

    // Flow: empty designer-format definition (start -> end). Completion +
    // captured trigger info is what we assert.
    let mut flow_node_id = None;
    for _ in 0..60 {
        let _ = client
            .post(format!(
                "{}/api/repository/{}/main/head/functions/",
                server.base_url, REPO
            ))
            .bearer_auth(&token)
            .json(
                &json!({"node": {"name": "flows", "node_type": "raisin:Folder", "properties": {}}}),
            )
            .send()
            .await;

        let response = client
            .post(format!(
                "{}/api/repository/{}/main/head/functions/flows",
                server.base_url, REPO
            ))
            .bearer_auth(&token)
            .json(&json!({
                "node": {
                    "name": "on-folder-created",
                    "node_type": "raisin:Flow",
                    "properties": {
                        "name": "on-folder-created",
                        "title": "On Folder Created",
                        "enabled": true,
                        "workflow_data": {
                            "nodes": [
                                { "id": "start", "step_type": "start", "next_node": "end" },
                                { "id": "end", "step_type": "end" }
                            ]
                        }
                    }
                }
            }))
            .send()
            .await
            .expect("create flow");
        if response.status().is_success() {
            // The create response may omit the node - fetch it by path
            let fetched = client
                .get(format!(
                    "{}/api/repository/{}/main/head/functions/flows/on-folder-created",
                    server.base_url, REPO
                ))
                .bearer_auth(&token)
                .send()
                .await
                .expect("fetch flow node");
            let body: Value = fetched.json().await.unwrap();
            flow_node_id = body["id"].as_str().map(String::from);
            // Read-after-write can lag on a fresh repo - keep retrying
            // until the node is actually visible
            if flow_node_id.is_some() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let flow_node_id = flow_node_id.expect("flow node id");
    println!("✅ Flow created: {}", flow_node_id);

    // Trigger: fire on raisin:Folder creation in the functions workspace,
    // referencing the flow (triggers live under a folder - workspace root
    // only allows folders)
    let _ = client
        .post(format!(
            "{}/api/repository/{}/main/head/functions/",
            server.base_url, REPO
        ))
        .bearer_auth(&token)
        .json(
            &json!({"node": {"name": "triggers", "node_type": "raisin:Folder", "properties": {}}}),
        )
        .send()
        .await;
    let trigger_response = client
        .post(format!(
            "{}/api/repository/{}/main/head/functions/triggers",
            server.base_url, REPO
        ))
        .bearer_auth(&token)
        .json(&json!({
            "node": {
                "name": "on-folder-created-trigger",
                "node_type": "raisin:Trigger",
                "properties": {
                    "name": "on-folder-created-trigger",
                    "title": "On Folder Created",
                    "trigger_type": "node_event",
                    "enabled": true,
                    "config": { "event_kinds": ["Created"] },
                    "filters": {
                        "workspaces": ["functions"],
                        "node_types": ["raisin:Folder"]
                    },
                    "function_flow": {
                        "raisin:ref": flow_node_id,
                        "raisin:workspace": "functions"
                    }
                }
            }
        }))
        .send()
        .await
        .expect("create trigger");
    assert!(
        trigger_response.status().is_success(),
        "trigger creation failed: {}",
        trigger_response.text().await.unwrap_or_default()
    );
    println!("✅ Trigger created");
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Fire the event: create a folder matching the filters
    let target = client
        .post(format!(
            "{}/api/repository/{}/main/head/functions/",
            server.base_url, REPO
        ))
        .bearer_auth(&token)
        .json(&json!({"node": {"name": "trigger-target", "node_type": "raisin:Folder", "properties": {}}}))
        .send()
        .await
        .expect("create target node");
    assert!(target.status().is_success());
    println!("✅ Matching node created - waiting for the triggered flow");

    // A flow instance should appear and complete. Query via SQL (the
    // ordered-children listing on freshly initialized repos lags - tracked
    // separately; SQL property lookup is the canonical query path anyway).
    let deadline = std::time::Instant::now() + Duration::from_secs(45);
    let mut triggered: Option<Value> = None;
    while std::time::Instant::now() < deadline {
        let response = client
            .post(format!("{}/api/sql/{}", server.base_url, REPO))
            .bearer_auth(&token)
            .json(&json!({
                "sql": "SELECT id, path, properties FROM 'raisin:system' WHERE node_type = 'raisin:FlowInstance'",
                "params": []
            }))
            .send()
            .await
            .expect("sql query");
        if response.status().is_success() {
            let body: Value = response.json().await.unwrap();
            let rows = body["rows"]
                .as_array()
                .cloned()
                .or_else(|| body["data"].as_array().cloned())
                .unwrap_or_default();
            if std::env::var("RAISIN_E2E_DEBUG").is_ok() {
                eprintln!(
                    "sql rows: {} | first: {}",
                    rows.len(),
                    rows.first().map(|r| r.to_string()).unwrap_or_default()
                );
            }
            triggered = rows.into_iter().find(|r| {
                r["properties"]["status"] == json!("completed")
                    || r["properties"]["status"]["String"] == json!("completed")
            });
            if triggered.is_some() {
                break;
            }
        } else if std::env::var("RAISIN_E2E_DEBUG").is_ok() {
            eprintln!("sql status: {}", response.status());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let instance = triggered.expect("triggered flow instance did not complete in time");
    let props = &instance["properties"];
    println!("✅ Triggered flow completed: {}", props["id"]);

    // The trigger context must be captured for template/condition use
    let trigger_info = &props["variables"]["__trigger_info"];
    assert_eq!(trigger_info["node_type"], json!("raisin:Folder"));
    assert_eq!(trigger_info["event_type"], json!("created"));

    println!("\n🎉 Node-event trigger -> workflow E2E passed\n");
}
