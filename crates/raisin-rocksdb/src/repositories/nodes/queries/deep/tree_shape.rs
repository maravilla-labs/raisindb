//! Know the shape of a subtree before recursing into it.
//!
//! The deep readers used to scan `ORDERED_CHILDREN` for EVERY node they
//! visited — including leaves, which are most nodes of a content tree — just to
//! learn there was nothing there. The subtree's PATH_INDEX entries, which the
//! readers fetch anyway, already say which node has children and how many:
//!
//! - 0 children: a leaf. No scan.
//! - 1 child: nothing to order. No scan.
//! - 2 or more: one `ORDERED_CHILDREN` scan for the editorial order. Never
//!   sort by `order_key` instead — it drifts from the index on legacy data.

use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use std::collections::{HashMap, HashSet};

/// Which node of a subtree has which children (unordered).
pub(super) struct TreeShape {
    /// Parent PATH -> child ids.
    children: HashMap<String, Vec<String>>,
    /// Node id -> path.
    paths: HashMap<String, String>,
}

impl TreeShape {
    /// Build from `(node_id, path)` pairs of a subtree.
    pub(super) fn new<'a>(nodes: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        let mut paths = HashMap::new();
        for (id, path) in nodes {
            children
                .entry(parent_of(path).to_string())
                .or_default()
                .push(id.to_string());
            paths.insert(id.to_string(), path.to_string());
        }
        Self { children, paths }
    }

    /// The children of the node at `path`, unordered.
    pub(super) fn children_of_path(&self, path: &str) -> &[String] {
        self.children
            .get(normalize(path))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The children of the node `id`, unordered.
    pub(super) fn children_of(&self, id: &str) -> &[String] {
        self.paths
            .get(id)
            .map(|path| self.children_of_path(path))
            .unwrap_or(&[])
    }
}

/// `/a/b` -> `/a`; `/a` -> `/`.
fn parent_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/",
        Some((parent, _)) => parent,
    }
}

/// The form `parent_of` produces: no trailing slash, `/` for the root.
fn normalize(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/"
    } else {
        trimmed
    }
}

impl NodeRepositoryImpl {
    /// `candidates` — the children of `parent_id` — in editorial order.
    ///
    /// Scans `ORDERED_CHILDREN` only when there are two or more to order. A
    /// candidate the index does not list is dropped, exactly as the
    /// scan-every-node readers dropped it.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn order_child_ids(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        candidates: &[String],
        max_revision: Option<&HLC>,
    ) -> Result<Vec<String>> {
        if candidates.len() < 2 {
            return Ok(candidates.to_vec());
        }
        let wanted: HashSet<&str> = candidates.iter().map(String::as_str).collect();
        Ok(self
            .get_ordered_child_ids(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id,
                max_revision,
            )
            .await?
            .into_iter()
            .filter(|id| wanted.contains(id.as_str()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parents_and_children_by_path() {
        let shape = TreeShape::new([
            ("a", "/a"),
            ("b", "/a/b"),
            ("c", "/a/c"),
            ("d", "/a/b/d"),
            ("e", "/e"),
        ]);
        let mut top = shape.children_of_path("/").to_vec();
        top.sort();
        assert_eq!(top, vec!["a", "e"]);
        let mut under_a = shape.children_of("a").to_vec();
        under_a.sort();
        assert_eq!(under_a, vec!["b", "c"]);
        assert_eq!(shape.children_of_path("/a/"), shape.children_of("a"));
        assert_eq!(shape.children_of("b"), ["d".to_string()]);
        assert!(shape.children_of("d").is_empty());
        assert!(shape.children_of("unknown").is_empty());
    }
}
