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

use crate::error::{GraphError, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{NodeRepository, Storage, StorageScope, UpdateNodeOptions};
use std::collections::HashMap;
use std::sync::Arc;

/// Write float results (e.g. PageRank) to node properties
pub async fn write_float_results<S: Storage>(
    storage: &Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    results: HashMap<String, f64>,
) -> Result<()> {
    let scope = StorageScope::new(tenant_id, repo_id, branch, workspace);
    for (node_id, value) in results {
        if let Some(mut node) = storage
            .nodes()
            .get(scope, &node_id, None)
            .await
            .map_err(|e| GraphError::Storage(e.to_string()))?
        {
            node.properties
                .insert(property_name.to_string(), PropertyValue::Float(value));

            storage
                .nodes()
                .update(scope, node, UpdateNodeOptions::default())
                .await
                .map_err(|e| GraphError::Storage(e.to_string()))?;
        }
    }
    Ok(())
}

/// Write integer results (e.g. Community ID) to node properties
pub async fn write_integer_results<S: Storage>(
    storage: &Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    results: HashMap<String, u32>,
) -> Result<()> {
    let scope = StorageScope::new(tenant_id, repo_id, branch, workspace);
    for (node_id, value) in results {
        if let Some(mut node) = storage
            .nodes()
            .get(scope, &node_id, None)
            .await
            .map_err(|e| GraphError::Storage(e.to_string()))?
        {
            node.properties.insert(
                property_name.to_string(),
                PropertyValue::Integer(value as i64),
            );

            storage
                .nodes()
                .update(scope, node, UpdateNodeOptions::default())
                .await
                .map_err(|e| GraphError::Storage(e.to_string()))?;
        }
    }
    Ok(())
}
