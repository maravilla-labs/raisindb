// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `raisin.agent_runs.*` for RaisinFunctionApi: the same transport-neutral
//! run API the HTTP routes serve, with the function's own caller identity.

use raisin_agent_runtime::api::{self, Caller};
use raisin_agent_runtime::ids::{RunId, SystemToken};
use raisin_error::{Error, Result};
use serde_json::Value;

use super::RaisinFunctionApi;

fn api_err(e: api::ApiError) -> Error {
    Error::Validation(format!("{}: {}", e.code, e.message))
}

fn field<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Validation(format!("agent_runs: '{name}' is required")))
}

fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| Error::Validation(format!("agent_runs: {e}")))
}

fn json<T: serde::Serialize>(v: T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| Error::Internal(e.to_string()))
}

impl RaisinFunctionApi {
    /// The caller a function acts as: its user, or the system when it runs
    /// in a system context (a trigger, `execution_context: system`).
    fn agent_run_caller(&self) -> Result<Caller> {
        match &self.context.auth_context {
            None => Ok(Caller {
                id: "system".into(),
                admin: true,
            }),
            Some(a) if a.is_system => Ok(Caller {
                id: "system".into(),
                admin: true,
            }),
            Some(a) if a.is_anonymous_principal() => Err(Error::Validation(
                "agent_runs: an anonymous caller cannot use runs".into(),
            )),
            Some(a) => Ok(Caller {
                id: a.user_id.clone().unwrap_or_default(),
                admin: false,
            }),
        }
    }

    pub(crate) async fn impl_agent_runs_call(&self, method: &str, args: Value) -> Result<Value> {
        let host = raisin_rocksdb::agent_runs::agent_run_host()
            .ok_or_else(|| Error::Validation("agent runs are not running on this server".into()))?;
        let caller = self.agent_run_caller()?;
        let ctx = &self.context;
        let branch = args
            .get("branch")
            .and_then(Value::as_str)
            .unwrap_or(&ctx.branch);
        let scope = api::scope(&ctx.tenant_id, &ctx.repo_id, Some(branch));
        match method {
            "create" => {
                // A run a function creates executes on the function's branch
                // unless it names another.
                let mut args = args;
                if let Some(obj) = args.as_object_mut() {
                    obj.entry("branch")
                        .or_insert_with(|| Value::String(ctx.branch.clone()));
                }
                json(
                    api::create(&host, &ctx.tenant_id, &ctx.repo_id, &caller, parse(args)?)
                        .await
                        .map_err(api_err)?,
                )
            }
            "get" => {
                let run = RunId(field(&args, "run_id")?.to_string());
                json(
                    api::get(&host, &scope, &run, &caller)
                        .await
                        .map_err(api_err)?,
                )
            }
            "events" => {
                let run = RunId(field(&args, "run_id")?.to_string());
                let after = args.get("after_seq").and_then(Value::as_u64).unwrap_or(0);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(500)
                    .min(5_000);
                json(
                    api::events(&host, &scope, &run, &caller, after, limit as usize)
                        .await
                        .map_err(api_err)?,
                )
            }
            "control" => {
                let run = RunId(field(&args, "run_id")?.to_string());
                let req: api::ControlRequest = parse(args)?;
                json(
                    api::control(&host, &scope, &run, &caller, req)
                        .await
                        .map_err(api_err)?,
                )
            }
            // A tool that answered `waiting` is completed later by whoever
            // holds its result — only a system context may deliver it.
            "deliver" => {
                if !caller.admin {
                    return Err(Error::Validation(
                        "agent_runs.deliver needs a system execution context".into(),
                    ));
                }
                let run = RunId(field(&args, "run_id")?.to_string());
                let envelope = args.get("envelope").cloned().unwrap_or(Value::Null);
                let ack = host
                    .service()
                    .deliver_external_result(
                        &scope,
                        &run,
                        field(&args, "resume_key")?,
                        field(&args, "delivery_id")?,
                        envelope,
                        &SystemToken::in_process(),
                    )
                    .await
                    .map_err(|e| Error::Validation(format!("agent_runs.deliver: {e}")))?;
                json(ack)
            }
            other => {
                self.impl_agent_runs_child_call(other, &host, &scope, &caller, args)
                    .await
            }
        }
    }
}
