// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Wait step handler for pausing workflow execution.
//!
//! This handler allows workflows to pause for:
//! - A specified duration (delay)
//! - A specific timestamp (scheduled)
//! - An external event (event-driven)

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

use crate::types::{FlowCallbacks, FlowContext, FlowError, FlowNode, FlowResult, StepResult};

use super::StepHandler;

/// Handler for wait steps that pause workflow execution.
///
/// Wait types:
/// - `delay`: Wait for a duration (e.g., "5s", "1h", "30m")
/// - `until`: Wait until a specific timestamp
/// - `event`: Wait for an external event
/// - `cron`: Wait for next cron schedule match
pub struct WaitHandler;

impl WaitHandler {
    /// Create a new wait handler
    pub fn new() -> Self {
        Self
    }
}

impl Default for WaitHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StepHandler for WaitHandler {
    async fn execute(
        &self,
        step: &FlowNode,
        _context: &mut FlowContext,
        _callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        let wait_type = step
            .get_string_property("wait_type")
            .unwrap_or_else(|| "delay".to_string());

        match wait_type.as_str() {
            "delay" => execute_delay_wait(step),
            "until" => execute_until_wait(step),
            "event" => execute_event_wait(step),
            "cron" => execute_cron_wait(step),
            _ => Err(FlowError::InvalidNodeConfiguration(format!(
                "Unknown wait type: {}",
                wait_type
            ))),
        }
    }
}

/// Execute a delay-based wait
fn execute_delay_wait(step: &FlowNode) -> FlowResult<StepResult> {
    // Get duration string (e.g., "5s", "30m", "1h", "1d")
    let duration_str = step
        .get_string_property("duration")
        .or_else(|| step.get_string_property("delay"))
        .ok_or_else(|| {
            FlowError::MissingProperty("duration required for delay wait".to_string())
        })?;

    let duration_ms = parse_duration(&duration_str)?;
    let resume_at = Utc::now() + Duration::milliseconds(duration_ms as i64);

    tracing::info!(
        step_id = %step.id,
        duration_ms = duration_ms,
        resume_at = %resume_at,
        "Waiting for delay"
    );

    Ok(StepResult::Wait {
        reason: "scheduled".to_string(),
        metadata: json!({
            "wait_type": "delay",
            "duration_ms": duration_ms,
            "resume_at": resume_at.to_rfc3339(),
            "timeout_ms": duration_ms,
        }),
    })
}

/// Execute an until-timestamp wait
fn execute_until_wait(step: &FlowNode) -> FlowResult<StepResult> {
    // Get target timestamp
    let until_str = step
        .get_string_property("until")
        .or_else(|| step.get_string_property("timestamp"))
        .ok_or_else(|| FlowError::MissingProperty("until timestamp required".to_string()))?;

    // Parse timestamp
    let resume_at: DateTime<Utc> = until_str
        .parse()
        .map_err(|e| FlowError::InvalidNodeConfiguration(format!("Invalid timestamp: {}", e)))?;

    let now = Utc::now();
    if resume_at <= now {
        // Already past - continue immediately
        tracing::info!(step_id = %step.id, "Wait timestamp already passed, continuing");
        let next_node = step.next_node.clone().unwrap_or_else(|| "end".to_string());
        return Ok(StepResult::Continue {
            next_node_id: next_node,
            output: json!({"waited": false, "reason": "timestamp_passed"}),
        });
    }

    let duration_ms = (resume_at - now).num_milliseconds() as u64;

    tracing::info!(
        step_id = %step.id,
        resume_at = %resume_at,
        duration_ms = duration_ms,
        "Waiting until timestamp"
    );

    Ok(StepResult::Wait {
        reason: "scheduled".to_string(),
        metadata: json!({
            "wait_type": "until",
            "resume_at": resume_at.to_rfc3339(),
            "timeout_ms": duration_ms,
        }),
    })
}

/// Execute an event-driven wait
fn execute_event_wait(step: &FlowNode) -> FlowResult<StepResult> {
    // Get event configuration
    let event_type = step.get_string_property("event_type").ok_or_else(|| {
        FlowError::MissingProperty("event_type required for event wait".to_string())
    })?;

    let event_filter = step.properties.get("event_filter").cloned();

    // Optional timeout
    let timeout_ms = step
        .get_string_property("timeout")
        .and_then(|s| parse_duration(&s).ok());

    tracing::info!(
        step_id = %step.id,
        event_type = %event_type,
        timeout_ms = ?timeout_ms,
        "Waiting for event"
    );

    Ok(StepResult::Wait {
        reason: "event".to_string(),
        metadata: json!({
            "wait_type": "event",
            "event_type": event_type,
            "event_filter": event_filter,
            "timeout_ms": timeout_ms,
        }),
    })
}

