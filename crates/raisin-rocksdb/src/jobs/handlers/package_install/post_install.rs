// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! The `post_install` phase: run the manifest's declared functions after the
//! package is installed.
//!
//! Contract (also in the package docs):
//! - runs on EVERY install, whatever the install mode, and on `main` only;
//! - entries run in order, each as a `FunctionExecution` job with
//!   `AuthContext::system()`, and the next starts only once the previous one
//!   finished (bounded wait);
//! - the first failure stops the phase and fails the install job; the content
//!   stays installed, and re-running the install retries every hook, so hooks
//!   must be idempotent.
//!
//! The install job never holds a transaction or a lock while it waits, and
//! function jobs run in the Realtime worker pool while package installs run in
//! the System pool, so the wait cannot starve the job it is waiting for.

use raisin_error::{Error, Result};
use raisin_models::auth::{agent_identity, AuthContext};
use raisin_packages::PostInstallHook;
use raisin_storage::jobs::{JobContext, JobId, JobStatus, JobType};
use raisin_storage::scope::StorageScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{NodeRepository, Storage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::handler::PackageInstallHandler;
use crate::jobs::{AUTH_CONTEXT_KEY, ORIGIN_AGENT_KEY};

/// The only branch hooks run on. Hooks set up live state (stores, stock,
/// publishing); running them again on every feature branch would duplicate it.
pub const POST_INSTALL_BRANCH: &str = "main";

/// How long one hook may take: the function job watchdog's 300 s plus room
/// for queueing.
pub const POST_INSTALL_HOOK_TIMEOUT: Duration = Duration::from_secs(330);

/// How long the whole phase may take. It runs inside the install job, whose
/// watchdog allows 1800 s for everything; this leaves the content phases room.
pub const POST_INSTALL_PHASE_BUDGET: Duration = Duration::from_secs(1200);

const FUNCTIONS_WORKSPACE: &str = "functions";
const FUNCTION_NODE_TYPE: &str = "raisin:Function";
const MAX_ERROR_LEN: usize = 500;
/// Most warnings kept per hook, and the longest each may be.
const MAX_WARNINGS: usize = 20;
const MAX_WARNING_LEN: usize = 300;
/// Longest rendering of a hook's return value kept on the package node.
const MAX_OUTPUT_LEN: usize = 1_000;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// What happened to one hook. Recorded on the package node as
/// `post_install_results` and returned in the install job result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostInstallResult {
    pub function: String,
    /// `completed`, `failed`, or `skipped` (not run: an earlier hook failed,
    /// or the install is not on `main`).
    pub status: String,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The hook's own `warnings` (a list of strings in its return value),
    /// bounded. What a seed hook reports it deferred or could not do.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The hook's return value as compact JSON, truncated. Absent when the
    /// hook returned nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// A bounded summary of a hook's return value: its `warnings` (strings, or
/// objects carrying a `message`) and the whole value as truncated JSON.
pub(super) fn summarize_output(value: Option<&serde_json::Value>) -> (Vec<String>, Option<String>) {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return (Vec::new(), None);
    };
    let warnings = value
        .get("warnings")
        .and_then(|w| w.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|w| match w {
                    serde_json::Value::String(s) => Some(s.clone()),
                    other => other
                        .get("message")
                        .and_then(|m| m.as_str())
                        .map(str::to_string)
                        .or_else(|| Some(other.to_string())),
                })
                .filter(|w| !w.trim().is_empty())
                .take(MAX_WARNINGS)
                .map(|w| truncate_chars(w.trim(), MAX_WARNING_LEN))
                .collect()
        })
        .unwrap_or_default();
    let output = truncate_chars(&value.to_string(), MAX_OUTPUT_LEN);
    (warnings, Some(output))
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// The phase's results, plus the first failure as `(function, message)`.
pub(super) struct PostInstallOutcome {
    pub results: Vec<PostInstallResult>,
    pub failure: Option<(String, String)>,
}

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Run the hooks in order; stop at the first failure.
    pub(super) async fn run_post_install(
        &self,
        hooks: &[PostInstallHook],
        package_name: &str,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> PostInstallOutcome {
        let mut results = Vec::with_capacity(hooks.len());
        if branch != POST_INSTALL_BRANCH {
            tracing::info!(
                package_name = %package_name,
                branch = %branch,
                hooks = hooks.len(),
                "Skipping post_install hooks: they run on main only"
            );
            for hook in hooks {
                results.push(skipped(hook, format!("branch '{branch}' is not main")));
            }
            return PostInstallOutcome {
                results,
                failure: None,
            };
        }

        let phase_start = Instant::now();
        let mut failure = None;
        for hook in hooks {
            if failure.is_some() {
                results.push(skipped(hook, "an earlier post_install hook failed".into()));
                continue;
            }
            let started = Instant::now();
            let remaining = POST_INSTALL_PHASE_BUDGET.saturating_sub(phase_start.elapsed());
            let timeout = self.post_install_hook_timeout.min(remaining);
            let outcome = self
                .run_one_hook(hook, package_name, tenant_id, repo_id, branch, timeout)
                .await;
            let duration_ms = started.elapsed().as_millis() as u64;
            match outcome {
                Ok(returned) => {
                    let (warnings, output) = summarize_output(returned.as_ref());
                    results.push(PostInstallResult {
                        function: hook.function.clone(),
                        status: "completed".into(),
                        duration_ms,
                        error: None,
                        warnings,
                        output,
                    })
                }
                Err(msg) => {
                    let msg = shorten(&msg);
                    tracing::warn!(
                        package_name = %package_name,
                        function = %hook.function,
                        error = %msg,
                        "post_install hook failed"
                    );
                    results.push(PostInstallResult {
                        function: hook.function.clone(),
                        status: "failed".into(),
                        duration_ms,
                        error: Some(msg.clone()),
                        warnings: Vec::new(),
                        output: None,
                    });
                    failure = Some((hook.function.clone(), msg));
                }
            }
        }
        PostInstallOutcome { results, failure }
    }

    /// Queue one hook's function job and wait for it. `Err` is the reason.
    async fn run_one_hook(
        &self,
        hook: &PostInstallHook,
        package_name: &str,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        timeout: Duration,
    ) -> std::result::Result<Option<serde_json::Value>, String> {
        if timeout.is_zero() {
            return Err("the post_install time budget is used up".into());
        }
        let scope = StorageScope::new(tenant_id, repo_id, branch, FUNCTIONS_WORKSPACE);
        match self
            .storage
            .nodes()
            .get_by_path(scope, &hook.function, None)
            .await
        {
            Ok(Some(node)) if node.node_type == FUNCTION_NODE_TYPE => {}
            Ok(Some(node)) => {
                return Err(format!(
                    "'{}' is a {}, not a {FUNCTION_NODE_TYPE}",
                    hook.function, node.node_type
                ))
            }
            Ok(None) => {
                return Err("function not found in the functions workspace; it must be \
                            installed by this package or an earlier one"
                    .to_string())
            }
            Err(e) => return Err(format!("could not look up the function: {e}")),
        }
        let data_store = self.job_data_store.as_ref().ok_or_else(|| {
            "function jobs cannot be queued here (no job data store configured)".to_string()
        })?;

        let marker = agent_identity::job(&format!("package-post-install:{package_name}"));
        let mut metadata = HashMap::new();
        metadata.insert("input".to_string(), hook.input.clone());
        metadata.insert(ORIGIN_AGENT_KEY.to_string(), serde_json::json!(marker));
        let auth = serde_json::to_value(AuthContext::system().with_agent(&marker))
            .map_err(|e| format!("could not encode the system auth context: {e}"))?;
        metadata.insert(AUTH_CONTEXT_KEY.to_string(), auth);
        let context = JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            workspace_id: FUNCTIONS_WORKSPACE.to_string(),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata,
        };

        // Context first, so a worker can never claim the job without it.
        let job_id = JobId::new();
        data_store
            .put(&job_id, &context)
            .map_err(|e| format!("could not store the function job context: {e}"))?;
        self.job_registry
            .register_job_with_id(
                job_id.clone(),
                JobType::FunctionExecution {
                    function_path: hook.function.clone(),
                    trigger_name: Some("post_install".to_string()),
                    execution_id: nanoid::nanoid!(),
                },
                tenant_id.to_string(),
                None,
                None,
                // No retries inside the install: a failure fails the install,
                // and re-running the install is the retry.
                Some(0),
            )
            .await
            .map_err(|e| format!("could not queue the function job: {e}"))?;

        self.wait_for_function_job(&job_id, timeout).await
    }

    /// Poll the registry until the job is terminal or `timeout` passes. The
    /// same polling the HTTP sync-invoke path uses; the registry has no
    /// completion notification to wait on.
    ///
    /// On success, the function's return value (the `result` of the
    /// `FunctionExecutionResult` the function job stores as its job result).
    async fn wait_for_function_job(
        &self,
        job_id: &JobId,
        timeout: Duration,
    ) -> std::result::Result<Option<serde_json::Value>, String> {
        let poll = async {
            loop {
                match self.job_registry.get_status(job_id).await {
                    Ok(JobStatus::Completed) => {
                        let returned = self
                            .job_registry
                            .get_job_info(job_id)
                            .await
                            .ok()
                            .and_then(|info| info.result)
                            .and_then(|r| r.get("result").cloned());
                        return Ok(returned);
                    }
                    Ok(JobStatus::Failed(e)) => return Err(e),
                    Ok(JobStatus::Cancelled) => return Err("the function job was cancelled".into()),
                    Ok(_) => tokio::time::sleep(POLL_INTERVAL).await,
                    Err(e) => return Err(format!("lost track of function job {job_id}: {e}")),
                }
            }
        };
        match tokio::time::timeout(timeout, poll).await {
            Ok(r) => r,
            Err(_) => Err(format!(
                "did not finish within {}s (job {job_id} may still be running)",
                timeout.as_secs()
            )),
        }
    }
}

/// The install job's error for a failed hook. Permanent: the content is
/// installed, so the worker must not re-run the whole install on its retry
/// budget; installing again (which the install endpoint allows for a `failed`
/// package even in skip mode) is the retry.
pub(super) fn post_install_error(function: &str, message: &str) -> Error {
    Error::permanent(format!(
        "installed; post_install {function} failed: {message}"
    ))
}

fn skipped(hook: &PostInstallHook, reason: String) -> PostInstallResult {
    PostInstallResult {
        function: hook.function.clone(),
        status: "skipped".into(),
        duration_ms: 0,
        error: Some(reason),
        warnings: Vec::new(),
        output: None,
    }
}

fn shorten(msg: &str) -> String {
    let msg = msg.trim();
    if msg.chars().count() <= MAX_ERROR_LEN {
        return msg.to_string();
    }
    let cut: String = msg.chars().take(MAX_ERROR_LEN).collect();
    format!("{cut}…")
}

/// Validation failure for a manifest's hooks, raised before anything installs.
pub(super) fn validate_hooks(hooks: &[PostInstallHook]) -> Result<()> {
    raisin_packages::validate_post_install(hooks)
        .map_err(|e| Error::Validation(format!("Invalid manifest.yaml: {e}")))
}
