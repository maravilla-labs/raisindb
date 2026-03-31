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

//! Repository management storage trait

use raisin_context::RepositoryConfig;
use raisin_context::RepositoryInfo;
use raisin_error::Result;

/// Repository management storage operations.
///
/// Provides CRUD operations for repositories at the storage layer.
/// This is used by the repository registry to track all repositories.
pub trait RepositoryManagementRepository: Send + Sync {
    /// Create a new repository
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `config` - Repository configuration
    ///
    /// # Returns
    /// The created repository information
    fn create_repository(
        &self,
        tenant_id: &str,
        repo_id: &str,
        config: RepositoryConfig,
    ) -> impl std::future::Future<Output = Result<RepositoryInfo>> + Send;

    /// Get repository information
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    ///
    /// # Returns
    /// Repository information if it exists
    fn get_repository(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<RepositoryInfo>>> + Send;

    /// List all repositories (admin operation)
    ///
    /// # Returns
    /// Vector of all repositories across all tenants
    fn list_repositories(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<RepositoryInfo>>> + Send;

    /// List repositories for a specific tenant
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    ///
    /// # Returns
    /// Vector of repositories for the tenant
    fn list_repositories_for_tenant(
        &self,
        tenant_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<RepositoryInfo>>> + Send;

    /// Delete a repository
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    ///
    /// # Returns
    /// `true` if deleted, `false` if not found
    fn delete_repository(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Check if repository exists
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    ///
    /// # Returns
    /// `true` if exists, `false` otherwise
    fn repository_exists(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Update repository configuration
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `config` - New configuration
    fn update_repository_config(
        &self,
        tenant_id: &str,
        repo_id: &str,
        config: RepositoryConfig,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}
