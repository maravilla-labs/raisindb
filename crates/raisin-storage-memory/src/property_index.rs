//! In-memory property indexing implementation
//!
//! Provides O(1) property lookups using HashMap-based indexes.
//! Maintains separate draft and published index spaces for proper publish/unpublish workflow.

use crate::index_types::PropertyIndex;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::scope::StorageScope;
use raisin_storage::{PropertyIndexRepository, PropertyScanEntry};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

/// In-memory property index repository
///
/// Uses nested HashMaps for O(1) lookups:
/// - composite_key (tenant/repo/workspace) -> property_name -> value_json -> node_ids
/// - Maintains separate draft and published indexes
#[derive(Clone)]
pub struct InMemoryPropertyIndexRepo {
    // Draft indexes: composite_key -> property_name -> value_json -> node_ids
    draft_indexes: PropertyIndex,
    // Published indexes: composite_key -> property_name -> value_json -> node_ids
    published_indexes: PropertyIndex,
}

impl InMemoryPropertyIndexRepo {
    /// Create a new in-memory property index repository
    pub fn new() -> Self {
        Self {
            draft_indexes: Arc::new(RwLock::new(HashMap::new())),
            published_indexes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Helper to serialize property value to consistent JSON string
    fn value_to_json(value: &PropertyValue) -> String {
        serde_json::to_string(value).unwrap_or_default()
    }

    /// Create composite key for repository isolation (includes branch)
    fn composite_key(tenant_id: &str, repo_id: &str, branch: &str, workspace: &str) -> String {
        format!("{}/{}/{}/{}", tenant_id, repo_id, branch, workspace)
    }
}

impl Default for InMemoryPropertyIndexRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl PropertyIndexRepository for InMemoryPropertyIndexRepo {
    async fn index_properties(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        is_published: bool,
    ) -> Result<()> {
        let indexes = if is_published {
            &self.published_indexes
        } else {
            &self.draft_indexes
        };

        let mut indexes = indexes.write().unwrap();
        let key = Self::composite_key(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
        );

        for (prop_name, prop_value) in properties {
            let value_json = Self::value_to_json(prop_value);

            indexes
                .entry(key.clone())
                .or_default()
                .entry(prop_name.clone())
                .or_default()
                .entry(value_json)
                .or_default()
                .insert(node_id.to_string());
        }

        Ok(())
    }

    async fn unindex_properties(&self, scope: StorageScope<'_>, node_id: &str) -> Result<()> {
        let key = Self::composite_key(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
        );

        // Remove from both draft and published indexes
        for indexes in [&self.draft_indexes, &self.published_indexes] {
            let mut indexes = indexes.write().unwrap();

            if let Some(workspace_indexes) = indexes.get_mut(&key) {
                for prop_indexes in workspace_indexes.values_mut() {
                    for node_set in prop_indexes.values_mut() {
                        node_set.remove(node_id);
                    }
                }
            }
        }

        Ok(())
    }

    async fn update_publish_status(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        is_published: bool,
    ) -> Result<()> {
        let key = Self::composite_key(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
        );

        // Remove from old index space
        let old_indexes = if is_published {
            &self.draft_indexes
        } else {
            &self.published_indexes
        };

        {
            let mut old_indexes = old_indexes.write().unwrap();
            if let Some(workspace_indexes) = old_indexes.get_mut(&key) {
                for prop_indexes in workspace_indexes.values_mut() {
                    for node_set in prop_indexes.values_mut() {
                        node_set.remove(node_id);
                    }
                }
            }
        }

        // Add to new index space
        self.index_properties(scope, node_id, properties, is_published)
            .await
    }

    async fn find_by_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        property_value: &PropertyValue,
        published_only: bool,
    ) -> Result<Vec<String>> {
        let indexes = if published_only {
            &self.published_indexes
        } else {
            &self.draft_indexes
        };

        let indexes = indexes.read().unwrap();
        let value_json = Self::value_to_json(property_value);
        let key = Self::composite_key(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
        );

        let node_ids = indexes
            .get(&key)
            .and_then(|props| props.get(property_name))
            .and_then(|values| values.get(&value_json))
            .cloned()
            .unwrap_or_default();

        Ok(node_ids.into_iter().collect())
    }

    async fn find_nodes_with_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        published_only: bool,
    ) -> Result<Vec<String>> {
        let indexes = if published_only {
            &self.published_indexes
        } else {
            &self.draft_indexes
        };

        let indexes = indexes.read().unwrap();
        let key = Self::composite_key(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
        );

        let mut all_node_ids = HashSet::new();

        if let Some(workspace_indexes) = indexes.get(&key) {
            if let Some(prop_values) = workspace_indexes.get(property_name) {
                for node_set in prop_values.values() {
                    all_node_ids.extend(node_set.iter().cloned());
                }
            }
        }

        Ok(all_node_ids.into_iter().collect())
    }

