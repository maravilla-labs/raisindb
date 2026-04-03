// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Reference-aware topological sorting for content entries
//!
//! Sorts [`ContentEntry::NodeDef`] entries so that referenced nodes are
//! installed before the nodes that reference them. Nodes involved in
//! circular references are flagged for two-pass installation.

use raisin_models::nodes::properties::PropertyValue;
use std::collections::{HashMap, HashSet, VecDeque};

use super::super::content_types::ContentEntry;

/// Result of topological sorting: ordered entries + circular-reference entries.
pub struct SortedEntries {
    /// Entries in dependency order (referenced nodes first)
    pub ordered: Vec<ContentEntry>,
    /// Non-NodeDef entries (BinaryFile, TranslationFile) in original order
    pub other: Vec<ContentEntry>,
    /// NodeDef entries involved in circular references (need two-pass install)
    pub circular: Vec<ContentEntry>,
}

/// Sort content entries by reference dependencies using topological sort.
///
/// - NodeDef entries are sorted so that nodes referenced by others come first.
/// - Parent path dependencies are also respected (parent folders before children).
/// - Nodes in circular reference cycles are separated out and flagged with
///   `__deferred_references` in their properties for two-pass handling.
/// - BinaryFile and TranslationFile entries are returned separately, unsorted.
pub fn sort_by_references(entries: Vec<ContentEntry>) -> SortedEntries {
    // Partition into NodeDefs vs other
    let mut node_entries: Vec<ContentEntry> = Vec::new();
    let mut other: Vec<ContentEntry> = Vec::new();

    for entry in entries {
        match &entry {
            ContentEntry::NodeDef { .. } => node_entries.push(entry),
            _ => other.push(entry),
        }
    }

    // Sort other entries: BinaryFiles before TranslationFiles
    other.sort_by_key(|e| match e {
        ContentEntry::BinaryFile { .. } => 0u8,
        ContentEntry::TranslationFile { .. } => 1,
        _ => 2,
    });

    let n = node_entries.len();
    if n == 0 {
        return SortedEntries {
            ordered: Vec::new(),
            other,
            circular: Vec::new(),
        };
    }

    // Map (workspace, path) → index
    let path_to_idx = build_path_index(&node_entries);

    // Build dependency graph: deps[i] = set of indices that node i depends on
    let deps = build_dependency_graph(&node_entries, &path_to_idx);

    // Kahn's algorithm for topological sort
    let (sorted_indices, circular_indices) = topological_sort(&deps, n);

    // Partition node_entries into ordered and circular
    let mut node_entries_opt: Vec<Option<ContentEntry>> =
        node_entries.into_iter().map(Some).collect();

    let mut ordered = Vec::with_capacity(sorted_indices.len());
    for idx in sorted_indices {
        if let Some(entry) = node_entries_opt[idx].take() {
            ordered.push(entry);
        }
    }

    let mut circular = Vec::with_capacity(circular_indices.len());
    for idx in circular_indices {
        if let Some(mut entry) = node_entries_opt[idx].take() {
            // Flag for two-pass install
            if let ContentEntry::NodeDef { node, .. } = &mut entry {
                node.properties.insert(
                    "__deferred_references".to_string(),
                    PropertyValue::Boolean(true),
                );
            }
            circular.push(entry);
        }
    }

    SortedEntries {
        ordered,
        other,
        circular,
    }
}

/// Build a map of (workspace, path) → index for NodeDef entries.
fn build_path_index(entries: &[ContentEntry]) -> HashMap<(String, String), usize> {
    let mut map = HashMap::new();
    for (idx, entry) in entries.iter().enumerate() {
        if let ContentEntry::NodeDef {
            workspace, node, ..
        } = entry
        {
            map.insert((workspace.clone(), node.path.clone()), idx);
        }
    }
    map
}

/// Build dependency graph: for each node, which other nodes must be installed first.
fn build_dependency_graph(
    entries: &[ContentEntry],
    path_to_idx: &HashMap<(String, String), usize>,
) -> Vec<HashSet<usize>> {
    let n = entries.len();
    let mut deps: Vec<HashSet<usize>> = vec![HashSet::new(); n];

    for (idx, entry) in entries.iter().enumerate() {
        if let ContentEntry::NodeDef {
            workspace, node, ..
        } = entry
        {
            // Reference dependencies
            for (ref_ws, ref_path) in collect_path_references(&node.properties) {
                let ws = if ref_ws.is_empty() {
                    workspace.clone()
                } else {
                    ref_ws
                };
                if let Some(&dep_idx) = path_to_idx.get(&(ws, ref_path)) {
                    if dep_idx != idx {
                        deps[idx].insert(dep_idx);
                    }
                }
            }

            // Parent path dependency (parent folders before children)
            if let Some((parent, _)) = node.path.rsplit_once('/') {
                let pp = if parent.is_empty() {
                    "/".to_string()
                } else {
                    parent.to_string()
                };
                if let Some(&dep_idx) = path_to_idx.get(&(workspace.clone(), pp)) {
                    if dep_idx != idx {
                        deps[idx].insert(dep_idx);
                    }
                }
            }
        }
    }

    deps
}

