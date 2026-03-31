# Reference Resolution

Automatically fetch and resolve node references in RaisinDB.

## Overview

When working with nodes that contain `PropertyValue::Reference` properties, you often need to fetch the actual referenced nodes to display their data. The `ReferenceResolver` service automates this process by:

1. Extracting all unique references from a node's properties
2. Fetching the referenced nodes from storage
3. Providing resolved data in convenient formats

This eliminates manual reference fetching and simplifies working with relational content structures.

## ReferenceResolver Service

The `ReferenceResolver` service provides two main resolution strategies:

- **`resolve()`**: Fetches referenced nodes and returns them in a map
- **`resolve_inline()`**: Replaces reference properties with full node objects

### Creating a Resolver

```rust
use raisin_core::ReferenceResolver;
use std::sync::Arc;

let storage = Arc::new(RocksStorage::open("./data")?);
let resolver = ReferenceResolver::new(
    storage,
    "my-tenant".to_string(),
    "my-repo".to_string(),
    "main".to_string(),
);
```

The resolver is scoped to a specific tenant, repository, and branch context.

## Basic Usage

### Simple Reference Resolution

```rust
use raisin_core::{NodeService, ReferenceResolver};
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use std::collections::HashMap;

// Create a node with a reference
let mut article = Node {
    name: "my-article".to_string(),
    node_type: "BlogPost".to_string(),
    properties: {
        let mut props = HashMap::new();
        props.insert("author".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "user-123".to_string(),
                workspace: "content".to_string(),
                path: "/users/john-doe".to_string(),
            }));
        props.insert("title".to_string(),
            PropertyValue::String("Getting Started with RaisinDB".to_string()));
        props
    },
    ..Default::default()
};

// Fetch the node
let node = service.get("content", "article-1").await?.unwrap();

// Resolve all references
let resolved = resolver.resolve("content", &node).await?;

// Access the original node
println!("Article: {}", resolved.node.name);

// Access resolved references
if let Some(author_node) = resolved.resolved_references.get("user-123") {
    println!("Author: {}", author_node.name);

    // Access author properties
    if let Some(PropertyValue::String(bio)) = author_node.properties.get("bio") {
        println!("Bio: {}", bio);
    }
}
```

### Return Type: ResolvedNode

```rust
pub struct ResolvedNode {
    /// The original node (unchanged)
    pub node: Node,
    /// Map of reference ID → resolved node
    pub resolved_references: HashMap<String, Node>,
}
```

## Inline Resolution

The `resolve_inline()` method replaces `PropertyValue::Reference` instances with `PropertyValue::Object` containing the full node data:

```rust
// Original node with reference
let node = service.get("content", "article-1").await?.unwrap();

// Before resolution:
// properties["author"] = Reference({ id: "user-123", ... })

// Resolve inline
let resolved_node = resolver.resolve_inline("content", &node).await?;

// After resolution:
// properties["author"] = Object({
//     "id": "user-123",
//     "name": "John Doe",
//     "path": "/users/john-doe",
//     "node_type": "User",
//     "bio": "Software engineer...",
//     ... all properties from the referenced node
// })

// Access resolved data
if let Some(PropertyValue::Object(author)) = resolved_node.properties.get("author") {
    if let Some(PropertyValue::String(name)) = author.get("name") {
        println!("Author: {}", name);
    }
    if let Some(PropertyValue::String(bio)) = author.get("bio") {
        println!("Bio: {}", bio);
    }
}
```

### When to Use Inline Resolution

**Use `resolve_inline()` when:**
- Serializing nodes to JSON for API responses
- Embedding full reference data in templates
- Creating self-contained data structures
- Working with GraphQL resolvers

**Use `resolve()` when:**
- You need the original node structure
- Processing references programmatically
- Building custom resolution logic
- Performance-sensitive operations (no cloning)

## Handling Missing References

The resolver gracefully handles references to non-existent nodes:

```rust
let node = Node {
    name: "article".to_string(),
    properties: {
        let mut props = HashMap::new();
        props.insert("author".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "deleted-user".to_string(),  // This user no longer exists
                workspace: "content".to_string(),
                path: "/users/deleted".to_string(),
            }));
        props
    },
    ..Default::default()
};

// Resolve (no error, just empty map for missing references)
let resolved = resolver.resolve("content", &node).await?;

// Check if reference was found
if !resolved.resolved_references.contains_key("deleted-user") {
    println!("Author reference could not be resolved");
}

// With inline resolution, missing references remain as Reference values
let inline = resolver.resolve_inline("content", &node).await?;

// Still a Reference (not converted to Object)
assert!(matches!(
    inline.properties.get("author"),
    Some(PropertyValue::Reference(_))
));
```

## Multiple References

### References in Arrays

