//! Reference resolution for path-based references
//!
//! This module resolves path-based references (where `raisin:ref` starts with `/`)
//! to UUID-based references during INSERT/UPDATE operations. It also auto-populates
//! the `raisin:path` field from the resolved node.
//!
//! # Path-Based References
//!
//! When authoring content, users can specify references using paths instead of UUIDs:
//! ```json
//! {"raisin:ref": "/demonews/tags/rust", "raisin:workspace": "social"}
//! ```
//!
//! During resolution, this becomes:
//! ```json
//! {"raisin:ref": "abc-123-uuid", "raisin:workspace": "social", "raisin:path": "/demonews/tags/rust"}
//! ```
//!
//! # Resolution Rules
//!
//! 1. If `raisin:ref` starts with `/`, treat as path and resolve to UUID
//! 2. If `raisin:ref` is a UUID and `raisin:path` is missing, look up path from node
//! 3. Resolution fails if referenced node doesn't exist (requires correct INSERT order)

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use std::collections::HashMap;

use super::super::read::{get_node, get_node_by_path};
use crate::indexing::walk_properties_mut;
use crate::transaction::RocksDBTransaction;

/// A reference found in the property tree, with its dot path — the ONE path
/// format every reference-index writer, tombstoner and rebuild uses.
struct FoundReference {
    path: String,
    reference: RaisinReference,
    /// Reached only through a `Composite` block, which this resolver did not
    /// descend before the walk was shared (see `resolve_references`).
    inside_composite: bool,
}

/// Resolve all path-based references in properties to UUIDs
///
/// Finds every `RaisinReference` — in arrays, objects, element content and
/// composite blocks alike, through the shared property walker — and resolves
/// path-based references to UUIDs.
///
/// # Dangling path references inside composite blocks are logged, not rejected
///
/// Composite blocks used to be skipped here entirely, so a path reference in
/// one was stored verbatim and never checked. A dangling one now in such a
/// block would turn writes that always succeeded into failures, so for this
/// release it is logged and kept as written. Everywhere else a dangling path
/// reference fails the write, as it always did.
///
/// # Arguments
///
/// * `tx` - The transaction instance (for node lookups)
/// * `properties` - The properties map to resolve (modified in place)
/// * `source_workspace` - The workspace context (used if reference doesn't specify workspace)
///
/// # Returns
///
/// Ok(()) on success, Error if a referenced node doesn't exist
pub async fn resolve_references(
    tx: &RocksDBTransaction,
    properties: &mut HashMap<String, PropertyValue>,
    source_workspace: &str,
) -> Result<()> {
    // Phase 1: collect every reference with its path.
    let found = collect_references(properties);
    if found.is_empty() {
        return Ok(());
    }

    // Phase 2: resolve each one.
    let mut resolved: HashMap<String, RaisinReference> = HashMap::with_capacity(found.len());
    for found in found {
        let is_path_ref = found.reference.id.starts_with('/');
        match resolve_single_reference(tx, found.reference, source_workspace).await {
            Ok(reference) => {
                resolved.insert(found.path, reference);
            }
            Err(Error::Validation(msg)) if is_path_ref && found.inside_composite => {
                tracing::warn!(
                    property = %found.path,
                    "{msg}: dangling path reference inside a composite block kept as \
                     written (log-only this release)"
                );
            }
            // Name the property holding the dangling reference: on a page
            // with forty blocks "Referenced node not found" alone does not
            // say which image or link to fix.
            Err(Error::Validation(msg)) => {
                return Err(Error::Validation(format!(
                    "{msg} (at {})",
                    display_path(&found.path)
                )));
            }
            Err(other) => return Err(other),
        }
    }

    // Phase 3: write the resolved references back, addressed by the same path.
    walk_properties_mut(properties, |cursor, value| {
        if !matches!(value, PropertyValue::Reference(_)) {
            return false;
        }
        if let Some(reference) = resolved.remove(cursor.path) {
            *value = PropertyValue::Reference(reference);
        }
        true
    });

    Ok(())
}

/// Every reference in `properties`, through the shared walker.
fn collect_references(properties: &mut HashMap<String, PropertyValue>) -> Vec<FoundReference> {
    let mut found = Vec::new();
    walk_properties_mut(properties, |cursor, value| match value {
        PropertyValue::Reference(reference) => {
            found.push(FoundReference {
                path: cursor.path.to_string(),
                reference: reference.clone(),
                inside_composite: cursor.inside_composite,
            });
            true
        }
        _ => false,
    });
    found
}

