//! ElementType inheritance resolution service
//!
//! Handles resolution of ElementType inheritance chains including:
//! - `extends` - single parent inheritance
//! - Field merging (parent first, child overrides)
//! - Layout inheritance
//! - Circular dependency detection

use raisin_error::{Error, Result};
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::element::field_types::{FieldSchema, FieldSchemaBase};
use raisin_models::nodes::types::element::fields::layout::LayoutNode;
use raisin_storage::{scope::BranchScope, ElementTypeRepository, Storage};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Maximum depth of inheritance chain to prevent stack overflow
const MAX_INHERITANCE_DEPTH: usize = 20;

/// Resolved ElementType with all inheritance applied
#[derive(Debug, Clone)]
pub struct ResolvedElementType {
    /// The original ElementType
    pub element_type: ElementType,
    /// All fields including inherited ones (child overrides parent)
    pub resolved_fields: Vec<FieldSchema>,
    /// Merged layout (child overrides parent if specified)
    pub resolved_layout: Option<Vec<LayoutNode>>,
    /// Inheritance chain for debugging (leaf to root)
    pub inheritance_chain: Vec<String>,
    /// Whether strict mode is enabled (merged from inheritance)
    pub resolved_strict: bool,
}

#[derive(Clone)]
pub struct ElementTypeResolver<S: Storage> {
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
}

impl<S: Storage> ElementTypeResolver<S> {
    pub fn new(storage: Arc<S>, tenant_id: String, repo_id: String, branch: String) -> Self {
        Self {
            storage,
            tenant_id,
            repo_id,
            branch,
        }
    }

    /// Resolve an ElementType with all inheritance applied
    pub async fn resolve(&self, element_type_name: &str) -> Result<ResolvedElementType> {
        let mut visited = HashSet::new();
        let mut chain = Vec::new();
        self.resolve_recursive(element_type_name, &mut visited, &mut chain)
            .await
    }

    /// Recursive resolution with circular dependency detection
    fn resolve_recursive<'a>(
        &'a self,
        element_type_name: &'a str,
        visited: &'a mut HashSet<String>,
        chain: &'a mut Vec<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ResolvedElementType>> + Send + 'a>>
    {
        Box::pin(async move {
            // Check for circular dependency
            if visited.contains(element_type_name) {
                return Err(Error::Validation(format!(
                    "Circular dependency detected in ElementType inheritance: {} -> {}",
                    chain.join(" -> "),
                    element_type_name
                )));
            }

            // Check max inheritance depth
            if chain.len() >= MAX_INHERITANCE_DEPTH {
                return Err(Error::Validation(format!(
                    "Maximum inheritance depth ({}) exceeded for ElementType. Chain: {}",
                    MAX_INHERITANCE_DEPTH,
                    chain.join(" -> ")
                )));
            }

            visited.insert(element_type_name.to_string());
            chain.push(element_type_name.to_string());

            // Fetch the ElementType
            let repo = self.storage.element_types();
            let element_type = repo
                .get(
                    BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                    element_type_name,
                    None,
                )
                .await?
                .ok_or_else(|| {
                    Error::NotFound(format!("ElementType not found: {}", element_type_name))
                })?;

            let mut field_map: HashMap<String, FieldSchema> = HashMap::new();
            let mut resolved_layout: Option<Vec<LayoutNode>> = None;
            let mut resolved_strict = false;

            // 1. Resolve parent (extends) first
            if let Some(ref extends) = element_type.extends {
                let parent_resolved = self.resolve_recursive(extends, visited, chain).await?;

                // Inherit parent fields
                for field in parent_resolved.resolved_fields {
                    field_map.insert(field.base_name().clone(), field);
                }

                // Inherit parent layout
                resolved_layout = parent_resolved.resolved_layout;

                // Inherit strict mode
                resolved_strict = parent_resolved.resolved_strict;
            }

            // 2. Apply current element type's own fields (override parent)
            // Note: ElementType.fields is Vec<FieldSchema>, not Option
            for field in &element_type.fields {
                field_map.insert(field.base_name().clone(), field.clone());
            }

            // 3. Override layout if specified
            if element_type.layout.is_some() {
                resolved_layout = element_type.layout.clone();
            }

            // 4. Override strict mode if specified
            if let Some(strict) = element_type.strict {
                resolved_strict = strict;
            }

            // Convert to sorted vec for consistency
            let mut resolved_fields: Vec<FieldSchema> = field_map.into_values().collect();
            resolved_fields.sort_by(|a, b| a.base_name().cmp(b.base_name()));

            Ok(ResolvedElementType {
                element_type,
                resolved_fields,
                resolved_layout,
                inheritance_chain: chain.clone(),
                resolved_strict,
            })
        })
    }