    async fn scan_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        published_only: bool,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<PropertyScanEntry>> {
        let indexes = if published_only {
            &self.published_indexes
        } else {
            &self.draft_indexes
        };

        let indexes = indexes.read().unwrap();
        let key = Self::composite_key(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
        );

        let mut pairs: Vec<(String, String)> = Vec::new();

        if let Some(workspace_indexes) = indexes.get(&key) {
            if let Some(prop_values) = workspace_indexes.get(property_name) {
                for (value, nodes) in prop_values {
                    for node_id in nodes {
                        pairs.push((value.clone(), node_id.clone()));
                    }
                }
            }
        }

        pairs.sort_by(|a, b| {
            if ascending {
                a.0.cmp(&b.0)
            } else {
                b.0.cmp(&a.0)
            }
        });

        let mut seen = HashSet::new();
        let mut results = Vec::new();

        for (value, node_id) in pairs {
            // Early termination if limit reached
            if let Some(lim) = limit {
                if results.len() >= lim {
                    break;
                }
            }

            if seen.insert(node_id.clone()) {
                results.push(PropertyScanEntry {
                    node_id,
                    property_value: value,
                });
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope<'a>(
        tenant_id: &'a str,
        repo_id: &'a str,
        branch: &'a str,
        workspace: &'a str,
    ) -> StorageScope<'a> {
        StorageScope::new(tenant_id, repo_id, branch, workspace)
    }

    #[tokio::test]
    async fn test_index_and_find_property() {
        let repo = InMemoryPropertyIndexRepo::new();

        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            PropertyValue::String("test@example.com".to_string()),
        );

        // Index property as draft
        repo.index_properties(
            scope("tenant1", "repo1", "main", "ws1"),
            "node1",
            &props,
            false,
        )
        .await
        .unwrap();

        // Find by property value
        let email_value = PropertyValue::String("test@example.com".to_string());
        let results = repo
            .find_by_property(
                scope("tenant1", "repo1", "main", "ws1"),
                "email",
                &email_value,
                false,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results.contains(&"node1".to_string()));
    }

    #[tokio::test]
    async fn test_publish_status_update() {
        let repo = InMemoryPropertyIndexRepo::new();

        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            PropertyValue::String("test@example.com".to_string()),
        );

        // Index as draft
        repo.index_properties(
            scope("tenant1", "repo1", "main", "ws1"),
            "node1",
            &props,
            false,
        )
        .await
        .unwrap();

        // Should find in draft
        let email_value = PropertyValue::String("test@example.com".to_string());
        let results = repo
            .find_by_property(
                scope("tenant1", "repo1", "main", "ws1"),
                "email",
                &email_value,
                false,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);

        // Should NOT find in published
        let results = repo
            .find_by_property(
                scope("tenant1", "repo1", "main", "ws1"),
                "email",
                &email_value,
                true,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 0);

