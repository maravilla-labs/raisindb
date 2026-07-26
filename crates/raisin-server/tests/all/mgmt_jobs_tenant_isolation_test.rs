//! Integration test: management /jobs endpoints are tenant-isolated.
//!
//! Verifies that a job registered under tenant-a is invisible (404 / empty list)
//! to any request bearing `x-tenant-id: tenant-b`.
//!
//! Run with: `cargo test --package raisin-server --test mgmt_jobs_tenant_isolation_test -- --ignored --nocapture`

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::Client;
use serde_json::json;

/// Schedule an integrity scan for the given tenant via the management API.
/// Returns the created job id (a string).
async fn schedule_integrity_for_tenant(
    base_url: &str,
    token: &str,
    tenant: &str,
) -> Result<String, String> {
    let client = Client::new();
    let url = format!("{}/management/jobs/schedule/integrity", base_url);
    let body = json!({ "interval_minutes": 1u64 });

    let resp = client
        .post(&url)
        .bearer_auth(token)
        .header("x-tenant-id", tenant)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("schedule request failed: {}", e))?;

    let status = resp.status();
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("bad schedule response: {}", e))?;
    if !status.is_success() {
        return Err(format!("schedule failed {} {}", status, v));
    }
    Ok(v["data"]
        .as_str()
        .or_else(|| v.get("ok").and_then(|o| o.as_str()))
        .unwrap_or_default()
        .to_string())
}

#[tokio::test]
#[ignore] // requires running server
async fn jobs_management_is_tenant_isolated() {
    let config = ServerConfig::new(8200);
    let server = ServerHandle::start(config)
        .await
        .expect("failed to start server");

    let token = authenticate(&server.base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("auth failed");

    // Register a job under tenant-a.
    let a_job_id = schedule_integrity_for_tenant(&server.base_url, &token, "tenant-a")
        .await
        .expect("schedule under tenant-a failed");
    assert!(!a_job_id.is_empty(), "expected a job id from schedule");

    let client = Client::new();

    // From tenant-b: GET /management/jobs/{id} should 404 (not visible).
    let url = format!("{}/management/jobs/{}", server.base_url, a_job_id);
    let r = client
        .get(&url)
        .bearer_auth(&token)
        .header("x-tenant-id", "tenant-b")
        .send()
        .await
        .expect("GET status failed");
    assert!(
        r.status().is_client_error() || r.status().is_server_error(),
        "tenant-b should not see tenant-a's job, got {}",
        r.status()
    );

    // From tenant-b: DELETE returns error (404 / 500-with-NotFound).
    let r = client
        .delete(&url)
        .bearer_auth(&token)
        .header("x-tenant-id", "tenant-b")
        .send()
        .await
        .expect("DELETE failed");
    assert!(
        !r.status().is_success(),
        "tenant-b should not delete tenant-a's job"
    );

    // From tenant-b: POST cancel returns error.
    let cancel_url = format!("{}/management/jobs/{}/cancel", server.base_url, a_job_id);
    let r = client
        .post(&cancel_url)
        .bearer_auth(&token)
        .header("x-tenant-id", "tenant-b")
        .send()
        .await
        .expect("cancel failed");
    assert!(
        !r.status().is_success(),
        "tenant-b should not cancel tenant-a's job"
    );

    // From tenant-b: list returns no jobs of tenant-a.
    let list_url = format!("{}/management/jobs", server.base_url);
    let r = client
        .get(&list_url)
        .bearer_auth(&token)
        .header("x-tenant-id", "tenant-b")
        .send()
        .await
        .expect("list failed");
    assert!(r.status().is_success());
    let v: serde_json::Value = r.json().await.expect("bad list response");
    let jobs = v["data"].as_array().expect("expected data array");
    for j in jobs {
        assert_ne!(
            j.get("id").and_then(|x| x.as_str()).unwrap_or_default(),
            a_job_id,
            "tenant-b list leaked tenant-a's job id"
        );
    }
}