```rust
// Node with multiple author references
let mut props = HashMap::new();
props.insert("authors".to_string(),
    PropertyValue::Array(vec![
        PropertyValue::Reference(RaisinReference {
            id: "user-123".to_string(),
            workspace: "content".to_string(),
            path: "/users/alice".to_string(),
        }),
        PropertyValue::Reference(RaisinReference {
            id: "user-456".to_string(),
            workspace: "content".to_string(),
            path: "/users/bob".to_string(),
        }),
    ]));

let node = Node {
    name: "article".to_string(),
    properties: props,
    ..Default::default()
};

// Resolve all references
let resolved = resolver.resolve("content", &node).await?;

// Both authors are in the resolved map
assert_eq!(resolved.resolved_references.len(), 2);
assert!(resolved.resolved_references.contains_key("user-123"));
assert!(resolved.resolved_references.contains_key("user-456"));
```

### Nested References in Objects

```rust
// Node with nested reference structure
let mut metadata = HashMap::new();
metadata.insert("reviewer".to_string(),
    PropertyValue::Reference(RaisinReference {
        id: "user-789".to_string(),
        workspace: "content".to_string(),
        path: "/users/reviewer".to_string(),
    }));

let mut props = HashMap::new();
props.insert("metadata".to_string(), PropertyValue::Object(metadata));

let node = Node {
    name: "document".to_string(),
    properties: props,
    ..Default::default()
};

// Resolver finds references anywhere in the property tree
let resolved = resolver.resolve("content", &node).await?;
assert!(resolved.resolved_references.contains_key("user-789"));
```

## Performance Considerations

### Deduplication

The resolver automatically deduplicates references:

```rust
// Same reference appears multiple times
let mut props = HashMap::new();
props.insert("author".to_string(),
    PropertyValue::Reference(RaisinReference {
        id: "user-123".to_string(),
        workspace: "content".to_string(),
        path: "/users/john".to_string(),
    }));
props.insert("reviewer".to_string(),
    PropertyValue::Reference(RaisinReference {
        id: "user-123".to_string(),  // Same user
        workspace: "content".to_string(),
        path: "/users/john".to_string(),
    }));

let node = Node {
    name: "doc".to_string(),
    properties: props,
    ..Default::default()
};

// Only fetches user-123 once
let resolved = resolver.resolve("content", &node).await?;
assert_eq!(resolved.resolved_references.len(), 1);
```

### Batch Resolution

When resolving multiple nodes, create a single resolver and reuse it:

```rust
let resolver = ReferenceResolver::new(
    storage.clone(), tenant_id.clone(), repo_id.clone(), branch.clone(),
);

// Reuse resolver for multiple nodes
for node in nodes {
    let resolved = resolver.resolve("content", &node).await?;
    // Process resolved node...
}
```

### Shallow vs Deep Resolution

The resolver performs **shallow resolution** - it resolves references in the source node but does not recursively resolve references in the referenced nodes:

```rust
// article references author
// author references organization
// => Only article→author is resolved, not author→organization

let article_resolved = resolver.resolve("content", &article).await?;

// To resolve author's references too:
if let Some(author) = article_resolved.resolved_references.get("author-id") {
    let author_resolved = resolver.resolve("content", author).await?;
    // Now author→organization is resolved
}
```

## Use Cases

### Blog Post with Author

```rust
use raisin_core::{NodeService, ReferenceResolver};

async fn get_blog_post_with_author(
    service: &NodeService<S>,
    resolver: &ReferenceResolver<S>,
    workspace: &str,
    post_id: &str,
) -> Result<serde_json::Value> {
    // Fetch the blog post
    let post = service.get(workspace, post_id).await?
        .ok_or(Error::NotFound("Post not found".into()))?;

    // Resolve references inline for easy serialization
    let resolved_post = resolver.resolve_inline(workspace, &post).await?;

    // Serialize to JSON (author is embedded)
    Ok(serde_json::to_value(resolved_post)?)
}
```

### Product Catalog with Categories

```rust
async fn get_product_with_category(
    service: &NodeService<S>,
    resolver: &ReferenceResolver<S>,
    workspace: &str,
    product_path: &str,
) -> Result<ProductView> {
    let product = service.get_by_path(workspace, product_path).await?
        .ok_or(Error::NotFound("Product not found".into()))?;

    let resolved = resolver.resolve(workspace, &product).await?;

    // Extract category information
    let category_info = if let Some(PropertyValue::Reference(cat_ref)) =
        product.properties.get("category")
    {
        resolved.resolved_references.get(&cat_ref.id)
            .map(|cat| CategoryInfo {
                id: cat.id.clone(),
                name: cat.name.clone(),
                // Extract more fields...
            })
    } else {
        None
    };

    Ok(ProductView {
        id: product.id,
        name: product.name,
        category: category_info,
        // ... other fields
    })
}
```

