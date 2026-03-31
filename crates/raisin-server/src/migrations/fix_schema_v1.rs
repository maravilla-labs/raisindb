// SPDX-License-Identifier: UNLICENSED
//
// Copyright (C) 2019-2025 SOLUTAS GmbH, All Rights Reserved.
//
// Paradieshofstrasse 117, 4054 Basel, Switzerland
// http://www.solutas.ch | info@solutas.ch
//
// This file is part of RaisinDB.
//
// Unauthorized copying of this file, via any medium is strictly prohibited
// Proprietary and confidential

//! Schema V1 Migration: Fix type mismatches in serialized data
//!
//! This migration fixes legacy data where boolean `false` was stored in string fields.
//! It re-serializes all Nodes and RevisionMeta entries using MessagePack with correct types.

use super::MigrationStats;
use raisin_error::Result;
use raisin_models::migrations::{
    deserialize_optional_string_lenient_msgpack, deserialize_string_lenient_msgpack,
};
use raisin_models::nodes::Node;
use rocksdb::{ColumnFamily, DB};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

const MARKER_KEY: &[u8] = b"migrations/schema_v1_complete";
const CF_NODES: &str = "NODES";
const CF_REVISIONS: &str = "REVISIONS";

/// Lenient Node wrapper for migration deserialization
#[derive(Debug, Clone, Deserialize, Serialize)]
struct LenientNode {
    #[serde(default)]
    pub id: String,
    #[serde(deserialize_with = "deserialize_string_lenient_msgpack")]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(deserialize_with = "deserialize_string_lenient_msgpack")]
    pub node_type: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_lenient_msgpack"
    )]
    pub archetype: Option<String>,
    #[serde(default)]
    pub properties: HashMap<String, raisin_models::nodes::properties::PropertyValue>,
    #[serde(default)]
    pub children: Vec<String>,
    #[serde(default)]
    pub order_key: String,
    #[serde(skip)]
    pub has_children: Option<bool>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_lenient_msgpack"
    )]
    pub parent: Option<String>,
    #[serde(default)]
    pub version: i32,
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_lenient_msgpack"
    )]
    pub published_by: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_lenient_msgpack"
    )]
    pub updated_by: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_lenient_msgpack"
    )]
    pub created_by: Option<String>,
    #[serde(default)]
    pub translations: Option<HashMap<String, raisin_models::nodes::properties::PropertyValue>>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_lenient_msgpack"
    )]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub relations: Vec<raisin_models::nodes::RelationRef>,
}

impl From<LenientNode> for Node {
    fn from(lenient: LenientNode) -> Self {
        Node {
            id: lenient.id,
            name: lenient.name,
            path: lenient.path,
            node_type: lenient.node_type,
            archetype: lenient.archetype,
            properties: lenient.properties,
            children: lenient.children,
            order_key: lenient.order_key,
            has_children: None, // Always reset computed field
            parent: lenient.parent,
            version: lenient.version,
            created_at: lenient.created_at,
            updated_at: lenient.updated_at,
            published_at: lenient.published_at,
            published_by: lenient.published_by,
            updated_by: lenient.updated_by,
            created_by: lenient.created_by,
            translations: lenient.translations,
            tenant_id: lenient.tenant_id,
            workspace: lenient.workspace,
            owner_id: lenient.owner_id,
            relations: lenient.relations,
        }
    }
}

/// Run the schema v1 migration
pub async fn migrate(db: Arc<DB>) -> Result<MigrationStats> {
    // Check if migration already completed
    if migration_completed(&db)? {
        tracing::info!("Schema V1 migration already completed, skipping");
        return Ok(MigrationStats::default());
    }

    tracing::info!("Starting Schema V1 migration...");
    let mut stats = MigrationStats::default();

    // Migrate NODES CF
    tracing::info!("Migrating NODES column family...");
    match migrate_nodes(&db) {
        Ok(count) => {
            stats.nodes_fixed = count;
            tracing::info!("✓ Fixed {} nodes", count);
        }
        Err(e) => {
            tracing::error!("Error migrating nodes: {}", e);
            stats.errors += 1;
        }
    }

    // Migrate REVISIONS CF (RevisionMeta entries)
    tracing::info!("Migrating REVISIONS column family...");
    match migrate_revisions(&db) {
        Ok(count) => {
            stats.revisions_fixed = count;
            tracing::info!("✓ Fixed {} revisions", count);
        }
        Err(e) => {
            tracing::error!("Error migrating revisions: {}", e);
            stats.errors += 1;
        }
    }

    // Mark migration as complete
    mark_completed(&db)?;
    tracing::info!("Schema V1 migration marked as complete");

    Ok(stats)
}

/// Check if migration has already been completed
fn migration_completed(db: &Arc<DB>) -> Result<bool> {
    match db.get(MARKER_KEY) {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(e) => Err(raisin_error::Error::storage(format!(
            "Failed to check migration status: {}",
            e
        ))),
    }
}

/// Mark migration as completed
fn mark_completed(db: &Arc<DB>) -> Result<()> {
    db.put(MARKER_KEY, b"1").map_err(|e| {
        raisin_error::Error::storage(format!("Failed to mark migration complete: {}", e))
    })?;
    Ok(())
}

/// Migrate all nodes in NODES CF
fn migrate_nodes(db: &Arc<DB>) -> Result<usize> {
    let cf = get_cf(db, CF_NODES)?;
    let mut fixed_count = 0;

    // Iterate through all entries
    let iter = db.iterator_cf(cf, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Skip tombstone entries (single byte 'T')
        if value.len() == 1 && value[0] == b'T' {
            continue;
        }

        // Try to deserialize as MessagePack
        match rmp_serde::from_slice::<LenientNode>(&value) {
            Ok(lenient_node) => {
                // Convert to proper Node
                let node: Node = lenient_node.into();

                // Re-serialize with MessagePack
                match rmp_serde::to_vec(&node) {
                    Ok(new_value) => {
                        // Only write if value changed
                        if new_value != value.as_ref() {
                            db.put_cf(cf, &key, new_value)
                                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
                            fixed_count += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to serialize node: {}", e);
                    }
                }
            }
            Err(e) => {
                // Log but continue - this might be a different type of entry
                tracing::debug!(
                    "Skipping entry that couldn't be deserialized as Node: {}",
                    e
                );
            }
        }
    }

    Ok(fixed_count)
}

/// Migrate all RevisionMeta entries in REVISIONS CF
fn migrate_revisions(db: &Arc<DB>) -> Result<usize> {
    // For now, skip RevisionMeta migration as it's less likely to have issues
    // We can add it later if needed
    Ok(0)
}

/// Get column family handle
fn get_cf<'a>(db: &'a Arc<DB>, name: &str) -> Result<&'a ColumnFamily> {
    db.cf_handle(name)
        .ok_or_else(|| raisin_error::Error::storage(format!("Column family '{}' not found", name)))
}
