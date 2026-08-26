// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Utility functions for flow execution
//!
//! Contains helper functions for wait types, backoff calculation,
//! context building, and context synchronization.

use crate::types::{FlowContext, FlowInstance, FlowNode, TriggerInfo, WaitType};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// Maximum number of retries for version conflicts
pub(crate) const MAX_VERSION_CONFLICT_RETRIES: u32 = 3;

/// Generate a unique subscription ID
pub(crate) fn generate_subscription_id() -> String {
    Uuid::new_v4().to_string()
}

/// Parse wait type from string
pub(crate) fn parse_wait_type(s: &str) -> WaitType {
    match s.to_lowercase().as_str() {
        "tool_call" => WaitType::ToolCall,
        "human_task" => WaitType::HumanTask,
        "scheduled" => WaitType::Scheduled,
        "event" => WaitType::Event,
        "retry" => WaitType::Retry,
        "join" => WaitType::Join,
        "function_call" => WaitType::FunctionCall,
        "chat_session" => WaitType::ChatSession,
        _ => WaitType::Event,
    }
}

/// Republish a step's primary value under the author's own name, when the step
/// declares an `output_key`.
///
/// Studio's agent and function steps have offered a "Save the answer as" field
/// since they were written, promising `steps.<id>.<output_key>` — and NOTHING
/// read it. Not the compiler, not the engine. An author who set it and wrote
/// the condition the field's own description shows got `null`, silently, with
/// no error anywhere. (`FunctionStepConfig::output_mapping` is the same story
/// from the other side: declared, never read.)
///
/// The projection ADDS a key rather than replacing the output, so
/// `steps.<id>.response` keeps working alongside `steps.<id>.<output_key>` and
/// no existing flow changes shape.
///
/// "Primary value" is per-step-type but expressible as one precedence, because
/// each step type contributes exactly one of these keys: an agent's parsed
/// structured output, else its text `response`, else a function's `result`,
/// else the whole output for a step whose shape this does not know.
pub(crate) fn project_output_key(step: &FlowNode, output: Value) -> Value {
    let Some(key) = step
        .get_string("output_key")
        .filter(|k| !k.trim().is_empty())
    else {
        return output;
    };
    let key = key.trim().to_string();

    let Value::Object(mut map) = output else {
        // A scalar or array output has no room for a second key, so the
        // author's name becomes the whole shape.
        return serde_json::json!({ key: output });
    };

    // Never overwrite a key the step itself produced: the step's own meaning
    // for `response` outranks an author who happened to reuse the word.
    if map.contains_key(&key) {
        return Value::Object(map);
    }

    let primary = map
        .get("structured_output")
        .or_else(|| map.get("response"))
        .or_else(|| map.get("result"))
        .cloned()
        .unwrap_or_else(|| Value::Object(map.clone()));

    map.insert(key, primary);
    Value::Object(map)
}

/// Calculate timeout from metadata
pub(crate) fn calculate_timeout(metadata: &Value) -> Option<chrono::DateTime<Utc>> {
    metadata
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .map(|ms| Utc::now() + chrono::Duration::milliseconds(ms as i64))
}

/// Resolve a step's named `retry_strategy` preset, if it names a known one.
///
/// The designer schema has advertised `retry_strategy`
/// (`none|quick|standard|aggressive|llm`) all along, and `runtime::retry`
/// implements every preset — but nothing ever read the property here, so
/// setting it was a SILENT NO-OP: an author asking for `aggressive` got the
/// type default of 3, and an `llm` step that should back off for ten seconds
/// retried on the legacy 10/30/60s ladder. A schema that advertises a knob the
/// engine ignores is worse than not having the knob.
///
/// An unrecognised name resolves to `None` (falling back to the defaults)
/// rather than to `none()`, so a typo does not silently disable retries.
pub(crate) fn retry_strategy_config(step: &FlowNode) -> Option<crate::runtime::retry::RetryConfig> {
    use crate::runtime::retry::strategies;
    let name = step
        .properties
        .get("retry_strategy")
        .and_then(|v| v.as_str())?;
    match name {
        "none" => Some(strategies::none()),
        "quick" => Some(strategies::quick()),
        "standard" => Some(strategies::standard()),
        "aggressive" => Some(strategies::aggressive()),
        "llm" => Some(strategies::llm()),
        other => {
            tracing::warn!(
                step_id = %step.id,
                retry_strategy = %other,
                "Unknown retry_strategy, falling back to the step-type default"
            );
            None
        }
    }
}

