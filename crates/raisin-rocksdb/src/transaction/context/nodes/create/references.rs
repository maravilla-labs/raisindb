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
use crate::transaction::RocksDBTransaction;

/// Location of a reference within the property tree
#[derive(Debug)]
struct RefLocation {
    /// Path of keys to reach this reference (e.g., ["tags", "0"] for tags[0])
    path: Vec<PathSegment>,
    /// The reference to resolve
    reference: RaisinReference,
}

/// A segment in the path to a value
#[derive(Debug, Clone)]
enum PathSegment {
    /// Key in an object/map
    Key(String),
    /// Index in an array
    Index(usize),
}

/// Resolve all path-based references in properties to UUIDs
///
/// This function uses an iterative approach to traverse the properties map,
/// finds all `RaisinReference` values, and resolves path-based references to UUIDs.
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
    // Phase 1: Collect all references and their locations (iterative traversal)
    let ref_locations = collect_references(properties);

    if ref_locations.is_empty() {
        return Ok(());
    }

    // Phase 2: Resolve each reference
    let mut resolved_refs = Vec::with_capacity(ref_locations.len());
    for loc in ref_locations {
        let resolved = resolve_single_reference(tx, loc.reference, source_workspace).await?;
        resolved_refs.push((loc.path, resolved));
    }

    // Phase 3: Apply resolved references back to properties
    for (path, resolved_ref) in resolved_refs {
        apply_resolved_reference(properties, &path, resolved_ref);
    }

    Ok(())
}

/// Collect all references from properties using iterative traversal
fn collect_references(properties: &HashMap<String, PropertyValue>) -> Vec<RefLocation> {
    let mut refs = Vec::new();

    // Stack of (current_path, value_to_process)
    // We clone values for inspection but track paths for later mutation
    let mut stack: Vec<(Vec<PathSegment>, &PropertyValue)> = Vec::new();

    // Initialize stack with top-level properties
    for (key, value) in properties {
        stack.push((vec![PathSegment::Key(key.clone())], value));
    }

    // Iterative depth-first traversal
    while let Some((current_path, value)) = stack.pop() {
        match value {
            PropertyValue::Reference(reference) => {
                refs.push(RefLocation {
                    path: current_path,
                    reference: reference.clone(),
                });
            }
            PropertyValue::Array(items) => {
                // Add array items to stack in reverse order (so we process in order)
                for (idx, item) in items.iter().enumerate().rev() {
                    let mut item_path = current_path.clone();
                    item_path.push(PathSegment::Index(idx));
                    stack.push((item_path, item));
                }
            }
            PropertyValue::Object(obj) => {
                // Add object entries to stack
                for (key, val) in obj {
                    let mut obj_path = current_path.clone();
                    obj_path.push(PathSegment::Key(key.clone()));
                    stack.push((obj_path, val));
                }
            }
            // Other types don't contain references
            _ => {}
        }
    }

    refs
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
    } else if reference.path.is_empty() {
        // UUID-based reference without path - look up to populate path
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
                "Populated path for UUID reference: {} -> {}",
                reference.id,
                reference.path
            );
        }
        // If node not found, we don't fail - the reference might be to a node
        // that will be created later or exists in a different context
    }

    Ok(reference)
}

/// Apply a resolved reference back to the properties at the given path
fn apply_resolved_reference(
    properties: &mut HashMap<String, PropertyValue>,
    path: &[PathSegment],
    resolved: RaisinReference,
) {
    if path.is_empty() {
        return;
    }

    // Navigate to the parent and update the target
    let mut current: &mut PropertyValue = match &path[0] {
        PathSegment::Key(key) => {
            if let Some(val) = properties.get_mut(key) {
                val
            } else {
                return;
            }
        }
        PathSegment::Index(_) => return, // Top-level can't be an index
    };

    // Navigate through intermediate path segments
    for segment in &path[1..] {
        current = match (current, segment) {
            (PropertyValue::Object(obj), PathSegment::Key(key)) => {
                if let Some(val) = obj.get_mut(key) {
                    val
                } else {
                    return;
                }
            }
            (PropertyValue::Array(arr), PathSegment::Index(idx)) => {
                if let Some(val) = arr.get_mut(*idx) {
                    val
                } else {
                    return;
                }
            }
            _ => return, // Path mismatch
        };
    }

    // Update the reference at the final location
    if let PropertyValue::Reference(ref_val) = current {
        *current = PropertyValue::Reference(resolved);
    }
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

    #[test]
    fn test_collect_references_empty() {
        let properties = HashMap::new();
        let refs = collect_references(&properties);
        assert!(refs.is_empty());
    }

    #[test]
    fn test_collect_references_flat() {
        let mut properties = HashMap::new();
        properties.insert(
            "ref1".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "/path/to/node".to_string(),
                workspace: "social".to_string(),
                path: String::new(),
            }),
        );
        properties.insert(
            "name".to_string(),
            PropertyValue::String("test".to_string()),
        );

        let refs = collect_references(&properties);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].reference.id, "/path/to/node");
    }

    #[test]
    fn test_collect_references_in_array() {
        let mut properties = HashMap::new();
        properties.insert(
            "tags".to_string(),
            PropertyValue::Array(vec![
                PropertyValue::Reference(RaisinReference {
                    id: "/tag1".to_string(),
                    workspace: "social".to_string(),
                    path: String::new(),
                }),
                PropertyValue::Reference(RaisinReference {
                    id: "/tag2".to_string(),
                    workspace: "social".to_string(),
                    path: String::new(),
                }),
            ]),
        );

        let refs = collect_references(&properties);
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn test_apply_resolved_reference() {
        let mut properties = HashMap::new();
        properties.insert(
            "ref1".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "/path/to/node".to_string(),
                workspace: "social".to_string(),
                path: String::new(),
            }),
        );

        let path = vec![PathSegment::Key("ref1".to_string())];
        let resolved = RaisinReference {
            id: "uuid-123".to_string(),
            workspace: "social".to_string(),
            path: "/path/to/node".to_string(),
        };

        apply_resolved_reference(&mut properties, &path, resolved);

        if let Some(PropertyValue::Reference(r)) = properties.get("ref1") {
            assert_eq!(r.id, "uuid-123");
            assert_eq!(r.path, "/path/to/node");
        } else {
            panic!("Expected reference");
        }
    }
}