        // Update to published
        repo.update_publish_status(
            scope("tenant1", "repo1", "main", "ws1"),
            "node1",
            &props,
            true,
        )
        .await
        .unwrap();

        // Should now find in published
        let results = repo
            .find_by_property(
                scope("tenant1", "repo1", "main", "ws1"),
                "email",
                &email_value,
                true,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);

        // Should NOT find in draft anymore
        let results = repo
            .find_by_property(
                scope("tenant1", "repo1", "main", "ws1"),
                "email",
                &email_value,
                false,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_workspace_isolation() {
        let repo = InMemoryPropertyIndexRepo::new();

        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            PropertyValue::String("test@example.com".to_string()),
        );

        // Index same property in two repos
        repo.index_properties(
            scope("tenant1", "repo1", "main", "ws1"),
            "node1",
            &props,
            false,
        )
        .await
        .unwrap();
        repo.index_properties(
            scope("tenant1", "repo2", "main", "ws1"),
            "node2",
            &props,
            false,
        )
        .await
        .unwrap();

        // repo1 should only see node1
        let email_value = PropertyValue::String("test@example.com".to_string());
        let results = repo
            .find_by_property(
                scope("tenant1", "repo1", "main", "ws1"),
                "email",
                &email_value,
                false,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results.contains(&"node1".to_string()));

        // repo2 should only see node2
        let results = repo
            .find_by_property(
                scope("tenant1", "repo2", "main", "ws1"),
                "email",
                &email_value,
                false,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results.contains(&"node2".to_string()));
    }

    #[tokio::test]
    async fn test_unindex_properties() {
        let repo = InMemoryPropertyIndexRepo::new();

        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            PropertyValue::String("test@example.com".to_string()),
        );
        props.insert("age".to_string(), PropertyValue::Integer(25));

        // Index properties
        repo.index_properties(
            scope("tenant1", "repo1", "main", "ws1"),
            "node1",
            &props,
            false,
        )
        .await
        .unwrap();

        // Verify indexed
        let email_value = PropertyValue::String("test@example.com".to_string());
        let results = repo
            .find_by_property(
                scope("tenant1", "repo1", "main", "ws1"),
                "email",
                &email_value,
                false,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);

        // Unindex all properties
        repo.unindex_properties(scope("tenant1", "repo1", "main", "ws1"), "node1")
            .await
            .unwrap();

        // Should no longer find
        let results = repo
            .find_by_property(
                scope("tenant1", "repo1", "main", "ws1"),
                "email",
                &email_value,
                false,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_find_nodes_with_property() {
        let repo = InMemoryPropertyIndexRepo::new();

        // Create nodes with different email values
        let mut props1 = HashMap::new();
        props1.insert(
            "email".to_string(),
            PropertyValue::String("user1@example.com".to_string()),
        );

        let mut props2 = HashMap::new();
        props2.insert(
            "email".to_string(),
            PropertyValue::String("user2@example.com".to_string()),
        );

        let mut props3 = HashMap::new();
        props3.insert(
            "name".to_string(),
            PropertyValue::String("User3".to_string()),
        );

        repo.index_properties(
            scope("tenant1", "repo1", "main", "ws1"),
            "node1",
            &props1,
            false,
        )
        .await
        .unwrap();
        repo.index_properties(
            scope("tenant1", "repo1", "main", "ws1"),
            "node2",
            &props2,
            false,
        )
        .await
        .unwrap();
        repo.index_properties(
            scope("tenant1", "repo1", "main", "ws1"),
            "node3",
            &props3,
            false,
        )
        .await
        .unwrap();

        // Should find node1 and node2 (both have email property)
        let results = repo
            .find_nodes_with_property(scope("tenant1", "repo1", "main", "ws1"), "email", false)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.contains(&"node1".to_string()));
        assert!(results.contains(&"node2".to_string()));
        assert!(!results.contains(&"node3".to_string()));
    }
}
