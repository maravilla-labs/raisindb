//! Archetype inheritance resolution service
//!
//! Handles resolution of Archetype inheritance chains including:
//! - `extends` - single parent inheritance
//! - Field merging (parent first, child overrides)
//! - Layout inheritance
//! - Circular dependency detection

use raisin_error::{Error, Result};
use raisin_models::nodes::types::archetype::Archetype;
use raisin_models::nodes::types::element::field_types::{FieldSchema, FieldSchemaBase};
use raisin_models::nodes::types::element::fields::layout::LayoutNode;
use raisin_storage::{scope::BranchScope, ArchetypeRepository, Storage};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Maximum depth of inheritance chain to prevent stack overflow
const MAX_INHERITANCE_DEPTH: usize = 20;

/// Resolved Archetype with all inheritance applied
#[derive(Debug, Clone)]
pub struct ResolvedArchetype {
    /// The original Archetype
    pub archetype: Archetype,
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
pub struct ArchetypeResolver<S: Storage> {
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
}

impl<S: Storage> ArchetypeResolver<S> {
    pub fn new(storage: Arc<S>, tenant_id: String, repo_id: String, branch: String) -> Self {
        Self {
            storage,
            tenant_id,
            repo_id,
            branch,
        }
    }

    /// Resolve an Archetype with all inheritance applied
    pub async fn resolve(&self, archetype_name: &str) -> Result<ResolvedArchetype> {
        let mut visited = HashSet::new();
        let mut chain = Vec::new();
        self.resolve_recursive(archetype_name, &mut visited, &mut chain)
            .await
    }

    /// Recursive resolution with circular dependency detection
    fn resolve_recursive<'a>(
        &'a self,
        archetype_name: &'a str,
        visited: &'a mut HashSet<String>,
        chain: &'a mut Vec<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ResolvedArchetype>> + Send + 'a>>
    {
        Box::pin(async move {
            // Check for circular dependency
            if visited.contains(archetype_name) {
                return Err(Error::Validation(format!(
                    "Circular dependency detected in Archetype inheritance: {} -> {}",
                    chain.join(" -> "),
                    archetype_name
                )));
            }

            // Check max inheritance depth
            if chain.len() >= MAX_INHERITANCE_DEPTH {
                return Err(Error::Validation(format!(
                    "Maximum inheritance depth ({}) exceeded for Archetype. Chain: {}",
                    MAX_INHERITANCE_DEPTH,
                    chain.join(" -> ")
                )));
            }

            visited.insert(archetype_name.to_string());
            chain.push(archetype_name.to_string());

            // Fetch the Archetype
            let repo = self.storage.archetypes();
            let archetype = repo
                .get(
                    BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                    archetype_name,
                    None,
                )
                .await?
                .ok_or_else(|| {
                    Error::NotFound(format!("Archetype not found: {}", archetype_name))
                })?;

            let mut field_map: HashMap<String, FieldSchema> = HashMap::new();
            let mut resolved_layout: Option<Vec<LayoutNode>> = None;
            let mut resolved_strict = false;

            // 1. Resolve parent (extends) first
            if let Some(ref extends) = archetype.extends {
                let parent_resolved = self.resolve_recursive(extends, visited, chain).await?;

                // Inherit parent fields
                for field in parent_resolved.resolved_fields {
                    field_map.insert(field.base_name().clone(), field);
                }

                // Inherit parent layout
                resolved_layout = parent_resolved.resolved_layout;

                // Inherit strict mode (if parent is strict, child is too)
                resolved_strict = parent_resolved.resolved_strict;
            }

            // 2. Apply current archetype's own fields (override parent)
            if let Some(ref fields) = archetype.fields {
                for field in fields {
                    field_map.insert(field.base_name().clone(), field.clone());
                }
            }

            // 3. Override layout if specified
            if archetype.layout.is_some() {
                resolved_layout = archetype.layout.clone();
            }

            // 4. Override strict mode if specified
            if let Some(strict) = archetype.strict {
                resolved_strict = strict;
            }

            // Convert to sorted vec for consistency
            let mut resolved_fields: Vec<FieldSchema> = field_map.into_values().collect();
            resolved_fields.sort_by(|a, b| a.base_name().cmp(b.base_name()));

            Ok(ResolvedArchetype {
                archetype,
                resolved_fields,
                resolved_layout,
                inheritance_chain: chain.clone(),
                resolved_strict,
            })
        })
    }

    /// Check if an Archetype exists and is published
    pub async fn validate_exists_and_published(&self, archetype_name: &str) -> Result<()> {
        let repo = self.storage.archetypes();
        let archetype = repo
            .get(
                BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                archetype_name,
                None,
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("Archetype not found: {}", archetype_name)))?;

