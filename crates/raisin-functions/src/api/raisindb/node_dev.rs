// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `raisin.node_dev.*` for RaisinFunctionApi: the transport-neutral
//! node-development dispatch, as this function's caller. When the call is a
//! tool call of an agent run (`__raisin_context.run_id`), the run's own
//! grant (`executor_config.node_dev.roots` on the RUN RECORD) narrows every
//! root the arguments ask for.

use raisin_core::services::node_dev::{dispatch, DevScope};
use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_node_dev_call(&self, method: &str, args: Value) -> Result<Value> {
        let svc = raisin_rocksdb::node_dev::node_dev().ok_or_else(|| {
            Error::Validation("the node-development surface is not running on this server".into())
        })?;
        let ctx = &self.context;
        let auth = match &ctx.auth_context {
            None => AuthContext::system(),
            Some(a) if a.is_anonymous_principal() && !a.is_system => {
                return Err(Error::Validation(
                    "node_dev: an anonymous caller cannot develop".into(),
                ))
            }
            Some(a) => a.clone(),
        };
        let branch = args
            .get("branch")
            .and_then(Value::as_str)
            .unwrap_or(&ctx.branch)
            .to_string();
        let scope = DevScope::new(&ctx.tenant_id, &ctx.repo_id, &branch);
        // The run is looked up on the function's OWN branch, never on a
        // branch the arguments name, so a `branch` argument cannot dodge the
        // run's grant; the grant then applies whatever branch is targeted.
        let grant = match raisin_rocksdb::node_dev::tool_run_id(&args) {
            Some(run) => {
                raisin_rocksdb::node_dev::run_grant(&ctx.tenant_id, &ctx.repo_id, &ctx.branch, &run)
                    .await
            }
            None => None,
        };
        dispatch::call(&svc, &scope, &auth, method, args, grant.as_deref())
            .await
            .map_err(|e| Error::Validation(format!("{}: {}", e.code, e.message)))
    }
}
