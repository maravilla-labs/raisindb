//! Indexing policy service
//!
//! Determines which properties should be indexed based on NodeType schema
//! rather than config files. This replaces the config-driven approach with
//! a schema-driven approach.

use crate::services::node_type_resolver::ResolvedNodeType;
use raisin_models::nodes::properties::schema::IndexType;

/// Helper service for determining indexing behavior based on schema
pub struct IndexingPolicy;

impl IndexingPolicy {
    /// Check if a node should be indexed at all for a specific index type
    ///
    /// Returns false if:
    /// - Node type has indexable: false
    /// - Node type doesn't include this index type in index_types
    ///
    /// # Example
    /// ```ignore
    /// if IndexingPolicy::should_index_node(&resolved, &IndexType::Vector) {
    ///     // Emit vector embedding job
    /// }
    /// ```
    pub fn should_index_node(resolved_type: &ResolvedNodeType, index_type: &IndexType) -> bool {
        // If node type is not indexable, skip all indexing
        if !resolved_type.resolved_indexable {
            return false;
        }

        // Check if this specific index type is enabled
        resolved_type.resolved_index_types.contains(index_type)
    }

    /// Get list of property names that should be indexed for a specific index type
    ///
    /// Only returns properties that:
    /// 1. Have a name defined
    /// 2. Have this index type in their `index` field
    ///
    /// # Example
    /// ```ignore
    /// let props = IndexingPolicy::properties_to_index(&resolved, &IndexType::Fulltext);
    /// // ["title", "content", "description"]
    /// ```
    pub fn properties_to_index(
        resolved_type: &ResolvedNodeType,
        index_type: &IndexType,
    ) -> Vec<String> {
        resolved_type
            .resolved_properties
            .iter()
            .filter_map(|prop| {
                // Check if property has a name
                let name = prop.name.as_ref()?;

                // Check if property includes this index type
                let indexes = prop.index.as_ref()?;
                if indexes.contains(index_type) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Check if a specific property should be indexed for a given index type
    ///
    /// This is a convenience method for checking a single property.
    ///
    /// # Example
    /// ```ignore
    /// if IndexingPolicy::should_index_property(&resolved, "content", &IndexType::Vector) {
    ///     // Include content in vector embedding
    /// }
    /// ```
    pub fn should_index_property(
        resolved_type: &ResolvedNodeType,
        property_name: &str,
        index_type: &IndexType,
    ) -> bool {
        resolved_type
            .resolved_properties
            .iter()
            .find(|prop| {
                prop.name
                    .as_ref()
                    .map(|n| n == property_name)
                    .unwrap_or(false)
            })
            .and_then(|prop| prop.index.as_ref())
            .map(|indexes| indexes.contains(index_type))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::schema::PropertyValueSchema;
    use raisin_models::nodes::types::NodeType;

    fn create_test_resolved_type(
        indexable: bool,
        index_types: Vec<IndexType>,
        properties: Vec<PropertyValueSchema>,
    ) -> ResolvedNodeType {
        ResolvedNodeType {
            node_type: minimal_node_type("test:Type"),
            resolved_properties: properties,
            resolved_allowed_children: vec![],
            resolved_indexable: indexable,
            resolved_index_types: index_types,
            inheritance_chain: vec!["test:Type".to_string()],
        }
    }

    #[test]
    fn test_should_index_node_when_indexable_true() {
        let resolved =
            create_test_resolved_type(true, vec![IndexType::Fulltext, IndexType::Vector], vec![]);

        assert!(IndexingPolicy::should_index_node(
            &resolved,
            &IndexType::Fulltext
        ));
        assert!(IndexingPolicy::should_index_node(
            &resolved,
            &IndexType::Vector
        ));
        assert!(!IndexingPolicy::should_index_node(
            &resolved,
            &IndexType::Property
        ));
    }

    #[test]
    fn test_should_index_node_when_indexable_false() {
        let resolved =
            create_test_resolved_type(false, vec![IndexType::Fulltext, IndexType::Vector], vec![]);

        assert!(!IndexingPolicy::should_index_node(
            &resolved,
            &IndexType::Fulltext
        ));
        assert!(!IndexingPolicy::should_index_node(
            &resolved,
            &IndexType::Vector
        ));
        assert!(!IndexingPolicy::should_index_node(
            &resolved,
            &IndexType::Property
        ));
    }

    #[test]
    fn test_properties_to_index() {
        let title = string_property_schema("title", vec![IndexType::Fulltext, IndexType::Vector]);

        let content = string_property_schema("content", vec![IndexType::Fulltext]);

        let file_type = string_property_schema("file_type", vec![IndexType::Property]);

        let resolved = create_test_resolved_type(
            true,
            vec![IndexType::Fulltext, IndexType::Vector, IndexType::Property],
            vec![title, content, file_type],
        );

        let fulltext_props = IndexingPolicy::properties_to_index(&resolved, &IndexType::Fulltext);
        assert_eq!(fulltext_props.len(), 2);
        assert!(fulltext_props.contains(&"title".to_string()));
        assert!(fulltext_props.contains(&"content".to_string()));

        let vector_props = IndexingPolicy::properties_to_index(&resolved, &IndexType::Vector);
        assert_eq!(vector_props.len(), 1);
        assert!(vector_props.contains(&"title".to_string()));

        let property_props = IndexingPolicy::properties_to_index(&resolved, &IndexType::Property);
        assert_eq!(property_props.len(), 1);
        assert!(property_props.contains(&"file_type".to_string()));
    }

    #[test]
    fn test_should_index_property() {
        let title = string_property_schema("title", vec![IndexType::Fulltext, IndexType::Vector]);

        let resolved = create_test_resolved_type(
            true,
            vec![IndexType::Fulltext, IndexType::Vector],
            vec![title],
        );

        assert!(IndexingPolicy::should_index_property(
            &resolved,
            "title",
            &IndexType::Fulltext
        ));
        assert!(IndexingPolicy::should_index_property(
            &resolved,
            "title",
            &IndexType::Vector
        ));
        assert!(!IndexingPolicy::should_index_property(
            &resolved,
            "title",
            &IndexType::Property
        ));
        assert!(!IndexingPolicy::should_index_property(
            &resolved,
            "unknown",
            &IndexType::Fulltext
        ));
    }

    fn string_property_schema(name: &str, index: Vec<IndexType>) -> PropertyValueSchema {
        PropertyValueSchema {
            name: Some(name.to_string()),
            property_type: raisin_models::nodes::properties::schema::PropertyType::String,
            required: None,
            unique: None,
            default: None,
            constraints: None,
            structure: None,
            items: None,
            value: None,
            meta: None,
            is_translatable: None,
            allow_additional_properties: None,
            index: Some(index),
        }
    }

    fn minimal_node_type(name: &str) -> NodeType {
        NodeType {
            id: Some(name.to_string()),
            strict: None,
            name: name.to_string(),
            extends: None,
            mixins: Vec::new(),
            overrides: None,
            description: None,
            icon: None,
            version: Some(1),
            properties: None,
            allowed_children: Vec::new(),
            required_nodes: Vec::new(),
            initial_structure: None,
            versionable: Some(true),
            publishable: Some(true),
            auditable: Some(false),
            indexable: Some(true),
            index_types: None,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            previous_version: None,
            compound_indexes: None,
            is_mixin: None,
        }
    }
}
