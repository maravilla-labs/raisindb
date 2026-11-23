// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! The ONE host gateway every function runtime calls `raisin.*` through.
//!
//! The work behind a host call — every `raisin.*` method — already had a
//! single implementation: the bindings registry's invoker over
//! [`FunctionApi`]. What each runtime carried its own copy of was everything
//! AROUND the call: the policy check, the execution's deadline, a stop, the
//! error envelope and the tracing. The copies drifted: a stopped agent run
//! dropped its provider request in JavaScript only, Starlark had neither a
//! deadline nor a stop, and WebAssembly ignored a stop.
//!
//! [`HostGateway`] owns all of that. A runtime adapter only marshals its own
//! values to JSON and back and, for the two runtimes whose interpreter is
//! synchronous (QuickJS, Starlark), blocks its thread on the call with
//! [`call_blocking`]. A deadline or a stop therefore behaves the same way in
//! every language: the call's future is dropped (with it any HTTP request it
//! was making) and the caller gets a [`HostError`] of class
//! [`HostErrorClass::Timeout`] or [`HostErrorClass::Cancelled`].
//!
//! Error MESSAGES are exactly what each runtime reported before, so code that
//! matches on them keeps working; the CLASS is new and travels beside the
//! message (`error_class` in the JSON envelope).

use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::api::FunctionApi;
use crate::runtime::bindings::methods::registry;
use crate::runtime::bindings::registry::InvokeResult;
use crate::types::ExecutionPolicy;

/// What kind of failure a host call ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostErrorClass {
    /// The execution's policy refuses host calls (a deterministic function).
    Denied,
    /// No such method.
    UnknownMethod,
    /// The arguments were not valid JSON.
    InvalidArguments,
    /// The execution's deadline passed during the call.
    Timeout,
    /// The execution was cancelled (a stopped agent run) during the call.
    Cancelled,
    /// The method itself failed.
    Failed,
}

impl HostErrorClass {
    /// The wire name (`error_class` in the envelope).
    pub fn as_str(self) -> &'static str {
        match self {
            HostErrorClass::Denied => "denied",
            HostErrorClass::UnknownMethod => "unknown_method",
            HostErrorClass::InvalidArguments => "invalid_arguments",
            HostErrorClass::Timeout => "timeout",
            HostErrorClass::Cancelled => "cancelled",
            HostErrorClass::Failed => "failed",
        }
    }
}

/// A failed host call: its class, and the message the runtimes always used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    /// What kind of failure.
    pub class: HostErrorClass,
    /// The message, unchanged from what the runtimes reported before.
    pub message: String,
}

