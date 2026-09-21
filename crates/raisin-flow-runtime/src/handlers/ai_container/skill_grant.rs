// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB

//! The skill grant on the AI container path.
//!
//! `load-skill` returns a skill's body only for a skill the caller was GIVEN,
//! and in a flow it learns what was given from one place: the
//! `__raisin_context.skill_grant` on its arguments. It cannot tell a grant the
//! runtime attached from one the model typed into its tool call, so the
//! runtime makes sure only one of those can ever arrive:
//!
//! 1. every tool call the model makes has its runtime-only keys
//!    (`__raisin_context`, `_skill_grant`) STRIPPED as soon as its arguments are parsed — before
//!    the call is stored in the conversation, shown in an event, parked for an
//!    explicit executor or run;
//! 2. at execution, a `load-skill` call — by its tool NAME or by the function
//!    PATH it resolves to, so a renamed mapping cannot slip past — gets
//!    `__raisin_context = {skill_grant}` from the callback's `_skill_grant`,
//!    which the server resolved from the agent and the step. A missing grant
//!    is an empty one, and the tool refuses.

use crate::handlers::ai_tool_loop;
use serde_json::Value;

/// Remove every runtime-only key from arguments the MODEL wrote. The one
/// definition lives in `ai_tool_loop`, shared by every flow path.
pub(super) fn strip_runtime_keys(arguments: Value) -> Value {
    ai_tool_loop::strip_runtime_keys(arguments)
}

/// The callback's `_skill_grant` on an accumulated response, when it sent one.
pub(super) fn from_response(response: &Value) -> Option<Value> {
    response
        .get("_skill_grant")
        .filter(|g| g.is_array())
        .cloned()
}