/// `content.0.items.2.image` as `content[0].items[2].image`, the spelling
/// validation errors use.
fn display_path(path: &str) -> String {
    let mut out = String::new();
    for segment in path.split('.') {
        if !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()) {
            out.push_str(&format!("[{segment}]"));
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(segment);
        }
    }
    out
}

/// Resolve a single reference
async fn resolve_single_reference(
    tx: &RocksDBTransaction,
    mut reference: RaisinReference,
    source_workspace: &str,
) -> Result<RaisinReference> {
    // Check if raisin:ref starts with '/' (path-based reference)
    if reference.id.starts_with('/') {
        let path = reference.id.clone();
        let workspace = if reference.workspace.is_empty() {
            source_workspace
        } else {
            &reference.workspace
        };

        // Look up node by path to get UUID
        let node = get_node_by_path(tx, workspace, &path)
            .await?
            .ok_or_else(|| {
                Error::Validation(format!("Referenced node not found: {}:{}", workspace, path))
            })?;

        // Replace path with UUID and populate raisin:path
        reference.id = node.id;
        reference.path = path;
        if reference.workspace.is_empty() {
            reference.workspace = workspace.to_string();
        }

        tracing::debug!(
            "Resolved path-based reference: {} -> {} (path: {})",
            node.path,
            reference.id,
            reference.path
        );
    } else {
        // UUID-based reference - (re)populate raisin:path from the target's
        // CURRENT path. We refresh even when path is already set so the
        // forward-index value and client-facing raisin:path stay accurate
        // after the target has moved (the id-keyed reverse index is unaffected
        // either way). Resolving on re-save keeps decoration in sync.
        let workspace = if reference.workspace.is_empty() {
            source_workspace
        } else {
            &reference.workspace
        };

        if let Some(node) = get_node(tx, workspace, &reference.id).await? {
            reference.path = node.path;
            if reference.workspace.is_empty() {
                reference.workspace = workspace.to_string();
            }

            tracing::debug!(
                "Refreshed path for UUID reference: {} -> {}",
                reference.id,
                reference.path
            );
        }
        // If node not found, we don't fail - the reference might be to a node
        // that will be created later or exists in a different context
    }

    Ok(reference)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_path_reference() {
        assert!("/some/path".starts_with('/'));
        assert!(!"uuid-123".starts_with('/'));
    }

    #[test]
    fn test_reference_struct_defaults() {
        let reference = RaisinReference {
            id: "/some/path".to_string(),
            workspace: "social".to_string(),
            path: String::new(), // Empty by default
        };

        assert!(reference.id.starts_with('/'));
        assert!(reference.path.is_empty());
    }

    fn reference(id: &str) -> PropertyValue {
        PropertyValue::Reference(RaisinReference {
            id: id.to_string(),
            workspace: "social".to_string(),
            path: String::new(),
        })
    }

    #[test]
    fn test_collect_references_empty() {
        let mut properties = HashMap::new();
        assert!(collect_references(&mut properties).is_empty());
    }

    #[test]
    fn test_collect_references_flat_and_in_array() {
        let mut properties = HashMap::new();
        properties.insert("ref1".to_string(), reference("/path/to/node"));
        properties.insert(
            "name".to_string(),
            PropertyValue::String("test".to_string()),
        );
        properties.insert(
            "tags".to_string(),
            PropertyValue::Array(vec![reference("/tag1"), reference("/tag2")]),
        );

        let mut paths: Vec<String> = collect_references(&mut properties)
            .into_iter()
            .map(|f| f.path)
            .collect();
        paths.sort();
        assert_eq!(paths, ["ref1", "tags.0", "tags.1"]);
    }

    /// Composite blocks are reached now, and flagged as such.
    #[test]
    fn test_collect_references_in_composite_blocks() {
        use raisin_models::nodes::properties::value::{Composite, Element};
        let block = Element {
            uuid: "b1".to_string(),
            element_type: "x:Teaser".to_string(),
            content: HashMap::from([("link".to_string(), reference("/target"))]),
        };
        let mut properties = HashMap::new();
        properties.insert(
            "blocks".to_string(),
            PropertyValue::Composite(Composite {
                uuid: "c1".to_string(),
                items: vec![block],
            }),
        );
        properties.insert("hero".to_string(), reference("/hero"));

        let mut found = collect_references(&mut properties);
        found.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].path, "blocks.0.link");
        assert!(found[0].inside_composite);
        assert_eq!(found[1].path, "hero");
        assert!(!found[1].inside_composite);
    }

    #[test]
    fn test_display_path_names_the_nested_property() {
        assert_eq!(
            display_path("content.0.items.2.image"),
            "content[0].items[2].image"
        );
        assert_eq!(display_path("hero"), "hero");
    }
}
