// SPDX-License-Identifier: BSL-1.1

//! Payload types for schema-definition operations.
//!
//! Covers [`NodeType`], [`Archetype`], and [`ElementType`] management
//! (CRUD, publish/unpublish, validation).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// NodeType operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypeCreatePayload {
    pub name: String,
    pub node_type: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypeGetPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypeListPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_only: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypeUpdatePayload {
    pub name: String,
    pub node_type: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypeDeletePayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypePublishPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypeUnpublishPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypeValidatePayload {
    pub node: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypeGetResolvedPayload {
    pub name: String,
}

// ---------------------------------------------------------------------------
// Archetype operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeCreatePayload {
    pub name: String,
    pub archetype: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeGetPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeListPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_only: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeUpdatePayload {
    pub name: String,
    pub archetype: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeDeletePayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypePublishPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeUnpublishPayload {
    pub name: String,
}

// ---------------------------------------------------------------------------
// ElementType operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementTypeCreatePayload {
    pub name: String,
    pub element_type: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementTypeGetPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementTypeListPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_only: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementTypeUpdatePayload {
    pub name: String,
    pub element_type: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementTypeDeletePayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementTypePublishPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementTypeUnpublishPayload {
    pub name: String,
}
