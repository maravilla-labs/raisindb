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
use serde::{Deserialize, Serialize};

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
