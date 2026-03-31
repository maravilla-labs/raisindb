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

//! Deep validation methods for NodeType, including initial_structure validation.

use super::definition::NodeType;

impl NodeType {
    /// Validates the entire NodeType including initial_structure and nested children.
    ///
    /// This performs deep validation that:
    /// - Runs basic validation (via validator crate)
    /// - Validates all InitialChild references exist
    /// - Recursively validates nested initial_structure
    ///
    /// # Arguments
    /// * `node_type_lookup` - Closure to lookup NodeTypes by name. Returns Ok(true) if exists.
    ///
    /// # Example
    /// ```ignore
    /// let node_type = NodeType { /* ... */ };
    /// node_type.validate_full(|name| async move {
    ///     storage
    ///         .node_types()
    ///         .get("tenant", "repo", "main", name, None)
    ///         .await
    ///         .map(|opt| opt.is_some())
    /// }).await?;
    /// ```
    pub async fn validate_full<F, Fut>(&self, node_type_lookup: F) -> Result<(), String>
    where
        F: Fn(String) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<bool, String>> + Send,
    {
        use validator::Validate;

        // Basic validation using validator crate
        self.validate().map_err(|e| e.to_string())?;

        // Validate initial_structure if present
        if let Some(initial_structure) = &self.initial_structure {
            if let Some(children) = &initial_structure.children {
                for child in children {
                    self.validate_initial_child(child, &node_type_lookup)
                        .await?;
                }
            }
        }

        Ok(())
    }

    /// Recursively validates an InitialChild and its nested children.
    #[allow(clippy::only_used_in_recursion)]
    fn validate_initial_child<'a, F, Fut>(
        &'a self,
        child: &'a crate::nodes::types::initial_structure::InitialChild,
        node_type_lookup: &'a F,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>>
    where
        F: Fn(String) -> Fut + Send + Sync + 'a,
        Fut: std::future::Future<Output = Result<bool, String>> + Send + 'a,
    {
        Box::pin(async move {
            use validator::Validate;

            // Validate using validator crate
            child.validate().map_err(|e| e.to_string())?;

            // Validate that the node_type exists
            let exists = node_type_lookup(child.node_type.clone()).await?;
            if !exists {
                return Err(format!(
                    "Referenced NodeType '{}' in initial_structure does not exist",
                    child.node_type
                ));
            }

            // Recursively validate nested children
            if let Some(nested_children) = &child.children {
                for nested_child in nested_children {
                    self.validate_initial_child(nested_child, node_type_lookup)
                        .await?;
                }
            }

            Ok(())
        })
    }
}
