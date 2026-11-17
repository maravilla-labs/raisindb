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
    /// NodeDef and BinaryFile entries in dependency order (referenced nodes
    /// and assets first)
    pub ordered: Vec<ContentEntry>,
    /// TranslationFile entries in original order (they overlay nodes that the
    /// two lists above create, so they go last)
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
    // Partition: NodeDefs AND BinaryFiles take part in the ordering; only
    // translations (overlays onto nodes that must already exist) stay apart.
    //
    // Binary files used to be appended AFTER every NodeDef. A page whose
    // `file:` references `assets:/press-kit/logo.svg` from the SAME package was
    // then installed before that asset existed, rejected with "Referenced node
    // not found", and the job failed on a fresh repo — the engine's retry ten
    // seconds later found the asset and succeeded, but the first attempt had
    // already failed the pipeline. A binary is a node like any other here: it
    // depends on its parent folder, and what references it depends on it.
    let mut node_entries: Vec<ContentEntry> = Vec::new();
    let mut other: Vec<ContentEntry> = Vec::new();

    for entry in entries {
        match &entry {
            ContentEntry::NodeDef { .. } | ContentEntry::BinaryFile { .. } => {
                node_entries.push(entry)
            }
            _ => other.push(entry),
        }
    }

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

/// The `(workspace, path)` an entry installs to. A binary file becomes the
/// asset at `/{parent_path}/{filename}`, the same path `install_binary_file`
/// writes.
pub(super) fn entry_node_path(entry: &ContentEntry) -> Option<(String, String)> {
    match entry {
        ContentEntry::NodeDef {
            workspace, node, ..
        } => Some((workspace.clone(), node.path.clone())),
        ContentEntry::BinaryFile {
            workspace,
            parent_path,
            filename,
            ..
        } => Some((
            workspace.clone(),
            if parent_path.is_empty() {
                format!("/{filename}")
            } else {
                format!("/{parent_path}/{filename}")
            },
        )),
        ContentEntry::TranslationFile { .. } => None,
    }
}

