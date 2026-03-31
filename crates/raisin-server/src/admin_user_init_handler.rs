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

//! Event handler for automatic superadmin user creation.
//!
//! This handler subscribes to `TenantCreated` events and automatically
//! creates a superadmin user for new tenants.

use anyhow::Result;
use raisin_events::{Event, EventHandler, RepositoryEventKind};
use raisin_rocksdb::AuthService;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Event handler that creates a superadmin user when a tenant is created
pub struct AdminUserInitHandler {
    auth_service: Arc<AuthService>,
    initial_password: Option<String>,
}

impl AdminUserInitHandler {
    /// Create a new admin user initialization handler
    pub fn new(auth_service: Arc<AuthService>) -> Self {
        Self {
            auth_service,
            initial_password: None,
        }
    }

    /// Create a new admin user initialization handler with a specific initial password
    pub fn with_initial_password(auth_service: Arc<AuthService>, password: String) -> Self {
        Self {
            auth_service,
            initial_password: Some(password),
        }
    }

    /// Handle tenant created event
    async fn handle_tenant_created(&self, tenant_id: &str) -> Result<()> {
        info!(
            tenant_id = tenant_id,
            "Checking if superadmin user needs to be created"
        );

        // Check if any admin users exist for this tenant
        match self.auth_service.has_users(tenant_id) {
            Ok(has_users) => {
                if has_users {
                    info!(
                        tenant_id = tenant_id,
                        "Admin users already exist, skipping superadmin creation"
                    );
                    return Ok(());
                }
            }
            Err(e) => {
                error!(
                    tenant_id = tenant_id,
                    error = %e,
                    "Failed to check if admin users exist"
                );
                return Err(e.into());
            }
        }

        // Create superadmin user
        info!(tenant_id = tenant_id, "Creating superadmin user");

        // Check if we have a predefined initial password
        if let Some(ref password) = self.initial_password {
            // Use the provided password
            match self.auth_service.create_superadmin_with_password(
                tenant_id.to_string(),
                "admin".to_string(),
                password.clone(),
            ) {
                Ok(user) => {
                    // Log success with the provided password
                    warn!(
                        "\n\
                        ╔═══════════════════════════════════════════════════════════════════════════╗\n\
                        ║                      SUPERADMIN USER CREATED                              ║\n\
                        ╠═══════════════════════════════════════════════════════════════════════════╣\n\
                        ║ Tenant ID:  {:<62}║\n\
                        ║ Username:   {:<62}║\n\
                        ║ Password:   {:<62}║\n\
                        ║                                                                           ║\n\
                        ║ ℹ️  Initial password was set via configuration                            ║\n\
                        ║ ⚠️  IMPORTANT: Change this password immediately after first login!        ║\n\
                        ╚═══════════════════════════════════════════════════════════════════════════╝",
                        tenant_id, user.username, password
                    );

                    info!(
                        tenant_id = tenant_id,
                        username = user.username,
                        user_id = user.user_id,
                        "Superadmin user created with configured password"
                    );

                    Ok(())
                }
                Err(e) => {
                    error!(
                        tenant_id = tenant_id,
                        error = %e,
                        "Failed to create superadmin user with configured password"
                    );
                    Err(e.into())
                }
            }
        } else {
            // Auto-generate a password
            match self
                .auth_service
                .create_superadmin(tenant_id.to_string(), "admin".to_string())
            {
                Ok((user, password)) => {
                    // Log the initial password (will be printed to console/logs)
                    warn!(
                        "\n\
                        ╔═══════════════════════════════════════════════════════════════════════════╗\n\
                        ║                      SUPERADMIN USER CREATED                              ║\n\
                        ╠═══════════════════════════════════════════════════════════════════════════╣\n\
                        ║ Tenant ID:  {:<62}║\n\
                        ║ Username:   {:<62}║\n\
                        ║ Password:   {:<62}║\n\
                        ║                                                                           ║\n\
                        ║ ⚠️  IMPORTANT: Change this password immediately after first login!        ║\n\
                        ║     This password will only be shown once.                                ║\n\
                        ╚═══════════════════════════════════════════════════════════════════════════╝",
                        tenant_id, user.username, password
                    );

                    info!(
                        tenant_id = tenant_id,
                        username = user.username,
                        user_id = user.user_id,
                        "Superadmin user created successfully"
                    );

                    // DEBUG: Verify the user was actually stored and can be retrieved
                    eprintln!("🔍 Verifying user was stored correctly...");
                    match self.auth_service.get_user(tenant_id, &user.username) {
                        Ok(Some(retrieved_user)) => {
                            eprintln!(
                                "✅ VERIFICATION SUCCESS: User '{}' can be retrieved from DB",
                                retrieved_user.username
                            );
                        }
                        Ok(None) => {
                            eprintln!("❌ VERIFICATION FAILED: User NOT found in DB immediately after creation!");
                        }
                        Err(e) => {
                            eprintln!("❌ VERIFICATION ERROR: Failed to retrieve user: {}", e);
                        }
                    }

                    Ok(())
                }
                Err(e) => {
                    error!(
                        tenant_id = tenant_id,
                        error = %e,
                        "Failed to create superadmin user"
                    );
                    Err(e.into())
                }
            }
        }
    }
}

impl EventHandler for AdminUserInitHandler {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                Event::Repository(repo_event)
                    if repo_event.kind == RepositoryEventKind::TenantCreated =>
                {
                    self.handle_tenant_created(&repo_event.tenant_id).await
                }
                _ => Ok(()), // Ignore other events
            }
        })
    }

    fn name(&self) -> &str {
        "AdminUserInitHandler"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_events::RepositoryEvent;
    use raisin_rocksdb::{AdminUserStore, RocksDBConfig, RocksDBStorage};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_superadmin_creation_on_repository_created() {
        // Setup
        let temp_dir = TempDir::new().unwrap();
        let config = RocksDBConfig::default().with_path(temp_dir.path());
        let storage = RocksDBStorage::with_config(config).unwrap();

        let store = AdminUserStore::new(storage.db().clone());
        let auth_service = Arc::new(AuthService::new(store, "test_secret".to_string()));
        let handler = AdminUserInitHandler::new(auth_service.clone());

        // Trigger tenant created event
        let event = Event::Repository(RepositoryEvent {
            tenant_id: "test_tenant".to_string(),
            repository_id: String::new(), // Not needed for tenant events
            kind: RepositoryEventKind::TenantCreated,
            workspace: None,
            revision_id: None,
            branch_name: None,
            tag_name: None,
            message: None,
            actor: None,
            metadata: None,
        });

        // Handle event
        handler.handle(&event).await.unwrap();

        // Verify superadmin was created
        assert!(auth_service.has_users("test_tenant").unwrap());

        // Verify that handling the same event again doesn't create another user
        handler.handle(&event).await.unwrap();

        // Count should still be 1
        // (we don't have a count method, but we can check that no error occurred)
    }
}
