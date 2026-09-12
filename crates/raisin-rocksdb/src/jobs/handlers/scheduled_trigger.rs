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

// TODO(v0.2): Cron expression matching for scheduled triggers
#![allow(dead_code)]

//! Scheduled trigger handler
//!
//! This module handles evaluation of cron/schedule-based triggers.
//! It runs periodically (typically every minute) and checks which
//! scheduled triggers should fire based on their cron expressions.

use crate::jobs::{AUTH_CONTEXT_KEY, ORIGIN_AGENT_KEY};
use raisin_error::{Error, Result};
use raisin_models::auth::{agent_identity, AuthContext};
use raisin_storage::jobs::{JobContext, JobInfo, JobRegistry, JobType};
use std::collections::HashMap;
use std::sync::Arc;

use crate::jobs::data_store::JobDataStore;
use crate::jobs::dispatcher::JobDispatcher;

/// A scheduled trigger that matches the current time
/// How long a fired tick stays claimed.
///
/// Only needs to outlive the spread between nodes reaching the same tick — a
/// sweep plus a trigger scan — while retiring well before the same wall-clock
/// minute could come round again. Minutes, not hours: an over-long TTL just
/// accumulates dead keys in Redis.
const SCHEDULED_TICK_TTL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct ScheduledTriggerMatch {
    /// Path to the function to execute
    pub function_path: String,
    /// Name of the trigger
    pub trigger_name: String,
    /// Tenant ID
    pub tenant_id: String,
    /// Repository ID
    pub repo_id: String,
    /// Branch name
    pub branch: String,
    /// Workspace
    pub workspace: String,
}

