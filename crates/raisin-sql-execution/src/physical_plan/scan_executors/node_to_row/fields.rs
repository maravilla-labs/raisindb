// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Node field insertion helpers for node-to-row conversion.
//!
//! Handles inserting standard, optional, computed, and property fields into
//! rows with fully qualified column names.

use crate::physical_plan::executor::Row;
use raisin_models::nodes::Node;

/// Insert standard node fields (id, path, node_type, name, timestamps, version).
pub(super) fn insert_standard_fields(
    row: &mut Row,
    node: &Node,
    qualifier: &str,
    should_include: &dyn Fn(&str) -> bool,
) {
    use raisin_models::nodes::properties::PropertyValue;

    if should_include("id") {
        row.insert(
            format!("{}.id", qualifier),
            PropertyValue::String(node.id.clone()),
        );
    }

    if should_include("path") {
        row.insert(
            format!("{}.path", qualifier),
            PropertyValue::String(node.path.clone()),
        );
    }

    if should_include("__node_type") || should_include("node_type") {
        row.insert(
            format!("{}.__node_type", qualifier),
            PropertyValue::String(node.node_type.clone()),
        );
        row.insert(
            format!("{}.node_type", qualifier),
            PropertyValue::String(node.node_type.clone()),
        );
    }

    if should_include("name") {
        row.insert(
            format!("{}.name", qualifier),
            PropertyValue::String(node.name.clone()),
        );
    }

    if should_include("created_at") {
        if let Some(created_at) = node.created_at {
            row.insert(
                format!("{}.created_at", qualifier),
                PropertyValue::Date(created_at.into()),
            );
        }
    }

    if should_include("updated_at") {
        if let Some(updated_at) = node.updated_at {
            row.insert(
                format!("{}.updated_at", qualifier),
                PropertyValue::Date(updated_at.into()),
            );
        }
    }

    if should_include("version") {
        row.insert(
            format!("{}.version", qualifier),
            PropertyValue::Integer(node.version as i64),
        );
    }
}

/// Insert optional node fields (archetype, published_at/by, created_by, updated_by, parent_name).
pub(super) fn insert_optional_fields(
    row: &mut Row,
    node: &Node,
    qualifier: &str,
    should_include: &dyn Fn(&str) -> bool,
) {
    use raisin_models::nodes::properties::PropertyValue;

    if should_include("archetype") {
        if let Some(ref archetype) = node.archetype {
            row.insert(
                format!("{}.archetype", qualifier),
                PropertyValue::String(archetype.clone()),
            );
        }
    }

    if should_include("published_at") {
        if let Some(published_at) = node.published_at {
            row.insert(
                format!("{}.published_at", qualifier),
                PropertyValue::Date(published_at.into()),
            );
        }
    }

    if should_include("published_by") {
        if let Some(ref published_by) = node.published_by {
            row.insert(
                format!("{}.published_by", qualifier),
                PropertyValue::String(published_by.clone()),
            );
        }
    }

    if should_include("created_by") {
        if let Some(ref created_by) = node.created_by {
            row.insert(
                format!("{}.created_by", qualifier),
                PropertyValue::String(created_by.clone()),
            );
        }
    }

    if should_include("updated_by") {
        if let Some(ref updated_by) = node.updated_by {
            row.insert(
                format!("{}.updated_by", qualifier),
                PropertyValue::String(updated_by.clone()),
            );
        }
    }

    if should_include("parent_name") {
        if let Some(ref parent_path) = node.parent {
            let parent_name = parent_path
                .rsplit('/')
                .find(|s| !s.is_empty())
                .unwrap_or(parent_path);
            row.insert(
                format!("{}.parent_name", qualifier),
                PropertyValue::String(parent_name.to_string()),
            );
        }
    }
}

/// Insert computed fields (properties, depth, __workspace, locale).
pub(super) fn insert_computed_fields(
    row: &mut Row,
    node: &Node,
    qualifier: &str,
    workspace: &str,
    effective_locale: &str,
    should_include: &dyn Fn(&str) -> bool,
) {
    use raisin_models::nodes::properties::PropertyValue;

    // Properties as JSONB column
    if should_include("properties") {
        row.insert(
            format!("{}.properties", qualifier),
            PropertyValue::Object(node.properties.clone()),
        );
    }

    // Computed column: depth
    if should_include("depth") {
        let depth = node.path.split('/').filter(|s| !s.is_empty()).count();
        row.insert(
            format!("{}.depth", qualifier),
            PropertyValue::Integer(depth as i64),
        );
    }

    // Virtual column: __workspace (only when explicitly requested, not in SELECT *)
    if should_include("__workspace") {
        row.insert(
            format!("{}.__workspace", qualifier),
            PropertyValue::String(workspace.to_string()),
        );
    }

    // Virtual column: locale (shows the effective locale after translation resolution)
    if should_include("locale") {
        row.insert(
            format!("{}.locale", qualifier),
            PropertyValue::String(effective_locale.to_string()),
        );
    }
}

/// Insert property fields from node.properties into the row.
pub(super) fn insert_property_fields(
    row: &mut Row,
    node: &Node,
    qualifier: &str,
    projection: &Option<Vec<String>>,
) {
    if let Some(proj) = projection {
        for col in proj {
            // Skip standard fields already handled
            if matches!(
                col.as_str(),
                "id" | "path"
                    | "__node_type"
                    | "node_type"
                    | "name"
                    | "created_at"
                    | "updated_at"
                    | "version"
                    | "depth"
                    | "__workspace"
                    | "locale"
                    | "archetype"
                    | "published_at"
                    | "published_by"
                    | "created_by"
                    | "updated_by"
                    | "parent_name"
                    | "properties"
                    | "embedding"
            ) {
                continue;
            }

            // Look for property in node.properties
            if let Some(value) = node.properties.get(col) {
                row.insert(format!("{}.{}", qualifier, col), value.clone());
            }
        }
    } else {
        // No projection - include all properties with qualified names
        for (key, value) in &node.properties {
            row.insert(format!("{}.{}", qualifier, key), value.clone());
        }
    }
}
