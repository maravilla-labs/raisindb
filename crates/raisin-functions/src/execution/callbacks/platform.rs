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

//! `raisin.platform.hook(name, payload?)` — call an OPERATOR-configured
//! platform endpoint from a function.
//!
//! # Why this exists
//!
//! `raisin.http.fetch` refuses loopback, private and link-local addresses, and
//! must keep doing so: it is content-authored code running with `raisin.secrets`
//! in scope, and the plugin module's rule is that a tenant function never makes
//! an internal HTTP call. But the platform around RaisinDB (Flightdeck's
//! "install the new Studio package into this tenant", for one) is reached on
//! exactly such an address. Opening the guard for an allowlisted origin would
//! open it for EVERY tenant function — the wrong trade.
//!
//! So the call is a trusted binding instead, the same shape as the media and
//! screenshot services: the OPERATOR names each hook in the server config
//! (`[platform.hooks.<name>] url, token`), and a function may only say WHICH
//! named hook to fire and hand it a JSON payload. The URL and token never pass
//! through tenant data, and the tenant id is injected from the execution
//! context — a function cannot fire a hook on another tenant's behalf.
//!
//! The request goes through the shared client, not the resolver-guarded one,
//! because the operator chose the address on purpose.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use raisin_error::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::PlatformHookCallback;
use crate::execution::types::shared_http_client;

/// One operator-configured endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformHook {
    /// Full URL the hook POSTs to.
    pub url: String,
    /// Shared secret sent in `token_header`. Prefer `token_env` in files that
    /// are checked in; `token` wins when both are set.
    #[serde(default)]
    pub token: Option<String>,
    /// Environment variable holding the token, read once at configure time.
    #[serde(default)]
    pub token_env: Option<String>,
    /// Header the token is sent in.
    #[serde(default = "default_token_header")]
    pub token_header: String,
    /// Per-call timeout.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_token_header() -> String {
    "x-internal-token".to_string()
}

fn default_timeout_ms() -> u64 {
    120_000
}

#[derive(Debug, Clone)]
struct ResolvedHook {
    url: String,
    token: Option<String>,
    token_header: String,
    timeout_ms: u64,
}

fn hooks() -> &'static RwLock<HashMap<String, ResolvedHook>> {
    static HOOKS: OnceLock<RwLock<HashMap<String, ResolvedHook>>> = OnceLock::new();
    HOOKS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Install the operator's `[platform.hooks]`. Called once from the server
/// binary; a later call replaces the set (so a config reload can be wired
/// without a restart).
///
/// A hook whose `token_env` is set but unset in the environment is installed
/// WITHOUT a token and logged, rather than dropped: the operator then sees
/// the endpoint's 401 instead of a "hook not configured" that looks like a
/// typo in the function.
pub fn configure_platform_hooks(config: HashMap<String, PlatformHook>) {
    let mut resolved = HashMap::with_capacity(config.len());
    for (name, hook) in config {
        let token = hook.token.clone().or_else(|| {
            hook.token_env.as_deref().and_then(|var| match std::env::var(var) {
                Ok(v) if !v.is_empty() => Some(v),
                _ => {
                    tracing::warn!(
                        hook = %name,
                        env = %var,
                        "[platform.hooks] token_env is not set; the hook will be called without a token"
                    );
                    None
                }
            })
        });
        resolved.insert(
            name,
            ResolvedHook {
                url: hook.url,
                token,
                token_header: hook.token_header,
                timeout_ms: hook.timeout_ms,
            },
        );
    }
    let count = resolved.len();
    *hooks().write().unwrap_or_else(|e| e.into_inner()) = resolved;
    tracing::info!(hooks = count, "platform hooks configured");
}

fn lookup(name: &str) -> Option<ResolvedHook> {
    hooks()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .cloned()
}

/// Create the `raisin.platform.hook` callback for one execution.
///
/// `tenant_id` / `repo_id` are stamped INTO the payload (overwriting anything
/// the function put there), so the receiving endpoint can trust them.
pub fn create_platform_hook(tenant_id: String, repo_id: String) -> PlatformHookCallback {
    Arc::new(move |name: String, payload: Value| {
        let tenant_id = tenant_id.clone();
        let repo_id = repo_id.clone();
        Box::pin(async move {
            let Some(hook) = lookup(&name) else {
                return Err(raisin_error::Error::Validation(format!(
                    "platform hook `{name}` is not configured on this server \
                     ([platform.hooks.{name}] in the server config)"
                )));
            };

            let mut body = match payload {
                Value::Object(map) => map,
                Value::Null => serde_json::Map::new(),
                _ => {
                    return Err(raisin_error::Error::Validation(
                        "platform hook payload must be a JSON object".to_string(),
                    ))
                }
            };
            body.insert("tenant_id".to_string(), Value::String(tenant_id));
            body.insert("repo_id".to_string(), Value::String(repo_id));
            body.insert("hook".to_string(), Value::String(name.clone()));

            let mut request = shared_http_client()
                .post(&hook.url)
                .timeout(std::time::Duration::from_millis(hook.timeout_ms))
                .json(&Value::Object(body));
            if let Some(token) = &hook.token {
                request = request.header(hook.token_header.as_str(), token.as_str());
            }

            let response = request.send().await.map_err(|e| {
                raisin_error::Error::Backend(format!("platform hook `{name}` failed: {e}"))
            })?;
            let status = response.status().as_u16();
            let text = response.text().await.map_err(|e| {
                raisin_error::Error::Backend(format!(
                    "platform hook `{name}`: failed to read response body: {e}"
                ))
            })?;
            let body_value = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
            Ok(json!({
                "ok": (200..300).contains(&status),
                "status": status,
                "body": body_value,
            }))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_unconfigured_hook_is_refused_by_name() {
        let cb = create_platform_hook("t".into(), "r".into());
        let err = cb("nope".to_string(), json!({}))
            .await
            .expect_err("unknown hook must be refused");
        assert!(err.to_string().contains("nope"));
    }

    #[tokio::test]
    async fn a_non_object_payload_is_refused() {
        configure_platform_hooks(HashMap::from([(
            "probe".to_string(),
            PlatformHook {
                url: "http://127.0.0.1:9/never".to_string(),
                token: None,
                token_env: None,
                token_header: default_token_header(),
                timeout_ms: 50,
            },
        )]));
        let cb = create_platform_hook("t".into(), "r".into());
        assert!(cb("probe".to_string(), json!([1, 2])).await.is_err());
    }
}
