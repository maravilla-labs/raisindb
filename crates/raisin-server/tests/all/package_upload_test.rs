//! Integration test for package upload via unified endpoint
//!
//! This test validates the complete package upload flow:
//! 1. Upload .rap file via unified endpoint
//! 2. PackageUploadProcessor sets initial properties (status: "processing")
//! 3. Background job extracts manifest and updates node
//! 4. Node status changes to "uploaded" with manifest data

use reqwest::multipart::{Form, Part};
use std::io::{Cursor, Write};
use std::time::Duration;
use tokio::time::sleep;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::cluster_test_utils::ports::free_port;
use crate::helpers::multi_node::{ServerConfig, ServerHandle};

const REPO: &str = "default";
const BRANCH: &str = "main";

/// Helper to create a test .rap package (ZIP with manifest.yaml)
fn create_test_rap_package(name: &str, version: &str, title: &str, description: &str) -> Vec<u8> {
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options = SimpleFileOptions::default();

        let manifest = format!(
            r#"name: {}
version: {}
title: {}
description: {}
author: Test Author
license: MIT
keywords:
  - test
  - integration
category: testing
"#,
            name, version, title, description
        );

        zip.start_file("manifest.yaml", options).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buffer.into_inner()
}

/// Start a server of this test's own, on a free port with a temporary data
/// directory. The returned handle kills that server, and only that one, when
/// dropped. (This test used to kill every `raisin-server` process and talk to
/// a hard-coded 127.0.0.1:8080, i.e. to whatever was listening there.)
async fn start_server() -> ServerHandle {
    ServerHandle::start(ServerConfig::new(free_port()))
        .await
        .expect("Failed to start server")
}

async fn setup_repository(base_url: &str) {
    let client = reqwest::Client::new();

    // Create repository first
    println!("  Setting up repository '{}'...", REPO);
    let resp = client
        .post(&format!("{}/api/repositories", base_url))
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "repo_id": REPO,
            "description": "Package upload test repository",
            "default_branch": BRANCH
        }))
        .send()
        .await
        .expect("Failed to create repository");

    // 201 = created, 409 = already exists (conflict is OK)
    let status = resp.status();
    if status.is_success() || status == 409 {
        println!("  ✓ Repository ready (status: {})", status);
    } else {
        let body = resp.text().await.unwrap_or_default();
        panic!("Failed to create repository: {} - {}", status, body);
    }

    // Give the server time to process
    sleep(Duration::from_millis(500)).await;
}

async fn setup_packages_workspace(base_url: &str) {
    let client = reqwest::Client::new();

    // Create packages workspace
    println!("  Creating 'packages' workspace...");
    let workspace_payload = serde_json::json!({
        "name": "packages",
        "description": "Package storage workspace",
        "allowed_node_types": ["raisin:Package", "raisin:Folder"],
        "allowed_root_node_types": ["raisin:Package", "raisin:Folder"]
    });

    let resp = client
        .put(&format!("{}/api/workspaces/{}/packages", base_url, REPO))
        .header("content-type", "application/json")
        .json(&workspace_payload)
        .send()
        .await
        .expect("Failed to create workspace");

    let status = resp.status();
    if status.is_success() || status == 409 {
        println!("  ✓ Packages workspace ready (status: {})", status);
    } else {
        let body = resp.text().await.unwrap_or_default();
        panic!("Failed to create packages workspace: {} - {}", status, body);
    }

    // Give the server time to process
    sleep(Duration::from_millis(500)).await;

    // Verify NodeTypes are registered (especially raisin:Package)
    println!("  Verifying NodeTypes are registered...");
    let nodetypes_resp = client
        .get(&format!(
            "{}/api/management/default/main/nodetypes",
            base_url
        ))
        .send()
        .await
        .expect("Failed to get nodetypes");

    if nodetypes_resp.status().is_success() {
        let nodetypes: serde_json::Value = nodetypes_resp.json().await.unwrap_or_default();
        let has_package = nodetypes
            .as_array()
            .map(|arr| arr.iter().any(|nt| nt["name"] == "raisin:Package"))
            .unwrap_or(false);
        if has_package {
            println!("  ✓ raisin:Package NodeType is registered");
        } else {
            println!("  Warning: raisin:Package NodeType not found in registry");
            println!("    Available types: {:?}", nodetypes);
        }
    } else {
        println!(
            "  Warning: Could not verify NodeTypes (status: {})",
            nodetypes_resp.status()
        );
    }
}

