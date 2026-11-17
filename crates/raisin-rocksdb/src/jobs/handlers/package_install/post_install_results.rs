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

//! Recording the `post_install` phase's results on the package node.

use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::Storage;
use std::collections::HashMap;

use super::handler::PackageInstallHandler;
use super::post_install::PostInstallResult;

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Write `post_install_results` on the package node. Best effort: the
    /// results are also in the job result and the logs, and a repository on
    /// an older strict `raisin:Package` schema refuses the property.
    pub(super) async fn record_post_install_results(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        package_node_id: &str,
        results: &[PostInstallResult],
    ) {
        let write = async {
            let tx = self.storage.begin_context().await?;
            tx.set_tenant_repo(tenant_id, repo_id)?;
            tx.set_branch(branch)?;
            tx.set_actor("package-installer")?;
            tx.set_content_import(true)?;
            tx.set_auth_context(AuthContext::system())?;
            tx.set_message("Record package post_install results")?;
            if let Some(mut node) = tx.get_node("packages", package_node_id).await? {
                node.properties.insert(
                    "post_install_results".to_string(),
                    results_property(results),
                );
                tx.upsert_node("packages", &node).await?;
                tx.commit().await?;
            }
            Ok::<(), Error>(())
        };
        if let Err(e) = write.await {
            tracing::warn!(
                package_node_id = %package_node_id,
                error = %e,
                "Failed to record post_install results on the package node"
            );
        }
    }
}

fn results_property(results: &[PostInstallResult]) -> PropertyValue {
    PropertyValue::Array(
        results
            .iter()
            .map(|r| {
                let mut obj = HashMap::from([
                    (
                        "function".to_string(),
                        PropertyValue::String(r.function.clone()),
                    ),
                    (
                        "status".to_string(),
                        PropertyValue::String(r.status.clone()),
                    ),
                    (
                        "duration_ms".to_string(),
                        PropertyValue::Integer(r.duration_ms as i64),
                    ),
                ]);
                if let Some(e) = &r.error {
                    obj.insert("error".to_string(), PropertyValue::String(e.clone()));
                }
                PropertyValue::Object(obj)
            })
            .collect(),
    )
}