/// Get max retries for a step.
///
/// Precedence: an explicit `max_retries` wins, then a named `retry_strategy`
/// preset, then the step TYPE default (see `StepType::default_max_retries`) —
/// where steps whose re-entry duplicates work (`parallel`, `sub_flow`, `loop`)
/// default to no retries and must opt in explicitly.
pub(crate) fn get_max_retries(step: &FlowNode) -> u32 {
    step.get_u32_property("max_retries")
        .or_else(|| retry_strategy_config(step).map(|c| c.max_retries))
        .unwrap_or_else(|| step.step_type.default_max_retries())
}

/// Calculate exponential backoff duration for a retry attempt.
///
/// Honors the step's retry config (`retry_base_delay_ms` doubled per
/// attempt, capped at `retry_max_delay_ms`); without a configured base
/// delay, falls back to the legacy 10/30/60/120s ladder.
pub(crate) fn calculate_backoff(retry_count: u32, step: Option<&FlowNode>) -> chrono::Duration {
    // A named preset supplies the base/max delay when the step did not set them
    // explicitly, so `retry_strategy: llm` actually waits like an LLM call
    // instead of falling through to the legacy 10/30/60s ladder.
    let strategy = step.and_then(retry_strategy_config);
    let base_delay_ms = step
        .and_then(|s| s.get_u64_property("retry_base_delay_ms"))
        .or_else(|| strategy.as_ref().map(|c| c.base_delay_ms));
    if let Some(base_ms) = base_delay_ms.filter(|ms| *ms > 0) {
        let max_ms = step
            .and_then(|s| s.get_u64_property("retry_max_delay_ms"))
            .or_else(|| strategy.as_ref().map(|c| c.max_delay_ms))
            .filter(|ms| *ms > 0)
            .unwrap_or(u64::MAX);
        let factor = 2u64.saturating_pow(retry_count.saturating_sub(1).min(32));
        let delay_ms = base_ms.saturating_mul(factor).min(max_ms);
        return chrono::Duration::milliseconds(delay_ms.min(i64::MAX as u64) as i64);
    }
    let seconds = match retry_count {
        1 => 10,
        2 => 30,
        3 => 60,
        _ => 120,
    };
    chrono::Duration::seconds(seconds)
}

/// Pull `(input_tokens, output_tokens)` out of whatever shape a provider used.
///
/// There is no single spelling: the platform's own AI layer writes
/// `usage.input_tokens` / `usage.output_tokens`, OpenAI-compatible providers
/// write `usage.prompt_tokens` / `usage.completion_tokens`, and some responses
/// hoist the counts to the top level. Accepting only one of those is how a
/// token counter reads zero against a live provider and looks broken.
///
/// Returns `None` when the value carries no usage at all, so a caller can tell
/// "no model call here" from "a call that reported nothing".
pub(crate) fn extract_token_usage(value: &Value) -> Option<(u64, u64)> {
    let scope = value.get("usage").unwrap_or(value);
    let pick = |names: &[&str]| -> Option<u64> {
        names
            .iter()
            .find_map(|n| scope.get(*n).and_then(|v| v.as_u64()))
            .or_else(|| {
                names
                    .iter()
                    .find_map(|n| value.get(*n).and_then(|v| v.as_u64()))
            })
    };
    let input = pick(&["input_tokens", "prompt_tokens"]);
    let output = pick(&["output_tokens", "completion_tokens"]);
    match (input, output) {
        (None, None) => None,
        (i, o) => Some((i.unwrap_or(0), o.unwrap_or(0))),
    }
}

/// The variable-bag key holding per-node visit counts.
///
/// Internal bookkeeping, hence the `__` prefix: the execution loop already
/// refuses to let a step output overwrite a `__*` key, so a step cannot forge
/// its own visit count.
pub(crate) const VISITS_KEY: &str = "__visits";

