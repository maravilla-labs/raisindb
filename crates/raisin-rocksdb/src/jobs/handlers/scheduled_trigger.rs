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

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobRegistry, JobType};
use std::collections::HashMap;
use std::sync::Arc;

use crate::jobs::data_store::JobDataStore;
use crate::jobs::dispatcher::JobDispatcher;

/// A scheduled trigger that matches the current time
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
        }
    }

    /// Set the trigger finder callback
    ///
    /// This should be called by the transport layer after initialization
    /// to provide the callback that finds scheduled triggers.
    pub fn with_trigger_finder(mut self, finder: ScheduledTriggerFinderCallback) -> Self {
        self.trigger_finder = Some(finder);
        self
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

            // Enqueue the function execution job
            // TODO: Extract max_retries from scheduled trigger properties when needed
            let function_job_id = self
                .job_registry
                .register_job(
                    function_job_type.clone(),
                    Some(trigger_match.tenant_id.clone()),
                    None,
                    None,
                    None, // Use default max_retries for scheduled triggers
                )
                .await?;

            self.job_data_store
                .put(&function_job_id, &function_context)?;

            // Dispatch to priority queue
            let priority = function_job_type.default_priority();
            self.dispatcher
                .dispatch(function_job_id.clone(), priority)
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
}
