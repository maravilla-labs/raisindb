//! Row types and embedding cache for query execution.
//!
//! This module defines the core `Row` type produced by query execution,
//! the `RowStream` type alias for async streaming, and the internal
//! `CachedEmbedding` type used by the embedding function evaluator.

use indexmap::IndexMap;
use raisin_models::nodes::properties::PropertyValue;

use std::pin::Pin;
use std::time::{Duration, Instant};

use futures::stream::Stream;
use raisin_error::Error;

/// Cached embedding with TTL
#[derive(Clone)]
pub(crate) struct CachedEmbedding {
    pub(crate) vector: Vec<f32>,
    pub(crate) cached_at: Instant,
}

impl CachedEmbedding {
    pub(crate) fn new(vector: Vec<f32>) -> Self {
        Self {
            vector,
            cached_at: Instant::now(),
        }
    }

    pub(crate) fn is_expired(&self, ttl: Duration) -> bool {
        self.cached_at.elapsed() > ttl
    }
}

/// Execution error type
pub type ExecutionError = Error;

/// A row of data produced by query execution
///
/// Each row is a mapping from column name to PropertyValue.
/// This matches the Node properties structure for seamless integration.
#[derive(Debug, Clone)]
pub struct Row {
    pub columns: IndexMap<String, PropertyValue>,
}

impl Row {
    /// Create a new empty row
    pub fn new() -> Self {
        Self {
            columns: IndexMap::new(),
        }
    }

    /// Create a row from an IndexMap preserving insertion order
    pub fn from_map(columns: IndexMap<String, PropertyValue>) -> Self {
        Self { columns }
    }

    /// Get a column value
    pub fn get(&self, name: &str) -> Option<&PropertyValue> {
        self.columns.get(name)
    }

    /// Insert a column value
    pub fn insert(&mut self, name: String, value: PropertyValue) {
        self.columns.insert(name, value);
    }

    /// Check if a column exists
    pub fn contains(&self, name: &str) -> bool {
        self.columns.contains_key(name)
    }

    /// Get all column names
    pub fn column_names(&self) -> Vec<&str> {
        self.columns.keys().map(|s| s.as_str()).collect()
    }

    /// Get a column value by unqualified name
    ///
    /// This method searches for a column by trying:
    /// 1. Exact match on the unqualified name
    /// 2. Match on any qualified name ending with `.{name}`
    ///
    /// This is useful for functions like DESCENDANT_OF that need to access
    /// columns without knowing the table qualifier.
    pub fn get_by_unqualified(&self, name: &str) -> Option<&PropertyValue> {
        // Try exact match first
        if let Some(value) = self.columns.get(name) {
            return Some(value);
        }

        // Try finding qualified name ending with .{name}
        let suffix = format!(".{}", name);
        for (key, value) in &self.columns {
            if key.ends_with(&suffix) {
                return Some(value);
            }
        }

        None
    }
}

impl Default for Row {
    fn default() -> Self {
        Self::new()
    }
}

/// Stream of rows produced by query execution
pub type RowStream = Pin<Box<dyn Stream<Item = Result<Row, ExecutionError>> + Send>>;
