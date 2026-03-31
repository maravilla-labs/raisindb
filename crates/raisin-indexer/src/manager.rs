// SPDX-License-Identifier: BSL-1.1

//! Index manager for coordinating multiple index plugins

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::{IndexPlugin, IndexQuery};

/// Manages multiple index plugins and routes queries to appropriate indexes
#[derive(Clone)]
pub struct IndexManager {
    plugins: Arc<RwLock<HashMap<String, Arc<dyn IndexPlugin>>>>,
}

impl IndexManager {
    /// Create a new empty index manager
    pub fn new() -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register an index plugin
    ///
    /// If a plugin with the same name already exists, it will be replaced.
    pub fn register_plugin(&self, plugin: Arc<dyn IndexPlugin>) {
        let name = plugin.index_name().to_string();
        let mut plugins = self.plugins.write().unwrap();
        plugins.insert(name, plugin);
    }

    /// Query a specific index by name
    ///
    /// Returns an error if the index doesn't exist.
    pub async fn query(&self, index_name: &str, query: IndexQuery) -> anyhow::Result<Vec<String>> {
        let plugin = {
            let plugins = self.plugins.read().unwrap();
            plugins
                .get(index_name)
                .ok_or_else(|| anyhow::anyhow!("Index '{}' not found", index_name))?
                .clone()
        }; // Lock is explicitly dropped here

        plugin.query(query).await
    }

    /// Query all indexes that support the given query type
    ///
    /// Returns the union of results from all supporting indexes.
    /// If no indexes support the query, returns an empty vec.
    pub async fn query_all(&self, query: IndexQuery) -> anyhow::Result<Vec<String>> {
        let supporting_plugins: Vec<_> = {
            let plugins = self.plugins.read().unwrap();
            plugins
                .values()
                .filter(|p| p.supports_query(&query))
                .cloned()
                .collect()
        };

        if supporting_plugins.is_empty() {
            return Ok(Vec::new());
        }

        // Query all supporting plugins and merge results
        let mut all_node_ids = std::collections::HashSet::new();

        for plugin in supporting_plugins {
            let results = plugin.query(query.clone()).await?;
            all_node_ids.extend(results);
        }

        Ok(all_node_ids.into_iter().collect())
    }

    /// Get a plugin by name
    pub fn get_plugin(&self, name: &str) -> Option<Arc<dyn IndexPlugin>> {
        let plugins = self.plugins.read().unwrap();
        plugins.get(name).cloned()
    }

    /// Remove a plugin by name
    pub fn remove_plugin(&self, name: &str) -> bool {
        let mut plugins = self.plugins.write().unwrap();
        plugins.remove(name).is_some()
    }

    /// Get names of all registered plugins
    pub fn plugin_names(&self) -> Vec<String> {
        let plugins = self.plugins.read().unwrap();
        plugins.keys().cloned().collect()
    }

    /// Clear all plugins
    pub fn clear(&self) {
        let mut plugins = self.plugins.write().unwrap();
        plugins.clear();
    }

    /// Get aggregate statistics from all plugins
    pub fn stats(&self) -> HashMap<String, serde_json::Value> {
        let plugins = self.plugins.read().unwrap();
        let mut all_stats = HashMap::new();

        for (name, plugin) in plugins.iter() {
            let plugin_stats = plugin.stats();
            all_stats.insert(name.clone(), serde_json::json!(plugin_stats));
        }

        all_stats
    }

    /// Rebuild all indexes
    pub async fn rebuild_all(&self) -> anyhow::Result<()> {
        let plugins_vec: Vec<_> = {
            let plugins = self.plugins.read().unwrap();
            plugins.values().cloned().collect()
        };

        for plugin in plugins_vec {
            plugin.rebuild().await?;
        }

        Ok(())
    }
}

impl Default for IndexManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IndexQuery;
    use raisin_events::Event;
    use std::pin::Pin;

    struct TestPlugin {
        name: String,
    }

    impl raisin_events::EventHandler for TestPlugin {
        fn handle<'a>(
            &'a self,
            _event: &'a Event,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    impl IndexPlugin for TestPlugin {
        fn index_name(&self) -> &str {
            &self.name
        }

        fn query(
            &self,
            _query: IndexQuery,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<String>>> + Send + '_>>
        {
            Box::pin(async { Ok(vec!["node1".to_string()]) })
        }

        fn supports_query(&self, query: &IndexQuery) -> bool {
            matches!(query, IndexQuery::FindByProperty { .. })
        }
    }

    #[tokio::test]
    async fn test_register_and_query() {
        let manager = IndexManager::new();
        let plugin = Arc::new(TestPlugin {
            name: "test_index".to_string(),
        });

        manager.register_plugin(plugin);

        let query = IndexQuery::FindByProperty {
            workspace: "test".to_string(),
            property_name: "email".to_string(),
            property_value: Box::new(raisin_models::nodes::properties::PropertyValue::String(
                "test@example.com".to_string(),
            )),
        };

        let results = manager.query("test_index", query).await.unwrap();
        assert_eq!(results, vec!["node1"]);
    }

    #[test]
    fn test_plugin_management() {
        let manager = IndexManager::new();

        assert_eq!(manager.plugin_names().len(), 0);

        let plugin1 = Arc::new(TestPlugin {
            name: "index1".to_string(),
        });
        let plugin2 = Arc::new(TestPlugin {
            name: "index2".to_string(),
        });

        manager.register_plugin(plugin1);
        manager.register_plugin(plugin2);

        assert_eq!(manager.plugin_names().len(), 2);

        assert!(manager.remove_plugin("index1"));
        assert_eq!(manager.plugin_names().len(), 1);

        manager.clear();
        assert_eq!(manager.plugin_names().len(), 0);
    }
}
