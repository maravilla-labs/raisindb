// SPDX-License-Identifier: BSL-1.1

//! Index query types

use raisin_models::nodes::properties::PropertyValue;
use serde_json::Value as JsonValue;

/// Query types for different index operations
#[derive(Debug, Clone)]
pub enum IndexQuery {
    /// Find nodes by exact property match
    FindByProperty {
        workspace: String,
        property_name: String,
        property_value: Box<PropertyValue>,
    },

    /// Find nodes by property value (using JSON comparison)
    FindByPropertyJson {
        workspace: String,
        property_name: String,
        property_value: JsonValue,
    },

    /// Find nodes that have a specific property (regardless of value)
    FindNodesWithProperty {
        workspace: String,
        property_name: String,
    },

    /// Find nodes by numeric property range
    FindByPropertyRange {
        workspace: String,
        property_name: String,
        min: Option<f64>,
        max: Option<f64>,
    },

    /// Find nodes by date property range
    FindByDateRange {
        workspace: String,
        property_name: String,
        start: Option<chrono::DateTime<chrono::Utc>>,
        end: Option<chrono::DateTime<chrono::Utc>>,
    },

    /// Find nodes that reference a specific target node
    FindReferences {
        workspace: String,
        target_node_id: String,
    },

    /// Full-text search (future enhancement)
    FullTextSearch { workspace: String, query: String },
}

impl IndexQuery {
    /// Get the workspace for this query
    pub fn workspace(&self) -> &str {
        match self {
            IndexQuery::FindByProperty { workspace, .. }
            | IndexQuery::FindByPropertyJson { workspace, .. }
            | IndexQuery::FindNodesWithProperty { workspace, .. }
            | IndexQuery::FindByPropertyRange { workspace, .. }
            | IndexQuery::FindByDateRange { workspace, .. }
            | IndexQuery::FindReferences { workspace, .. }
            | IndexQuery::FullTextSearch { workspace, .. } => workspace,
        }
    }
}
