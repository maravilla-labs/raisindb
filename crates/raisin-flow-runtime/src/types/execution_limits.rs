//! Lifetime execution limits bound the whole instance, including async resumes.

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{FlowError, FlowInstance, FlowResult};

pub const DEFAULT_MAX_STEP_ATTEMPTS: u32 = 10_000;
pub const DEFAULT_MAX_ELAPSED_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy)]
pub struct FlowExecutionLimits {
    pub max_step_attempts: u32,
    pub max_elapsed_ms: u64,
}

impl Default for FlowExecutionLimits {
    fn default() -> Self {
        Self {
            max_step_attempts: DEFAULT_MAX_STEP_ATTEMPTS,
            max_elapsed_ms: DEFAULT_MAX_ELAPSED_MS,
        }
    }
}

impl FlowExecutionLimits {
    /// Policy is immutable with the definition: metadata.custom.execution_limits.
    /// Invalid limits must not silently become disabled or revert to defaults.
    pub fn parse(policy: Option<&Value>) -> FlowResult<Self> {
        let Some(policy) = policy else {
            return Ok(Self::default());
        };
        let object = policy.as_object().ok_or_else(|| {
            FlowError::InvalidDefinition("execution_limits must be an object".into())
        })?;
        for key in object.keys() {
            if key != "max_step_attempts" && key != "max_elapsed_ms" {
                return Err(FlowError::InvalidDefinition(format!(
                    "unknown execution limit {key}"
                )));
            }
        }
        let integer = |key: &str, fallback: u64, maximum: u64| -> FlowResult<u64> {
            match object.get(key) {
                None => Ok(fallback),
                Some(value) => value
                    .as_u64()
                    .filter(|v| *v > 0 && *v <= maximum)
                    .ok_or_else(|| {
                        FlowError::InvalidDefinition(format!(
                            "execution_limits.{key} must be an integer from 1 to {maximum}"
                        ))
                    }),
            }
        };
        Ok(Self {
            max_step_attempts: integer(
                "max_step_attempts",
                DEFAULT_MAX_STEP_ATTEMPTS as u64,
                1_000_000,
            )? as u32,
            max_elapsed_ms: integer(
                "max_elapsed_ms",
                DEFAULT_MAX_ELAPSED_MS,
                365 * 24 * 60 * 60 * 1000,
            )?,
        })
    }

    /// Check before a handler can act. Attempts include failed calls and async
    /// result consumption. The counter is saved with the instance, not a job.
    pub fn charge_attempt(
        &self,
        instance: &mut FlowInstance,
        now: DateTime<Utc>,
    ) -> FlowResult<()> {
        let elapsed_ms = now
            .signed_duration_since(instance.started_at)
            .num_milliseconds()
            .max(0) as u64;
        if elapsed_ms >= self.max_elapsed_ms {
            return Err(FlowError::TimeoutExceeded {
                duration_ms: self.max_elapsed_ms,
            });
        }
        // Older persisted instances lack the new counter. Their known executed
        // steps establish a conservative lower bound rather than starting at 0.
        let attempts = instance
            .metrics
            .execution_attempts
            .max(instance.metrics.step_count as u64);
        if attempts >= self.max_step_attempts as u64 {
            return Err(FlowError::MaxIterationsExceeded {
                limit: self.max_step_attempts,
            });
        }
        instance.metrics.execution_attempts = attempts + 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn limits_reject_disabled_malformed_or_unknown_policies() {
        for policy in [
            json!(null),
            json!(false),
            json!([]),
            json!({ "max_step_attempts": 0 }),
            json!({ "max_step_attempts": -1 }),
            json!({ "max_step_attempts": 2.5 }),
            json!({ "max_step_attempts": "10" }),
            json!({ "max_step_attempts": 1_000_001 }),
            json!({ "max_elapsed_ms": 0 }),
            json!({ "max_elapsed_ms": null }),
            json!({ "steps": 3 }),
        ] {
            assert!(
                FlowExecutionLimits::parse(Some(&policy)).is_err(),
                "{policy}"
            );
        }
        assert_eq!(
            FlowExecutionLimits::parse(None).unwrap().max_step_attempts,
            DEFAULT_MAX_STEP_ATTEMPTS
        );
        assert_eq!(
            FlowExecutionLimits::parse(Some(&json!({"max_step_attempts": 4})))
                .unwrap()
                .max_elapsed_ms,
            DEFAULT_MAX_ELAPSED_MS
        );
    }
}
