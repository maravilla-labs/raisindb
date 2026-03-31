// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Wait subscription management for O(1) flow resumption lookup.
//!
//! When a flow enters a waiting state, it registers a subscription with a unique ID.
//! When an event occurs (e.g., tool result arrives, human task completed), we can
//! quickly look up which flow instance is waiting for it using the subscription ID.
//!
//! This module provides an in-memory HashMap for fast lookups. In the future,
//! this could be moved to RocksDB for persistence across restarts.

use crate::types::{FlowResult, WaitInfo};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

/// Subscription registry for waiting flow instances.
///
/// This maintains an in-memory index of subscription_id -> instance_id mappings
/// for O(1) lookup when events occur.
#[derive(Clone)]
pub struct SubscriptionRegistry {
    /// Map of subscription_id -> instance_id
    subscriptions: Arc<RwLock<HashMap<String, String>>>,

    /// Map of instance_id -> subscription_id (for cleanup)
    instance_subscriptions: Arc<RwLock<HashMap<String, String>>>,
}

impl SubscriptionRegistry {
    /// Create a new subscription registry
    pub fn new() -> Self {
        Self {
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            instance_subscriptions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a wait subscription for a flow instance.
    ///
    /// # Arguments
    ///
    /// * `instance_id` - The flow instance ID
    /// * `wait_info` - Wait information containing the subscription ID
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the subscription was registered successfully.
    pub fn register_wait_subscription(
        &self,
        instance_id: &str,
        wait_info: &WaitInfo,
    ) -> FlowResult<()> {
        info!(
            "Registering wait subscription {} for instance {}",
            wait_info.subscription_id, instance_id
        );

        let mut subscriptions = self
            .subscriptions
            .write()
            .map_err(|e| crate::types::FlowError::Other(format!("Lock error: {}", e)))?;

        let mut instance_subs = self
            .instance_subscriptions
            .write()
            .map_err(|e| crate::types::FlowError::Other(format!("Lock error: {}", e)))?;

        // Remove old subscription for this instance if exists
        if let Some(old_sub_id) = instance_subs.get(instance_id) {
            debug!(
                "Removing old subscription {} for instance {}",
                old_sub_id, instance_id
            );
            subscriptions.remove(old_sub_id);
        }

        // Register new subscription
        subscriptions.insert(wait_info.subscription_id.clone(), instance_id.to_string());
        instance_subs.insert(instance_id.to_string(), wait_info.subscription_id.clone());

        debug!(
            "Registered subscription: {} -> {}",
            wait_info.subscription_id, instance_id
        );

        Ok(())
    }

    /// Look up which flow instance is waiting for a subscription.
    ///
    /// # Arguments
    ///
    /// * `subscription_id` - The subscription ID to look up
    ///
    /// # Returns
    ///
    /// Returns the instance ID if found, or `None` if no flow is waiting.
    pub fn lookup_subscription(&self, subscription_id: &str) -> Option<String> {
        debug!("Looking up subscription: {}", subscription_id);

        let subscriptions = self.subscriptions.read().ok()?;
        let instance_id = subscriptions.get(subscription_id).cloned();

        if let Some(ref id) = instance_id {
            debug!("Found instance {} for subscription {}", id, subscription_id);
        } else {
            debug!("No instance found for subscription {}", subscription_id);
        }

        instance_id
    }

    /// Remove a subscription when a flow resumes or completes.
    ///
    /// # Arguments
    ///
    /// * `instance_id` - The flow instance ID
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the subscription was removed.
    pub fn remove_subscription(&self, instance_id: &str) -> FlowResult<()> {
        info!("Removing subscription for instance {}", instance_id);

        let mut subscriptions = self
            .subscriptions
            .write()
            .map_err(|e| crate::types::FlowError::Other(format!("Lock error: {}", e)))?;

        let mut instance_subs = self
            .instance_subscriptions
            .write()
            .map_err(|e| crate::types::FlowError::Other(format!("Lock error: {}", e)))?;

        if let Some(sub_id) = instance_subs.remove(instance_id) {
            subscriptions.remove(&sub_id);
            debug!(
                "Removed subscription {} for instance {}",
                sub_id, instance_id
            );
        } else {
            warn!("No subscription found for instance {}", instance_id);
        }

        Ok(())
    }

    /// Get the number of active subscriptions
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.read().map(|s| s.len()).unwrap_or(0)
    }

    /// Clear all subscriptions (useful for testing)
    pub fn clear(&self) -> FlowResult<()> {
        let mut subscriptions = self
            .subscriptions
            .write()
            .map_err(|e| crate::types::FlowError::Other(format!("Lock error: {}", e)))?;

        let mut instance_subs = self
            .instance_subscriptions
            .write()
            .map_err(|e| crate::types::FlowError::Other(format!("Lock error: {}", e)))?;

        subscriptions.clear();
        instance_subs.clear();

        Ok(())
    }
}

impl Default for SubscriptionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WaitType;
    use chrono::Utc;

