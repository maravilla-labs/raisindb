// SPDX-License-Identifier: BSL-1.1

//! Subscription management and filter matching.

use crate::protocol::SubscriptionFilters;

use super::path_matching::path_matches;
use super::state::ConnectionState;

impl ConnectionState {
    /// Add a subscription with deduplication
    ///
    /// If identical filters already exist, returns the existing subscription_id.
    /// Otherwise, adds the new subscription and returns the provided subscription_id.
    ///
    /// # Returns
    /// The subscription_id to use (may be existing if duplicate)
    pub fn add_subscription(
        &self,
        subscription_id: String,
        filters: SubscriptionFilters,
    ) -> String {
        let hash = self.hash_filters(&filters);

        // Check for existing subscription with identical filters
        if let Some(existing_id) = self.filter_index.get(&hash) {
            tracing::debug!(
                existing_id = %existing_id.value(),
                "Reusing existing subscription with identical filters"
            );
            return existing_id.value().clone();
        }

        // Add new subscription
        self.subscriptions.insert(subscription_id.clone(), filters);
        self.filter_index.insert(hash, subscription_id.clone());
        subscription_id
    }

    /// Remove a subscription
    ///
    /// Also removes from the filter index.
    pub fn remove_subscription(&self, subscription_id: &str) -> bool {
        if let Some((_, filters)) = self.subscriptions.remove(subscription_id) {
            let hash = self.hash_filters(&filters);
            self.filter_index.remove(&hash);
            true
        } else {
            false
        }
    }

    /// Hash subscription filters for deduplication
    ///
    /// Two subscriptions with identical filters will produce the same hash.
    fn hash_filters(&self, filters: &SubscriptionFilters) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        filters.workspace.hash(&mut hasher);
        filters.path.hash(&mut hasher);
        filters.node_type.hash(&mut hasher);
        filters.include_node.hash(&mut hasher);

        if let Some(ref event_types) = filters.event_types {
            let mut sorted = event_types.clone();
            sorted.sort();
            sorted.hash(&mut hasher);
        } else {
            None::<Vec<String>>.hash(&mut hasher);
        }

        format!("{:016x}", hasher.finish())
    }

    /// Get all subscriptions
    pub fn get_subscriptions(&self) -> Vec<(String, SubscriptionFilters)> {
        self.subscriptions
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Check if an event matches any subscription filters
    /// Returns matching subscription IDs with their filters (for include_node handling)
    pub fn matches_subscription(
        &self,
        workspace: &str,
        path: &str,
        event_type: &str,
        node_type: Option<&str>,
    ) -> Vec<(String, SubscriptionFilters)> {
        let mut matching_subs = Vec::new();

        for entry in self.subscriptions.iter() {
            let subscription_id = entry.key();
            let filters = entry.value();

            tracing::debug!(
                subscription_id = %subscription_id,
                "Checking subscription filter match"
            );

            // Check workspace filter
            if let Some(ref filter_workspace) = filters.workspace {
                if filter_workspace != workspace {
                    tracing::debug!(
                        subscription_id = %subscription_id,
                        filter_workspace = %filter_workspace,
                        event_workspace = %workspace,
                        "Workspace mismatch"
                    );
                    continue;
                }
                tracing::debug!(
                    subscription_id = %subscription_id,
                    workspace = %workspace,
                    "Workspace matches"
                );
            }

            // Check path filter (supports wildcards)
            if let Some(ref filter_path) = filters.path {
                if !path_matches(path, filter_path) {
                    tracing::debug!(
                        subscription_id = %subscription_id,
                        filter_path = %filter_path,
                        event_path = %path,
                        "Path mismatch"
                    );
                    continue;
                }
                tracing::debug!(
                    subscription_id = %subscription_id,
                    path = %path,
                    filter_path = %filter_path,
                    "Path matches"
                );
            }

            // Check event type filter
            if let Some(ref filter_event_types) = filters.event_types {
                if !filter_event_types.contains(&event_type.to_string()) {
                    tracing::debug!(
                        subscription_id = %subscription_id,
                        filter_event_types = ?filter_event_types,
                        event_type = %event_type,
                        "Event type mismatch"
                    );
                    continue;
                }
                tracing::debug!(
                    subscription_id = %subscription_id,
                    event_type = %event_type,
                    "Event type matches"
                );
            }

            // Check node type filter
            if let Some(ref filter_node_type) = filters.node_type {
                if let Some(node_type) = node_type {
                    if filter_node_type != node_type {
                        tracing::debug!(
                            subscription_id = %subscription_id,
                            filter_node_type = %filter_node_type,
                            event_node_type = %node_type,
                            "Node type mismatch"
                        );
                        continue;
                    }
                    tracing::debug!(
                        subscription_id = %subscription_id,
                        node_type = %node_type,
                        "Node type matches"
                    );
                } else {
                    tracing::debug!(
                        subscription_id = %subscription_id,
                        filter_node_type = %filter_node_type,
                        "Node type filter exists but event has no node_type"
                    );
                    continue;
                }
            }

            tracing::debug!(
                subscription_id = %subscription_id,
                include_node = %filters.include_node,
                "All filters matched!"
            );
            matching_subs.push((subscription_id.clone(), filters.clone()));
        }

        matching_subs
    }
}
