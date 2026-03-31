//! Reference resolution service
//!
//! Handles resolution of PropertyValue::Reference instances in nodes by fetching
//! the referenced nodes and replacing references with actual node data.
//!
//! The resolver:
//! 1. Extracts all unique references from a node's properties
//! 2. Fetches the referenced nodes in parallel
//! 3. Returns a resolved node with references replaced by full node objects

use raisin_error::Result;
use raisin_models::nodes::properties::{extract_references, PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Maximum depth for recursive reference resolution to prevent runaway queries.
const MAX_RESOLUTION_DEPTH: u32 = 10;

/// Boxed future type for recursive async property resolution
type ResolvePropertiesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<HashMap<String, PropertyValue>>> + Send + 'a>>;

/// Resolved node with all references replaced
#[derive(Debug, Clone)]
pub struct ResolvedNode {
    /// The original node
    pub node: Node,
    /// Map of reference IDs to resolved nodes
    pub resolved_references: HashMap<String, Node>,
}

#[derive(Clone)]
pub struct ReferenceResolver<S: Storage> {
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
}

impl<S: Storage> ReferenceResolver<S> {
    pub fn new(storage: Arc<S>, tenant_id: String, repo_id: String, branch: String) -> Self {
        Self {
            storage,
            tenant_id,
            repo_id,
            branch,
        }
    }

    /// Resolve all references in a node
    ///
    /// Returns a ResolvedNode containing the original node and a map of all
    /// resolved reference nodes.
    pub async fn resolve(&self, workspace: &str, node: &Node) -> Result<ResolvedNode> {
        // Extract all unique references from the node's properties
        let reference_ids = Self::extract_reference_ids(&node.properties);

        // Fetch all referenced nodes
        let mut resolved_references = HashMap::new();
        let repo = self.storage.nodes();

        let scope = StorageScope::new(&self.tenant_id, &self.repo_id, &self.branch, workspace);
        for ref_id in reference_ids {
            if let Some(referenced_node) = repo.get(scope, &ref_id, None).await? {
                resolved_references.insert(ref_id, referenced_node);
            }
        }

        Ok(ResolvedNode {
            node: node.clone(),
            resolved_references,
        })
    }

    /// Resolve references and return a new node with references replaced inline
    ///
    /// This replaces PropertyValue::Reference with PropertyValue::Object containing
    /// the full referenced node data.
    pub async fn resolve_inline(&self, workspace: &str, node: &Node) -> Result<Node> {
        let resolved = self.resolve(workspace, node).await?;

        let mut new_node = node.clone();
        new_node.properties =
            Self::replace_references_inline(&node.properties, &resolved.resolved_references);

        Ok(new_node)
    }

    /// Resolve references with depth control
    ///
    /// - depth=0: no resolution (return as-is)
    /// - depth=1: resolve immediate references (current behavior)
    /// - depth=N: resolve references, then resolve references within resolved nodes
    ///
    /// Max depth is capped at 10 to prevent runaway resolution.
    pub async fn resolve_inline_with_depth(
        &self,
        workspace: &str,
        node: &Node,
        max_depth: u32,
    ) -> Result<Node> {
        let depth = max_depth.min(MAX_RESOLUTION_DEPTH);
        if depth == 0 {
            return Ok(node.clone());
        }

        // Resolve level 1
        let mut resolved_node = self.resolve_inline(workspace, node).await?;

        // For depth > 1, walk the resolved properties for remaining references
        // and resolve again with decremented depth
        if depth > 1 {
            resolved_node.properties = self
                .resolve_properties_recursive(workspace, &resolved_node.properties, depth - 1)
                .await?;
        }

        Ok(resolved_node)
    }

    /// Resolve references in a properties map (for SQL RESOLVE function)
    ///
    /// Uses `extract_references` to find all references with their paths,
    /// fetches the referenced nodes, and replaces them inline.
    ///
    /// - depth=0: return as-is
    /// - depth=1: resolve immediate references
    /// - depth=N: recursively resolve references within resolved nodes
    ///
    /// Tracks visited node IDs to prevent infinite loops from circular references.
    pub async fn resolve_properties(
        &self,
        workspace: &str,
        properties: &HashMap<String, PropertyValue>,
        max_depth: u32,
    ) -> Result<HashMap<String, PropertyValue>> {
        let mut visited = std::collections::HashSet::new();
        self.resolve_properties_inner(
            workspace,
            properties,
            max_depth.min(MAX_RESOLUTION_DEPTH),
            &mut visited,
        )
        .await
    }

    /// Resolve a single `RaisinReference` and return the resolved node as JSON
    pub async fn resolve_single_reference(
        &self,
        workspace: &str,
        reference: &RaisinReference,
        max_depth: u32,
    ) -> Result<Option<serde_json::Value>> {
        let depth = max_depth.min(MAX_RESOLUTION_DEPTH);
        if depth == 0 {
            // Return the reference itself as JSON
            return Ok(Some(serde_json::to_value(reference).map_err(|e| {
                raisin_error::Error::Internal(format!("Failed to serialize reference: {}", e))
            })?));
        }

        let ref_workspace = if reference.workspace.is_empty() {
            workspace
        } else {
            &reference.workspace
        };
        let scope = StorageScope::new(&self.tenant_id, &self.repo_id, &self.branch, ref_workspace);
        let repo = self.storage.nodes();

        let Some(node) = repo.get(scope, &reference.id, None).await? else {
            return Ok(None);
        };

        // For depth > 1, resolve references within the fetched node
        let final_node = if depth > 1 {
            self.resolve_inline_with_depth(ref_workspace, &node, depth - 1)
                .await?
        } else {
            node
        };

        Ok(Some(node_to_json_value(&final_node)))
    }

    /// Internal: resolve references in properties with visited-set tracking
    ///
    /// Uses `Box::pin` for recursive async calls since the compiler cannot
    /// determine the size of the recursive future at compile time.
    fn resolve_properties_inner<'a>(
        &'a self,
        workspace: &'a str,
        properties: &'a HashMap<String, PropertyValue>,
        remaining_depth: u32,
        visited: &'a mut std::collections::HashSet<String>,
    ) -> ResolvePropertiesFuture<'a> {
        Box::pin(async move {
            if remaining_depth == 0 {
                return Ok(properties.clone());
            }

            let refs = extract_references(properties);
            if refs.is_empty() {
                return Ok(properties.clone());
            }

            // Fetch references, skipping already-visited IDs (circular reference guard)
            let mut resolved_nodes: HashMap<String, Node> = HashMap::new();
            let repo = self.storage.nodes();
            for (_, raisin_ref) in &refs {
                if resolved_nodes.contains_key(&raisin_ref.id) || visited.contains(&raisin_ref.id) {
                    continue;
                }
                visited.insert(raisin_ref.id.clone());

                let ref_workspace = if raisin_ref.workspace.is_empty() {
                    workspace
                } else {
                    &raisin_ref.workspace
                };
                let scope =
                    StorageScope::new(&self.tenant_id, &self.repo_id, &self.branch, ref_workspace);
                if let Some(node) = repo.get(scope, &raisin_ref.id, None).await? {
                    resolved_nodes.insert(raisin_ref.id.clone(), node);
                }
            }

            let result = Self::replace_references_inline(properties, &resolved_nodes);

            // For depth > 1, recurse to resolve nested references in the resolved nodes
            if remaining_depth > 1 {
                self.resolve_properties_inner(workspace, &result, remaining_depth - 1, visited)
                    .await
            } else {
                Ok(result)
            }
        })
    }

    /// Internal: recursively resolve remaining references in already-resolved properties
    async fn resolve_properties_recursive(
        &self,
        workspace: &str,
        properties: &HashMap<String, PropertyValue>,
        remaining_depth: u32,
    ) -> Result<HashMap<String, PropertyValue>> {
        if remaining_depth == 0 {
            return Ok(properties.clone());
        }

        let refs = extract_references(properties);
        if refs.is_empty() {
            return Ok(properties.clone());
        }

        // Fetch remaining references
        let mut resolved_nodes: HashMap<String, Node> = HashMap::new();
        let repo = self.storage.nodes();
        for (_, raisin_ref) in &refs {
            if resolved_nodes.contains_key(&raisin_ref.id) {
                continue;
            }
            let ref_workspace = if raisin_ref.workspace.is_empty() {
                workspace
            } else {
                &raisin_ref.workspace
            };
            let scope =
                StorageScope::new(&self.tenant_id, &self.repo_id, &self.branch, ref_workspace);
            if let Some(node) = repo.get(scope, &raisin_ref.id, None).await? {
                resolved_nodes.insert(raisin_ref.id.clone(), node);
            }
        }

        let result = Self::replace_references_inline(properties, &resolved_nodes);

        // Use Box::pin for recursive async call
        if remaining_depth > 1 {
            Box::pin(self.resolve_properties_recursive(workspace, &result, remaining_depth - 1))
                .await
        } else {
            Ok(result)
        }
    }

    /// Extract all unique reference IDs from a PropertyValue tree
    ///
    /// Returns just the IDs (not paths). For path-aware extraction, use
    /// `raisin_models::nodes::properties::extract_references` instead.
    fn extract_reference_ids(properties: &HashMap<String, PropertyValue>) -> Vec<String> {
        let mut references = Vec::new();

        fn extract_from_value(value: &PropertyValue, refs: &mut Vec<String>) {
            match value {
                PropertyValue::Reference(ref_val) => {
                    let id = &ref_val.id;
                    if !refs.contains(id) {
                        refs.push(id.clone());
                    }
                }
                PropertyValue::Array(items) => {
                    for item in items {
                        extract_from_value(item, refs);
                    }
                }
                PropertyValue::Object(obj) => {
                    for v in obj.values() {
                        extract_from_value(v, refs);
                    }
                }
                _ => {}
            }
        }

        for value in properties.values() {
            extract_from_value(value, &mut references);
        }

        references
    }

    /// Replace all references in a PropertyValue tree with full node objects
    fn replace_references_inline(
        properties: &HashMap<String, PropertyValue>,
        resolved: &HashMap<String, Node>,
    ) -> HashMap<String, PropertyValue> {
        fn replace_in_value(
            value: &PropertyValue,
            resolved: &HashMap<String, Node>,
        ) -> PropertyValue {
            match value {
                PropertyValue::Reference(ref_val) => {
                    let id = &ref_val.id;
                    if let Some(node) = resolved.get(id) {
                        // Convert node to PropertyValue::Object
                        let mut obj = HashMap::new();
                        obj.insert("id".to_string(), PropertyValue::String(node.id.clone()));
                        obj.insert("name".to_string(), PropertyValue::String(node.name.clone()));
                        obj.insert("path".to_string(), PropertyValue::String(node.path.clone()));
                        obj.insert(
                            "node_type".to_string(),
                            PropertyValue::String(node.node_type.clone()),
                        );

                        // Include properties from the referenced node
                        for (k, v) in &node.properties {
                            obj.insert(k.clone(), v.clone());
                        }

                        return PropertyValue::Object(obj);
                    }
                    // If reference not found, keep the reference as-is
                    value.clone()
                }
                PropertyValue::Array(items) => PropertyValue::Array(
                    items
                        .iter()
                        .map(|v| replace_in_value(v, resolved))
                        .collect(),
                ),
                PropertyValue::Object(obj) => PropertyValue::Object(
                    obj.iter()
                        .map(|(k, v)| (k.clone(), replace_in_value(v, resolved)))
                        .collect(),
                ),
                _ => value.clone(),
            }
        }

        properties
            .iter()
            .map(|(k, v)| (k.clone(), replace_in_value(v, resolved)))
            .collect()
    }
}