    #[test]
    fn test_register_and_lookup_subscription() {
        let registry = SubscriptionRegistry::new();

        let wait_info = WaitInfo {
            subscription_id: "sub-123".to_string(),
            wait_type: WaitType::ToolCall,
            target_path: Some("/jobs/job-456".to_string()),
            expected_event: Some("job_completed".to_string()),
            timeout_at: Some(Utc::now()),
        };

        // Register subscription
        registry
            .register_wait_subscription("instance-1", &wait_info)
            .unwrap();

        // Lookup subscription
        let instance_id = registry.lookup_subscription("sub-123");
        assert_eq!(instance_id, Some("instance-1".to_string()));

        // Lookup non-existent subscription
        let missing = registry.lookup_subscription("sub-999");
        assert_eq!(missing, None);
    }

    #[test]
    fn test_remove_subscription() {
        let registry = SubscriptionRegistry::new();

        let wait_info = WaitInfo {
            subscription_id: "sub-123".to_string(),
            wait_type: WaitType::HumanTask,
            target_path: None,
            expected_event: None,
            timeout_at: None,
        };

        // Register subscription
        registry
            .register_wait_subscription("instance-1", &wait_info)
            .unwrap();

        assert_eq!(registry.subscription_count(), 1);

        // Remove subscription
        registry.remove_subscription("instance-1").unwrap();

        assert_eq!(registry.subscription_count(), 0);

        // Lookup should fail now
        let instance_id = registry.lookup_subscription("sub-123");
        assert_eq!(instance_id, None);
    }

    #[test]
    fn test_replace_subscription() {
        let registry = SubscriptionRegistry::new();

        // Register first subscription
        let wait_info1 = WaitInfo {
            subscription_id: "sub-123".to_string(),
            wait_type: WaitType::ToolCall,
            target_path: None,
            expected_event: None,
            timeout_at: None,
        };

        registry
            .register_wait_subscription("instance-1", &wait_info1)
            .unwrap();

        assert_eq!(registry.subscription_count(), 1);

        // Register second subscription for same instance (should replace)
        let wait_info2 = WaitInfo {
            subscription_id: "sub-456".to_string(),
            wait_type: WaitType::Retry,
            target_path: None,
            expected_event: None,
            timeout_at: None,
        };

        registry
            .register_wait_subscription("instance-1", &wait_info2)
            .unwrap();

        // Should still be 1 subscription
        assert_eq!(registry.subscription_count(), 1);

        // Old subscription should be gone
        assert_eq!(registry.lookup_subscription("sub-123"), None);

        // New subscription should work
        assert_eq!(
            registry.lookup_subscription("sub-456"),
            Some("instance-1".to_string())
        );
    }

    #[test]
    fn test_clear_subscriptions() {
        let registry = SubscriptionRegistry::new();

        // Register multiple subscriptions
        for i in 1..=5 {
            let wait_info = WaitInfo {
                subscription_id: format!("sub-{}", i),
                wait_type: WaitType::Event,
                target_path: None,
                expected_event: None,
                timeout_at: None,
            };

            registry
                .register_wait_subscription(&format!("instance-{}", i), &wait_info)
                .unwrap();
        }

        assert_eq!(registry.subscription_count(), 5);

        // Clear all subscriptions
        registry.clear().unwrap();

        assert_eq!(registry.subscription_count(), 0);
    }
}