/// Callback type for finding scheduled triggers that should fire
///
/// This callback is provided by the transport layer which has access to query triggers.
/// Arguments: (tenant_id, repo_id, current_time_unix)
/// Returns: List of scheduled triggers that should execute now
pub type ScheduledTriggerFinderCallback = Arc<
    dyn Fn(
            Option<String>, // tenant_id filter (None = all)
            Option<String>, // repo_id filter (None = all)
            i64,            // current Unix timestamp (seconds)
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<ScheduledTriggerMatch>>> + Send>,
        > + Send
        + Sync,
>;

/// Handler for scheduled trigger evaluation jobs
///
/// This handler processes ScheduledTriggerCheck jobs by finding all scheduled
/// triggers whose cron expressions match the current time and enqueueing
/// FunctionExecution jobs for each.
pub struct ScheduledTriggerHandler {
    /// Job registry for enqueueing function execution jobs
    job_registry: Arc<JobRegistry>,
    /// Job data store for storing job context
    job_data_store: Arc<JobDataStore>,
    /// Job dispatcher for routing jobs to worker queues
    dispatcher: Arc<JobDispatcher>,
    /// Optional callback to find scheduled triggers (set by transport layer)
    trigger_finder: Option<ScheduledTriggerFinderCallback>,
    /// Cluster-wide single-fire guard. See [`Self::claim_tick`].
    lock_manager: Option<raisin_locks::LockManagerHandle>,
}

impl ScheduledTriggerHandler {
    /// Create a new scheduled trigger handler
    pub fn new(
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
        dispatcher: Arc<JobDispatcher>,
    ) -> Self {
        Self {
            job_registry,
            job_data_store,
            dispatcher,
            trigger_finder: None,
            lock_manager: None,
        }
    }

    /// Provide the lock manager that makes a fire single-shot across the cluster.
    pub fn with_lock_manager(
        mut self,
        lock_manager: Option<raisin_locks::LockManagerHandle>,
    ) -> Self {
        self.lock_manager = lock_manager;
        self
    }

    /// Set the trigger finder callback
    ///
    /// This should be called by the transport layer after initialization
    /// to provide the callback that finds scheduled triggers.
    pub fn with_trigger_finder(mut self, finder: ScheduledTriggerFinderCallback) -> Self {
        self.trigger_finder = Some(finder);
        self
    }

    /// Claim one trigger's one tick, cluster-wide.
    ///
    /// Returns `true` if THIS node should fire it.
    ///
    /// **The lease is taken and never released** — a held lease IS the "already
    /// fired" marker, and its TTL retires it once the tick can no longer be
    /// re-fired. Releasing it would reopen the window for a slower node still
    /// working through its own copy of the match list. Same idiom as the
    /// authorization server's one-shot code redemption (`claim_once`).
    ///
    /// The key is per trigger AND per tick, so two different triggers never
    /// contend and the same trigger fires again next minute.
    ///
    /// With no lock manager, or with the in-process backend, this coordinates
    /// within ONE node only — which is exactly right for a single-node
    /// deployment and is why that was the previous supported configuration. A
    /// cluster needs `[locks]` with `backend = "redis"`, the same requirement
    /// ticket inventory and OAuth refresh already carry.
    async fn claim_tick(&self, trigger_match: &ScheduledTriggerMatch, now: i64) -> bool {
        let Some(locks) = self.lock_manager.as_ref() else {
            // No locks subsystem: single-node semantics, fire it.
            return true;
        };

        // Cron granularity is the minute, so the minute IS the tick identity.
        // Anything finer would let two nodes a few seconds apart both claim.
        let tick = now / 60;
        let key = raisin_locks::scoped_key(
            &trigger_match.tenant_id,
            &trigger_match.repo_id,
            &trigger_match.branch,
            &format!("scheduled-trigger:{}:{}", trigger_match.trigger_name, tick),
        );

        match locks
            .try_acquire(&key, "scheduled-trigger", SCHEDULED_TICK_TTL)
            .await
        {
            Ok(Some(_guard)) => true,
            Ok(None) => {
                tracing::debug!(
                    trigger = %trigger_match.trigger_name,
                    tick,
                    "another node claimed this tick; skipping"
                );
                false
            }
            Err(e) => {
                // Degrade to firing rather than silently dropping a scheduled
                // job: a lock backend outage must not stop every cron in the
                // system. The cost of the other choice is an email that never
                // goes out and no record of why; the cost of this one is a
                // duplicate during an outage.
                tracing::warn!(
                    error = %e,
                    trigger = %trigger_match.trigger_name,
                    "lock backend error claiming a scheduled tick; firing anyway (may duplicate)"
                );
                true
            }
        }
    }

    /// Handle scheduled trigger check job
    ///
    /// Finds all scheduled triggers that should fire now and enqueues
    /// FunctionExecution jobs for each.
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::ScheduledTriggerCheck variant
    /// * `_context` - Job context (not used for this job type)
    pub async fn handle(&self, job: &JobInfo, _context: &JobContext) -> Result<()> {
        // Extract filter info from JobType
        let (tenant_filter, repo_filter) = match &job.job_type {
            JobType::ScheduledTriggerCheck { tenant_id, repo_id } => {
                (tenant_id.clone(), repo_id.clone())
            }
            _ => {
                return Err(Error::Validation(
                    "Expected ScheduledTriggerCheck job type".to_string(),
                ))
            }
        };

        tracing::info!(
            job_id = %job.id,
            tenant_filter = ?tenant_filter,
            repo_filter = ?repo_filter,
            "Processing scheduled trigger check"
        );

        // Check if trigger finder is available
        let finder = self.trigger_finder.as_ref().ok_or_else(|| {
            Error::Validation(
                "Scheduled trigger finder not configured. The transport layer must provide the finder callback.".to_string()
            )
        })?;

        // Get current time
        let current_time = chrono::Utc::now().timestamp();

        // Find scheduled triggers that should fire
        let matches = finder(tenant_filter.clone(), repo_filter.clone(), current_time).await?;

        if matches.is_empty() {
            tracing::debug!(
                job_id = %job.id,
                tenant_filter = ?tenant_filter,
                repo_filter = ?repo_filter,
                "No scheduled triggers to fire"
            );
            return Ok(());
        }

        tracing::info!(
            job_id = %job.id,
            match_count = matches.len(),
            "Found scheduled triggers to fire"
        );

        // Enqueue FunctionExecution jobs for each match
        for trigger_match in matches {
            // EVERY node runs this loop, and that is the design — no leader
            // election, no "only run one node in production". What must happen
            // once is the SIDE EFFECT, so the claim sits immediately before it:
            // whichever node claims the tick fires it, and every other node
            // finds the claim taken and quietly does nothing. Losing the race is
            // the normal, correct outcome for N-1 nodes, not an error.
            if !self.claim_tick(&trigger_match, current_time).await {
                continue;
            }

            let execution_id = nanoid::nanoid!();

            let function_job_type = JobType::FunctionExecution {
                function_path: trigger_match.function_path.clone(),
                trigger_name: Some(trigger_match.trigger_name.clone()),
                execution_id: execution_id.clone(),
            };

            // Build execution context with schedule event data
            let mut metadata = HashMap::new();
            metadata.insert(
                "trigger_name".to_string(),
                serde_json::json!(trigger_match.trigger_name),
            );
            metadata.insert("event_type".to_string(), serde_json::json!("Scheduled"));
            // Provenance: a cron trigger has no node path in this match (only a
            // name), so the function it fires is its stable identity. Same
            // channel the node-event trigger path uses, so a scheduled write is
            // attributed exactly like an event-driven one.
            let marker = agent_identity::schedule(&trigger_match.function_path);
            metadata.insert(ORIGIN_AGENT_KEY.to_string(), serde_json::json!(marker));
            if let Ok(auth) = serde_json::to_value(AuthContext::system().with_agent(&marker)) {
                metadata.insert(AUTH_CONTEXT_KEY.to_string(), auth);
            }
            metadata.insert(
                "scheduled_time".to_string(),
                serde_json::json!(current_time),
            );
            metadata.insert(
                "input".to_string(),
                serde_json::json!({
                    "event": {
                        "type": "Scheduled",
                        "trigger_name": trigger_match.trigger_name,
                        "scheduled_time": current_time,
                        "scheduled_time_iso": chrono::Utc::now().to_rfc3339(),
                    }
                }),
            );

            let function_context = JobContext {
                tenant_id: trigger_match.tenant_id.clone(),
                repo_id: trigger_match.repo_id.clone(),
                branch: trigger_match.branch.clone(),
                workspace_id: trigger_match.workspace.clone(),
                revision: raisin_hlc::HLC::new(0, 0),
                metadata,
            };

            // Store the context BEFORE registering so dispatch can never
            // observe the job without its context.
            let function_job_id = raisin_storage::jobs::JobId::new();
            self.job_data_store
                .put(&function_job_id, &function_context)?;

            // Enqueue the function execution job
            // TODO: Extract max_retries from scheduled trigger properties when needed
            self.job_registry
                .register_job_with_id(
                    function_job_id.clone(),
                    function_job_type.clone(),
                    trigger_match.tenant_id.clone(),
                    None,
                    None,
                    None, // Use default max_retries for scheduled triggers
                )
                .await?;

            // Dispatch to priority queue
            let priority = function_job_type.default_priority();
            self.dispatcher
                .dispatch(function_job_id.clone(), priority, &trigger_match.tenant_id)
                .await;

            tracing::debug!(
                job_id = %function_job_id,
                execution_id = %execution_id,
                function_path = %trigger_match.function_path,
                trigger_name = %trigger_match.trigger_name,
                tenant_id = %trigger_match.tenant_id,
                repo_id = %trigger_match.repo_id,
                priority = %priority,
                "Enqueued and dispatched scheduled function execution job"
            );
        }

        Ok(())
    }
}

/// Parse a cron expression and check if it matches the given time
///
/// Supports standard 5-field cron format: minute hour day month day_of_week
/// Also supports special strings: @hourly, @daily, @weekly, @monthly, @yearly
///
/// # Arguments
///
/// * `cron_expr` - The cron expression to evaluate
/// * `time` - Unix timestamp to check against
///
/// # Returns
///
/// True if the cron expression matches the given time
pub fn cron_matches(cron_expr: &str, time: i64) -> bool {
    // Use chrono to get the time components
    let datetime = match chrono::DateTime::from_timestamp(time, 0) {
        Some(dt) => dt,
        None => return false,
    };

    let minute = datetime
        .format("%M")
        .to_string()
        .parse::<u32>()
        .unwrap_or(0);
    let hour = datetime
        .format("%H")
        .to_string()
        .parse::<u32>()
        .unwrap_or(0);
    let day = datetime
        .format("%d")
        .to_string()
        .parse::<u32>()
        .unwrap_or(1);
    let month = datetime
        .format("%m")
        .to_string()
        .parse::<u32>()
        .unwrap_or(1);
    let dow = datetime
        .format("%u")
        .to_string()
        .parse::<u32>()
        .unwrap_or(1); // 1=Monday, 7=Sunday

    // Handle special strings
    match cron_expr.trim() {
        "@yearly" | "@annually" => return minute == 0 && hour == 0 && day == 1 && month == 1,
        "@monthly" => return minute == 0 && hour == 0 && day == 1,
        "@weekly" => return minute == 0 && hour == 0 && dow == 1, // Monday
        "@daily" | "@midnight" => return minute == 0 && hour == 0,
        "@hourly" => return minute == 0,
        "@every_minute" => return true, // Special for testing
        _ => {}
    }

    // Parse 5-field cron: minute hour day month day_of_week
    let fields: Vec<&str> = cron_expr.split_whitespace().collect();
    if fields.len() != 5 {
        tracing::warn!(cron_expr = %cron_expr, "Invalid cron expression: expected 5 fields");
        return false;
    }

    let matches_field = |field: &str, value: u32, max: u32| -> bool {
        if field == "*" {
            return true;
        }

        // Handle */N (step values)
        if let Some(step_str) = field.strip_prefix("*/") {
            if let Ok(step) = step_str.parse::<u32>() {
                return step > 0 && value % step == 0;
            }
            return false;
        }

        // Handle ranges (e.g., 1-5)
        if field.contains('-') {
            let parts: Vec<&str> = field.split('-').collect();
            if parts.len() == 2 {
                if let (Ok(start), Ok(end)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                    return value >= start && value <= end;
                }
            }
            return false;
        }

        // Handle lists (e.g., 1,3,5)
        if field.contains(',') {
            return field
                .split(',')
                .any(|v| v.parse::<u32>().map(|n| n == value).unwrap_or(false));
        }

        // Simple numeric match
        field.parse::<u32>().map(|n| n == value).unwrap_or(false)
    };

    matches_field(fields[0], minute, 59)
        && matches_field(fields[1], hour, 23)
        && matches_field(fields[2], day, 31)
        && matches_field(fields[3], month, 12)
        && matches_field(fields[4], dow, 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_matches_every_minute() {
        // Any time should match @every_minute
        assert!(cron_matches("@every_minute", 1700000000));
    }

    #[test]
    fn test_cron_matches_hourly() {
        // 2023-11-14 00:00:00 UTC - minute 0
        assert!(cron_matches("@hourly", 1699920000));
        // 2023-11-14 00:30:00 UTC - minute 30
        assert!(!cron_matches("@hourly", 1699921800));
    }

    #[test]
    fn test_cron_matches_wildcard() {
        // * * * * * should match any time
        assert!(cron_matches("* * * * *", 1700000000));
    }

    #[test]
    fn test_cron_matches_specific_minute() {
        // 0 * * * * should match at minute 0
        assert!(cron_matches("0 * * * *", 1699920000)); // 2023-11-14 00:00 UTC
        assert!(!cron_matches("0 * * * *", 1699921800)); // 2023-11-14 00:30 UTC
    }

    #[test]
    fn test_cron_matches_step() {
        // */15 * * * * should match every 15 minutes
        assert!(cron_matches("*/15 * * * *", 1699920000)); // minute 0
        assert!(cron_matches("*/15 * * * *", 1699920900)); // minute 15
        assert!(cron_matches("*/15 * * * *", 1699921800)); // minute 30
        assert!(!cron_matches("*/15 * * * *", 1699920600)); // minute 10
    }

    fn a_match(trigger: &str) -> ScheduledTriggerMatch {
        ScheduledTriggerMatch {
            function_path: "/functions/nightly".to_string(),
            trigger_name: trigger.to_string(),
            tenant_id: "t".to_string(),
            repo_id: "r".to_string(),
            branch: "main".to_string(),
            workspace: "ws".to_string(),
        }
    }

    /// The key every node computes for one trigger's one tick.
    ///
    /// Reproduced here rather than reached into, because what matters is that
    /// the SHAPE is stable: two nodes deriving different keys both fire, which
    /// is precisely the duplicate-side-effect bug this guards against.
    fn key_for(m: &ScheduledTriggerMatch, now: i64) -> String {
        raisin_locks::scoped_key(
            &m.tenant_id,
            &m.repo_id,
            &m.branch,
            &format!("scheduled-trigger:{}:{}", m.trigger_name, now / 60),
        )
    }

    /// One node fires; every other finds the tick claimed and skips. A
    /// cron-fired side effect — "send the reminder email" — must happen once
    /// per tick, not once per node.
    #[tokio::test]
    async fn only_one_node_claims_a_tick() {
        let locks: raisin_locks::LockManagerHandle =
            std::sync::Arc::new(raisin_locks::InProcessLockManager::new());
        let key = key_for(&a_match("nightly-digest"), 1_000_000);

        assert!(locks
            .try_acquire(&key, "node-a", SCHEDULED_TICK_TTL)
            .await
            .unwrap()
            .is_some());
        assert!(
            locks
                .try_acquire(&key, "node-b", SCHEDULED_TICK_TTL)
                .await
                .unwrap()
                .is_none(),
            "a second node must not fire the same tick"
        );
    }

    /// The NEXT minute is a different tick, or one claim would suppress the
    /// whole schedule for as long as its TTL.
    #[tokio::test]
    async fn the_next_minute_is_a_new_tick() {
        let locks: raisin_locks::LockManagerHandle =
            std::sync::Arc::new(raisin_locks::InProcessLockManager::new());
        let m = a_match("nightly-digest");

        assert!(locks
            .try_acquire(&key_for(&m, 1_000_000), "node-a", SCHEDULED_TICK_TTL)
            .await
            .unwrap()
            .is_some());
        assert!(
            locks
                .try_acquire(&key_for(&m, 1_000_060), "node-a", SCHEDULED_TICK_TTL)
                .await
                .unwrap()
                .is_some(),
            "claiming one minute must not suppress the next"
        );
    }

    /// Seconds inside one minute are the SAME tick. Nodes do not reach a
    /// trigger at the same instant, and a finer granularity would let two of
    /// them seconds apart both claim.
    ///
    /// The minute is also exactly the granularity the CRON MATCH uses, which is
    /// what makes the boundary safe rather than merely likely to work: a node
    /// evaluating at 12:00:59 matches minute 12:00 and claims that tick; one
    /// evaluating at 12:01:00 is asking a different question — does this cron
    /// fire at 12:01 — and for anything but `* * * * *` the answer is no, so it
    /// never reaches the claim. Two nodes straddling the boundary therefore
    /// cannot double-fire one occurrence.
    #[test]
    fn seconds_within_one_minute_are_one_tick() {
        let m = a_match("nightly-digest");
        // A real minute boundary: 16_666 * 60.
        let minute_start = 999_960;
        assert_eq!(key_for(&m, minute_start), key_for(&m, minute_start + 59));
        assert_ne!(key_for(&m, minute_start), key_for(&m, minute_start + 60));
    }

    /// Two different triggers must never contend, or one would suppress the
    /// other every minute.
    #[tokio::test]
    async fn different_triggers_do_not_contend() {
        let locks: raisin_locks::LockManagerHandle =
            std::sync::Arc::new(raisin_locks::InProcessLockManager::new());

        assert!(locks
            .try_acquire(
                &key_for(&a_match("one"), 1_000_000),
                "n",
                SCHEDULED_TICK_TTL
            )
            .await
            .unwrap()
            .is_some());
        assert!(locks
            .try_acquire(
                &key_for(&a_match("two"), 1_000_000),
                "n",
                SCHEDULED_TICK_TTL
            )
            .await
            .unwrap()
            .is_some());
    }
}
