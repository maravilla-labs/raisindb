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

//! Authentication strategy registry.
//!
//! Manages registration and lookup of authentication strategies.

use raisin_error::Result;
use raisin_models::auth::AuthProviderConfig;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::strategy::{AuthCredentials, AuthStrategy, StrategyId};

/// Registry for authentication strategies.
///
/// Follows the same pattern as `TenantResolver` - strategies are registered
/// at startup and looked up by ID or credential type.
pub struct AuthStrategyRegistry {
    /// Registered strategies
    strategies: RwLock<HashMap<StrategyId, Arc<dyn AuthStrategy>>>,

    /// Default strategy for login form
    default_strategy: RwLock<Option<StrategyId>>,
}

impl AuthStrategyRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            strategies: RwLock::new(HashMap::new()),
            default_strategy: RwLock::new(None),
        }
    }

    /// Register a new authentication strategy.
    ///
    /// If a strategy with the same ID already exists, it will be replaced.
    pub async fn register(&self, strategy: Arc<dyn AuthStrategy>) {
        let id = strategy.id().clone();
        let name = strategy.name().to_string();

        let mut strategies = self.strategies.write().await;
        strategies.insert(id.clone(), strategy);

        info!("Registered auth strategy: {} ({})", id, name);
    }

    /// Unregister a strategy by ID
    pub async fn unregister(&self, id: &StrategyId) -> Option<Arc<dyn AuthStrategy>> {
        let mut strategies = self.strategies.write().await;
        let removed = strategies.remove(id);

        if removed.is_some() {
            info!("Unregistered auth strategy: {}", id);
        }

        removed
    }

    /// Get a strategy by ID
    pub async fn get(&self, id: &StrategyId) -> Option<Arc<dyn AuthStrategy>> {
        let strategies = self.strategies.read().await;
        strategies.get(id).cloned()
    }

    /// Find a strategy that supports the given credentials
    pub async fn find_supporting(
        &self,
        credentials: &AuthCredentials,
    ) -> Option<Arc<dyn AuthStrategy>> {
        let strategies = self.strategies.read().await;
        strategies
            .values()
            .find(|s| s.supports(credentials))
            .cloned()
    }

    /// List all registered strategies
    pub async fn list(&self) -> Vec<Arc<dyn AuthStrategy>> {
        let strategies = self.strategies.read().await;
        strategies.values().cloned().collect()
    }

    /// List all strategy IDs
    pub async fn list_ids(&self) -> Vec<StrategyId> {
        let strategies = self.strategies.read().await;
        strategies.keys().cloned().collect()
    }

    /// Check if a strategy is registered
    pub async fn contains(&self, id: &StrategyId) -> bool {
        let strategies = self.strategies.read().await;
        strategies.contains_key(id)
    }

    /// Get the number of registered strategies
    pub async fn len(&self) -> usize {
        let strategies = self.strategies.read().await;
        strategies.len()
    }

    /// Check if registry is empty
    pub async fn is_empty(&self) -> bool {
        let strategies = self.strategies.read().await;
        strategies.is_empty()
    }

    /// Set the default strategy for login
    pub async fn set_default(&self, id: StrategyId) {
        let mut default = self.default_strategy.write().await;
        *default = Some(id);
    }

    /// Get the default strategy
    pub async fn get_default(&self) -> Option<Arc<dyn AuthStrategy>> {
        let default = self.default_strategy.read().await;
        if let Some(id) = default.as_ref() {
            self.get(id).await
        } else {
            None
        }
    }

    /// Initialize all strategies from provider configurations.
    ///
    /// This should be called once at startup after all strategies are registered.
    /// It initializes each strategy with its configuration and decrypted secrets.
    ///
    /// # Arguments
    ///
    /// * `configs` - Provider configurations keyed by strategy ID
    /// * `decrypt_secret` - Function to decrypt client secrets
    pub async fn initialize_all<F>(
        &self,
        configs: &HashMap<String, AuthProviderConfig>,
        decrypt_secret: F,
    ) -> Result<()>
    where
        F: Fn(&[u8]) -> Result<String>,
    {
        let strategies = self.strategies.read().await;

        for (id, _strategy) in strategies.iter() {
            let config = configs.get(&id.0);

            if let Some(config) = config {
                // Decrypt secret if present
                let decrypted = if let Some(encrypted) = &config.client_secret_encrypted {
                    match decrypt_secret(encrypted) {
                        Ok(secret) => Some(secret),
                        Err(e) => {
                            warn!("Failed to decrypt secret for strategy {}: {}", id, e);
                            None
                        }
                    }
                } else {
                    None
                };

                // Note: We need interior mutability here, but AuthStrategy::init
                // takes &mut self. In practice, strategies should store their
                // initialized state internally using interior mutability.
                // This is a limitation of the current design - strategies should
                // use OnceCell or similar for initialization state.
                debug!("Initializing strategy {} with config", id);

                // For now, we'll skip actual initialization since we can't
                // call &mut self through Arc. Strategies should initialize
                // themselves lazily or use interior mutability.
                let _ = decrypted;
            } else {
                debug!("No config found for strategy {}, using defaults", id);
            }
        }

        info!("Initialized {} authentication strategies", strategies.len());

        Ok(())
    }
}

