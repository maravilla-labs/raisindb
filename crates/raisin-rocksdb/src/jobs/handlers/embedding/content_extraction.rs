//! Content extraction for embedding generation.
//!
//! Extracts embeddable text content from nodes based on schema-driven indexing policies.
//! Provides utilities for converting property values to text and hashing.

use crate::RocksDBStorage;
use raisin_core::services::indexing_policy::IndexingPolicy;
use raisin_core::services::node_type_resolver::NodeTypeResolver;
use raisin_embeddings::config::TenantEmbeddingConfig;
use raisin_error::Result;
use raisin_models::nodes::properties::schema::IndexType;
use raisin_models::nodes::Node;
use raisin_storage::jobs::JobContext;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Extract embeddable content from node based on schema
///
/// This is the new schema-driven approach that replaces the config-driven approach.
/// It uses the NodeType schema to determine which properties should be embedded.
pub async fn extract_embeddable_content(
    node: &Node,
    config: &TenantEmbeddingConfig,
    storage: Arc<RocksDBStorage>,
    context: &JobContext,
) -> Result<String> {
    let mut parts = Vec::new();

    // 1. Include node name (from global config)
    if config.include_name {
        parts.push(node.name.clone());
    }

    // 2. Include node path (from global config)
    if config.include_path {
        parts.push(node.path.clone());
    }

    // 3. Resolve node type to get schema with inheritance
    let resolver = NodeTypeResolver::new(
        storage,
        context.tenant_id.clone(),
        context.repo_id.clone(),
        context.branch.clone(),
    );

    let resolved = resolver.resolve(&node.node_type).await?;

    // 4. Check if this node type should be indexed for vector embeddings
    if !IndexingPolicy::should_index_node(&resolved, &IndexType::Vector) {
        tracing::debug!(
            node_type = %node.node_type,
            "Node type is not configured for vector indexing, returning name+path only"
        );
        return Ok(parts.join("\n"));
    }

    // 5. Get properties that should be embedded according to schema
    let properties_to_embed = IndexingPolicy::properties_to_index(&resolved, &IndexType::Vector);

    tracing::debug!(
        node_type = %node.node_type,
        properties_to_embed = ?properties_to_embed,
        "Extracting content from properties marked for vector indexing"
    );

    // 6. Extract text from configured properties
    for prop_name in &properties_to_embed {
        if let Some(prop_value) = node.properties.get(prop_name) {
            if let Some(text) = property_value_to_text(prop_value) {
                parts.push(text);
            }
        }
    }

    // Join all parts with newlines
    Ok(parts.join("\n"))
}

/// Convert property value to text for embedding using iterative approach
///
/// This uses a work stack instead of recursion to handle deeply nested structures
/// without risk of stack overflow. It extracts all textual content from nested
/// Objects, Arrays, Composites, and Blocks.
///
/// # Performance
///
/// - O(n) where n is total number of values in the nested structure
/// - Uses stack-based iteration to avoid recursion overhead
/// - Handles arbitrary nesting depth without stack overflow
pub fn property_value_to_text(
    value: &raisin_models::nodes::properties::PropertyValue,
) -> Option<String> {
    use raisin_models::nodes::properties::PropertyValue;

    let mut result_parts = Vec::new();
    let mut work_stack = vec![value];

    // Process items iteratively using a stack
    while let Some(current) = work_stack.pop() {
        match current {
            // Primitive textual values - extract directly
            PropertyValue::String(s) => {
                if !s.is_empty() {
                    result_parts.push(s.clone());
                }
            }
            PropertyValue::Integer(i) => result_parts.push(i.to_string()),
            PropertyValue::Float(f) => result_parts.push(f.to_string()),
            PropertyValue::Boolean(b) => result_parts.push(b.to_string()),
            PropertyValue::Date(d) => result_parts.push(d.to_string()),
            PropertyValue::Url(u) => {
                if !u.url.is_empty() {
                    result_parts.push(u.url.clone());
                }
            }

            // Arrays - add all items to work stack for processing
            PropertyValue::Array(arr) => {
                // Process in reverse order to maintain original order when popping
                for item in arr.iter().rev() {
                    work_stack.push(item);
                }
            }

            // Objects - recursively extract all nested values
            PropertyValue::Object(obj) => {
                // Add all object values to work stack
                for value in obj.values() {
                    work_stack.push(value);
                }
            }

            // Composite - extract all content from all blocks
            PropertyValue::Composite(bc) => {
                for block in &bc.items {
                    // Add all content values from each block
                    for value in block.content.values() {
                        work_stack.push(value);
                    }
                }
            }

            // Block - extract all content values
            PropertyValue::Element(block) => {
                for value in block.content.values() {
                    work_stack.push(value);
                }
            }

            // Decimal - convert to string for text extraction
            PropertyValue::Decimal(d) => result_parts.push(d.to_string()),

            // Skip Null, Reference, Resource, Vector, and Geometry types
            // - Null has no content
            // - Reference/Resource are identifiers, not content
            // - Vector embeddings are numeric data, not embeddable text
            // - Geometry is spatial data, not embeddable text
            PropertyValue::Null
            | PropertyValue::Reference(_)
            | PropertyValue::Resource(_)
            | PropertyValue::Vector(_)
            | PropertyValue::Geometry(_) => {}
        }
    }

    // Return aggregated text or None if empty
    if result_parts.is_empty() {
        None
    } else {
        // Join with newlines for better semantic meaning in embeddings
        Some(result_parts.join("\n"))
    }
}

/// Hash text for detecting changes
pub fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}
