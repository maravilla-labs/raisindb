// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `raisin.packages.*` for RaisinFunctionApi (E1): start, cancel and query a
//! streaming package build as this function's caller, in its repository.

use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_packages_call(&self, method: &str, args: Value) -> Result<Value> {
        let ctx = &self.context;
        // A function with no recorded caller runs as the server (a trigger);
        // an anonymous caller may never export.
        let auth = match &ctx.auth_context {
            None => AuthContext::system(),
            Some(a) if a.is_anonymous_principal() && !a.is_system => {
                return Err(Error::Forbidden(
                    "forbidden: an anonymous caller cannot build packages".into(),
                ))
            }
            Some(a) => a.clone(),
        };
        raisin_rocksdb::package_build::call(
            raisin_rocksdb::package_build::CallScope {
                tenant: &ctx.tenant_id,
                repo: &ctx.repo_id,
                branch: &ctx.branch,
            },
            &auth,
            method,
            args,
        )
        .await
    }
}