impl HostError {
    fn new(class: HostErrorClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
        }
    }

    /// The JSON error envelope the text runtimes hand their wrapper code:
    /// `{"error": true, "message": …, "error_class": …}`.
    pub fn envelope(&self) -> String {
        serde_json::to_string(&serde_json::json!({
            "error": true,
            "message": self.message,
            "error_class": self.class.as_str(),
        }))
        .unwrap_or_else(|_| r#"{"error":true,"message":"Unknown error"}"#.to_string())
    }
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HostError {}

/// One execution's door to the host. Cheap to clone.
#[derive(Clone)]
pub struct HostGateway {
    api: Arc<dyn FunctionApi>,
    allow: bool,
    deadline: Option<Instant>,
    cancel: Option<CancellationToken>,
}

impl HostGateway {
    /// A gateway for an execution under `policy`, with no deadline and no
    /// cancellation.
    pub fn new(api: Arc<dyn FunctionApi>, policy: ExecutionPolicy) -> Self {
        Self {
            api,
            allow: policy.allows_host_calls(),
            deadline: None,
            cancel: None,
        }
    }

    /// The gateway of execution `execution_id`: its deadline, and the
    /// cancellation a caller registered for it (an agent run's operation; see
    /// `raisin_storage::jobs::execution_cancel`), if any.
    pub fn for_execution(
        api: Arc<dyn FunctionApi>,
        policy: ExecutionPolicy,
        execution_id: &str,
        deadline: Instant,
    ) -> Self {
        Self::new(api, policy).with_deadline(deadline).with_cancel(
            raisin_storage::jobs::execution_cancel::token_for(execution_id),
        )
    }

    /// Bound every call by `deadline`.
    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Give up any call when `cancel` fires.
    pub fn with_cancel(mut self, cancel: Option<CancellationToken>) -> Self {
        self.cancel = cancel;
        self
    }

    /// The API the gateway calls into.
    pub fn api(&self) -> &Arc<dyn FunctionApi> {
        &self.api
    }

    /// The execution's cancellation, if it has one.
    pub fn cancel_token(&self) -> Option<&CancellationToken> {
        self.cancel.as_ref()
    }

    /// Whether the execution was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.is_cancelled())
    }

    /// Refuse a host call when the policy denies them.
    pub fn check_allowed(&self, method: &str) -> Result<(), HostError> {
        if self.allow {
            Ok(())
        } else {
            Err(HostError::new(
                HostErrorClass::Denied,
                format!("{}: {method}", ExecutionPolicy::HOST_DENIED),
            ))
        }
    }

    /// Whether `method` is a registry (`raisin.*`) method.
    pub fn is_registry_method(method: &str) -> bool {
        registry().find_by_internal_name(method).is_some()
    }

    /// A JSON argument string as the registry's argument list: an array is
    /// the list, anything else is one argument.
    pub fn parse_args(method: &str, args_json: &str) -> Result<Vec<Value>, HostError> {
        match serde_json::from_str(args_json) {
            Ok(Value::Array(items)) => Ok(items),
            Ok(other) => Ok(vec![other]),
            Err(e) => {
                tracing::error!(method = %method, error = %e, "host call: invalid arguments JSON");
                Err(HostError::new(
                    HostErrorClass::InvalidArguments,
                    format!("Invalid arguments for {}: {}", method, e),
                ))
            }
        }
    }

    /// Call registry method `method` with JSON-encoded `args_json`: the
    /// policy, then the method, then the arguments, then the call.
    pub async fn call_json(&self, method: &str, args_json: &str) -> Result<String, HostError> {
        self.check_allowed(method)?;
        if !Self::is_registry_method(method) {
            tracing::error!(method = %method, "host call: unknown method");
            return Err(HostError::new(
                HostErrorClass::UnknownMethod,
                format!("Unknown raisin API method: {}", method),
            ));
        }
        let args = Self::parse_args(method, args_json)?;
        self.call(method, args).await.map(|r| r.to_json_string())
    }

    /// Call registry method `method` with `args`.
    pub async fn call(&self, method: &str, args: Vec<Value>) -> Result<InvokeResult, HostError> {
        self.check_allowed(method)?;
        let Some(descriptor) = registry().find_by_internal_name(method) else {
            tracing::error!(method = %method, "host call: unknown method");
            return Err(HostError::new(
                HostErrorClass::UnknownMethod,
                format!("Unknown raisin API method: {}", method),
            ));
        };
        let invoker = descriptor.invoker;
        let result = self
            .guarded(method, (invoker)(self.api.clone(), args))
            .await?;
        result.map_err(|e| {
            tracing::error!(method = %method, error = %e, "raisin API call failed");
            HostError::new(HostErrorClass::Failed, e.to_string())
        })
    }

    /// Call function-binding plugin method `method` (`raisin.<ns>.*`).
    pub async fn plugin_call(&self, method: &str, args: Value) -> Result<Value, HostError> {
        self.check_allowed(method)?;
        let result = self
            .guarded(method, self.api.plugin_call(method, args))
            .await?;
        result.map_err(|e| {
            tracing::error!(method = %method, error = %e, "plugin API call failed");
            HostError::new(HostErrorClass::Failed, e.to_string())
        })
    }

    /// Run `fut` under the execution's deadline and cancellation. Either one
    /// drops `fut` — and with it any I/O it was waiting on.
    pub async fn guarded<F: std::future::Future>(
        &self,
        what: &str,
        fut: F,
    ) -> Result<F::Output, HostError> {
        let timed_out = || {
            HostError::new(
                HostErrorClass::Timeout,
                format!("function timed out during host call `{what}`"),
            )
        };
        let cancelled = || {
            HostError::new(
                HostErrorClass::Cancelled,
                format!("execution cancelled during host call `{what}`"),
            )
        };
        if self.is_cancelled() {
            return Err(cancelled());
        }
        let deadline = async {
            match self.deadline {
                Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(d)).await,
                None => std::future::pending::<()>().await,
            }
        };
        let stop = async {
            match &self.cancel {
                Some(c) => c.cancelled().await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            r = fut => Ok(r),
            _ = deadline => Err(timed_out()),
            _ = stop => Err(cancelled()),
        }
    }
}

