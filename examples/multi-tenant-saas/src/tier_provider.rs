//! Simple tier provider for the example

use raisin_context::{Operation, ServiceTier, TierProvider};

/// A simple tier provider that maps tenant IDs to tiers
pub struct SimpleTierProvider;

impl SimpleTierProvider {
    pub fn new() -> Self {
        Self
    }

    /// Determine tier based on tenant ID
    /// In production, this would query your billing database
    fn get_tier_for_tenant(&self, tenant_id: &str) -> ServiceTier {
        match tenant_id {
            // Enterprise customers (example)
            "enterprise-acme" | "enterprise-bigcorp" => ServiceTier::Enterprise {
                dedicated_db: true,
                max_requests_per_minute: 10_000,
                custom_features: vec!["advanced-analytics".to_string()],
            },

            // Professional customers (example)
            tid if tid.starts_with("pro-") => ServiceTier::Professional {
                max_nodes: 100_000,
                max_requests_per_minute: 1_000,
            },

            // Free tier for everyone else
            _ => ServiceTier::Free {
                max_nodes: 1_000,
                max_requests_per_minute: 100,
            },
        }
    }
}

impl TierProvider for SimpleTierProvider {
    async fn get_tier(&self, tenant_id: &str) -> ServiceTier {
        self.get_tier_for_tenant(tenant_id)
    }

    async fn check_limits(&self, tenant_id: &str, operation: &Operation) -> Result<(), String> {
        let tier = self.get_tier_for_tenant(tenant_id);

        // Example: check rate limits (in production, use actual rate limiter)
        match operation {
            Operation::CreateNode => {
                // Check if tenant has room for more nodes
                if let Some(max) = tier.max_nodes() {
                    // TODO: Actually count existing nodes
                    // For now, always allow
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn record_usage(&self, tenant_id: &str, operation: &Operation) {
        // In production, record to analytics/billing database
        println!("📊 Usage: tenant={}, operation={:?}", tenant_id, operation);
    }
}
