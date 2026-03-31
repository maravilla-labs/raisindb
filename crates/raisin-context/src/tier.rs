//! Service tier and provider traits

use std::future::Future;

/// Represents different service tiers for multi-tenant deployments
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceTier {
    /// Free tier with resource limits
    Free {
        max_nodes: usize,
        max_requests_per_minute: usize,
    },

    /// Professional tier with higher limits
    Professional {
        max_nodes: usize,
        max_requests_per_minute: usize,
    },

    /// Enterprise tier with dedicated resources
    Enterprise {
        dedicated_db: bool,
        max_requests_per_minute: usize,
        custom_features: Vec<String>,
    },
}

impl ServiceTier {
    /// Get the maximum number of nodes allowed for this tier
    pub fn max_nodes(&self) -> Option<usize> {
        match self {
            ServiceTier::Free { max_nodes, .. } => Some(*max_nodes),
            ServiceTier::Professional { max_nodes, .. } => Some(*max_nodes),
            ServiceTier::Enterprise { .. } => None, // Unlimited
        }
    }

    /// Get the rate limit for this tier
    pub fn rate_limit(&self) -> usize {
        match self {
            ServiceTier::Free {
                max_requests_per_minute,
                ..
            } => *max_requests_per_minute,
            ServiceTier::Professional {
                max_requests_per_minute,
                ..
            } => *max_requests_per_minute,
            ServiceTier::Enterprise {
                max_requests_per_minute,
                ..
            } => *max_requests_per_minute,
        }
    }

    /// Check if this tier has a dedicated database
    pub fn has_dedicated_db(&self) -> bool {
        matches!(
            self,
            ServiceTier::Enterprise {
                dedicated_db: true,
                ..
            }
        )
    }
}

/// Represents an operation that may have limits
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    CreateNode,
    UpdateNode,
    DeleteNode,
    Query,
    Upload,
    Custom(String),
}

/// Trait for providing tier information and enforcing limits
///
/// Implement this trait to integrate with your billing/subscription system.
///
/// # Examples
///
/// ```rust
/// use raisin_context::{TierProvider, ServiceTier, Operation};
///
/// struct SimpleTierProvider;
///
/// impl TierProvider for SimpleTierProvider {
///     async fn get_tier(&self, tenant_id: &str) -> ServiceTier {
///         // In real implementation, query your billing database
///         if tenant_id.starts_with("enterprise-") {
///             ServiceTier::Enterprise {
///                 dedicated_db: true,
///                 max_requests_per_minute: 10000,
///                 custom_features: vec![],
///             }
///         } else {
///             ServiceTier::Free {
///                 max_nodes: 1000,
///                 max_requests_per_minute: 100,
///             }
///         }
///     }
///
///     async fn check_limits(
///         &self,
///         tenant_id: &str,
///         operation: &Operation,
///     ) -> Result<(), String> {
///         // Implement your limit checking logic
///         Ok(())
///     }
/// }
/// ```
pub trait TierProvider: Send + Sync {
    /// Get the service tier for a tenant
    fn get_tier(&self, tenant_id: &str) -> impl Future<Output = ServiceTier> + Send;

    /// Check if an operation is allowed for a tenant
    ///
    /// Returns `Ok(())` if allowed, or `Err(reason)` if the limit is exceeded
    fn check_limits(
        &self,
        tenant_id: &str,
        operation: &Operation,
    ) -> impl Future<Output = Result<(), String>> + Send;

    /// Record that an operation was performed (for usage tracking)
    fn record_usage(
        &self,
        tenant_id: &str,
        operation: &Operation,
    ) -> impl Future<Output = ()> + Send {
        async move {
            // Default implementation does nothing
            let _ = (tenant_id, operation);
        }
    }
}

/// A simple tier provider that returns the same tier for all tenants
#[allow(dead_code)]
pub struct StaticTierProvider {
    tier: ServiceTier,
}

#[allow(dead_code)]
impl StaticTierProvider {
    pub fn new(tier: ServiceTier) -> Self {
        Self { tier }
    }

    pub fn free() -> Self {
        Self::new(ServiceTier::Free {
            max_nodes: 1000,
            max_requests_per_minute: 100,
        })
    }

    pub fn professional() -> Self {
        Self::new(ServiceTier::Professional {
            max_nodes: 100_000,
            max_requests_per_minute: 1000,
        })
    }
}

impl TierProvider for StaticTierProvider {
    async fn get_tier(&self, _tenant_id: &str) -> ServiceTier {
        self.tier.clone()
    }

    async fn check_limits(&self, _tenant_id: &str, _operation: &Operation) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_tier_limits() {
        let free = ServiceTier::Free {
            max_nodes: 100,
            max_requests_per_minute: 50,
        };
        assert_eq!(free.max_nodes(), Some(100));
        assert_eq!(free.rate_limit(), 50);
        assert!(!free.has_dedicated_db());

        let enterprise = ServiceTier::Enterprise {
            dedicated_db: true,
            max_requests_per_minute: 10000,
            custom_features: vec![],
        };
        assert_eq!(enterprise.max_nodes(), None);
        assert!(enterprise.has_dedicated_db());
    }

    #[tokio::test]
    async fn test_static_tier_provider() {
        let provider = StaticTierProvider::free();
        let tier = provider.get_tier("any-tenant").await;
        assert_eq!(tier.max_nodes(), Some(1000));
    }
}