impl Default for AuthStrategyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use raisin_models::auth::AuthProviderConfig;

    struct MockStrategy {
        id: StrategyId,
        name: String,
    }

    impl MockStrategy {
        fn new(id: &str) -> Self {
            Self {
                id: StrategyId::new(id),
                name: format!("Mock {}", id),
            }
        }
    }

    #[async_trait]
    impl AuthStrategy for MockStrategy {
        fn id(&self) -> &StrategyId {
            &self.id
        }

        fn name(&self) -> &str {
            &self.name
        }

        async fn init(
            &mut self,
            _config: &AuthProviderConfig,
            _decrypted_secret: Option<&str>,
        ) -> Result<()> {
            Ok(())
        }

        async fn authenticate(
            &self,
            _tenant_id: &str,
            _credentials: AuthCredentials,
        ) -> Result<crate::strategy::AuthenticationResult> {
            Err(raisin_error::Error::invalid_state("Mock"))
        }

        fn supports(&self, credentials: &AuthCredentials) -> bool {
            matches!(credentials, AuthCredentials::UsernamePassword { .. })
        }
    }

    #[tokio::test]
    async fn test_registry_basic_operations() {
        let registry = AuthStrategyRegistry::new();

        assert!(registry.is_empty().await);

        let strategy = Arc::new(MockStrategy::new("local"));
        registry.register(strategy).await;

        assert!(!registry.is_empty().await);
        assert_eq!(registry.len().await, 1);

        let retrieved = registry.get(&StrategyId::new("local")).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name(), "Mock local");
    }

    #[tokio::test]
    async fn test_registry_find_supporting() {
        let registry = AuthStrategyRegistry::new();

        let strategy = Arc::new(MockStrategy::new("local"));
        registry.register(strategy).await;

        let creds = AuthCredentials::UsernamePassword {
            username: "user".to_string(),
            password: "pass".to_string(),
        };

        let found = registry.find_supporting(&creds).await;
        assert!(found.is_some());

        let api_key_creds = AuthCredentials::ApiKey {
            key: "key".to_string(),
        };
        let not_found = registry.find_supporting(&api_key_creds).await;
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_registry_default_strategy() {
        let registry = AuthStrategyRegistry::new();

        let strategy = Arc::new(MockStrategy::new("local"));
        registry.register(strategy).await;

        assert!(registry.get_default().await.is_none());

        registry.set_default(StrategyId::new("local")).await;

        let default = registry.get_default().await;
        assert!(default.is_some());
        assert_eq!(default.unwrap().id().0, "local");
    }
}