    /// Check if an ElementType exists and is published
    pub async fn validate_exists_and_published(&self, element_type_name: &str) -> Result<()> {
        let repo = self.storage.element_types();
        let element_type = repo
            .get(
                BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                element_type_name,
                None,
            )
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!("ElementType not found: {}", element_type_name))
            })?;

        if !element_type.publishable.unwrap_or(false) {
            return Err(Error::Validation(format!(
                "ElementType '{}' is not published",
                element_type_name
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::types::element::field_types::FieldSchema;
    use raisin_models::nodes::types::element::fields::base_field::FieldTypeSchema;
    use raisin_storage::{CommitMetadata, Storage};
    use raisin_storage_memory::InMemoryStorage;

    fn make_field_base(name: &str, required: bool) -> FieldTypeSchema {
        FieldTypeSchema {
            name: name.to_string(),
            title: None,
            label: None,
            required: if required { Some(true) } else { None },
            description: None,
            help_text: None,
            default_value: None,
            validations: None,
            is_hidden: None,
            multiple: None,
            design_value: None,
            translatable: None,
        }
    }

    async fn create_element_type(
        storage: &InMemoryStorage,
        name: &str,
        extends: Option<String>,
        fields: Vec<FieldSchema>,
        strict: Option<bool>,
    ) {
        let element_type = ElementType {
            id: name.to_string(),
            name: name.to_string(),
            extends,
            title: None,
            icon: None,
            description: None,
            fields,
            initial_content: None,
            layout: None,
            meta: None,
            version: Some(1),
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            publishable: Some(true),
            strict,
            previous_version: None,
        };

        storage
            .element_types()
            .upsert(
                BranchScope::new("test", "main", "main"),
                element_type,
                CommitMetadata::system("create element type"),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_simple_resolution_no_inheritance() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ElementTypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        create_element_type(
            &storage,
            "test:Simple",
            None,
            vec![FieldSchema::TextField {
                base: make_field_base("content", false),
                config: None,
            }],
            None,
        )
        .await;

        let resolved = resolver.resolve("test:Simple").await.unwrap();

        assert_eq!(resolved.element_type.name, "test:Simple");
        assert_eq!(resolved.resolved_fields.len(), 1);
        assert_eq!(resolved.resolved_fields[0].base_name(), "content");
        assert_eq!(resolved.inheritance_chain, vec!["test:Simple"]);
    }

    #[tokio::test]
    async fn test_single_level_inheritance() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ElementTypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        // Create base element type
        create_element_type(
            &storage,
            "test:BaseBlock",
            None,
            vec![
                FieldSchema::TextField {
                    base: make_field_base("id", true),
                    config: None,
                },
                FieldSchema::TextField {
                    base: make_field_base("class", false),
                    config: None,
                },
            ],
            None,
        )
        .await;

        // Create child element type
        create_element_type(
            &storage,
            "test:HeroBlock",
            Some("test:BaseBlock".to_string()),
            vec![FieldSchema::TextField {
                base: make_field_base("headline", true),
                config: None,
            }],
            None,
        )
        .await;

        let resolved = resolver.resolve("test:HeroBlock").await.unwrap();

        // Should have 3 fields: id, class (inherited) + headline (own)
        assert_eq!(resolved.resolved_fields.len(), 3);

        // Check inheritance chain
        assert_eq!(
            resolved.inheritance_chain,
            vec!["test:HeroBlock", "test:BaseBlock"]
        );

        // Verify all field names are present
        let field_names: Vec<&str> = resolved
            .resolved_fields
            .iter()
            .map(|f| f.base_name().as_str())
            .collect();
        assert!(field_names.contains(&"id"));
        assert!(field_names.contains(&"class"));
        assert!(field_names.contains(&"headline"));
    }

    #[tokio::test]
    async fn test_field_override() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ElementTypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        // Create base with content field (not required)
        create_element_type(
            &storage,
            "test:Base",
            None,
            vec![FieldSchema::TextField {
                base: make_field_base("content", false),
                config: None,
            }],
            None,
        )
        .await;

        // Create child that overrides content to required
        create_element_type(
            &storage,
            "test:Child",
            Some("test:Base".to_string()),
            vec![FieldSchema::TextField {
                base: make_field_base("content", true), // Override to required
                config: None,
            }],
            None,
        )
        .await;

        let resolved = resolver.resolve("test:Child").await.unwrap();

        // Should have 1 field (child overrides parent)
        assert_eq!(resolved.resolved_fields.len(), 1);

        // Check that the field is now required (child's version)
        let content_field = &resolved.resolved_fields[0];
        match content_field {
            FieldSchema::TextField { base, .. } => {
                assert_eq!(base.required, Some(true));
            }
            _ => panic!("Expected TextField"),
        }
    }

    #[tokio::test]
    async fn test_circular_dependency_detection() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ElementTypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        // Create circular dependency: A extends B, B extends A
        create_element_type(&storage, "test:A", Some("test:B".to_string()), vec![], None).await;
        create_element_type(&storage, "test:B", Some("test:A".to_string()), vec![], None).await;

        let result = resolver.resolve("test:A").await;
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Circular dependency"));
    }

    #[tokio::test]
    async fn test_strict_mode_inheritance() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ElementTypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        // Create base with strict mode
        create_element_type(&storage, "test:StrictBase", None, vec![], Some(true)).await;

        // Create child without specifying strict
        create_element_type(
            &storage,
            "test:Child",
            Some("test:StrictBase".to_string()),
            vec![],
            None,
        )
        .await;

        let resolved = resolver.resolve("test:Child").await.unwrap();

        // Should inherit strict mode from parent
        assert!(resolved.resolved_strict);
    }

    #[tokio::test]
    async fn test_multi_level_inheritance() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ElementTypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        // Create 3-level inheritance: Base -> Container -> Card
        create_element_type(
            &storage,
            "test:Base",
            None,
            vec![FieldSchema::TextField {
                base: make_field_base("id", true),
                config: None,
            }],
            None,
        )
        .await;

        create_element_type(
            &storage,
            "test:Container",
            Some("test:Base".to_string()),
            vec![FieldSchema::TextField {
                base: make_field_base("class", false),
                config: None,
            }],
            None,
        )
        .await;

        create_element_type(
            &storage,
            "test:Card",
            Some("test:Container".to_string()),
            vec![FieldSchema::TextField {
                base: make_field_base("title", true),
                config: None,
            }],
            None,
        )
        .await;

        let resolved = resolver.resolve("test:Card").await.unwrap();

        // Should have 3 fields: id (from Base), class (from Container), title (own)
        assert_eq!(resolved.resolved_fields.len(), 3);

        // Check inheritance chain
        assert_eq!(
            resolved.inheritance_chain,
            vec!["test:Card", "test:Container", "test:Base"]
        );
    }
}