/// The arguments a container tool call is executed with: the model's, with
/// runtime-only keys stripped, and — for `load-skill` only (by tool name or
/// resolved function path) — the server's grant attached. Same rule as
/// agent_step and the chat step: [`ai_tool_loop::tool_arguments`].
pub(super) fn tool_arguments(
    name: &str,
    function_ref: &str,
    model_arguments: Value,
    skill_grant: Option<&Value>,
) -> Value {
    ai_tool_loop::tool_arguments(name, function_ref, model_arguments, skill_grant)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::ai_tool_loop::{is_load_skill, LOAD_SKILL_PATH};
    use crate::handlers::StepHandler;
    use crate::types::{
        FlowCallbacks, FlowContext, FlowInstance, FlowNode, FlowResult, StepResult, StepType,
    };
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Shared with load-skill's JS test, which hands `executed_arguments` to
    /// the real tool and asserts it refuses.
    const FORGED: &str = include_str!(
        "../../../../../builtin-packages/ai-tools/tests/fixtures/load-skill-forged-grant.json"
    );

    fn fixture() -> Value {
        serde_json::from_str(FORGED).expect("fixture is JSON")
    }

    /// Returns one AI response and records the step skills the call carried
    /// and every function executed.
    struct Callbacks {
        response: Value,
        skills: Arc<Mutex<Vec<Vec<Value>>>>,
        executed: Arc<Mutex<Vec<(String, Value)>>>,
    }

    impl Callbacks {
        fn new(response: Value) -> Self {
            Self {
                response,
                skills: Arc::default(),
                executed: Arc::default(),
            }
        }
        fn executed(&self) -> Vec<(String, Value)> {
            self.executed.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl FlowCallbacks for Callbacks {
        async fn load_instance(&self, _path: &str) -> FlowResult<FlowInstance> {
            unimplemented!()
        }
        async fn save_instance(&self, _instance: &FlowInstance) -> FlowResult<()> {
            Ok(())
        }
        async fn save_instance_with_version(
            &self,
            _instance: &FlowInstance,
            _expected_version: i32,
        ) -> FlowResult<()> {
            Ok(())
        }
        async fn create_node(
            &self,
            _node_type: &str,
            _path: &str,
            _properties: Value,
        ) -> FlowResult<Value> {
            Ok(json!({}))
        }
        async fn update_node(&self, _path: &str, _properties: Value) -> FlowResult<Value> {
            Ok(json!({}))
        }
        async fn get_node(&self, _path: &str) -> FlowResult<Option<Value>> {
            Ok(None)
        }
        async fn queue_job(&self, _job_type: &str, _payload: Value) -> FlowResult<String> {
            Ok("job-1".to_string())
        }
        async fn call_ai(
            &self,
            _agent_workspace: &str,
            _agent_ref: &str,
            _messages: Vec<Value>,
            _response_format: Option<Value>,
        ) -> FlowResult<Value> {
            Ok(self.response.clone())
        }
        async fn call_ai_streaming_with_options(
            &self,
            agent_workspace: &str,
            agent_ref: &str,
            messages: Vec<Value>,
            response_format: Option<Value>,
            _extra_tools: Vec<Value>,
            skills: Vec<Value>,
        ) -> FlowResult<tokio::sync::mpsc::Receiver<Value>> {
            self.skills.lock().unwrap().push(skills);
            // The whole response as ONE chunk — the shape the real callback's
            // non-streaming fallback sends, `_tool_map` and `_skill_grant` included.
            self.call_ai_streaming(agent_workspace, agent_ref, messages, response_format)
                .await
        }
        async fn execute_function(&self, function_ref: &str, input: Value) -> FlowResult<Value> {
            self.executed
                .lock()
                .unwrap()
                .push((function_ref.to_string(), input));
            Ok(json!({ "success": true }))
        }
    }

    fn step(extra: &[(&str, Value)]) -> FlowNode {
        let mut properties = HashMap::new();
        properties.insert("agent_ref".to_string(), json!("/agents/writer"));
        properties.insert("tool_mode".to_string(), json!("auto"));
        for (k, v) in extra {
            properties.insert(k.to_string(), v.clone());
        }
        FlowNode {
            id: "writer".to_string(),
            step_type: StepType::AIContainer,
            properties,
            children: vec![],
            next_node: Some("end".to_string()),
        }
    }

    fn context() -> FlowContext {
        FlowContext::new("i-1".to_string(), json!({ "user_message": "Write it." }))
    }

    /// The AI turn: the given tool calls, plus the callback's side band.
    fn turn(fx: &Value, tool_calls: Value) -> Value {
        json!({
            "content": "",
            "finish_reason": "tool_calls",
            "tool_calls": tool_calls,
            "_tool_map": fx["tool_map"],
            "_skill_grant": fx["server_grant"],
        })
    }

    async fn run(callbacks: &Callbacks, node: &FlowNode) {
        let result = crate::handlers::AiContainerHandler::new()
            .execute(node, &mut context(), callbacks)
            .await
            .expect("container step runs");
        assert!(matches!(result, StepResult::SameStep { .. }), "{result:?}");
    }

    #[tokio::test]
    async fn a_forged_grant_in_the_models_arguments_never_reaches_load_skill() {
        let fx = fixture();
        let callbacks = Callbacks::new(turn(&fx, json!([fx["model_tool_call"]])));
        run(&callbacks, &step(&[])).await;

        let executed = callbacks.executed();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].0, LOAD_SKILL_PATH);
        // Exactly the server's grant — no forged entry, no forged chat_path,
        // no `_skill_grant` key: the arguments load-skill's test refuses with.
        assert_eq!(executed[0].1, fx["executed_arguments"]);
    }

    #[tokio::test]
    async fn an_honest_call_gets_the_servers_grant_and_the_step_skills_are_sent() {
        let fx = fixture();
        let honest = json!([{
            "id": "call_1",
            "type": "function",
            "function": { "name": "load-skill", "arguments": "{\"name\":\"granted\"}" }
        }]);
        let callbacks = Callbacks::new(turn(&fx, honest));
        let step_skill = json!({ "raisin:ref": "s1", "raisin:path": "/skills/house-style", "raisin:workspace": "functions" });
        run(
            &callbacks,
            &step(&[("skills", json!([step_skill.clone()]))]),
        )
        .await;

        assert_eq!(*callbacks.skills.lock().unwrap(), vec![vec![step_skill]]);
        let executed = callbacks.executed();
        assert_eq!(
            executed[0].1,
            json!({ "name": "granted", "__raisin_context": { "skill_grant": fx["server_grant"] } })
        );
    }

    #[tokio::test]
    async fn no_grant_from_the_server_means_an_empty_one_whatever_the_model_wrote() {
        let fx = fixture();
        let mut response = turn(&fx, json!([fx["model_tool_call"]]));
        response.as_object_mut().unwrap().remove("_skill_grant");
        let callbacks = Callbacks::new(response);
        run(&callbacks, &step(&[])).await;

        assert_eq!(
            callbacks.executed()[0].1,
            json!({ "name": "secret", "__raisin_context": { "skill_grant": [] } })
        );
    }

    #[tokio::test]
    async fn other_tools_lose_runtime_keys_and_get_no_grant() {
        let fx = fixture();
        let call = json!([{
            "id": "call_2",
            "function": {
                "name": "lookup",
                "arguments": "{\"q\":\"x\",\"__raisin_context\":{\"skill_grant\":[]},\"_skill_grant\":[]}"
            }
        }]);
        // `lookup` must be OFFERED to run at all (an unoffered name is refused).
        let mut response = turn(&fx, call);
        response["_tool_map"]["lookup"] = json!("/lib/x/lookup");
        let callbacks = Callbacks::new(response);
        run(&callbacks, &step(&[])).await;
        assert_eq!(callbacks.executed()[0].1, json!({ "q": "x" }));
    }

    #[tokio::test]
    async fn a_call_parked_for_an_explicit_executor_carries_no_forged_context() {
        let fx = fixture();
        let callbacks = Callbacks::new(turn(&fx, json!([fx["model_tool_call"]])));
        let result = crate::handlers::AiContainerHandler::new()
            .execute(
                &step(&[("tool_mode", json!("explicit"))]),
                &mut context(),
                &callbacks,
            )
            .await
            .expect("container step runs");
        let StepResult::Wait { metadata, .. } = result else {
            panic!("expected Wait, got {result:?}");
        };
        assert!(callbacks.executed().is_empty());
        assert_eq!(
            metadata["tool_calls"][0]["arguments"],
            json!({ "name": "secret" })
        );
    }

    #[test]
    fn load_skill_is_recognised_by_its_path_under_any_name() {
        assert!(is_load_skill("load-skill", "load-skill"));
        assert!(is_load_skill("skills", "/lib/raisin/ai/load-skill"));
        assert!(is_load_skill(
            "skills",
            "functions:/lib/raisin/ai/load-skill/"
        ));
        assert!(!is_load_skill("lookup", "/lib/studio/lookup"));

        let grant = json!([{ "name": "a", "workspace": "functions", "path": "/skills/a" }]);
        let args = tool_arguments(
            "skills",
            "/lib/raisin/ai/load-skill",
            json!({ "name": "b", "__raisin_context": { "skill_grant": [{ "name": "b" }] } }),
            Some(&grant),
        );
        assert_eq!(
            args,
            json!({ "name": "b", "__raisin_context": { "skill_grant": grant } })
        );
    }

    #[test]
    fn stripping_leaves_non_objects_and_other_keys_alone() {
        assert_eq!(strip_runtime_keys(json!("raw")), json!("raw"));
        assert_eq!(
            strip_runtime_keys(json!({ "a": 1, "__raisin_context": {}, "_skill_grant": [] })),
            json!({ "a": 1 })
        );
    }
}
