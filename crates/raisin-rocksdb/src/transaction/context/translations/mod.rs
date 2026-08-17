//! Translation operations module
//!
//! This module organizes translation operations into focused submodules:
//! - `write`: Translation write operations (store_translation)
//! - `read`: Translation read operations (get_translation, list_translations_for_node)

pub(super) mod blocks;
pub(super) mod read;
pub(super) mod write;

// Re-export all public functions for use by parent module
pub(super) use blocks::{
    get_block_translation, list_block_translations_for_node, store_block_translation,
};
pub(super) use read::{get_translation, list_translations_for_node};
pub(super) use write::store_translation;
