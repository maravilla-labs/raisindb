// SPDX-License-Identifier: BSL-1.1

//! Property index plugin for efficient property lookups

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use raisin_events::{Event, EventHandler, NodeEventKind};
use raisin_models::nodes::properties::PropertyValue;
use serde_json::Value as JsonValue;

use crate::{IndexPlugin, IndexQuery};

/// workspace -> property_name -> value_json -> node_ids
type PropertyIndexMap =
    Arc<RwLock<HashMap<String, HashMap<String, HashMap<String, HashSet<String>>>>>>;

/// In-memory property index for O(1) property lookups
///
/// Structure: workspace -> property_name -> property_value_json -> [node_ids]
///
/// This index maintains a mapping from property values to node IDs,
/// allowing for efficient unique property validation and property-based queries.
#[derive(Clone, Default)]
pub struct PropertyIndexPlugin {
    /// workspace -> property_name -> property_value (as JSON) -> set of node_ids
    index: PropertyIndexMap,
}

impl PropertyIndexPlugin {
    /// Create a new empty property index
    pub fn new() -> Self {
        Self {
            index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Index a single node's properties
    fn index_node_properties(
        &self,
        workspace: &str,
        node_id: &str,
        properties: &HashMap<String, JsonValue>,
    ) -> anyhow::Result<()> {
        let mut index = self.index.write().unwrap();
        let workspace_index = index.entry(workspace.to_string()).or_default();

        for (prop_name, prop_value) in properties {
            let property_index = workspace_index.entry(prop_name.clone()).or_default();

            // Serialize property value to JSON string for indexing
            let value_key = serde_json::to_string(prop_value)?;

            property_index
                .entry(value_key)
                .or_default()
                .insert(node_id.to_string());
        }

        Ok(())
    }

    /// Remove a node from all indexes in a workspace
    fn remove_from_indexes(&self, workspace: &str, node_id: &str) {
        let mut index = self.index.write().unwrap();

        if let Some(workspace_index) = index.get_mut(workspace) {
            // Remove node from all property value sets
            for property_index in workspace_index.values_mut() {
                for node_set in property_index.values_mut() {
                    node_set.remove(node_id);
                }

                // Clean up empty value sets
                property_index.retain(|_, nodes| !nodes.is_empty());
            }

            // Clean up empty property indexes
            workspace_index.retain(|_, values| !values.is_empty());
        }

        // Clean up empty workspace indexes
        index.retain(|_, props| !props.is_empty());
    }

    /// Convert PropertyValue to JSON for comparison
    fn property_value_to_json(value: &PropertyValue) -> anyhow::Result<String> {
        // Serialize to JSON for consistent comparison
        let json = serde_json::to_value(value)?;
        Ok(serde_json::to_string(&json)?)
    }

    /// Find nodes by property value
    fn find_by_property_internal(
        &self,
        workspace: &str,
        property_name: &str,
        property_value_json: &str,
    ) -> Vec<String> {
        let index = self.index.read().unwrap();

        index
            .get(workspace)
            .and_then(|props| props.get(property_name))
            .and_then(|values| values.get(property_value_json))
            .map(|nodes| nodes.iter().cloned().collect())
            .unwrap_or_default()
    }
}

impl EventHandler for PropertyIndexPlugin {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // Only handle node events
            let node_event = match event {
                Event::Node(ne) => ne,
                _ => return Ok(()), // Ignore non-node events
            };

            // Create workspace key from tenant_id/repository_id/branch
            let workspace = format!(
                "{}/{}/{}",
                node_event.tenant_id, node_event.repository_id, node_event.branch
            );

            match &node_event.kind {
                NodeEventKind::Created | NodeEventKind::Updated => {
                    // First remove old indexes for this node
                    self.remove_from_indexes(&workspace, &node_event.node_id);

                    // Extract properties from metadata
                    if let Some(ref metadata) = node_event.metadata {
                        if let Some(properties_value) = metadata.get("properties") {
                            if let Some(properties_obj) = properties_value.as_object() {
                                let properties: HashMap<String, JsonValue> = properties_obj
                                    .iter()
                                    .map(|(k, v)| (k.clone(), v.clone()))
                                    .collect();

                                self.index_node_properties(
                                    &workspace,
                                    &node_event.node_id,
                                    &properties,
                                )?;
                            }
                        }
                    }
                }
                NodeEventKind::Deleted => {
                    self.remove_from_indexes(&workspace, &node_event.node_id);
                }
                NodeEventKind::PropertyChanged { property } => {
                    // For property-level changes, extract from metadata
                    if let Some(ref metadata) = node_event.metadata {
                        // Remove old value if present
                        if let Some(old_value) = metadata.get("old_value") {
                            let value_key = serde_json::to_string(old_value)?;
                            let mut index = self.index.write().unwrap();
                            if let Some(workspace_index) = index.get_mut(&workspace) {
                                if let Some(property_index) = workspace_index.get_mut(property) {
                                    if let Some(node_set) = property_index.get_mut(&value_key) {
                                        node_set.remove(&node_event.node_id);
                                        if node_set.is_empty() {
                                            property_index.remove(&value_key);
                                        }
                                    }
                                }
                            }
                        }

                        // Add new value if present
                        if let Some(new_value) = metadata.get("new_value") {
                            let value_key = serde_json::to_string(new_value)?;
                            let mut index = self.index.write().unwrap();
                            let workspace_index = index.entry(workspace.clone()).or_default();
                            let property_index =
                                workspace_index.entry(property.clone()).or_default();
                            property_index
                                .entry(value_key)
                                .or_default()
                                .insert(node_event.node_id.clone());
                        }
                    }
                }
                _ => {} // Ignore Published, Unpublished
            }

            Ok(())
        })
    }

    fn name(&self) -> &str {
        "PropertyIndexPlugin"
    }
}

