//! Shared type aliases for in-memory index data structures.
//!
//! Extracts complex nested types used across multiple modules to reduce
//! `clippy::type_complexity` warnings and improve readability.

use raisin_context::Tag;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::RaisinReference;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock as TokioRwLock;

// ---------------------------------------------------------------------------
// Property index types
// ---------------------------------------------------------------------------

/// composite_key -> property_name -> value_json -> node_ids
pub type PropertyIndex =
    Arc<std::sync::RwLock<HashMap<String, HashMap<String, HashMap<String, HashSet<String>>>>>>;

// ---------------------------------------------------------------------------
// Reference index types
// ---------------------------------------------------------------------------

/// Forward reference index: composite_key -> node_id -> Vec<(property_path, RaisinReference)>
pub type ForwardReferenceIndex =
    Arc<std::sync::RwLock<HashMap<String, HashMap<String, Vec<(String, RaisinReference)>>>>>;

/// Reverse reference index: composite_key -> target_key -> Vec<(source_node_id, property_path)>
pub type ReverseReferenceIndex =
    Arc<std::sync::RwLock<HashMap<String, HashMap<String, Vec<(String, String)>>>>>;

// ---------------------------------------------------------------------------
// Tag index types
// ---------------------------------------------------------------------------

/// Tag storage: (tenant_id, repo_id, tag_name) -> Tag
pub type TagIndex = Arc<TokioRwLock<HashMap<(String, String, String), Tag>>>;

// ---------------------------------------------------------------------------
// Revision types
// ---------------------------------------------------------------------------

/// Translation snapshot storage: key -> Vec<(revision, overlay_bytes)>
pub type TranslationSnapshotIndex = Arc<TokioRwLock<HashMap<String, Vec<(HLC, Vec<u8>)>>>>;
