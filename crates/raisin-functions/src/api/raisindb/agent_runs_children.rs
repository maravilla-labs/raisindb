// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `raisin.agent_runs.*` child-run, mailbox, checkpoint and usage methods —
//! the same transport-neutral API as `/api/agent-runs/{repo}/{run}/children…`.
//!
//! Inside a tool call the parent is the CALLING run: `run_id` defaults to
//! `__raisin_context.run_id`, so a delegation tool needs no plumbing.

use raisin_agent_runtime::api::Caller;
use raisin_agent_runtime::api_child as child;
use raisin_agent_runtime::host::AgentRunHost;
use raisin_agent_runtime::ids::{RunId, RunScope};
use raisin_error::{Error, Result};
use serde_json::Value;

use super::RaisinFunctionApi;

fn api_err(e: raisin_agent_runtime::api::ApiError) -> Error {
    Error::Validation(format!("{}: {}", e.code, e.message))
}

fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| Error::Validation(format!("agent_runs: {e}")))
}

fn json<T: serde::Serialize>(v: T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| Error::Internal(e.to_string()))
}

/// `run_id`, or the calling run from `__raisin_context`.
fn run_of(args: &Value) -> Result<RunId> {
    args.get("run_id")
        .and_then(Value::as_str)
        .or_else(|| {
            args.pointer("/__raisin_context/run_id")
                .and_then(Value::as_str)
        })
        .map(|s| RunId(s.to_string()))
        .ok_or_else(|| Error::Validation("agent_runs: 'run_id' is required".into()))
}

fn child_of(args: &Value) -> Result<RunId> {
    args.get("child_run_id")
        .and_then(Value::as_str)
        .map(|s| RunId(s.to_string()))
        .ok_or_else(|| Error::Validation("agent_runs: 'child_run_id' is required".into()))
}

/// The request body without the addressing fields.
fn body(mut args: Value) -> Value {
    if let Some(o) = args.as_object_mut() {
        for k in ["run_id", "child_run_id", "branch", "__raisin_context"] {
            o.remove(k);
        }
    }
    args
}

impl RaisinFunctionApi {
    pub(crate) async fn impl_agent_runs_child_call(
        &self,
        method: &str,
        host: &AgentRunHost,
        scope: &RunScope,
        caller: &Caller,
        args: Value,
    ) -> Result<Value> {
        let run = run_of(&args);
        match method {
            "spawn_child" => json(
                child::spawn(host, scope, &run?, caller, parse(body(args))?)
                    .await
                    .map_err(api_err)?,
            ),
            "children" => json(
                child::children(host, scope, &run?, caller)
                    .await
                    .map_err(api_err)?,
            ),
            "inspect_child" => {
                let c = child_of(&args)?;
                let after = args.get("after_seq").and_then(Value::as_u64).unwrap_or(0);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(200)
                    .min(5_000) as usize;
                json(
                    child::inspect(host, scope, &run?, &c, caller, after, limit)
                        .await
                        .map_err(api_err)?,
                )
            }
            "control_child" => {
                let c = child_of(&args)?;
                json(
                    child::control_child(host, scope, &run?, &c, caller, parse(body(args))?)
                        .await
                        .map_err(api_err)?,
                )
            }
            "wait_child" => {
                let c = child_of(&args)?;
                json(
                    child::wait_child(host, scope, &run?, &c, caller, parse(body(args))?)
                        .await
                        .map_err(api_err)?,
                )
            }
            "mailbox" => json(
                child::mailbox(host, scope, &run?, caller)
                    .await
                    .map_err(api_err)?,
            ),
            "ack_mailbox" => {
                let up_to = args.get("up_to").and_then(Value::as_u64).ok_or_else(|| {
                    Error::Validation("agent_runs.ack_mailbox: 'up_to' is required".into())
                })?;
                json(
                    child::ack_mailbox(host, scope, &run?, caller, up_to)
                        .await
                        .map_err(api_err)?,
                )
            }
            "post_to_parent" => json(
                child::post_to_parent(host, scope, &run?, caller, parse(body(args))?)
                    .await
                    .map_err(api_err)?,
            ),
            "checkpoint" => {
                // A compaction running as the calling operation names itself.
                let mut b = body(args.clone());
                if let (Some(o), Some(op)) = (
                    b.as_object_mut(),
                    args.pointer("/__raisin_context/operation_id"),
                ) {
                    o.entry("operation_id").or_insert_with(|| op.clone());
                }
                json(
                    child::checkpoint(host, scope, &run?, caller, parse(b)?)
                        .await
                        .map_err(api_err)?,
                )
            }
            "read_checkpoint" => {
                let n = args
                    .get("checkpoint_no")
                    .and_then(Value::as_u64)
                    .map(|n| n as u32);
                json(
                    child::read_checkpoint(host, scope, &run?, caller, n)
                        .await
                        .map_err(api_err)?,
                )
            }
            "usage" => json(
                child::usage(host, scope, &run?, caller)
                    .await
                    .map_err(api_err)?,
            ),
            other => Err(Error::Validation(format!(
                "agent_runs: unknown method '{other}'"
            ))),
        }
    }
}