impl IndexPlugin for PropertyIndexPlugin {
    fn index_name(&self) -> &str {
        "property_unique"
    }

    fn query(
        &self,
        query: IndexQuery,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<String>>> + Send + '_>> {
        Box::pin(async move {
            match query {
                IndexQuery::FindByProperty {
                    workspace,
                    property_name,
                    property_value,
                } => {
                    let value_json = Self::property_value_to_json(&property_value)?;
                    Ok(self.find_by_property_internal(&workspace, &property_name, &value_json))
                }
                IndexQuery::FindByPropertyJson {
                    workspace,
                    property_name,
                    property_value,
                } => {
                    let value_json = serde_json::to_string(&property_value)?;
                    Ok(self.find_by_property_internal(&workspace, &property_name, &value_json))
                }
                IndexQuery::FindNodesWithProperty {
                    workspace,
                    property_name,
                } => {
                    let index = self.index.read().unwrap();
                    let node_ids: HashSet<String> = index
                        .get(&workspace)
                        .and_then(|props| props.get(&property_name))
                        .map(|values| {
                            values
                                .values()
                                .flat_map(|nodes| nodes.iter().cloned())
                                .collect()
                        })
                        .unwrap_or_default();
                    Ok(node_ids.into_iter().collect())
                }
                _ => Ok(Vec::new()), // Unsupported query types
            }
        })
    }

    fn supports_query(&self, query: &IndexQuery) -> bool {
        matches!(
            query,
            IndexQuery::FindByProperty { .. }
                | IndexQuery::FindByPropertyJson { .. }
                | IndexQuery::FindNodesWithProperty { .. }
        )
    }

