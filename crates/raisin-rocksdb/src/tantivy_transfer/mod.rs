//! Tantivy fulltext index transfer for cluster catch-up
//!
//! This module provides functionality to transfer Tantivy fulltext indexes
//! between nodes during cluster catch-up operations.

mod manager;
mod receiver;
#[cfg(test)]
mod tests;
mod types;

pub use manager::TantivyIndexManager;
pub use receiver::TantivyIndexReceiver;
pub use types::TantivyIndexMetadata;