/// Convert a Node to a `serde_json::Value` for RESOLVE() SQL function output
///
/// Returns an object with: id, name, path, node_type, plus all properties flattened.
pub fn node_to_json_value(node: &Node) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), serde_json::Value::String(node.id.clone()));
    map.insert(
        "name".to_string(),
        serde_json::Value::String(node.name.clone()),
    );
    map.insert(
        "path".to_string(),
        serde_json::Value::String(node.path.clone()),
    );
    map.insert(
        "node_type".to_string(),
        serde_json::Value::String(node.node_type.clone()),
    );

    // Flatten properties into the object
    if let Ok(serde_json::Value::Object(props_map)) = serde_json::to_value(&node.properties) {
        for (k, v) in props_map {
            map.insert(k, v);
        }
    }

    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::RaisinReference;
    use raisin_storage::Storage;
    use raisin_storage_memory::InMemoryStorage;

    async fn create_test_node(
        storage: &InMemoryStorage,
        workspace: &str,
        id: &str,
        name: &str,
        path: &str,
        properties: HashMap<String, PropertyValue>,
    ) {
        let node = Node {
            id: id.to_string(),
            name: name.to_string(),
            path: path.to_string(),
            node_type: "test:Content".to_string(),
            archetype: None,
            properties,
            children: vec![],
            order_key: String::new(),
            has_children: None,
            parent: None,
            version: 1,
            created_at: Some(chrono::Utc::now()),
            updated_at: None,
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: None,
            tenant_id: None,
            workspace: Some(workspace.to_string()),
            owner_id: None,
            relations: Vec::new(),
        };

        let scope = StorageScope::new("default", "default", "main", workspace);
        storage
            .nodes()
            .create(scope, node, raisin_storage::CreateNodeOptions::default())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_simple_reference_resolution() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ReferenceResolver::new(
            storage.clone(),
            "default".to_string(),
            "default".to_string(),
            "main".to_string(),
        );

        // Create referenced node
        create_test_node(
            &storage,
            "test",
            "target-id",
            "Target Node",
            "/target",
            HashMap::new(),
        )
        .await;

        // Create node with reference
        let mut props = HashMap::new();
        props.insert(
            "author".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "target-id".to_string(),
                workspace: "test".to_string(),
                path: "/target".to_string(),
            }),
        );

        create_test_node(&storage, "test", "source-id", "Source", "/source", props).await;

        // Resolve references
        let source = storage
            .nodes()
            .get(
                StorageScope::new("default", "default", "main", "test"),
                "source-id",
                None,
            )
            .await
            .unwrap()
            .unwrap();
        let resolved = resolver.resolve("test", &source).await.unwrap();

        assert_eq!(resolved.resolved_references.len(), 1);
        assert!(resolved.resolved_references.contains_key("target-id"));
        assert_eq!(
            resolved.resolved_references.get("target-id").unwrap().name,
            "Target Node"
        );
    }

    #[tokio::test]
    async fn test_multiple_references() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ReferenceResolver::new(
            storage.clone(),
            "default".to_string(),
            "default".to_string(),
            "main".to_string(),
        );

        // Create multiple referenced nodes
        create_test_node(&storage, "test", "ref1", "Ref 1", "/ref1", HashMap::new()).await;
        create_test_node(&storage, "test", "ref2", "Ref 2", "/ref2", HashMap::new()).await;
        create_test_node(&storage, "test", "ref3", "Ref 3", "/ref3", HashMap::new()).await;

        // Create node with multiple references
        let mut props = HashMap::new();
        props.insert(
            "authors".to_string(),
            PropertyValue::Array(vec![
                PropertyValue::Reference(RaisinReference {
                    id: "ref1".to_string(),
                    workspace: "test".to_string(),
                    path: "/ref1".to_string(),
                }),
                PropertyValue::Reference(RaisinReference {
                    id: "ref2".to_string(),
                    workspace: "test".to_string(),
                    path: "/ref2".to_string(),
                }),
            ]),
        );
        props.insert(
            "editor".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "ref3".to_string(),
                workspace: "test".to_string(),
                path: "/ref3".to_string(),
            }),
        );

        create_test_node(&storage, "test", "source-id", "Source", "/source", props).await;

        // Resolve references
        let source = storage
            .nodes()
            .get(
                StorageScope::new("default", "default", "main", "test"),
                "source-id",
                None,
            )
            .await
            .unwrap()
            .unwrap();
        let resolved = resolver.resolve("test", &source).await.unwrap();

        assert_eq!(resolved.resolved_references.len(), 3);
        assert!(resolved.resolved_references.contains_key("ref1"));
        assert!(resolved.resolved_references.contains_key("ref2"));
        assert!(resolved.resolved_references.contains_key("ref3"));
    }

    #[tokio::test]
    async fn test_inline_resolution() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ReferenceResolver::new(
            storage.clone(),
            "default".to_string(),
            "default".to_string(),
            "main".to_string(),
        );

        // Create referenced node with properties
        let mut target_props = HashMap::new();
        target_props.insert(
            "bio".to_string(),
            PropertyValue::String("Author bio".to_string()),
        );

        create_test_node(
            &storage,
            "test",
            "author-id",
            "John Doe",
            "/authors/john",
            target_props,
        )
        .await;

        // Create node with reference
        let mut props = HashMap::new();
        props.insert(
            "author".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "author-id".to_string(),
                workspace: "test".to_string(),
                path: "/authors/john".to_string(),
            }),
        );

        create_test_node(&storage, "test", "article-id", "Article", "/article", props).await;

        // Resolve inline
        let source = storage
            .nodes()
            .get(
                StorageScope::new("default", "default", "main", "test"),
                "article-id",
                None,
            )
            .await
            .unwrap()
            .unwrap();
        let resolved_node = resolver.resolve_inline("test", &source).await.unwrap();

        // Check that the reference was replaced with an object
        if let Some(PropertyValue::Object(author)) = resolved_node.properties.get("author") {
            assert_eq!(
                author.get("name"),
                Some(&PropertyValue::String("John Doe".to_string()))
            );
            assert_eq!(
                author.get("bio"),
                Some(&PropertyValue::String("Author bio".to_string()))
            );
        } else {
            panic!("Expected author to be resolved to an Object");
        }
    }

    #[tokio::test]
    async fn test_missing_reference() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ReferenceResolver::new(
            storage.clone(),
            "default".to_string(),
            "default".to_string(),
            "main".to_string(),
        );

        // Create node with reference to non-existent node
        let mut props = HashMap::new();
        props.insert(
            "author".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "non-existent-id".to_string(),
                workspace: "test".to_string(),
                path: "/non-existent".to_string(),
            }),
        );

        create_test_node(&storage, "test", "source-id", "Source", "/source", props).await;

        // Resolve references - should succeed but with empty resolved map
        let source = storage
            .nodes()
            .get(
                StorageScope::new("default", "default", "main", "test"),
                "source-id",
                None,
            )
            .await
            .unwrap()
            .unwrap();
        let resolved = resolver.resolve("test", &source).await.unwrap();

        assert_eq!(resolved.resolved_references.len(), 0);
    }

    #[tokio::test]
    async fn test_nested_references_in_object() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ReferenceResolver::new(
            storage.clone(),
            "default".to_string(),
            "default".to_string(),
            "main".to_string(),
        );

        // Create referenced node
        create_test_node(&storage, "test", "ref1", "Ref", "/ref", HashMap::new()).await;

        // Create node with nested reference in object
        let mut inner_obj = HashMap::new();
        inner_obj.insert(
            "person".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "ref1".to_string(),
                workspace: "test".to_string(),
                path: "/ref".to_string(),
            }),
        );

        let mut props = HashMap::new();
        props.insert("metadata".to_string(), PropertyValue::Object(inner_obj));

        create_test_node(&storage, "test", "source-id", "Source", "/source", props).await;

        // Resolve references
        let source = storage
            .nodes()
            .get(
                StorageScope::new("default", "default", "main", "test"),
                "source-id",
                None,
            )
            .await
            .unwrap()
            .unwrap();
        let resolved = resolver.resolve("test", &source).await.unwrap();

        assert_eq!(resolved.resolved_references.len(), 1);
        assert!(resolved.resolved_references.contains_key("ref1"));
    }

    #[tokio::test]
    async fn test_resolve_with_depth_zero() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ReferenceResolver::new(
            storage.clone(),
            "default".to_string(),
            "default".to_string(),
            "main".to_string(),
        );

        create_test_node(
            &storage,
            "test",
            "target-id",
            "Target",
            "/target",
            HashMap::new(),
        )
        .await;

        let mut props = HashMap::new();
        props.insert(
            "author".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "target-id".to_string(),
                workspace: "test".to_string(),
                path: "/target".to_string(),
            }),
        );
        create_test_node(&storage, "test", "source-id", "Source", "/source", props).await;

        let source = storage
            .nodes()
            .get(
                StorageScope::new("default", "default", "main", "test"),
                "source-id",
                None,
            )
            .await
            .unwrap()
            .unwrap();

        // depth=0 should return node as-is (no resolution)
        let resolved = resolver
            .resolve_inline_with_depth("test", &source, 0)
            .await
            .unwrap();
        assert!(matches!(
            resolved.properties.get("author"),
            Some(PropertyValue::Reference(_))
        ));
    }

    #[tokio::test]
    async fn test_resolve_properties() {
        let storage = Arc::new(InMemoryStorage::default());
        let resolver = ReferenceResolver::new(
            storage.clone(),
            "default".to_string(),
            "default".to_string(),
            "main".to_string(),
        );

        let mut target_props = HashMap::new();
        target_props.insert(
            "bio".to_string(),
            PropertyValue::String("A bio".to_string()),
        );
        create_test_node(
            &storage,
            "test",
            "author-id",
            "Author",
            "/author",
            target_props,
        )
        .await;

        let mut props = HashMap::new();
        props.insert(
            "author".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "author-id".to_string(),
                workspace: "test".to_string(),
                path: "/author".to_string(),
            }),
        );
        props.insert(
            "title".to_string(),
            PropertyValue::String("Article".to_string()),
        );

        let resolved = resolver
            .resolve_properties("test", &props, 1)
            .await
            .unwrap();

        // Title should be unchanged
        assert_eq!(
            resolved.get("title"),
            Some(&PropertyValue::String("Article".to_string()))
        );

        // Author should be resolved to Object
        if let Some(PropertyValue::Object(author)) = resolved.get("author") {
            assert_eq!(
                author.get("name"),
                Some(&PropertyValue::String("Author".to_string()))
            );
            assert_eq!(
                author.get("bio"),
                Some(&PropertyValue::String("A bio".to_string()))
            );
        } else {
            panic!("Expected author to be resolved to an Object");
        }
    }

    #[tokio::test]
    async fn test_node_to_json_value() {
        let mut props = HashMap::new();
        props.insert(
            "bio".to_string(),
            PropertyValue::String("Test bio".to_string()),
        );

        let node = Node {
            id: "test-id".to_string(),
            name: "Test Node".to_string(),
            path: "/test".to_string(),
            node_type: "test:Content".to_string(),
            archetype: None,
            properties: props,
            children: vec![],
            order_key: String::new(),
            has_children: None,
            parent: None,
            version: 1,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: None,
            tenant_id: None,
            workspace: None,
            owner_id: None,
            relations: Vec::new(),
        };

        let json = node_to_json_value(&node);
        assert_eq!(json["id"], "test-id");
        assert_eq!(json["name"], "Test Node");
        assert_eq!(json["path"], "/test");
        assert_eq!(json["node_type"], "test:Content");
        assert_eq!(json["bio"], "Test bio");
    }
}
