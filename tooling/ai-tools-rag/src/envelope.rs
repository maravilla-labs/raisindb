//! `raisin.tool-result/1` — the result shape every ai-tools tool returns when
//! an AgentRun invokes it. A port of the read-only half of
//! `agent-shared/tool-envelope.js`: both functions here are read-only tools.
//!
//! A tool is invoked by a run when its args carry `__raisin_context.run_id` and
//! `.operation_id` (core injects both). Outside a run the tool answers with its
//! plain result — the envelope never leaks to a caller that did not ask for it.

use serde_json::{json, Value};

pub const TOOL_RESULT_ENVELOPE: &str = "raisin.tool-result/1";

/// The operation id, when a run invoked this tool.
pub fn run_operation_id(input: &Value) -> Option<String> {
    let ctx = input.get("__raisin_context")?;
    ctx.get("run_id")?.as_str()?;
    ctx.get("operation_id")?.as_str().map(str::to_string)
}

/// The error classes, in the order the JavaScript classifier tries them.
fn classify(message: &str) -> &'static str {
    let m = message.to_ascii_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| m.contains(w));
    if any(&[
        "rate limit",
        "ratelimit",
        "rate-limit",
        "too many requests",
        "429",
        "quota",
    ]) {
        "rate_limited"
    } else if any(&["timed out", "time out", "timeout", "deadline exceeded"]) {
        "timeout"
    } else if any(&[
        "500",
        "502",
        "503",
        "504",
        "overloaded",
        "temporarily",
        "unavailable",
        "econnreset",
        "socket hang up",
        "network",
        "connection reset",
        "connection refused",
        "connection closed",
    ]) {
        "transient"
    } else if any(&[
        "permission denied",
        "forbidden",
        "not allowed",
        "unauthorized",
        "unauthorised",
        "403",
        "access denied",
    ]) {
        "permission_denied"
    } else if any(&["not found", "does not exist", "no such", "404"]) {
        "not_found"
    } else if any(&[
        "already exists",
        "conflict",
        "stale",
        "revision mismatch",
        "409",
    ]) {
        "conflict"
    } else if any(&["not supported", "unsupported", "not implemented"]) {
        "unsupported"
    } else if any(&[
        "required",
        "invalid",
        "must be",
        "expected",
        "missing",
        "malformed",
        "validation",
    ]) {
        "invalid_input"
    } else {
        "tool_error"
    }
}

fn status_for(class: &str) -> &'static str {
    match class {
        "transient" | "timeout" | "rate_limited" => "retryable",
        "permission_denied" | "unsupported" => "blocked",
        _ => "failed",
    }
}

fn envelope(
    operation_id: &str,
    status: &str,
    payload: Value,
    next: Value,
    diagnostics: Value,
    retry: Value,
) -> Value {
    json!({
        "envelope": TOOL_RESULT_ENVELOPE,
        "operation_id": operation_id,
        "status": status,
        "reads": [],
        "writes": [],
        "artifact_refs": [],
        "evidence": [],
        "diagnostics": diagnostics,
        "suggested_next_actions": next,
        "retry_policy": retry,
        "payload": payload,
    })
}

/// A successful read-only result, enveloped.
pub fn success(operation_id: &str, payload: Value, next: Value) -> Value {
    envelope(
        operation_id,
        "succeeded",
        payload,
        next,
        json!([]),
        json!({ "retryable": false, "max_attempts": 1, "backoff_ms": 0, "reason": "read_only" }),
    )
}

/// A failure, enveloped, classified the way the JavaScript tools classify.
pub fn failure(operation_id: &str, message: &str) -> Value {
    let class = classify(message);
    let status = status_for(class);
    let retryable = status == "retryable";
    let severity_class = if retryable {
        "transient"
    } else if class == "invalid_input" {
        "repairable"
    } else {
        "blocking"
    };
    envelope(
        operation_id,
        status,
        json!({ "error": message, "error_class": class }),
        json!([]),
        json!([{ "code": class, "severity": "error", "message": message, "class": severity_class, "fix": null, "path": null }]),
        if retryable {
            json!({ "retryable": true, "max_attempts": 3, "backoff_ms": if class == "rate_limited" { 5000 } else { 1000 }, "reason": class })
        } else {
            json!({ "retryable": false, "max_attempts": 1, "backoff_ms": 0, "reason": class })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_run_gets_an_envelope() {
        assert_eq!(run_operation_id(&json!({ "query": "x" })), None);
        assert_eq!(
            run_operation_id(
                &json!({ "__raisin_context": { "run_id": "r", "operation_id": "op-1" } })
            ),
            Some("op-1".to_string())
        );
    }

    #[test]
    fn failures_classify_like_the_javascript_tools() {
        assert_eq!(
            failure("op", "429 Too Many Requests")["status"],
            json!("retryable")
        );
        assert_eq!(
            failure("op", "permission denied")["status"],
            json!("blocked")
        );
        assert_eq!(
            failure("op", "A non-empty `query` is required")["payload"]["error_class"],
            json!("invalid_input")
        );
        assert_eq!(
            success("op", json!({}), json!([]))["envelope"],
            json!("raisin.tool-result/1")
        );
    }
}