/// Build a map of (workspace, path) → index for NodeDef and BinaryFile entries.
fn build_path_index(entries: &[ContentEntry]) -> HashMap<(String, String), usize> {
    let mut map = HashMap::new();
    for (idx, entry) in entries.iter().enumerate() {
        if let Some(key) = entry_node_path(entry) {
            map.insert(key, idx);
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
        let Some((workspace, path)) = entry_node_path(entry) else {
            continue;
        };
        // Reference dependencies (a binary file references nothing).
        if let ContentEntry::NodeDef { node, .. } = entry {
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
        }

        // Parent path dependency (parent folders before children)
        if let Some((parent, _)) = path.rsplit_once('/') {
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
            // Content elements (Hero, Teaser, ActionButton…) carry their fields —
            // including path references like cta/target/link — inside `content`,
            // keyed like an object. Descend so element-nested refs are counted as
            // dependencies; otherwise the topo-sort misses them and the referenced
            // node may be installed AFTER the element that references it (the
            // insert-time resolver in create/references.rs descends Elements too).
            PropertyValue::Element(element) => stack.extend(element.content.values()),
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
        // Strip path refs nested inside content elements too, so the first pass
        // of a circular-reference element node carries no unresolved path refs.
        PropertyValue::Element(element) => {
            let mut e = element.clone();
            e.content = element
                .content
                .iter()
                .map(|(k, v)| (k.clone(), strip_recursive(v)))
                .collect();
            PropertyValue::Element(e)
        }
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
            legacy_path: None,
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

    /// Build a node whose only reference is nested inside a `content` element
    /// (e.g. a Hero's `cta`, a Teaser's `target`) — the real-world shape that
    /// regressed: refs live inside elements, not as top-level properties.
    fn make_node_with_element_ref(
        workspace: &str,
        path: &str,
        ref_ws: &str,
        ref_path: &str,
    ) -> ContentEntry {
        use raisin_models::nodes::properties::value::Element;
        let mut el_content = HashMap::new();
        el_content.insert(
            "cta".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: ref_path.to_string(),
                workspace: ref_ws.to_string(),
                path: String::new(),
            }),
        );
        let element = PropertyValue::Element(Element {
            uuid: "el-1".to_string(),
            element_type: "standard:Hero".to_string(),
            content: el_content,
        });
        let mut properties = HashMap::new();
        properties.insert("content".to_string(), PropertyValue::Array(vec![element]));
        ContentEntry::NodeDef {
            workspace: workspace.to_string(),
            yaml_path: String::new(),
            legacy_path: None,
            node: Box::new(Node {
                id: nanoid::nanoid!(),
                node_type: "studio:Page".to_string(),
                name: path.rsplit('/').next().unwrap_or("node").to_string(),
                path: path.to_string(),
                workspace: Some(workspace.to_string()),
                properties,
                ..Default::default()
            }),
        }
    }

    #[test]
    fn test_reference_nested_in_content_element_is_ordered() {
        // Regression: /home has a Hero whose `cta` references /products. The
        // referenced node must still be installed first even though the ref is
        // buried inside a content Element (not a top-level property).
        let entries = vec![
            make_node_with_element_ref("stories", "/home", "stories", "/products"),
            make_node("stories", "/products", vec![]),
        ];
        let sorted = sort_by_references(entries);
        let paths = get_paths(&sorted.ordered);

        assert_eq!(paths.len(), 2, "no false cycle");
        assert!(sorted.circular.is_empty());
        let products_pos = paths.iter().position(|p| p == "/products").unwrap();
        let home_pos = paths.iter().position(|p| p == "/home").unwrap();
        assert!(
            products_pos < home_pos,
            "element-nested ref target must come first"
        );
    }

    fn binary(workspace: &str, parent_path: &str, filename: &str) -> ContentEntry {
        ContentEntry::BinaryFile {
            workspace: workspace.to_string(),
            zip_path: format!("content/{workspace}/{parent_path}/{filename}"),
            filename: filename.to_string(),
            parent_path: parent_path.to_string(),
            data: vec![1, 2, 3],
            metadata: Box::new(None),
            content_hash: "h".to_string(),
        }
    }

    /// A page that references an asset shipped as a binary in the SAME package
    /// must be installed after it. Binaries used to be appended after every
    /// NodeDef, so on a fresh repo the page was rejected ("Referenced node not
    /// found: assets:/press-kit/…") and the first install attempt failed.
    #[test]
    fn a_binary_asset_is_installed_before_the_node_that_references_it() {
        let entries = vec![
            make_node(
                "stories",
                "/site/press/kit/photo",
                vec![("assets", "/press-kit/photo.png")],
            ),
            make_node("assets", "/press-kit", vec![]),
            binary("assets", "press-kit", "photo.png"),
            binary("assets", "press-kit", "unreferenced.pdf"),
        ];
        let sorted = sort_by_references(entries);
        assert!(sorted.circular.is_empty());
        assert!(
            sorted.other.is_empty(),
            "binaries are ordered, not appended"
        );
        let keys: Vec<(String, String)> =
            sorted.ordered.iter().filter_map(entry_node_path).collect();
        let pos = |ws: &str, p: &str| {
            keys.iter()
                .position(|(w, q)| w == ws && q == p)
                .unwrap_or_else(|| panic!("{ws}:{p} missing from {keys:?}"))
        };
        assert!(pos("assets", "/press-kit") < pos("assets", "/press-kit/photo.png"));
        assert!(pos("assets", "/press-kit/photo.png") < pos("stories", "/site/press/kit/photo"));
        assert!(pos("assets", "/press-kit") < pos("assets", "/press-kit/unreferenced.pdf"));
    }

    #[test]
    fn translations_still_come_last() {
        let entries = vec![binary("assets", "", "a.png"), make_node("ws", "/a", vec![])];
        let sorted = sort_by_references(entries);
        assert_eq!(sorted.ordered.len(), 2);
        assert!(sorted.other.is_empty());
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
                legacy_path: None,
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
        assert!(matches!(
            stripped.get("uuid_ref"),
            Some(PropertyValue::Reference(_))
        ));
    }
}
