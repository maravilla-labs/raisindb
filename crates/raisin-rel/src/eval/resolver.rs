//! Graph relation resolver for RELATES expressions.
//!
//! This module defines the trait for resolving graph relationships
//! used in RELATES conditions within permission rules.

use crate::ast::RelDirection;
use crate::error::EvalError;

/// Trait for resolving graph relationships in permission conditions.
///
/// Implementations should use efficient graph traversal algorithms (BFS/DFS)
/// to determine if a path exists between nodes.
#[async_trait::async_trait]
pub trait RelationResolver: Send + Sync {
    /// Check if a path exists between source and target nodes.
    ///
    /// # Arguments
    /// * `source_id` - ID of the source node
    /// * `target_id` - ID of the target node
    /// * `relation_types` - Relationship types to follow
    /// * `min_depth` - Minimum path length (inclusive)
    /// * `max_depth` - Maximum path length (inclusive)
    /// * `direction` - Direction of traversal
    ///
    /// # Returns
    /// `Ok(true)` if a path exists within the depth range, `Ok(false)` otherwise.
    ///
    /// # Example
    /// ```ignore
    /// // Check if user "alice" has a path to document "doc1"
    /// // through 1-3 "owns" or "manages" relationships
    /// let has_access = resolver.has_path(
    ///     "alice",
    ///     "doc1",
    ///     &["owns".to_string(), "manages".to_string()],
    ///     1,
    ///     3,
    ///     RelDirection::Outgoing,
    /// ).await?;
    /// ```
    async fn has_path(
        &self,
        source_id: &str,
        target_id: &str,
        relation_types: &[String],
        min_depth: u32,
        max_depth: u32,
        direction: RelDirection,
    ) -> Result<bool, EvalError>;
}

/// No-op resolver that always returns false
///
/// This is the default implementation used when no resolver is provided.
/// It's useful for testing and for contexts where graph relationships
/// are not available.
pub struct NoOpResolver;

#[async_trait::async_trait]
impl RelationResolver for NoOpResolver {
    async fn has_path(
        &self,
        _source_id: &str,
        _target_id: &str,
        _relation_types: &[String],
        _min_depth: u32,
        _max_depth: u32,
        _direction: RelDirection,
    ) -> Result<bool, EvalError> {
        // Always return false - no relationships exist
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_noop_resolver() {
        let resolver = NoOpResolver;
        let result = resolver
            .has_path(
                "node1",
                "node2",
                &[String::from("FRIENDS_WITH")],
                1,
                1,
                RelDirection::Any,
            )
            .await
            .unwrap();

        assert_eq!(result, false);
    }
}
