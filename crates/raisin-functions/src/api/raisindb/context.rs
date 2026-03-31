// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Date/time, logging, and execution context implementations for RaisinFunctionApi

use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;
use crate::types::{LogEntry, LogLevel};

impl RaisinFunctionApi {
    // ========== Date/Time Operations ==========

    pub(crate) fn impl_date_now(&self) -> String {
        Utc::now().to_rfc3339()
    }

    pub(crate) fn impl_date_timestamp(&self) -> i64 {
        Utc::now().timestamp()
    }

    pub(crate) fn impl_date_timestamp_millis(&self) -> i64 {
        Utc::now().timestamp_millis()
    }

    pub(crate) fn impl_date_parse(&self, date_str: &str, format: Option<&str>) -> Result<i64> {
        let dt = match format {
            Some(fmt) => {
                let naive = NaiveDateTime::parse_from_str(date_str, fmt).map_err(|e| {
                    raisin_error::Error::Validation(format!("Invalid date format: {}", e))
                })?;
                Utc.from_utc_datetime(&naive)
            }
            None => {
                // Try RFC3339/ISO 8601 first
                DateTime::parse_from_rfc3339(date_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .or_else(|_| {
                        // Try common date-only format
                        NaiveDateTime::parse_from_str(
                            &format!("{}T00:00:00", date_str),
                            "%Y-%m-%dT%H:%M:%S",
                        )
                        .map(|naive| Utc.from_utc_datetime(&naive))
                    })
                    .map_err(|e| {
                        raisin_error::Error::Validation(format!("Invalid ISO date: {}", e))
                    })?
            }
        };
        Ok(dt.timestamp())
    }

    pub(crate) fn impl_date_format(&self, timestamp: i64, format: Option<&str>) -> Result<String> {
        let dt = Utc
            .timestamp_opt(timestamp, 0)
            .single()
            .ok_or_else(|| raisin_error::Error::Validation("Invalid timestamp".to_string()))?;
        let fmt = format.unwrap_or("%Y-%m-%dT%H:%M:%SZ");
        Ok(dt.format(fmt).to_string())
    }

    pub(crate) fn impl_date_add_days(&self, timestamp: i64, days: i64) -> Result<i64> {
        let dt = Utc
            .timestamp_opt(timestamp, 0)
            .single()
            .ok_or_else(|| raisin_error::Error::Validation("Invalid timestamp".to_string()))?;
        let new_dt = dt + Duration::days(days);
        Ok(new_dt.timestamp())
    }

    pub(crate) fn impl_date_diff_days(&self, ts1: i64, ts2: i64) -> i64 {
        (ts2 - ts1) / 86400
    }

    // ========== Logging ==========

    pub(crate) fn impl_log(&self, level: &str, message: &str) {
        let log_level = match level {
            "debug" => LogLevel::Debug,
            "info" => LogLevel::Info,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => LogLevel::Info,
        };

        let entry = LogEntry::new(log_level, message);
        self.logs.lock().unwrap().push(entry.clone());

        // Also emit to tracing
        match log_level {
            LogLevel::Debug => {
                tracing::debug!(execution_id = %self.context.execution_id, "{}", message)
            }
            LogLevel::Info => {
                tracing::info!(execution_id = %self.context.execution_id, "{}", message)
            }
            LogLevel::Warn => {
                tracing::warn!(execution_id = %self.context.execution_id, "{}", message)
            }
            LogLevel::Error => {
                tracing::error!(execution_id = %self.context.execution_id, "{}", message)
            }
        }
    }

    // ========== Context ==========

    pub(crate) fn impl_get_context(&self) -> Value {
        let mut ctx = serde_json::json!({
            "tenant_id": self.context.tenant_id,
            "repo_id": self.context.repo_id,
            "branch": self.context.branch,
            "workspace_id": self.context.workspace_id,
            "actor": self.context.actor,
            "execution_id": self.context.execution_id,
        });

        // Include event data if available (from trigger-based execution)
        if let Some(event_data) = &self.context.event_data {
            ctx["event"] = event_data.clone();
        }

        // Include trigger name if available
        if let Some(trigger_name) = &self.context.trigger_name {
            ctx["trigger_name"] = serde_json::json!(trigger_name);
        }

        // Include the full input for access to node data
        if !self.context.input.is_null() {
            ctx["input"] = self.context.input.clone();
        }

        ctx
    }
}
