// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Durable agent run bindings (`raisin.agent_runs.*`), for every runtime.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

macro_rules! run_method {
    ($internal:literal, $name:literal) => {
        run_method!($internal, $name, $name)
    };
    ($internal:literal, $js:literal, $name:literal) => {
        ApiMethodDescriptor {
            internal_name: $internal,
            js_name: $js,
            py_name: $name,
            category: "agent_runs",
            args: vec![ArgSpec::new("request", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let request = parser.json()?;
                    let result = api.agent_runs_call($name, request).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        }
    };
}

/// All agent run method descriptors.
///
/// - `create(request)`: `{ subject, input?, reducer?, agent_ref?, as_agent?,
///   budgets?, create_key?, executor_config?, branch? }` → `{ run_id,
///   created, status }`
/// - `get({ run_id })` → `{ run, status, projection }`
/// - `events({ run_id, after_seq?, limit? })` → `[event]`
/// - `control({ run_id, control_id, command, capability? })` → ack
/// - `deliver({ run_id, resume_key, delivery_id, envelope })` → ack
///   (system context only: completes a tool that answered `waiting`)
///
/// Child runs, mailbox, checkpoints, usage. `run_id` defaults to the calling
/// run (`__raisin_context.run_id`) inside a tool call:
/// - `spawn_child({ run_id?, objective, budgets?, on_exceeded?, spawn_key?,
///   subject?, as_agent?, agent_ref?, input?, executor_config?, reducer?,
///   inherit_reducer? })` → `{ child_run_id, child_no, created, budgets,
///   resume_key }` — a tool may then answer `waiting` with `resume_key` and
///   is completed by the child's hand-back
/// - `children({ run_id? })` → `[{ link, status, usage }]`
/// - `inspect_child({ run_id?, child_run_id, after_seq?, limit? })`
/// - `control_child({ run_id?, child_run_id, control_id, action: message |
///   steer | interrupt | resume, message? | input? | mode? , reason? })` → ack
/// - `wait_child({ run_id?, child_run_id, fence, expires_at_ms? })` → status
/// - `mailbox({ run_id? })` / `ack_mailbox({ run_id?, up_to })`
/// - `post_to_parent({ run_id?, message_id, message })` → `{ mail_no }`
/// - `checkpoint({ run_id?, fence?, operation_id?, reason?, summary?,
///   transcript_cutoff?, state?, large_refs? })` → checkpoint
/// - `read_checkpoint({ run_id?, checkpoint_no? })` → `{ checkpoint, state }`
/// - `usage({ run_id? })` → usage accounting
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        run_method!("agent_runs_create", "create"),
        run_method!("agent_runs_get", "get"),
        run_method!("agent_runs_events", "events"),
        run_method!("agent_runs_control", "control"),
        run_method!("agent_runs_deliver", "deliver"),
        run_method!("agent_runs_spawn_child", "spawnChild", "spawn_child"),
        run_method!("agent_runs_children", "children"),
        run_method!("agent_runs_inspect_child", "inspectChild", "inspect_child"),
        run_method!("agent_runs_control_child", "controlChild", "control_child"),
        run_method!("agent_runs_wait_child", "waitChild", "wait_child"),
        run_method!("agent_runs_mailbox", "mailbox"),
        run_method!("agent_runs_ack_mailbox", "ackMailbox", "ack_mailbox"),
        run_method!(
            "agent_runs_post_to_parent",
            "postToParent",
            "post_to_parent"
        ),
        run_method!("agent_runs_checkpoint", "checkpoint"),
        run_method!(
            "agent_runs_read_checkpoint",
            "readCheckpoint",
            "read_checkpoint"
        ),
        run_method!("agent_runs_usage", "usage"),
    ]
}
