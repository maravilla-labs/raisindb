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

//! Tenant NodeType initialization helpers
//!
//! This module provides functionality to initialize NodeTypes for specific tenants
//! in a multi-tenant environment. It works with any Storage implementation.

use crate::scope::BranchScope;
use crate::{CommitMetadata, NodeTypeRepository, RegistryRepository, Storage};
use raisin_error::Result;
use raisin_models::nodes::types::NodeType;

/// Initialize NodeTypes for a specific tenant/deployment
///
/// This function performs lazy initialization of built-in NodeTypes for a tenant.
/// It should be called on first request for each tenant/deployment combination.
///
/// # Type Parameters
/// * `S` - Storage type that supports scoping (implements ScopableStorage)
/// * `F` - Function that provides NodeType definitions and version hash
///
/// # Arguments
/// * `storage` - Base storage instance (will be cloned and scoped)
/// * `tenant_id` - Tenant identifier
/// * `deployment_key` - Deployment key (e.g., "production", "staging")
/// * `get_nodetypes` - Function that returns (version_hash, Vec<NodeType>)
///
/// # Process
/// 1. Calculate current global NodeType version hash
/// 2. Check registry for deployment's last initialized version
/// 3. If versions match, skip initialization (already up-to-date)
/// 4. If different or missing, initialize NodeTypes for this tenant
/// 5. Update registry with new version
///
/// # Example
/// ```rust,ignore
/// use raisin_storage::{init_tenant_nodetypes, TenantContext};
/// use raisin_storage_rocks::RocksStorage;
///
/// async fn setup_tenant(storage: RocksStorage) -> Result<()> {
///     init_tenant_nodetypes(
///         storage,
///         "tenant-123",
///         "production",
///         || {
///             let version = calculate_version();
///             let nodetypes = load_nodetypes();
///             (version, nodetypes)
///         }
///     ).await?;
///     Ok(())
/// }
/// ```
pub async fn init_tenant_nodetypes<S, F>(
    storage: S,
    tenant_id: &str,
    deployment_key: &str,
    get_nodetypes: F,
) -> Result<()>
where
    S: Storage,
    F: FnOnce() -> (String, Vec<NodeType>),
{
    // Check if deployment exists and get its current version
    let registry = storage.registry();

    // Get NodeTypes and version from the provider function
    let (current_version, node_types) = get_nodetypes();

    if let Some(deployment) = registry.get_deployment(tenant_id, deployment_key).await? {
        if let Some(existing_version) = deployment.nodetype_version {
            if existing_version == current_version {
                // Already up-to-date, skip initialization
                tracing::debug!(
                    "Tenant {}/{} NodeTypes already at version {}",
                    tenant_id,
                    deployment_key,
                    current_version
                );
                return Ok(());
            }
        }
    }

    tracing::info!(
        "Initializing NodeTypes for tenant {}/{} with version {}",
        tenant_id,
        deployment_key,
        current_version
    );

    // Register tenant and deployment if they don't exist
    registry
        .register_tenant(tenant_id, std::collections::HashMap::new())
        .await?;
    registry
        .register_deployment(tenant_id, deployment_key)
        .await?;

    // Initialize each NodeType on the main branch of the default repository
    // NodeTypes are repository-scoped in repository-first architecture
    let repo = storage.node_types();
    let repo_id = "main"; // Default repository for tenant initialization
    let branch_name = "main"; // Seed NodeTypes on default branch

    let scope = BranchScope::new(tenant_id, repo_id, branch_name);

    for node_type in node_types {
        let commit = CommitMetadata::system(format!("Initialize node type {}", node_type.name));

        #[allow(deprecated)]
        repo.put(scope, node_type.clone(), commit).await?;

        tracing::debug!(
            "Initialized NodeType '{}' for tenant {}/{} (repo: {}, branch: {})",
            node_type.name,
            tenant_id,
            deployment_key,
            repo_id,
            branch_name
        );
    }

    // Update deployment version in registry
    registry
        .update_deployment_nodetype_version(tenant_id, deployment_key, &current_version)
        .await?;

    tracing::info!(
        "Completed NodeType initialization for tenant {}/{} (version {})",
        tenant_id,
        deployment_key,
        current_version
    );

    Ok(())
}

/// Check if a tenant/deployment needs NodeType initialization
///
/// This is a lightweight check that only queries the registry without
/// loading or initializing any NodeTypes.
///
/// # Returns
/// - `Ok(true)` if initialization is needed (version mismatch or not initialized)
/// - `Ok(false)` if already up-to-date
#[allow(dead_code)]
pub async fn needs_initialization<S: Storage>(
    storage: &S,
    tenant_id: &str,
    deployment_key: &str,
    current_version: &str,
) -> Result<bool> {
    let registry = storage.registry();

    if let Some(deployment) = registry.get_deployment(tenant_id, deployment_key).await? {
        if let Some(existing_version) = deployment.nodetype_version {
            return Ok(existing_version != current_version);
        }
    }

    // No deployment found or no version set - needs initialization
    Ok(true)
}