/// Run Kahn's algorithm. Returns (sorted_indices, circular_indices).
fn topological_sort(deps: &[HashSet<usize>], n: usize) -> (Vec<usize>, Vec<usize>) {
    let mut in_degree: Vec<usize> = vec![0; n];
    let mut reverse_deps: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (idx, dep_set) in deps.iter().enumerate() {
        in_degree[idx] = dep_set.len();
        for &dep in dep_set {
            reverse_deps[dep].push(idx);
        }
    }

    let mut queue: VecDeque<usize> = VecDeque::new();
    for (idx, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(idx);
        }
    }

    let mut sorted = Vec::with_capacity(n);
    while let Some(idx) = queue.pop_front() {
        sorted.push(idx);
        for &dependent in &reverse_deps[idx] {
            in_degree[dependent] -= 1;
            if in_degree[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }

    let sorted_set: HashSet<usize> = sorted.iter().copied().collect();
    let circular: Vec<usize> = (0..n).filter(|i| !sorted_set.contains(i)).collect();

    (sorted, circular)
}

/// Recursively collect all path-based references from a properties map.
///
/// Returns `(workspace, path)` pairs for each `PropertyValue::Reference`
/// whose `raisin:ref` (id) starts with `/`.
fn collect_path_references(properties: &HashMap<String, PropertyValue>) -> Vec<(String, String)> {
    let mut refs = Vec::new();
    let mut stack: Vec<&PropertyValue> = properties.values().collect();

    while let Some(value) = stack.pop() {
        match value {
            PropertyValue::Reference(r) if r.id.starts_with('/') => {
                refs.push((r.workspace.clone(), r.id.clone()));
            }
            PropertyValue::Array(items) => stack.extend(items.iter()),
            PropertyValue::Object(obj) => stack.extend(obj.values()),
            _ => {}
        }
    }

    refs
}

/// Strip all path-based references from a properties map, replacing them
/// with `Null`. Used for the first pass of circular-reference nodes.
pub fn strip_path_references(
    properties: &HashMap<String, PropertyValue>,
) -> HashMap<String, PropertyValue> {
    properties
        .iter()
        .filter(|(k, _)| k.as_str() != "__deferred_references")
        .map(|(k, v)| (k.clone(), strip_recursive(v)))
        .collect()
}

fn strip_recursive(value: &PropertyValue) -> PropertyValue {
    match value {
        PropertyValue::Reference(r) if r.id.starts_with('/') => PropertyValue::Null,
        PropertyValue::Array(items) => {
            PropertyValue::Array(items.iter().map(strip_recursive).collect())
        }
        PropertyValue::Object(obj) => PropertyValue::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), strip_recursive(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::RaisinReference;
    use raisin_models::nodes::Node;

    fn make_node(workspace: &str, path: &str, refs: Vec<(&str, &str)>) -> ContentEntry {
        let mut properties = HashMap::new();
        for (i, (ref_ws, ref_path)) in refs.into_iter().enumerate() {
            properties.insert(
                format!("ref_{}", i),
                PropertyValue::Reference(RaisinReference {
                    id: ref_path.to_string(),
                    workspace: ref_ws.to_string(),
                    path: String::new(),
                }),
            );
        }
        ContentEntry::NodeDef {
            workspace: workspace.to_string(),
            yaml_path: String::new(),
            node: Box::new(Node {
                id: nanoid::nanoid!(),
                node_type: "test:Node".to_string(),
                name: path.rsplit('/').next().unwrap_or("node").to_string(),
                path: path.to_string(),
                workspace: Some(workspace.to_string()),
                properties,
                ..Default::default()
            }),
        }
    }

    fn get_paths(entries: &[ContentEntry]) -> Vec<String> {
        entries
            .iter()
            .filter_map(|e| match e {
                ContentEntry::NodeDef { node, .. } => Some(node.path.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_no_references_preserves_order() {
        let entries = vec![
            make_node("ws", "/a", vec![]),
            make_node("ws", "/b", vec![]),
            make_node("ws", "/c", vec![]),
        ];
        let sorted = sort_by_references(entries);
        assert_eq!(sorted.ordered.len(), 3);
        assert!(sorted.circular.is_empty());
    }

    #[test]
    fn test_reference_ordering() {
        // /schedule references /child — child must come first
        let entries = vec![
            make_node("ws", "/schedule", vec![("ws", "/child")]),
            make_node("ws", "/child", vec![]),
        ];
        let sorted = sort_by_references(entries);
        let paths = get_paths(&sorted.ordered);

        assert_eq!(paths.len(), 2);
        assert!(sorted.circular.is_empty());

        let child_pos = paths.iter().position(|p| p == "/child").unwrap();
        let schedule_pos = paths.iter().position(|p| p == "/schedule").unwrap();
        assert!(child_pos < schedule_pos, "child must come before schedule");
    }

    #[test]
    fn test_chain_dependencies() {
        // /c → /b → /a — must install a, b, c in order
        let entries = vec![
            make_node("ws", "/c", vec![("ws", "/b")]),
            make_node("ws", "/a", vec![]),
            make_node("ws", "/b", vec![("ws", "/a")]),
        ];
        let sorted = sort_by_references(entries);
        let paths = get_paths(&sorted.ordered);

        assert_eq!(paths.len(), 3);
        assert!(sorted.circular.is_empty());

        let a_pos = paths.iter().position(|p| p == "/a").unwrap();
        let b_pos = paths.iter().position(|p| p == "/b").unwrap();
        let c_pos = paths.iter().position(|p| p == "/c").unwrap();
        assert!(a_pos < b_pos && b_pos < c_pos);
    }

    #[test]
    fn test_circular_references_detected() {
        // /a → /b and /b → /a — circular
        let entries = vec![
            make_node("ws", "/a", vec![("ws", "/b")]),
            make_node("ws", "/b", vec![("ws", "/a")]),
        ];
        let sorted = sort_by_references(entries);

        assert!(sorted.ordered.is_empty());
        assert_eq!(sorted.circular.len(), 2);

        // Circular nodes should have __deferred_references flag
        for entry in &sorted.circular {
            if let ContentEntry::NodeDef { node, .. } = entry {
                assert!(node.properties.contains_key("__deferred_references"));
            }
        }
    }

    #[test]
    fn test_mixed_circular_and_linear() {
        // /x is standalone, /a ↔ /b are circular
        let entries = vec![
            make_node("ws", "/x", vec![]),
            make_node("ws", "/a", vec![("ws", "/b")]),
            make_node("ws", "/b", vec![("ws", "/a")]),
        ];
        let sorted = sort_by_references(entries);

        assert_eq!(sorted.ordered.len(), 1);
        assert_eq!(get_paths(&sorted.ordered), vec!["/x"]);
        assert_eq!(sorted.circular.len(), 2);
    }

    #[test]
    fn test_external_references_ignored() {
        // /node references /external which is NOT in the package — should not block
        let entries = vec![make_node("ws", "/node", vec![("other-ws", "/external")])];
        let sorted = sort_by_references(entries);

        assert_eq!(sorted.ordered.len(), 1);
        assert!(sorted.circular.is_empty());
    }

    #[test]
    fn test_deep_nested_references() {
        // Reference buried inside an array inside an object
        let mut inner_obj = HashMap::new();
        inner_obj.insert(
            "items".to_string(),
            PropertyValue::Array(vec![PropertyValue::Reference(RaisinReference {
                id: "/target".to_string(),
                workspace: "ws".to_string(),
                path: String::new(),
            })]),
        );

        let mut properties = HashMap::new();
        properties.insert("nested".to_string(), PropertyValue::Object(inner_obj));

        let entries = vec![
            ContentEntry::NodeDef {
                workspace: "ws".to_string(),
                yaml_path: String::new(),
                node: Box::new(Node {
                    id: nanoid::nanoid!(),
                    node_type: "test:Node".to_string(),
                    name: "source".to_string(),
                    path: "/source".to_string(),
                    workspace: Some("ws".to_string()),
                    properties,
                    ..Default::default()
                }),
            },
            make_node("ws", "/target", vec![]),
        ];

        let sorted = sort_by_references(entries);
        let paths = get_paths(&sorted.ordered);

        assert_eq!(paths.len(), 2);
        let target_pos = paths.iter().position(|p| p == "/target").unwrap();
        let source_pos = paths.iter().position(|p| p == "/source").unwrap();
        assert!(target_pos < source_pos, "target must come before source");
    }

    #[test]
    fn test_strip_path_references() {
        let mut properties = HashMap::new();
        properties.insert(
            "title".to_string(),
            PropertyValue::String("Hello".to_string()),
        );
        properties.insert(
            "ref".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "/some/path".to_string(),
                workspace: "ws".to_string(),
                path: String::new(),
            }),
        );
        properties.insert(
            "uuid_ref".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "some-uuid".to_string(),
                workspace: "ws".to_string(),
                path: String::new(),
            }),
        );

        let stripped = strip_path_references(&properties);

        // Title preserved
        assert_eq!(
            stripped.get("title"),
            Some(&PropertyValue::String("Hello".to_string()))
        );
        // Path-based reference stripped to Null
        assert_eq!(stripped.get("ref"), Some(&PropertyValue::Null));
        // UUID-based reference preserved
        assert!(matches!(stripped.get("uuid_ref"), Some(PropertyValue::Reference(_))));
    }
}
