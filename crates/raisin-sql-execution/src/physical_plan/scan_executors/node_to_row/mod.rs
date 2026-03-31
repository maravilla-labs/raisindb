// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Node-to-Row conversion for scan executors.
//!
//! Converts `Node` instances into `Row` values with qualified column names,
//! projection support, and virtual column population (embedding, locale, etc.).
//!
//! # Module Structure
//!
//! - `fields` - Standard, optional, computed, and property field insertion
//! - `embedding` - Virtual embedding field fetched from RocksDB

mod embedding;
mod fields;

use crate::physical_plan::executor::{ExecutionContext, Row};
use raisin_error::Error;
use raisin_models::nodes::Node;
use raisin_storage::Storage;

/// Convert a Node to a Row, populating virtual columns including embedding.
///
/// This function is async because it may need to fetch the embedding from RocksDB storage
/// when the `embedding` column is requested in the projection.
///
/// The `effective_locale` parameter specifies which locale this node represents
/// (for the virtual locale column).
///
/// # Column Naming
///
/// All columns are fully qualified with the workspace/alias qualifier:
/// - Node metadata: `qualifier.id`, `qualifier.path`, `qualifier.node_type`, etc.
/// - Node properties: `qualifier.property_name`
/// - Computed columns: `qualifier.depth`, `qualifier.__workspace`, `qualifier.locale`
pub(crate) async fn node_to_row<S: Storage>(
    node: &Node,
    qualifier: &str,
    workspace: &str,
    projection: &Option<Vec<String>>,
    ctx: &ExecutionContext<S>,
    effective_locale: &str,
) -> Result<Row, Error> {
    use raisin_models::nodes::properties::PropertyValue;

    let mut row = Row::new();

    // Fast path for simple, common projections to avoid expensive conditional checks
    // This optimization significantly improves LIMIT queries performance
    if let Some(proj) = projection {
        match proj.len() {
            // id-only projection (very common for LIMIT queries)
            1 if proj[0] == "id" => {
                row.insert(
                    format!("{}.id", qualifier),
                    PropertyValue::String(node.id.clone()),
                );
                return Ok(row);
            }
            // id + path projection (also common)
            2 if proj.contains(&"id".to_string()) && proj.contains(&"path".to_string()) => {
                row.insert(
                    format!("{}.id", qualifier),
                    PropertyValue::String(node.id.clone()),
                );
                row.insert(
                    format!("{}.path", qualifier),
                    PropertyValue::String(node.path.clone()),
                );
                return Ok(row);
            }
            _ => {
                // Fall through to full path for other projection patterns
            }
        }
    }

    // Helper to check if column should be included (checks unqualified name)
    let should_include = |col: &str| {
        projection
            .as_ref()
            .is_none_or(|p| p.contains(&col.to_string()))
    };

    // Map standard node fields with qualified names
    fields::insert_standard_fields(&mut row, node, qualifier, &should_include);
    fields::insert_optional_fields(&mut row, node, qualifier, &should_include);
    fields::insert_computed_fields(
        &mut row,
        node,
        qualifier,
        workspace,
        effective_locale,
        &should_include,
    );

    // Virtual column: embedding (fetched from RocksDB embedding storage)
    if should_include("embedding") {
        embedding::insert_embedding_field(&mut row, node, qualifier, workspace, ctx).await?;
    }

    // Include properties with qualified names
    fields::insert_property_fields(&mut row, node, qualifier, projection);

    Ok(row)
}
