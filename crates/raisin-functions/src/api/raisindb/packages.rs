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
        // Who is calling, as the server knows it: the execution's own
        // identity (an absent one is the bare system, which may act on no
        // build), and — only for a genuine trigger run — the node whose event
        // fired it. Nothing here is read from the arguments.
        let auth = match &ctx.auth_context {
            Some(a) if a.is_anonymous_principal() && !a.is_system => {
                return Err(Error::Forbidden(
                    "forbidden: an anonymous caller cannot use package builds".into(),
                ))
            }
            Some(a) => a.clone(),
            None => AuthContext::system(),
        };
        // The invoking person, if any: `caller` is set by the server from the
        // invoke's credential (never from input), and is a person only when
        // it is neither the system (an API key, an admin credential) nor
        // anonymous.
        let invoking_person = ctx
            .caller
            .as_ref()
            .filter(|c| !c.is_system && !c.anonymous)
            .and_then(|c| c.user_id.clone());
        let invoker =
            raisin_rocksdb::package_build::Invoker::from_execution(auth, ctx.event_data.as_ref())
                .with_invoking_person(invoking_person);
        raisin_rocksdb::package_build::call(
            raisin_rocksdb::package_build::CallScope {
                tenant: &ctx.tenant_id,
                repo: &ctx.repo_id,
                branch: &ctx.branch,
            },
            &invoker,
            method,
            args,
        )
        .await
    }
}
