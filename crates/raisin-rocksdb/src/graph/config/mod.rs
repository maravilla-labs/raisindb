//! Graph algorithm configuration loader
//!
//! Loads `raisin:GraphAlgorithmConfig` nodes from the `/raisin:access_control/graph-config/` folder.

mod parsers;
#[cfg(test)]
mod tests;

use super::types::{GraphAlgorithm, GraphScope, GraphTarget, RefreshConfig, TargetMode};
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use std::collections::HashMap;

/// Parsed and validated graph algorithm configuration
#[derive(Debug, Clone)]
pub struct GraphAlgorithmConfig {
    /// Unique identifier (node name)
    pub id: String,
    /// The algorithm to run
    pub algorithm: GraphAlgorithm,
    /// Whether this config is active
    pub enabled: bool,
    /// Branch/revision targeting
    pub target: GraphTarget,
    /// Node scoping (paths, types, workspaces, relations)
    pub scope: GraphScope,
    /// Algorithm-specific configuration
    pub config: HashMap<String, serde_json::Value>,
    /// Refresh trigger configuration
    pub refresh: RefreshConfig,
}

impl GraphAlgorithmConfig {
    /// Parse a config from a Node with type `raisin:GraphAlgorithmConfig`
    pub fn from_node(node: &Node) -> Result<Self> {
        let id = node.name.clone();

        let algorithm_str = node
            .properties
            .get("algorithm")
            .and_then(parsers::get_string)
            .ok_or_else(|| {
                Error::Validation("GraphAlgorithmConfig missing 'algorithm' property".to_string())
            })?;

        let algorithm: GraphAlgorithm = algorithm_str.parse().map_err(Error::Validation)?;

        let enabled = node
            .properties
            .get("enabled")
            .and_then(parsers::get_bool)
            .unwrap_or(true);

        let target = parsers::parse_target(node)?;
        let scope = parsers::parse_scope(node)?;

        let config = node
            .properties
            .get("config")
            .and_then(|v| match v {
                PropertyValue::Object(map) => {
                    let json_map: HashMap<String, serde_json::Value> = map
                        .iter()
                        .filter_map(|(k, v)| {
                            parsers::property_value_to_json(v).map(|json| (k.clone(), json))
                        })
                        .collect();
                    Some(json_map)
                }
                _ => None,
            })
            .unwrap_or_default();

        let refresh = parsers::parse_refresh(node)?;

        Ok(Self {
            id,
            algorithm,
            enabled,
            target,
            scope,
            config,
            refresh,
        })
    }

    /// Get a config parameter as f64
    pub fn get_config_f64(&self, key: &str) -> Option<f64> {
        self.config.get(key).and_then(|v| v.as_f64())
    }

    /// Get a config parameter as u64
    pub fn get_config_u64(&self, key: &str) -> Option<u64> {
        self.config.get(key).and_then(|v| v.as_u64())
    }

    /// Get a config parameter as i64
    pub fn get_config_i64(&self, key: &str) -> Option<i64> {
        self.config.get(key).and_then(|v| v.as_i64())
    }

    /// Get a config parameter as string
    pub fn get_config_str(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(|v| v.as_str())
    }

    /// Check if this config targets a specific branch
    pub fn targets_branch(&self, branch_id: &str) -> bool {
        match &self.target.mode {
            TargetMode::Branch => self.target.branches.contains(&branch_id.to_string()),
            TargetMode::AllBranches => true,
            TargetMode::BranchPattern => {
                if let Some(pattern) = &self.target.branch_pattern {
                    parsers::glob_match(pattern, branch_id)
                } else {
                    false
                }
            }
            TargetMode::Revision => false,
        }
    }

    /// Check if this config targets a specific revision
    pub fn targets_revision(&self, revision_id: &str) -> bool {
        match &self.target.mode {
            TargetMode::Revision => self.target.revisions.contains(&revision_id.to_string()),
            _ => false,
        }
    }

    /// Check if this is a branch-tracking config
    pub fn is_branch_tracking(&self) -> bool {
        matches!(
            self.target.mode,
            TargetMode::Branch | TargetMode::AllBranches | TargetMode::BranchPattern
        )
    }

    /// Check if this is an immutable revision config
    pub fn is_revision_mode(&self) -> bool {
        matches!(self.target.mode, TargetMode::Revision)
    }
}
