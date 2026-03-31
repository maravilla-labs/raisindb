//! Trigger parsing logic: inline triggers, standalone triggers, and filter extraction

use super::registry::TriggerRegistry;
use super::snapshot::TriggerRegistrySnapshot;
use super::types::{CachedTrigger, TriggerFilters};
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};
use serde_json::Value as JsonValue;

impl<S: Storage> TriggerRegistry<S> {
    /// Load a fresh snapshot from storage
    pub(super) async fn load_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<TriggerRegistrySnapshot> {
        let mut triggers = Vec::new();

        // Load inline triggers from raisin:Function nodes
        let functions = self
            .storage
            .nodes()
            .list_by_type(
                StorageScope::new(tenant_id, repo_id, branch, "functions"),
                "raisin:Function",
                ListOptions::default(),
            )
            .await?;

        for func in functions {
            // Check if function is enabled
            let enabled = func
                .properties
                .get("enabled")
                .and_then(|v| match v {
                    PropertyValue::Boolean(b) => Some(*b),
                    _ => None,
                })
                .unwrap_or(true);

            if !enabled {
                continue;
            }

            // Extract inline triggers
            if let Some(PropertyValue::Array(trigger_array)) = func.properties.get("triggers") {
                for (idx, trigger_value) in trigger_array.iter().enumerate() {
                    if let Ok(trigger_json) = serde_json::to_value(trigger_value) {
                        if let Some(cached) =
                            self.parse_inline_trigger(&func.id, &func.path, &trigger_json, idx)
                        {
                            triggers.push(cached);
                        }
                    }
                }
            }
        }

        // Load standalone raisin:Trigger nodes
        let standalone_triggers = self
            .storage
            .nodes()
            .list_by_type(
                StorageScope::new(tenant_id, repo_id, branch, "functions"),
                "raisin:Trigger",
                ListOptions::default(),
            )
            .await
            .unwrap_or_default();

        for trigger_node in standalone_triggers {
            if let Some(cached) = self
                .parse_standalone_trigger(trigger_node, tenant_id, repo_id, branch)
                .await
            {
                triggers.push(cached);
            }
        }

        let version = self.current.load().version + 1;
        Ok(TriggerRegistrySnapshot::build_indexes(triggers, version))
    }

    /// Parse an inline trigger from a raisin:Function node
    fn parse_inline_trigger(
        &self,
        func_id: &str,
        func_path: &str,
        trigger_json: &JsonValue,
        idx: usize,
    ) -> Option<CachedTrigger> {
        // Check if trigger is enabled
        let enabled = trigger_json
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if !enabled {
            return None;
        }

        // Check trigger type
        let trigger_type = trigger_json
            .get("trigger_type")
            .or_else(|| trigger_json.get("type"))
            .and_then(|v| v.as_str());

        if trigger_type != Some("node_event") && trigger_type != Some("NodeEvent") {
            return None;
        }

        // Extract event_kinds
        let event_kinds = trigger_json
            .get("event_kinds")
            .or_else(|| {
                trigger_json
                    .get("config")
                    .and_then(|c| c.get("event_kinds"))
            })
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if event_kinds.is_empty() {
            return None;
        }

        // Extract filters
        let filters = parse_filters(trigger_json.get("filters"));

        // Extract metadata
        let trigger_name = trigger_json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        let priority = trigger_json
            .get("priority")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        let max_retries = trigger_json
            .get("max_retries")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        Some(CachedTrigger {
            id: format!("{}#inline-{}", func_id, idx),
            function_path: Some(func_path.to_string()),
            trigger_name,
            trigger_path: None,
            priority,
            enabled: true,
            event_kinds,
            filters,
            max_retries,
            workflow_data: None,
        })
    }

