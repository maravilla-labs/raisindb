// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB

//! Streaming AI call with retry and timeout for AI container steps.

use crate::handlers::ai_tool_loop;
use crate::types::{AiExecutionConfig, FlowCallbacks, FlowError, FlowResult};
use serde_json::Value;
use tracing::{debug, error, warn};

/// Call AI with streaming, retry, and timeout.
///
/// Returns the aggregated response as a JSON Value with `content`, `tool_calls`,
/// `usage`, `model`, and `finish_reason` fields. Text and thought chunks are
/// emitted as real-time events during streaming.
pub(super) async fn call_ai_streaming_with_retry(
    callbacks: &dyn FlowCallbacks,
    agent_workspace: &str,
    agent_path: &str,
    messages: &[Value],
    response_format: Option<Value>,
    instance_id: &str,
    exec: &AiExecutionConfig,
) -> FlowResult<Value> {
    let timeout_duration = std::time::Duration::from_millis(exec.timeout_ms.unwrap_or(30000));
    let max_attempts = exec.max_retries + 1;
    let mut last_err: Option<FlowError> = None;

    for attempt in 0..max_attempts {
        if attempt > 0 {
            let delay = exec.retry_delay_ms * (1u64 << (attempt - 1).min(4));
            debug!(
                "Retrying AI call (attempt {}/{}), backoff {}ms",
                attempt + 1,
                max_attempts,
                delay
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }

        let future = callbacks.call_ai_streaming(
            agent_workspace,
            agent_path,
            messages.to_vec(),
            response_format.clone(),
        );

        match tokio::time::timeout(timeout_duration, future).await {
            Ok(Ok(mut rx)) => {
                let mut tool_map = std::collections::HashMap::new();
                let response =
                    ai_tool_loop::accumulate_stream(&mut rx, callbacks, instance_id, &mut tool_map).await;
                return Ok(response);
            }
            Ok(Err(e)) => {
                let err = FlowError::AIProvider(format!("AI call failed: {}", e));
                let is_last = attempt + 1 >= max_attempts;
                if !is_last && is_transient_ai_error(&err) {
                    warn!("Transient AI failure (attempt {}): {}", attempt + 1, e);
                } else {
                    error!("AI call failed: {}", e);
                }
                last_err = Some(err.clone());
                if is_last || !is_transient_ai_error(&err) {
                    break;
                }
            }
            Err(_) => {
                let timeout_val = exec.timeout_ms.unwrap_or(30000);
                let is_last = attempt + 1 >= max_attempts;
                if is_last {
                    error!(
                        "AI call timed out after {}ms (all retries exhausted)",
                        timeout_val
                    );
                } else {
                    warn!(
                        "AI call timed out (attempt {}/{})",
                        attempt + 1,
                        max_attempts
                    );
                }
                last_err = Some(FlowError::TimeoutExceeded {
                    duration_ms: timeout_val,
                });
                if is_last {
                    break;
                }
            }
        }
    }

    Err(last_err
        .unwrap_or_else(|| FlowError::AIProvider("AI call failed with no error details".to_string())))
}

/// Check if a flow error represents a transient AI failure worth retrying.
pub(super) fn is_transient_ai_error(err: &FlowError) -> bool {
    match err {
        FlowError::TimeoutExceeded { .. } => true,
        FlowError::AIProvider(msg) => {
            let lower = msg.to_lowercase();
            lower.contains("timeout")
                || lower.contains("rate limit")
                || lower.contains("429")
                || lower.contains("500")
                || lower.contains("502")
                || lower.contains("503")
                || lower.contains("504")
                || lower.contains("overloaded")
                || lower.contains("temporarily unavailable")
        }
        _ => false,
    }
}