        if !archetype.publishable.unwrap_or(false) {
            return Err(Error::Validation(format!(
                "Archetype '{}' is not published",
                archetype_name
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

    async fn create_archetype(
        storage: &InMemoryStorage,
        name: &str,
        extends: Option<String>,
        fields: Option<Vec<FieldSchema>>,
        strict: Option<bool>,
    ) {
        let archetype = Archetype {
            id: name.to_string(),
            name: name.to_string(),
            extends,
            icon: None,
            title: None,
            description: None,
            base_node_type: None,
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
            .archetypes()
            .upsert(
                "test",
                "main",
                "main",
                archetype,
                CommitMetadata::system("create archetype"),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_simple_resolution_no_inheritance() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ArchetypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        create_archetype(
            &storage,
            "test:Simple",
            None,
            Some(vec![FieldSchema::TextField {
                base: make_field_base("title", false),
                config: None,
            }]),
            None,
        )
        .await;

        let resolved = resolver.resolve("test:Simple").await.unwrap();

        assert_eq!(resolved.archetype.name, "test:Simple");
        assert_eq!(resolved.resolved_fields.len(), 1);
        assert_eq!(resolved.resolved_fields[0].base_name(), "title");
        assert_eq!(resolved.inheritance_chain, vec!["test:Simple"]);
    }

    #[tokio::test]
    async fn test_single_level_inheritance() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ArchetypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        // Create base archetype
        create_archetype(
            &storage,
            "test:Base",
            None,
            Some(vec![
                FieldSchema::TextField {
                    base: make_field_base("seo_title", true),
                    config: None,
                },
                FieldSchema::TextField {
                    base: make_field_base("seo_description", false),
                    config: None,
                },
            ]),
            None,
        )
        .await;

        // Create child archetype
        create_archetype(
            &storage,
            "test:Hero",
            Some("test:Base".to_string()),
            Some(vec![FieldSchema::TextField {
                base: make_field_base("hero_title", true),
                config: None,
            }]),
            None,
        )
        .await;

        let resolved = resolver.resolve("test:Hero").await.unwrap();

        // Should have 3 fields: seo_title, seo_description (inherited) + hero_title (own)
        assert_eq!(resolved.resolved_fields.len(), 3);

        // Check inheritance chain
        assert_eq!(resolved.inheritance_chain, vec!["test:Hero", "test:Base"]);

        // Verify all field names are present
        let field_names: Vec<&str> = resolved
            .resolved_fields
            .iter()
            .map(|f| f.base_name().as_str())
            .collect();
        assert!(field_names.contains(&"seo_title"));
        assert!(field_names.contains(&"seo_description"));
        assert!(field_names.contains(&"hero_title"));
    }

    #[tokio::test]
    async fn test_field_override() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ArchetypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        // Create base with title field (not required)
        create_archetype(
            &storage,
            "test:Base",
            None,
            Some(vec![FieldSchema::TextField {
                base: make_field_base("title", false),
                config: None,
            }]),
            None,
        )
        .await;

        // Create child that overrides title to required
        create_archetype(
            &storage,
            "test:Child",
            Some("test:Base".to_string()),
            Some(vec![FieldSchema::TextField {
                base: make_field_base("title", true), // Override to required
                config: None,
            }]),
            None,
        )
        .await;

        let resolved = resolver.resolve("test:Child").await.unwrap();

        // Should have 1 field (child overrides parent)
        assert_eq!(resolved.resolved_fields.len(), 1);

        // Check that the field is now required (child's version)
        let title_field = &resolved.resolved_fields[0];
        match title_field {
            FieldSchema::TextField { base, .. } => {
                assert_eq!(base.required, Some(true));
            }
            _ => panic!("Expected TextField"),
        }
    }

    #[tokio::test]
    async fn test_circular_dependency_detection() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ArchetypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        // Create circular dependency: A extends B, B extends A
        create_archetype(&storage, "test:A", Some("test:B".to_string()), None, None).await;
        create_archetype(&storage, "test:B", Some("test:A".to_string()), None, None).await;

        let result = resolver.resolve("test:A").await;
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Circular dependency"));
    }

    #[tokio::test]
    async fn test_strict_mode_inheritance() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ArchetypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        // Create base with strict mode
        create_archetype(&storage, "test:StrictBase", None, None, Some(true)).await;

        // Create child without specifying strict
        create_archetype(
            &storage,
            "test:Child",
            Some("test:StrictBase".to_string()),
            None,
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
        let resolver = ArchetypeResolver::new(
            storage.clone(),
            "test".to_string(),
            "main".to_string(),
            "main".to_string(),
        );

        // Create 3-level inheritance: Entity -> Content -> Article
        create_archetype(
            &storage,
            "test:Entity",
            None,
            Some(vec![FieldSchema::TextField {
                base: make_field_base("id", true),
                config: None,
            }]),
            None,
        )
        .await;

        create_archetype(
            &storage,
            "test:Content",
            Some("test:Entity".to_string()),
            Some(vec![FieldSchema::TextField {
                base: make_field_base("title", true),
                config: None,
            }]),
            None,
        )
        .await;

        create_archetype(
            &storage,
            "test:Article",
            Some("test:Content".to_string()),
            Some(vec![FieldSchema::TextField {
                base: make_field_base("body", false),
                config: None,
            }]),
            None,
        )
        .await;

        let resolved = resolver.resolve("test:Article").await.unwrap();

        // Should have 3 fields: id (from Entity), title (from Content), body (own)
        assert_eq!(resolved.resolved_fields.len(), 3);

        // Check inheritance chain
        assert_eq!(
            resolved.inheritance_chain,
            vec!["test:Article", "test:Content", "test:Entity"]
        );
    }
}
