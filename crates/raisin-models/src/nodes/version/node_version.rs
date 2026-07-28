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

// NodeVersion and NodeVersionSummary types
use crate::nodes::Node;
use chrono::Utc;
use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};

/// A single entry in a node's revision history (git-log style).
///
/// Returned by the node-history APIs (HTTP `…/nodes/{id}/history`, the WS
/// `node_history` request, `raisin.nodes.history()` in functions, and the JS
/// SDK `node.history()`). Lightweight by design: it carries enough to render a
/// history list (when / who / deleted) and the `revision` to fetch the full
/// snapshot on demand via the existing `at/{revision}` reads.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeRevisionEntry {
    /// The HLC revision at which this change was committed. Serializes as the
    /// canonical `"timestamp-counter"` string usable with `at_revision` reads.
    pub revision: HLC,
    /// When this revision was written (the snapshot's `updated_at`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<chrono::DateTime<Utc>>,
    /// Who authored this revision (the snapshot's `updated_by`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
    /// True when this revision is a tombstone (the node was deleted at this revision).
    pub deleted: bool,
    /// The commit message for the revision this change was part of.
    ///
    /// Joined from the revision's commit metadata. Without it a per-node history
    /// is a list of opaque timestamps: the message is written on every mutation
    /// (`"Updated node: <id>"`, `"MOVE … TO …"`, a caller-supplied one) but used
    /// to be readable only through the repo-wide revisions endpoint — which is
    /// why the admin console fetched EVERY revision and filtered client-side.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// True when the revision was system-initiated rather than user-driven.
    #[serde(default)]
    pub is_system: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeVersion {
    pub id: String,      // Unique identifier for the NodeVersion
    pub node_id: String, // Reference to the original Node
    pub version: i32,    // Version number
    pub node_data: Node, // The Node data at this version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at: Option<chrono::DateTime<Utc>>,
    pub updated_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeVersionSummary {
    pub id: String,      // Unique identifier for the NodeVersion
    pub node_id: String, // Reference to the original Node
    pub version: i32,    // Version number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at: Option<chrono::DateTime<Utc>>,
    pub updated_at: Option<chrono::DateTime<Utc>>,
}