/// Execute a cron-based wait
fn execute_cron_wait(step: &FlowNode) -> FlowResult<StepResult> {
    // Get cron expression
    let cron_expr = step
        .get_string_property("cron")
        .or_else(|| step.get_string_property("schedule"))
        .ok_or_else(|| FlowError::MissingProperty("cron expression required".to_string()))?;

    // Calculate next occurrence
    // For now, use a simplified approach - full implementation would use cron parser
    let resume_at = calculate_next_cron_occurrence(&cron_expr)?;

    let now = Utc::now();
    let duration_ms = (resume_at - now).num_milliseconds().max(0) as u64;

    tracing::info!(
        step_id = %step.id,
        cron = %cron_expr,
        resume_at = %resume_at,
        "Waiting for cron schedule"
    );

    Ok(StepResult::Wait {
        reason: "scheduled".to_string(),
        metadata: json!({
            "wait_type": "cron",
            "cron": cron_expr,
            "resume_at": resume_at.to_rfc3339(),
            "timeout_ms": duration_ms,
        }),
    })
}

/// Parse a duration string into milliseconds
///
/// Supported formats:
/// - "5s" or "5 seconds" - 5 seconds
/// - "30m" or "30 minutes" - 30 minutes
/// - "1h" or "1 hour" - 1 hour
/// - "1d" or "1 day" - 1 day
/// - "1500" or "1500ms" - 1500 milliseconds
fn parse_duration(s: &str) -> FlowResult<u64> {
    let s = s.trim().to_lowercase();

    // Try to parse as just a number (milliseconds)
    if let Ok(ms) = s.parse::<u64>() {
        return Ok(ms);
    }

    // Parse with unit suffix
    // Check multi-word suffixes before single-char to avoid e.g. "2 hours" matching 's'
    let (num_str, unit) = if s.ends_with("milliseconds") || s.ends_with("ms") {
        (
            s.trim_end_matches("milliseconds")
                .trim_end_matches("ms")
                .trim(),
            "ms",
        )
    } else if s.ends_with("seconds") || s.ends_with("second") {
        (
            s.trim_end_matches("seconds")
                .trim_end_matches("second")
                .trim(),
            "s",
        )
    } else if s.ends_with("minutes") || s.ends_with("minute") || s.ends_with("min") {
        (
            s.trim_end_matches("minutes")
                .trim_end_matches("minute")
                .trim_end_matches("min")
                .trim(),
            "m",
        )
    } else if s.ends_with("hours") || s.ends_with("hour") {
        (
            s.trim_end_matches("hours")
                .trim_end_matches("hour")
                .trim(),
            "h",
        )
    } else if s.ends_with("days") || s.ends_with("day") {
        (
            s.trim_end_matches("days")
                .trim_end_matches("day")
                .trim(),
            "d",
        )
    } else if s.ends_with('s') {
        (s.trim_end_matches('s').trim(), "s")
    } else if s.ends_with('m') {
        (s.trim_end_matches('m').trim(), "m")
    } else if s.ends_with('h') {
        (s.trim_end_matches('h').trim(), "h")
    } else if s.ends_with('d') {
        (s.trim_end_matches('d').trim(), "d")
    } else {
        return Err(FlowError::InvalidNodeConfiguration(format!(
            "Invalid duration format: {}",
            s
        )));
    };

    let num: u64 = num_str.parse().map_err(|_| {
        FlowError::InvalidNodeConfiguration(format!("Invalid duration number: {}", num_str))
    })?;

    let ms = match unit {
        "ms" => num,
        "s" => num * 1000,
        "m" => num * 60 * 1000,
        "h" => num * 60 * 60 * 1000,
        "d" => num * 24 * 60 * 60 * 1000,
        _ => unreachable!(),
    };

    Ok(ms)
}

/// Calculate the next occurrence for a cron expression
///
/// This is a simplified implementation - full version would use a cron parser library
fn calculate_next_cron_occurrence(cron_expr: &str) -> FlowResult<DateTime<Utc>> {
    // For now, just add a fixed interval based on common patterns
    // Full implementation would parse cron and calculate actual next occurrence

    let now = Utc::now();

    // Simple pattern matching for common cron expressions
    let next = if cron_expr.contains("@hourly") || cron_expr.starts_with("0 * * * *") {
        // Next hour
        now + Duration::hours(1)
    } else if cron_expr.contains("@daily") || cron_expr.starts_with("0 0 * * *") {
        // Next day at midnight
        now + Duration::days(1)
    } else if cron_expr.contains("@weekly") || cron_expr.starts_with("0 0 * * 0") {
        // Next week
        now + Duration::weeks(1)
    } else if cron_expr.contains("@monthly") {
        // Next month (approximation)
        now + Duration::days(30)
    } else {
        // Default: next minute
        now + Duration::minutes(1)
    };

    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_ms() {
        assert_eq!(parse_duration("1000").unwrap(), 1000);
        assert_eq!(parse_duration("500ms").unwrap(), 500);
    }

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("5s").unwrap(), 5000);
        assert_eq!(parse_duration("30 seconds").unwrap(), 30000);
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("5m").unwrap(), 300000);
        assert_eq!(parse_duration("1 minute").unwrap(), 60000);
    }

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(parse_duration("1h").unwrap(), 3600000);
        assert_eq!(parse_duration("2 hours").unwrap(), 7200000);
    }

    #[test]
    fn test_parse_duration_days() {
        assert_eq!(parse_duration("1d").unwrap(), 86400000);
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("invalid").is_err());
        assert!(parse_duration("5x").is_err());
    }
}
