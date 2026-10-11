//! The build request, as recorded in the job context's metadata.
//!
//! Metadata keys (all set by [`super::submit`]):
//!
//! | key | meaning |
//! |---|---|
//! | `selected_paths` | `[{workspace, path}]`; `path/*` is the node and its subtree, `/*` the whole workspace |
//! | `record` | `{workspace, path}` of the build record to mirror onto (E2), optional |
//! | `register_package` | write `packages:/<name>` (legacy, default `true`); `false` writes nothing there (E4) |
//! | `title`, `description`, `author` | manifest fields (E11) |
//! | `requester_auth`, `requester_permissions` | who asked (E3) |

use super::mirror::RecordRef;
use super::types::SelectedPath;
use raisin_error::{Error, Result};
use std::collections::HashMap;

pub(super) const KEY_SELECTED_PATHS: &str = "selected_paths";
pub(super) const KEY_RECORD: &str = "record";
pub(super) const KEY_REGISTER_PACKAGE: &str = "register_package";

/// One root of the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SelectionRoot {
    pub workspace: String,
    /// The node's path; `/` for the whole workspace.
    pub path: String,
    /// The node's subtree too, not just the node.
    pub recursive: bool,
}

impl SelectionRoot {
    pub(super) fn whole_workspace(&self) -> bool {
        self.path == "/"
    }

    /// The selection entry as the caller wrote it.
    pub(super) fn spelling(&self) -> String {
        match (self.whole_workspace(), self.recursive) {
            (true, _) => format!("{}:/*", self.workspace),
            (false, true) => format!("{}:{}/*", self.workspace, self.path),
            (false, false) => format!("{}:{}", self.workspace, self.path),
        }
    }
}

/// Parse and normalise the selection. `path/*` → recursive, `/*` (or an
/// empty path, or `/`) → the whole workspace (E18). Roots are sorted by
/// `(workspace, path)` and exact duplicates dropped, so the archive layout
/// does not depend on the order the caller listed them in.
pub(super) fn selection_roots(selected: &[SelectedPath]) -> Result<Vec<SelectionRoot>> {
    let mut roots = Vec::with_capacity(selected.len());
    for s in selected {
        if s.workspace.trim().is_empty() {
            return Err(Error::Validation(
                "A selected path names no workspace".to_string(),
            ));
        }
        let (raw, recursive) = match s.path.strip_suffix("/*") {
            Some(p) => (p, true),
            None => (s.path.as_str(), false),
        };
        let trimmed = raw.trim_end_matches('/');
        let (path, recursive) = if trimmed.is_empty() {
            // "/*", "/", "" all mean the workspace root, which has no node of
            // its own: only "everything in it" is meaningful.
            ("/".to_string(), true)
        } else if trimmed.starts_with('/') {
            (trimmed.to_string(), recursive)
        } else {
            (format!("/{trimmed}"), recursive)
        };
        roots.push(SelectionRoot {
            workspace: s.workspace.clone(),
            path,
            recursive,
        });
    }
    roots.sort_by(|a, b| {
        a.workspace
            .as_bytes()
            .cmp(b.workspace.as_bytes())
            .then(a.path.as_bytes().cmp(b.path.as_bytes()))
            .then(b.recursive.cmp(&a.recursive))
    });
    roots.dedup_by(|b, a| {
        a.workspace == b.workspace && a.path == b.path && (a.recursive || !b.recursive)
    });
    Ok(roots)
}

/// Is `path` inside a recursive root that was already walked? Used to skip
/// a node that two overlapping roots both select, so it is archived once.
pub(super) fn covered_by(roots: &[SelectionRoot], idx: usize) -> bool {
    let me = &roots[idx];
    roots[..idx].iter().any(|r| {
        r.workspace == me.workspace
            && r.recursive
            && (r.whole_workspace()
                || me.path == r.path
                || me.path.starts_with(&format!("{}/", r.path)))
    })
}

pub(super) fn record_from_metadata(
    metadata: &HashMap<String, serde_json::Value>,
) -> Result<Option<RecordRef>> {
    match metadata.get(KEY_RECORD) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => {
            let r: RecordRef = serde_json::from_value(v.clone())
                .map_err(|e| Error::Validation(format!("Invalid build record: {e}")))?;
            if r.workspace.is_empty() || !r.path.starts_with('/') {
                return Err(Error::Validation(
                    "Build record needs a workspace and an absolute path".to_string(),
                ));
            }
            Ok(Some(r))
        }
    }
}

pub(super) fn register_package_from_metadata(
    metadata: &HashMap<String, serde_json::Value>,
) -> bool {
    metadata
        .get(KEY_REGISTER_PACKAGE)
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

pub(super) fn string_from_metadata(
    metadata: &HashMap<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    metadata
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(ws: &str, path: &str) -> SelectedPath {
        SelectedPath {
            workspace: ws.into(),
            path: path.into(),
        }
    }

    #[test]
    fn the_selection_is_normalised_and_sorted() {
        let roots = selection_roots(&[
            sp("stories", "/acme/*"),
            sp("assets", "/*"),
            sp("stories", "/acme/home"),
            sp("assets", ""),
            sp("stories", "blog/"),
        ])
        .unwrap();
        let got: Vec<String> = roots.iter().map(|r| r.spelling()).collect();
        assert_eq!(
            got,
            vec![
                "assets:/*",
                "stories:/acme/*",
                "stories:/acme/home",
                "stories:/blog"
            ]
        );
        assert!(!covered_by(&roots, 0));
        assert!(!covered_by(&roots, 1));
        // /acme/home is inside /acme/*: walked once, by the first root.
        assert!(covered_by(&roots, 2));
        assert!(!covered_by(&roots, 3));
    }

    #[test]
    fn a_recursive_root_absorbs_its_node_only_twin() {
        let roots = selection_roots(&[sp("ws", "/a"), sp("ws", "/a/*")]).unwrap();
        assert_eq!(roots.len(), 1);
        assert!(roots[0].recursive);
    }

    #[test]
    fn a_selection_without_a_workspace_is_refused() {
        assert!(selection_roots(&[sp(" ", "/a")]).is_err());
    }

    #[test]
    fn the_record_is_optional_and_validated() {
        let mut m = HashMap::new();
        assert_eq!(record_from_metadata(&m).unwrap(), None);
        m.insert(
            KEY_RECORD.into(),
            serde_json::json!({"workspace": "settings", "path": "/packages/x/b-1"}),
        );
        assert_eq!(
            record_from_metadata(&m).unwrap(),
            Some(RecordRef {
                workspace: "settings".into(),
                path: "/packages/x/b-1".into()
            })
        );
        m.insert(
            KEY_RECORD.into(),
            serde_json::json!({"workspace": "settings", "path": "rel"}),
        );
        assert!(record_from_metadata(&m).is_err());
        assert!(register_package_from_metadata(&HashMap::new()));
    }
}
