// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Virtual embedding field insertion.
//!
//! Fetches the embedding vector from RocksDB embedding storage and inserts
//! it as a virtual column in the row.

use crate::physical_plan::executor::{ExecutionContext, Row};
use raisin_error::Error;
use raisin_models::nodes::Node;
use raisin_storage::Storage;

/// Insert the virtual embedding field (fetched from RocksDB embedding storage).
pub(super) async fn insert_embedding_field<S: Storage>(
    row: &mut Row,
    node: &Node,
    qualifier: &str,
    workspace: &str,
    ctx: &ExecutionContext<S>,
) -> Result<(), Error> {
    use raisin_models::nodes::properties::PropertyValue;

    if let Some(embedding_storage) = &ctx.embedding_storage {
        tracing::debug!(
            node_id = %node.id,
            tenant = %ctx.tenant_id,
            repo = %ctx.repo_id,
            branch = %ctx.branch,
            workspace = %workspace,
            "Fetching embedding for virtual column"
        );

        let revision = ctx.max_revision;

        match embedding_storage
            .get_embedding(
                &ctx.tenant_id,
                &ctx.repo_id,
                &ctx.branch,
                workspace,
                &node.id,
                revision.as_ref(),
            )
            .map_err(|e| Error::Backend(format!("Failed to get embedding: {}", e)))?
        {
            Some(embedding_data) => {
                let dimensions = embedding_data.vector.len();
                row.insert(
                    format!("{}.embedding", qualifier),
                    PropertyValue::Vector(embedding_data.vector),
                );
                tracing::trace!(
                    node_id = %node.id,
                    dimensions = dimensions,
                    "Populated virtual embedding column"
                );
            }
            None => {
                tracing::trace!(
                    node_id = %node.id,
                    "Node has no embedding in storage (embedding column will be NULL)"
                );
            }
        }
    } else {
        tracing::warn!(
            "embedding column requested but embedding_storage not configured in ExecutionContext"
        );
    }

    Ok(())
}