### Hierarchical Menu with Links

```rust
async fn resolve_menu_items(
    resolver: &ReferenceResolver<S>,
    workspace: &str,
    menu_node: &Node,
) -> Result<Vec<MenuItem>> {
    let resolved = resolver.resolve(workspace, menu_node).await?;

    let mut items = Vec::new();

    if let Some(PropertyValue::Array(menu_items)) = menu_node.properties.get("items") {
        for item in menu_items {
            if let PropertyValue::Reference(link_ref) = item {
                if let Some(linked_page) = resolved.resolved_references.get(&link_ref.id) {
                    items.push(MenuItem {
                        title: extract_string(&linked_page.properties, "title")
                            .unwrap_or_else(|| linked_page.name.clone()),
                        url: linked_page.path.clone(),
                        // ... more fields
                    });
                }
            }
        }
    }

    Ok(items)
}
```

### API Response with Embedded References

```rust
use serde::{Serialize, Deserialize};

#[derive(Serialize)]
struct ArticleResponse {
    id: String,
    title: String,
    content: String,
    author: Option<UserInfo>,
    tags: Vec<TagInfo>,
}

async fn get_article_response(
    service: &NodeService<S>,
    resolver: &ReferenceResolver<S>,
    article_id: &str,
) -> Result<ArticleResponse> {
    let article = service.get("content", article_id).await?
        .ok_or(Error::NotFound("Article not found".into()))?;

    let resolved = resolver.resolve("content", &article).await?;

    // Extract title and content
    let title = extract_string(&article.properties, "title")
        .unwrap_or_default();
    let content = extract_string(&article.properties, "content")
        .unwrap_or_default();

    // Resolve author
    let author = if let Some(PropertyValue::Reference(author_ref)) =
        article.properties.get("author")
    {
        resolved.resolved_references.get(&author_ref.id)
            .map(|user| UserInfo {
                id: user.id.clone(),
                name: user.name.clone(),
                bio: extract_string(&user.properties, "bio"),
            })
    } else {
        None
    };

    // Resolve tags array
    let tags = if let Some(PropertyValue::Array(tag_refs)) =
        article.properties.get("tags")
    {
        tag_refs.iter()
            .filter_map(|tag_ref| {
                if let PropertyValue::Reference(r) = tag_ref {
                    resolved.resolved_references.get(&r.id).map(|tag| TagInfo {
                        id: tag.id.clone(),
                        name: tag.name.clone(),
                    })
                } else {
                    None
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(ArticleResponse {
        id: article.id,
        title,
        content,
        author,
        tags,
    })
}
```

## Best Practices

### 1. Reuse Resolver Instances

```rust
// ✅ Good - create once, use many times
let resolver = ReferenceResolver::new(
    storage, tenant_id.clone(), repo_id.clone(), branch.clone(),
);
for node in nodes {
    let resolved = resolver.resolve("content", &node).await?;
}

// ❌ Bad - creates new resolver every time
for node in nodes {
    let resolver = ReferenceResolver::new(
    storage.clone(), tenant_id.clone(), repo_id.clone(), branch.clone(),
);
    let resolved = resolver.resolve("content", &node).await?;
}
```

### 2. Check for Missing References

```rust
let resolved = resolver.resolve("content", &node).await?;

// Extract reference ID
if let Some(PropertyValue::Reference(ref_val)) = node.properties.get("author") {
    // Check if it was resolved
    if let Some(author) = resolved.resolved_references.get(&ref_val.id) {
        // Use author
    } else {
        // Handle missing reference (deleted user, permission issue, etc.)
        log::warn!("Could not resolve author reference: {}", ref_val.id);
    }
}
```

### 3. Choose the Right Resolution Method

```rust
// For API responses - use inline
let api_response = resolver.resolve_inline("content", &node).await?;
return Json(api_response);

// For processing - use resolve
let resolved = resolver.resolve("content", &node).await?;
for (ref_id, ref_node) in resolved.resolved_references {
    process_reference(&ref_node)?;
}
```

### 4. Combine with Reference Index

```rust
use raisin_storage::Storage;

// Find all articles by this author
let ref_index = storage.reference_index();
let article_ids = ref_index
    .find_nodes_referencing("content", "author-123", true) // true = published only
    .await?;

// Fetch and resolve each article
let mut articles = Vec::new();
for article_id in article_ids {
    if let Some(article) = service.get("content", &article_id).await? {
        let resolved = resolver.resolve("content", &article).await?;
        articles.push(resolved);
    }
}
```

## Next Steps

- [Property Type Reference](./property-reference.md) - Complete reference for property types including Reference
- [Core Services API](../api/core-services.md) - API documentation for ReferenceResolver
- [Storage Traits](../api/storage-traits.md) - ReferenceIndexRepository trait documentation