/// Record that `step_id` is being entered, and return which visit this is
/// (1 on the first).
///
/// Counted on ENTRY rather than on completion so a step can read its own
/// attempt number, and so a step that fails still burns an attempt — otherwise
/// a cycle guarded by `visits.x < 3` would never terminate on the failure path,
/// which is the one that actually loops.
pub(crate) fn bump_visit_count(instance: &mut FlowInstance, step_id: &str) -> u64 {
    let Some(vars) = instance.variables.as_object_mut() else {
        return 1;
    };
    let visits = vars
        .entry(VISITS_KEY.to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let Some(map) = visits.as_object_mut() else {
        return 1;
    };
    let next = map.get(step_id).and_then(|v| v.as_u64()).unwrap_or(0) + 1;
    map.insert(step_id.to_string(), Value::from(next));
    next
}

/// The variable-bag key holding each node's per-visit output history.
pub(crate) const HISTORY_KEY: &str = "__history";

/// How many visits of a single node keep their output.
///
/// Bounded because the instance is one persisted node: an unbounded history
/// over a 5000-item `for_each` would grow the row without limit. Twenty is far
/// more than a review cycle needs (three or four revisions) and far less than a
/// long iteration produces, so in practice the case that wants history keeps
/// all of it and the case that would explode keeps a tail.
pub(crate) const MAX_HISTORY_PER_STEP: usize = 20;

/// Retract the flat variables this step contributed LAST time, then record the
/// new output: `step_outputs.<id>` (latest), `__history.<id>` (per visit), and
/// the flat merge.
///
/// **The retraction is what makes a re-visited node honest.** A step's object
/// output is merged key-by-key into the shared variable bag, so before this a
/// second visit that produced a smaller object left the first visit's extra keys
/// standing, and a visit that produced nothing left ALL of them — indefinitely,
/// under names that read like current data.
///
/// It retracts a key only when the value is still exactly what this step wrote.
/// That is the whole subtlety: blindly removing the keys would delete a value
/// another step legitimately overwrote in between, turning a stale-read bug into
/// a lost-write bug. Unchanged means unclaimed; changed means someone else owns
/// it now and it is not ours to remove.
pub(crate) fn record_step_output(
    vars: &mut serde_json::Map<String, Value>,
    step_id: &str,
    output: &Value,
) {
    // What this step contributed on its previous visit (its whole previous
    // output — the flat keys it merged ARE that object's keys, so no separate
    // provenance table is needed).
    let previous = vars
        .get("step_outputs")
        .and_then(|o| o.get(step_id))
        .cloned()
        .unwrap_or(Value::Null);

    if let Value::Object(prev_map) = &previous {
        for (key, prev_value) in prev_map {
            if key.starts_with("__") || key == "step_outputs" {
                continue;
            }
            if vars.get(key) == Some(prev_value) {
                vars.remove(key);
            }
        }
    }

    // Latest output, unconditional (see the null-output note in the executor).
    vars.entry("step_outputs".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .map(|outputs| outputs.insert(step_id.to_string(), output.clone()));

    // Per-visit history, so a critic CAN compare revision 1 against revision 3.
    // `steps.<id>` is one slot and always the latest; without this an author
    // asking "is this better than the last attempt" has nothing to read.
    if let Some(history) = vars
        .entry(HISTORY_KEY.to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
    {
        let entries = history
            .entry(step_id.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(list) = entries.as_array_mut() {
            list.push(output.clone());
            // Drop from the FRONT so the newest is always last; a reader after
            // the cap wants the recent attempts, not the first ones.
            while list.len() > MAX_HISTORY_PER_STEP {
                list.remove(0);
            }
        }
    }

    vars.insert("__last_output".to_string(), output.clone());

    if let Value::Object(map) = output {
        for (key, value) in map {
            // Never let a step output clobber internal bookkeeping.
            if key.starts_with("__") || key == "step_outputs" {
                continue;
            }
            vars.insert(key.clone(), value.clone());
        }
    }
}

/// Build a FlowContext from a FlowInstance
///
/// Converts the instance state into a context that handlers can use.
pub(crate) fn build_context_from_instance(instance: &FlowInstance) -> FlowContext {
    // Extract step outputs from instance variables if they exist
    let step_outputs: HashMap<String, Value> = instance
        .variables
        .as_object()
        .and_then(|obj| obj.get("step_outputs"))
        .and_then(|v| v.as_object())
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    // Extract flow variables (excluding step_outputs)
    let mut variables: HashMap<String, Value> = instance
        .variables
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(k, _)| k.as_str() != "step_outputs")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();

    // If variables is empty, initialize as empty HashMap
    if variables.is_empty() {
        variables = HashMap::new();
    }

    // Extract trigger info from instance variables if stored there
    let trigger_info = instance
        .variables
        .as_object()
        .and_then(|obj| obj.get("__trigger_info"))
        .and_then(|v| serde_json::from_value::<TriggerInfo>(v.clone()).ok());

    // The previous step's output (recorded by the execution loop) - used by
    // loop steps to collect per-iteration results via context.current_output
    let current_output = instance
        .variables
        .as_object()
        .and_then(|obj| obj.get("__last_output"))
        .filter(|v| !v.is_null())
        .cloned();

    FlowContext {
        instance_id: instance.id.clone(),
        trigger_info,
        input: instance.input.clone(),
        step_outputs,
        variables,
        current_output,
        error: None,
        context_stack: Vec::new(),
        compensation_stack: instance.compensation_stack.clone(),
    }
}

/// Sync FlowContext changes back to FlowInstance
///
/// Updates the instance with any changes made to the context during step execution.
pub(crate) fn sync_context_to_instance(context: &FlowContext, instance: &mut FlowInstance) {
    use serde_json::Map;

    // Build variables object
    let mut vars_map = Map::new();

    // Add all flow variables
    for (key, value) in &context.variables {
        vars_map.insert(key.clone(), value.clone());
    }

    // Add step outputs under "step_outputs" namespace
    if !context.step_outputs.is_empty() {
        let step_outputs_obj: Map<String, Value> = context
            .step_outputs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        vars_map.insert("step_outputs".to_string(), Value::Object(step_outputs_obj));
    }

    instance.variables = Value::Object(vars_map);

    // Sync compensation stack back
    instance.compensation_stack = context.compensation_stack.clone();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_backoff() {
        assert_eq!(calculate_backoff(1, None), chrono::Duration::seconds(10));
        assert_eq!(calculate_backoff(2, None), chrono::Duration::seconds(30));
        assert_eq!(calculate_backoff(3, None), chrono::Duration::seconds(60));
        assert_eq!(calculate_backoff(4, None), chrono::Duration::seconds(120));
    }

    /// Steps whose re-entry duplicates work must NOT retry by default: their
    /// effects already happened, so a retry repeats them.
    #[test]
    fn test_work_duplicating_steps_do_not_retry_by_default() {
        use crate::types::StepType;
        let node = |step_type| FlowNode {
            id: "s1".to_string(),
            step_type,
            properties: HashMap::new(),
            children: Vec::new(),
            next_node: None,
        };

        assert_eq!(get_max_retries(&node(StepType::Parallel)), 0);
        assert_eq!(get_max_retries(&node(StepType::SubFlow)), 0);
        assert_eq!(get_max_retries(&node(StepType::Loop)), 0);
        assert_eq!(get_max_retries(&node(StepType::FunctionStep)), 3);
        assert_eq!(get_max_retries(&node(StepType::HumanTask)), 3);

        // An explicit setting still wins, for genuinely idempotent branches
        let mut opted_in = node(StepType::Parallel);
        opted_in
            .properties
            .insert("max_retries".to_string(), serde_json::json!(2));
        assert_eq!(get_max_retries(&opted_in), 2);
    }

    #[test]
    fn test_calculate_backoff_uses_step_retry_config() {
        let mut step = FlowNode {
            id: "s1".to_string(),
            step_type: crate::types::StepType::FunctionStep,
            properties: HashMap::new(),
            children: Vec::new(),
            next_node: None,
        };
        step.properties.insert(
            "retry_base_delay_ms".to_string(),
            serde_json::json!(1000u64),
        );
        step.properties
            .insert("retry_max_delay_ms".to_string(), serde_json::json!(5000u64));

        // Exponential: 1000, 2000, 4000, then capped at 5000
        assert_eq!(
            calculate_backoff(1, Some(&step)),
            chrono::Duration::milliseconds(1000)
        );
        assert_eq!(
            calculate_backoff(2, Some(&step)),
            chrono::Duration::milliseconds(2000)
        );
        assert_eq!(
            calculate_backoff(3, Some(&step)),
            chrono::Duration::milliseconds(4000)
        );
        assert_eq!(
            calculate_backoff(4, Some(&step)),
            chrono::Duration::milliseconds(5000)
        );

        // Zero base delay falls back to the legacy ladder
        step.properties
            .insert("retry_base_delay_ms".to_string(), serde_json::json!(0u64));
        assert_eq!(
            calculate_backoff(1, Some(&step)),
            chrono::Duration::seconds(10)
        );
    }

    #[test]
    fn test_generate_subscription_id() {
        let id1 = generate_subscription_id();
        let id2 = generate_subscription_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 36); // UUID v4 format
    }

    #[test]
    fn test_parse_wait_type() {
        assert_eq!(parse_wait_type("tool_call"), WaitType::ToolCall);
        assert_eq!(parse_wait_type("human_task"), WaitType::HumanTask);
        assert_eq!(parse_wait_type("retry"), WaitType::Retry);
        assert_eq!(parse_wait_type("unknown"), WaitType::Event);
    }

    // -----------------------------------------------------------------------
    // record_step_output: provenance retraction + per-visit history
    // -----------------------------------------------------------------------

    fn vars_of(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    /// A second visit that produces LESS must not leave the first visit's extra
    /// keys standing in the shared bag.
    #[test]
    fn test_revisit_retracts_its_own_stale_flat_keys() {
        let mut vars = vars_of(serde_json::json!({}));

        record_step_output(
            &mut vars,
            "draft",
            &serde_json::json!({
                "text": "v1", "tone": "formal"
            }),
        );
        assert_eq!(vars.get("tone").and_then(|v| v.as_str()), Some("formal"));

        // Second visit drops `tone` entirely.
        record_step_output(&mut vars, "draft", &serde_json::json!({ "text": "v2" }));

        assert_eq!(vars.get("text").and_then(|v| v.as_str()), Some("v2"));
        assert!(
            vars.get("tone").is_none(),
            "the first visit's key must not survive a visit that did not produce it"
        );
    }

    /// A visit that produces NOTHING retracts everything it contributed.
    #[test]
    fn test_null_revisit_retracts_everything_it_contributed() {
        let mut vars = vars_of(serde_json::json!({}));
        record_step_output(
            &mut vars,
            "translate",
            &serde_json::json!({
                "locale": "de", "translations": ["guten tag"]
            }),
        );

        record_step_output(&mut vars, "translate", &Value::Null);

        assert!(
            vars.get("locale").is_none(),
            "stale locale must not survive"
        );
        assert!(
            vars.get("translations").is_none(),
            "stale payload must not survive"
        );
        assert!(
            vars["step_outputs"]["translate"].is_null(),
            "and the latest output must read as nothing"
        );
    }

    /// THE SUBTLETY: retraction is conservative. A key another step overwrote in
    /// the meantime belongs to that step now, and must survive — blindly
    /// removing this step's old keys would turn a stale-read bug into a
    /// lost-write bug.
    #[test]
    fn test_retraction_leaves_a_key_another_step_overwrote() {
        let mut vars = vars_of(serde_json::json!({}));
        record_step_output(&mut vars, "a", &serde_json::json!({ "shared": "from-a" }));
        record_step_output(&mut vars, "b", &serde_json::json!({ "shared": "from-b" }));

        // `a` runs again and produces nothing. `shared` is now B's value.
        record_step_output(&mut vars, "a", &Value::Null);

        assert_eq!(
            vars.get("shared").and_then(|v| v.as_str()),
            Some("from-b"),
            "a value another step owns must not be retracted by this one"
        );
    }

    /// Every visit is kept, oldest first, so a critic can compare revisions.
    #[test]
    fn test_history_records_each_visit_in_order() {
        let mut vars = vars_of(serde_json::json!({}));
        for n in 1..=3 {
            record_step_output(&mut vars, "draft", &serde_json::json!({ "rev": n }));
        }

        let history = vars[HISTORY_KEY]["draft"].as_array().unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0]["rev"], 1, "oldest first");
        assert_eq!(history[2]["rev"], 3);
        // `steps.<id>` stays the latest.
        assert_eq!(vars["step_outputs"]["draft"]["rev"], 3);
    }

    /// History is bounded: the instance is one persisted node, and a long
    /// `for_each` would otherwise grow it without limit. The TAIL is what is
    /// kept, because a reader past the cap wants the recent attempts.
    #[test]
    fn test_history_is_bounded_and_keeps_the_most_recent() {
        let mut vars = vars_of(serde_json::json!({}));
        let total = MAX_HISTORY_PER_STEP + 5;
        for n in 1..=total {
            record_step_output(&mut vars, "loop_body", &serde_json::json!({ "i": n }));
        }

        let history = vars[HISTORY_KEY]["loop_body"].as_array().unwrap();
        assert_eq!(history.len(), MAX_HISTORY_PER_STEP);
        assert_eq!(history[history.len() - 1]["i"], total, "newest retained");
        assert_eq!(
            history[0]["i"],
            total - MAX_HISTORY_PER_STEP + 1,
            "oldest retained"
        );
    }

    // -----------------------------------------------------------------------
    // retry_strategy presets (previously a silent no-op)
    // -----------------------------------------------------------------------

    fn step_with(props: &[(&str, Value)]) -> FlowNode {
        FlowNode {
            id: "s".to_string(),
            step_type: crate::types::StepType::FunctionStep,
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            children: Vec::new(),
            next_node: None,
        }
    }

    /// A named preset now decides the retry budget. Before this it was read by
    /// nothing: `aggressive` silently got the step-type default of 3.
    #[test]
    fn test_retry_strategy_preset_sets_max_retries() {
        assert_eq!(
            get_max_retries(&step_with(&[("retry_strategy", Value::from("aggressive"))])),
            10
        );
        assert_eq!(
            get_max_retries(&step_with(&[("retry_strategy", Value::from("llm"))])),
            5
        );
        assert_eq!(
            get_max_retries(&step_with(&[("retry_strategy", Value::from("none"))])),
            0
        );
    }

    /// An explicit `max_retries` still wins over the preset.
    #[test]
    fn test_explicit_max_retries_beats_the_preset() {
        let step = step_with(&[
            ("retry_strategy", Value::from("aggressive")),
            ("max_retries", Value::from(1)),
        ]);
        assert_eq!(get_max_retries(&step), 1);
    }

    /// A typo must NOT silently disable retries - it falls back to the default.
    #[test]
    fn test_unknown_retry_strategy_falls_back_not_to_zero() {
        let step = step_with(&[("retry_strategy", Value::from("agressive"))]);
        assert_eq!(
            get_max_retries(&step),
            crate::types::StepType::FunctionStep.default_max_retries(),
            "a misspelled strategy must not read as 'no retries'"
        );
    }

    /// The preset also supplies the backoff, so `llm` waits like an LLM call
    /// instead of falling through to the legacy 10/30/60s ladder.
    #[test]
    fn test_retry_strategy_preset_drives_backoff() {
        let step = step_with(&[("retry_strategy", Value::from("llm"))]);
        // llm: base 10_000ms, doubling per attempt, capped at 120_000ms.
        assert_eq!(calculate_backoff(1, Some(&step)).num_milliseconds(), 10_000);
        assert_eq!(calculate_backoff(2, Some(&step)).num_milliseconds(), 20_000);
        assert_eq!(
            calculate_backoff(6, Some(&step)).num_milliseconds(),
            120_000
        );

        // Without a strategy the legacy ladder still applies.
        let plain = step_with(&[]);
        assert_eq!(calculate_backoff(1, Some(&plain)).num_seconds(), 10);
    }

    // -----------------------------------------------------------------------
    // Token usage extraction
    // -----------------------------------------------------------------------

    /// Providers do not agree on a spelling. Accepting only one is how a token
    /// counter reads zero against a live provider and looks broken.
    #[test]
    fn test_extract_token_usage_accepts_every_provider_shape() {
        // Platform shape
        assert_eq!(
            extract_token_usage(&serde_json::json!({
                "usage": { "input_tokens": 120, "output_tokens": 30 }
            })),
            Some((120, 30))
        );
        // OpenAI-compatible
        assert_eq!(
            extract_token_usage(&serde_json::json!({
                "usage": { "prompt_tokens": 8, "completion_tokens": 4 }
            })),
            Some((8, 4))
        );
        // Hoisted to the top level
        assert_eq!(
            extract_token_usage(&serde_json::json!({
                "input_tokens": 5, "output_tokens": 6
            })),
            Some((5, 6))
        );
    }

    /// "No model call" and "a call that reported nothing" are different facts.
    #[test]
    fn test_extract_token_usage_distinguishes_absent_from_zero() {
        assert_eq!(
            extract_token_usage(&serde_json::json!({ "status": "ok" })),
            None,
            "a plain function output is not a model call"
        );
        assert_eq!(
            extract_token_usage(&serde_json::json!({ "usage": { "input_tokens": 0 } })),
            Some((0, 0)),
            "a reported zero is still a call"
        );
    }

    // -- project_output_key ------------------------------------------------
    //
    // The field was authored, documented and dead: Studio's step-agent.yaml
    // promises `steps.<id>.<output_key>` and nothing read the property.

    /// An agent step declaring (or not declaring) an `output_key`.
    fn keyed(output_key: Option<&str>) -> FlowNode {
        match output_key {
            Some(k) => step_with(&[("output_key", serde_json::json!(k))]),
            None => step_with(&[]),
        }
    }

    #[test]
    fn no_output_key_leaves_the_output_alone() {
        let out = serde_json::json!({"response": "hi"});
        assert_eq!(project_output_key(&keyed(None), out.clone()), out);
        // An empty string is "not configured", not a key named "".
        assert_eq!(project_output_key(&keyed(Some("  ")), out.clone()), out);
    }

    #[test]
    fn an_agents_text_answer_lands_under_the_authors_name() {
        let out = project_output_key(
            &keyed(Some("summary")),
            serde_json::json!({"response": "the gist", "model": "m"}),
        );
        assert_eq!(out["summary"], "the gist");
        // ...without taking the original shape away.
        assert_eq!(out["response"], "the gist");
        assert_eq!(out["model"], "m");
    }

    #[test]
    fn structured_output_outranks_the_text_response() {
        let out = project_output_key(
            &keyed(Some("verdict")),
            serde_json::json!({
                "response": "{\"ok\":true}",
                "structured_output": {"ok": true},
            }),
        );
        assert_eq!(out["verdict"]["ok"], true);
    }

    #[test]
    fn a_functions_result_is_its_primary_value() {
        let out = project_output_key(
            &keyed(Some("order")),
            serde_json::json!({"result": {"id": 7}}),
        );
        assert_eq!(out["order"]["id"], 7);
    }

    #[test]
    fn a_shape_we_do_not_know_is_republished_whole() {
        let out = project_output_key(
            &keyed(Some("data")),
            serde_json::json!({"rows": [1, 2]}),
        );
        assert_eq!(out["data"]["rows"][1], 2);
    }

    #[test]
    fn a_scalar_output_becomes_the_key() {
        let out = project_output_key(&keyed(Some("count")), serde_json::json!(3));
        assert_eq!(out["count"], 3);
    }

    #[test]
    fn the_steps_own_key_is_never_overwritten() {
        // An author naming their key `response` must not lose the step's
        // meaning for it.
        let out = project_output_key(
            &keyed(Some("response")),
            serde_json::json!({"response": "mine", "structured_output": {"x": 1}}),
        );
        assert_eq!(out["response"], "mine");
    }
}