/// Block the current thread on `future` without stalling the async runtime's
/// other tasks. For the runtimes whose interpreter is synchronous (QuickJS,
/// Starlark): a host call made from inside the interpreter can only be waited
/// for here. Needs the multi-threaded runtime, like every text function.
pub fn call_blocking<F: std::future::Future>(future: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::MockFunctionApi;
    use std::time::Duration;

    fn api() -> Arc<dyn FunctionApi> {
        Arc::new(MockFunctionApi::new(serde_json::json!({})))
    }

    #[tokio::test]
    async fn a_denied_call_names_the_method_and_its_class() {
        let gw = HostGateway::new(api(), ExecutionPolicy::Deterministic);
        let e = gw.call_json("context_get", "[]").await.unwrap_err();
        assert_eq!(e.class, HostErrorClass::Denied);
        assert_eq!(
            e.message,
            format!("{}: context_get", ExecutionPolicy::HOST_DENIED)
        );
        let env: Value = serde_json::from_str(&e.envelope()).unwrap();
        assert_eq!(env["error"], true);
        assert_eq!(env["error_class"], "denied");
    }

    #[tokio::test]
    async fn unknown_methods_and_bad_arguments_keep_their_messages() {
        let gw = HostGateway::new(api(), ExecutionPolicy::Standard);
        let e = gw.call_json("no_such_method", "[]").await.unwrap_err();
        assert_eq!(e.class, HostErrorClass::UnknownMethod);
        assert_eq!(e.message, "Unknown raisin API method: no_such_method");
        let e = gw.call_json("context_get", "{not json").await.unwrap_err();
        assert_eq!(e.class, HostErrorClass::InvalidArguments);
        assert!(e.message.starts_with("Invalid arguments for context_get: "));
    }

    #[tokio::test]
    async fn the_deadline_and_the_stop_drop_the_call() {
        let gw = HostGateway::new(api(), ExecutionPolicy::Standard)
            .with_deadline(Instant::now() + Duration::from_millis(50));
        let e = gw
            .guarded("ai_completion", tokio::time::sleep(Duration::from_secs(30)))
            .await
            .unwrap_err();
        assert_eq!(e.class, HostErrorClass::Timeout);
        assert_eq!(
            e.message,
            "function timed out during host call `ai_completion`"
        );

        let token = CancellationToken::new();
        let gw =
            HostGateway::new(api(), ExecutionPolicy::Standard).with_cancel(Some(token.clone()));
        let t = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            t.cancel();
        });
        let e = gw
            .guarded("ai_completion", tokio::time::sleep(Duration::from_secs(30)))
            .await
            .unwrap_err();
        assert_eq!(e.class, HostErrorClass::Cancelled);
        assert_eq!(
            e.message,
            "execution cancelled during host call `ai_completion`"
        );
    }
}