    fn clear(&self) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let mut index = self.index.write().unwrap();
            index.clear();
            Ok(())
        })
    }

    fn stats(&self) -> HashMap<String, JsonValue> {
        let index = self.index.read().unwrap();
        let mut stats = HashMap::new();

        let workspace_count = index.len();
        let mut total_properties = 0;
        let mut total_values = 0;
        let mut total_nodes = 0;

        for workspace_index in index.values() {
            total_properties += workspace_index.len();
            for property_index in workspace_index.values() {
                total_values += property_index.len();
                for node_set in property_index.values() {
                    total_nodes += node_set.len();
                }
            }
        }

        stats.insert("workspace_count".to_string(), workspace_count.into());
        stats.insert("property_count".to_string(), total_properties.into());
        stats.insert("unique_value_count".to_string(), total_values.into());
        stats.insert("total_indexed_entries".to_string(), total_nodes.into());

        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_events::{Event, NodeEvent};

    fn create_test_event(
        workspace: &str,
        node_id: &str,
        kind: NodeEventKind,
        properties: Option<HashMap<String, JsonValue>>,
    ) -> Event {
        let node_event = NodeEvent {
            tenant_id: "test".to_string(),
            repository_id: "repo1".to_string(),
            branch: "main".to_string(),
            node_id: node_id.to_string(),
            node_type: Some("test:Type".to_string()),
            kind,
            path: None,
            metadata: properties.map(|props| {
                let mut meta = HashMap::new();
                meta.insert(
                    "properties".to_string(),
                    JsonValue::Object(props.into_iter().collect()),
                );
                meta
            }),
            workspace_id: todo!(),
            revision: todo!(),
        };
        Event::Node(node_event)
    }

    #[tokio::test]
    async fn test_property_index_basic() {
        let index = PropertyIndexPlugin::new();

        // Create node with property
        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            JsonValue::String("user@example.com".to_string()),
        );

        let event = create_test_event("ws1", "node1", NodeEventKind::Created, Some(props));
        index.handle(&event).await.unwrap();

        // Query by property - workspace key is "test/repo1/main" (tenant/repo/branch)
        let query = IndexQuery::FindByPropertyJson {
            workspace: "test/repo1/main".to_string(),
            property_name: "email".to_string(),
            property_value: JsonValue::String("user@example.com".to_string()),
        };

        let results = index.query(query).await.unwrap();
        assert_eq!(results, vec!["node1"]);
    }

    #[tokio::test]
    async fn test_property_index_update() {
        let index = PropertyIndexPlugin::new();

        // Create node
        let mut props1 = HashMap::new();
        props1.insert(
            "email".to_string(),
            JsonValue::String("old@example.com".to_string()),
        );

        let event1 = create_test_event("ws1", "node1", NodeEventKind::Created, Some(props1));
        index.handle(&event1).await.unwrap();

        // Update node
        let mut props2 = HashMap::new();
        props2.insert(
            "email".to_string(),
            JsonValue::String("new@example.com".to_string()),
        );

        let event2 = create_test_event("ws1", "node1", NodeEventKind::Updated, Some(props2));
        index.handle(&event2).await.unwrap();

        // Query old value - should be empty (workspace key is "test/repo1/main")
        let query_old = IndexQuery::FindByPropertyJson {
            workspace: "test/repo1/main".to_string(),
            property_name: "email".to_string(),
            property_value: JsonValue::String("old@example.com".to_string()),
        };
        let results_old = index.query(query_old).await.unwrap();
        assert!(results_old.is_empty());

        // Query new value - should find node
        let query_new = IndexQuery::FindByPropertyJson {
            workspace: "test/repo1/main".to_string(),
            property_name: "email".to_string(),
            property_value: JsonValue::String("new@example.com".to_string()),
        };
        let results_new = index.query(query_new).await.unwrap();
        assert_eq!(results_new, vec!["node1"]);
    }

    #[tokio::test]
    async fn test_property_index_delete() {
        let index = PropertyIndexPlugin::new();

        // Create node
        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            JsonValue::String("user@example.com".to_string()),
        );

        let event1 = create_test_event("ws1", "node1", NodeEventKind::Created, Some(props));
        index.handle(&event1).await.unwrap();

        // Delete node
        let event2 = create_test_event("ws1", "node1", NodeEventKind::Deleted, None);
        index.handle(&event2).await.unwrap();

        // Query - should be empty (workspace key is "test/repo1/main")
        let query = IndexQuery::FindByPropertyJson {
            workspace: "test/repo1/main".to_string(),
            property_name: "email".to_string(),
            property_value: JsonValue::String("user@example.com".to_string()),
        };
        let results = index.query(query).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_property_index_stats() {
        let index = PropertyIndexPlugin::new();

        let mut props = HashMap::new();
        props.insert(
            "email".to_string(),
            JsonValue::String("user@example.com".to_string()),
        );
        props.insert(
            "name".to_string(),
            JsonValue::String("John Doe".to_string()),
        );

        let event = create_test_event("ws1", "node1", NodeEventKind::Created, Some(props));
        index.handle(&event).await.unwrap();

        let stats = index.stats();
        assert_eq!(stats.get("workspace_count"), Some(&JsonValue::from(1)));
        assert_eq!(stats.get("property_count"), Some(&JsonValue::from(2)));
    }
}
