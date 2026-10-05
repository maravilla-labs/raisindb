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
mod localized;

use crate::physical_plan::executor::{ExecutionContext, Row};
use raisin_error::Error;
use raisin_models::nodes::Node;
use raisin_storage::Storage;
use std::borrow::Cow;

/// Values a scan already holds for the row it is emitting.
///
/// Scans driven by the `ORDERED_CHILDREN` index already hold the node's order
/// label (it is part of the scanned key), and tree traversals additionally know
/// the chain of ancestor labels. Passing them through avoids a re-lookup and is
/// the only way `__tree_order` can be known at all. `LocalizedPathLookup`
/// likewise knows the row's canonical localized path: it emits the node only
/// when that path IS the requested one.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct OrderContext<'a> {
    /// The node's own editorial order label among its siblings.
    ///
    /// `None` falls back to the node record's stored `order_key`.
    pub order_label: Option<&'a str>,
    /// Ancestor order labels joined root-first — sorts as document order.
    ///
    /// `None` leaves `__tree_order` NULL, which is correct for any scan that is
    /// not a tree traversal.
    pub tree_order: Option<&'a str>,
    /// The node's canonical localized path in the row's locale, when the
    /// scan resolved it. `None` computes it (only when `__localized_path` is
    /// projected) — walking every ancestor (plan Phase 13b measured that
    /// recomputation at ~40 % of a localized path lookup).
    pub localized_path: Option<&'a str>,
}

impl<'a> OrderContext<'a> {
    /// Context carrying only a sibling order label.
    pub(crate) fn label(order_label: &'a str) -> Self {
        Self {
            order_label: Some(order_label),
            ..Self::default()
        }
    }
}

/// Convert a Node to a Row, populating virtual columns including embedding.
///
/// This function is async because it may need to fetch the embedding from RocksDB storage
/// when the `embedding` column is named in the projection. It is never fetched for a
/// scan without a projection: `embedding` is opt-in, not part of `SELECT *`.
///
/// The `effective_locale` parameter specifies which locale this node represents
/// (for the virtual locale column). `order_ctx` supplies editorial-ordering
/// values the scan already knows; see [`OrderContext`].
///
/// # Column Naming
///
/// All columns are fully qualified with the workspace/alias qualifier:
/// - Node metadata: `qualifier.id`, `qualifier.path`, `qualifier.node_type`, etc.
/// - Node properties: `qualifier.property_name`
/// - Computed columns: `qualifier.depth`, `qualifier.__workspace`, `qualifier.locale`,
///   `qualifier.__order`, `qualifier.__tree_order`
pub(crate) async fn node_to_row<S: Storage>(
    node: &Node,
    qualifier: &str,
    workspace: &str,
    projection: &Option<Vec<String>>,
    ctx: &ExecutionContext<S>,
    effective_locale: &str,
    order_ctx: Option<&OrderContext<'_>>,
) -> Result<Row, Error> {
    convert(
        Cow::Borrowed(node),
        qualifier,
        workspace,
        projection,
        ctx,
        effective_locale,
        order_ctx,
    )
    .await
}

/// [`node_to_row`] for a node the caller is done with: its property map is
/// MOVED into the `properties` column instead of cloned (the batched index
/// scans hand over every node they read).
pub(crate) async fn node_to_row_owned<S: Storage>(
    node: Node,
    qualifier: &str,
    workspace: &str,
    projection: &Option<Vec<String>>,
    ctx: &ExecutionContext<S>,
    effective_locale: &str,
    order_ctx: Option<&OrderContext<'_>>,
) -> Result<Row, Error> {
    convert(
        Cow::Owned(node),
        qualifier,
        workspace,
        projection,
        ctx,
        effective_locale,
        order_ctx,
    )
    .await
}

/// The ONE conversion behind both entry points.
async fn convert<S: Storage>(
    node: Cow<'_, Node>,
    qualifier: &str,
    workspace: &str,
    projection: &Option<Vec<String>>,
    ctx: &ExecutionContext<S>,
    effective_locale: &str,
    order_ctx: Option<&OrderContext<'_>>,
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

    let owned = matches!(node, Cow::Owned(_));

    // Map standard node fields with qualified names
    fields::insert_standard_fields(&mut row, &node, qualifier, &should_include);
    fields::insert_optional_fields(&mut row, &node, qualifier, &should_include);
    fields::insert_computed_fields(
        &mut row,
        &node,
        qualifier,
        workspace,
        effective_locale,
        order_ctx,
        &should_include,
        owned,
    );

    // Virtual column: embedding (fetched from RocksDB embedding storage) —
    // only when NAMED. It costs a storage read per row and returns a vector
    // nobody asked for, so an unprojected scan (`SELECT *`) never fetches it.
    if projection
        .as_ref()
        .is_some_and(|p| p.iter().any(|c| c == "embedding"))
    {
        embedding::insert_embedding_field(&mut row, &node, qualifier, workspace, ctx).await?;
    }

    // Virtual columns: __node_name / __localized_path — only when NAMED.
    localized::insert_localized_fields(
        &mut row,
        &node,
        qualifier,
        workspace,
        ctx,
        effective_locale,
        projection,
        order_ctx.and_then(|c| c.localized_path),
    )
    .await?;

    // Include properties with qualified names
    fields::insert_property_fields(&mut row, &node, qualifier, projection);

    // The placeholder `insert_computed_fields` left keeps its position.
    if let Cow::Owned(node) = node {
        if should_include("properties") {
            row.insert(
                format!("{}.properties", qualifier),
                PropertyValue::Object(node.properties),
            );
        }
    }

    Ok(row)
}