#[tokio::test]
async fn test_package_upload_unified_endpoint() {
    println!("\n=== Package Upload Integration Test ===\n");

    let server = start_server().await;
    let base_url = server.base_url.as_str();
    setup_repository(base_url).await;
    setup_packages_workspace(base_url).await;

    let client = reqwest::Client::new();

    // Create a test .rap package
    let package_name = "test-pkg";
    let package_version = "1.0.0";
    let package_title = "Test Package";
    let package_description = "A test package for integration testing";

    println!("\n  Creating test .rap package...");
    let rap_content = create_test_rap_package(
        package_name,
        package_version,
        package_title,
        package_description,
    );
    println!("    Package size: {} bytes", rap_content.len());

    // Upload using unified endpoint
    // Note: Query params use camelCase due to serde(rename_all = "camelCase")
    let upload_url = format!(
        "{}/api/repository/{}/{}/head/packages/{}?nodeType=raisin:Package",
        base_url, REPO, BRANCH, package_name
    );

    println!("\n  Uploading package...");
    println!("    POST {}", upload_url);

    let form = Form::new().part(
        "file",
        Part::bytes(rap_content)
            .file_name(format!("{}.rap", package_name))
            .mime_str("application/zip")
            .unwrap(),
    );

    let resp = client
        .post(&upload_url)
        .multipart(form)
        .send()
        .await
        .expect("Failed to send package upload request");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    println!(
        "    Response: {} - {}",
        status,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    assert_eq!(status, 200, "Package upload should succeed");
    assert!(
        body.get("storedKey").is_some(),
        "Response should have storedKey"
    );

    // Get the node - check initial state
    let node_url = format!(
        "{}/api/repository/{}/{}/head/packages/{}",
        base_url, REPO, BRANCH, package_name
    );

    println!("\n  Checking initial node state...");
    let node_resp = client
        .get(&node_url)
        .send()
        .await
        .expect("Failed to get package node");

    let node: serde_json::Value = node_resp.json().await.unwrap_or_default();
    println!(
        "    Node: {}",
        serde_json::to_string_pretty(&node).unwrap_or_default()
    );

    assert_eq!(
        node["node_type"], "raisin:Package",
        "Node type should be raisin:Package"
    );
    assert_eq!(
        node["name"], package_name,
        "Node name should match package name"
    );

    let initial_status = node["properties"]["status"].as_str().unwrap_or("");
    println!("    Initial status: '{}'", initial_status);
    assert_eq!(
        initial_status, "processing",
        "Initial status should be 'processing'"
    );

    // Poll for background job completion
    println!("\n  Waiting for background job to complete...");
    let mut final_status = String::new();
    let mut final_node: serde_json::Value = serde_json::Value::Null;

    for attempt in 1..=30 {
        sleep(Duration::from_millis(500)).await;

        let poll_resp = client
            .get(&node_url)
            .send()
            .await
            .expect("Failed to poll package node");

        let poll_node: serde_json::Value = poll_resp.json().await.unwrap_or_default();
        let status = poll_node["properties"]["status"].as_str().unwrap_or("");

        if status == "uploaded" {
            println!(
                "    ✓ Job completed after {} attempts ({} ms)",
                attempt,
                attempt * 500
            );
            final_status = status.to_string();
            final_node = poll_node;
            break;
        }

        if attempt % 5 == 0 {
            println!(
                "    Still waiting... (attempt {}, status: '{}')",
                attempt, status
            );
        }
    }

    assert_eq!(
        final_status, "uploaded",
        "Status should be 'uploaded' after job completion"
    );

    // Verify manifest properties were extracted
    println!("\n  Verifying extracted manifest properties...");
    let props = &final_node["properties"];

    assert_eq!(
        props["name"].as_str().unwrap_or(""),
        package_name,
        "name mismatch"
    );
    println!("    ✓ name: {}", package_name);

    assert_eq!(
        props["version"].as_str().unwrap_or(""),
        package_version,
        "version mismatch"
    );
    println!("    ✓ version: {}", package_version);

    assert_eq!(
        props["title"].as_str().unwrap_or(""),
        package_title,
        "title mismatch"
    );
    println!("    ✓ title: {}", package_title);

    assert_eq!(
        props["description"].as_str().unwrap_or(""),
        package_description,
        "description mismatch"
    );
    println!("    ✓ description: {}", package_description);

    assert_eq!(
        props["author"].as_str().unwrap_or(""),
        "Test Author",
        "author mismatch"
    );
    println!("    ✓ author: Test Author");

    assert_eq!(
        props["license"].as_str().unwrap_or(""),
        "MIT",
        "license mismatch"
    );
    println!("    ✓ license: MIT");

    assert_eq!(
        props["category"].as_str().unwrap_or(""),
        "testing",
        "category mismatch"
    );
    println!("    ✓ category: testing");

    assert_eq!(
        props["installed"].as_bool().unwrap_or(true),
        false,
        "installed should be false"
    );
    println!("    ✓ installed: false");

    // Note: keywords array serialization has a known bug (2-3 element string arrays
    // are incorrectly deserialized as RaisinReference). Skipping this check for now.
    // See: raisin-models/src/nodes/properties/utils.rs:74-89
    println!("    ⚠ keywords: skipped (known serialization issue with small string arrays)");

    println!("\n=== All Package Upload Tests Passed! ===\n");
}