    /// Parse a standalone raisin:Trigger node
    async fn parse_standalone_trigger(
        &self,
        trigger_node: raisin_models::nodes::Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Option<CachedTrigger> {
        // Check if enabled
        let enabled = trigger_node
            .properties
            .get("enabled")
            .and_then(|v| match v {
                PropertyValue::Boolean(b) => Some(*b),
                _ => None,
            })
            .unwrap_or(true);

        if !enabled {
            return None;
        }

        // Check trigger type
        let trigger_type = trigger_node
            .properties
            .get("trigger_type")
            .and_then(|v| match v {
                PropertyValue::String(s) => Some(s.as_str()),
                _ => None,
            });

        if trigger_type != Some("node_event") && trigger_type != Some("NodeEvent") {
            return None;
        }

        // Extract config
        let config_json = trigger_node
            .properties
            .get("config")
            .and_then(|v| serde_json::to_value(v).ok())
            .unwrap_or_default();

        // Extract event_kinds
        let event_kinds = config_json
            .get("event_kinds")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if event_kinds.is_empty() {
            return None;
        }

        // Extract filters
        let filters_json = trigger_node
            .properties
            .get("filters")
            .and_then(|v| serde_json::to_value(v).ok())
            .unwrap_or_default();

        let filters = parse_filters(Some(&filters_json));

        // Extract metadata
        let trigger_name = trigger_node
            .properties
            .get("name")
            .and_then(|v| match v {
                PropertyValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| trigger_node.name.clone());

        let priority = trigger_node
            .properties
            .get("priority")
            .and_then(|v| match v {
                PropertyValue::Integer(i) => Some(*i as i32),
                _ => None,
            })
            .unwrap_or(0);

        let max_retries = trigger_node
            .properties
            .get("max_retries")
            .and_then(|v| match v {
                PropertyValue::Integer(i) => Some(*i as u32),
                _ => None,
            });

        // Extract function_path
        let function_path = trigger_node
            .properties
            .get("function_path")
            .and_then(|v| match v {
                PropertyValue::String(s) => Some(s.clone()),
                _ => None,
            });

        // Resolve workflow_data from function_flow reference
        let workflow_data = self
            .resolve_workflow_data(&trigger_node, tenant_id, repo_id, branch)
            .await;

        // Must have either workflow_data or function_path
        if workflow_data.is_none() && function_path.is_none() {
            tracing::warn!(
                trigger_path = %trigger_node.path,
                "Standalone trigger has neither function_flow reference nor function_path, skipping"
            );
            return None;
        }

        Some(CachedTrigger {
            id: trigger_node.id.clone(),
            function_path,
            trigger_name,
            trigger_path: Some(trigger_node.path.clone()),
            priority,
            enabled: true,
            event_kinds,
            filters,
            max_retries,
            workflow_data,
        })
    }

    /// Resolve workflow_data from a function_flow reference on a trigger node
    async fn resolve_workflow_data(
        &self,
        trigger_node: &raisin_models::nodes::Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Option<JsonValue> {
        let ref_val = match trigger_node.properties.get("function_flow") {
            Some(PropertyValue::Reference(r)) => r,
            _ => return None,
        };

        match self
            .storage
            .nodes()
            .get(
                StorageScope::new(tenant_id, repo_id, branch, &ref_val.workspace),
                &ref_val.id,
                None,
            )
            .await
        {
            Ok(Some(flow_node)) => flow_node
                .properties
                .get("workflow_data")
                .and_then(|v| serde_json::to_value(v).ok()),
            Ok(None) => {
                tracing::warn!(
                    ref_id = %ref_val.id,
                    ref_workspace = %ref_val.workspace,
                    trigger_path = %trigger_node.path,
                    "Referenced flow node not found for function_flow"
                );
                None
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    trigger_path = %trigger_node.path,
                    "Failed to resolve function_flow reference"
                );
                None
            }
        }
    }
}

/// Parse filters from JSON into a `TriggerFilters` struct
fn parse_filters(filters_json: Option<&JsonValue>) -> TriggerFilters {
    let mut filters = TriggerFilters::default();

    if let Some(f) = filters_json {
        // Extract workspaces
        filters.workspaces = f.get("workspaces").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

        // Extract node_types
        filters.node_types = f.get("node_types").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

        // Extract paths
        filters.paths = f.get("paths").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

        // Extract property_filters
        filters.property_filters = f
            .get("property_filters")
            .and_then(|v| v.as_object().cloned());
    }

    filters
}
