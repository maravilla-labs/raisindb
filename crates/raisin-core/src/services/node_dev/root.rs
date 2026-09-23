// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Scoped working roots: resolving a path WITHOUT allowing escape.
//!
//! A path is normalized lexically (`.`, `..`, repeated `/`) before it is
//! compared, so `/apps/x/../../secrets` is `/secrets`, which is outside an
//! `/apps` root and refused. A relative path resolves against the first
//! root and can never climb above it.

use super::types::{DevResult, NodeDevError, OpKind, Target, WorkRoot};

/// Normalize an absolute path. `None` when `..` climbs above `/`.
pub fn normalize(path: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s),
        }
    }
    Some(format!("/{}", out.join("/")))
}

/// Is `path` equal to or below `root` (both normalized)?
pub fn is_within(path: &str, root: &str) -> bool {
    root == "/" || path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

/// Parent of a normalized path (`/` for a top-level node).
pub fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((p, _)) => p.to_string(),
    }
}

/// Join a parent and a name.
pub fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", parent.trim_end_matches('/'))
    }
}

/// The set of roots a request may touch.
#[derive(Debug, Clone)]
pub struct Roots {
    roots: Vec<WorkRoot>,
}

impl Roots {
    /// Validate and normalize.
    pub fn new(roots: Vec<WorkRoot>) -> DevResult<Self> {
        if roots.is_empty() {
            return Err(NodeDevError::invalid("at least one root is required"));
        }
        let mut out = Vec::with_capacity(roots.len());
        for mut r in roots {
            if r.workspace.trim().is_empty() {
                return Err(NodeDevError::invalid("a root needs a workspace"));
            }
            r.path = normalize(&r.path)
                .ok_or_else(|| NodeDevError::escape(format!("root '{}' climbs above /", r.path)))?;
            out.push(r);
        }
        Ok(Self { roots: out })
    }

    /// All roots.
    pub fn all(&self) -> &[WorkRoot] {
        &self.roots
    }

    /// The first (default) root.
    pub fn first(&self) -> &WorkRoot {
        &self.roots[0]
    }

    /// Resolve a path (absolute, or relative to the first root) in `workspace`
    /// (default: the first root's) to a normalized absolute path inside a root.
    pub fn resolve(&self, workspace: Option<&str>, path: &str) -> DevResult<(String, String)> {
        let ws = workspace.unwrap_or(&self.first().workspace).to_string();
        let abs = if path.starts_with('/') {
            normalize(path)
        } else {
            let base = &self.first().path;
            if workspace.is_some_and(|w| w != self.first().workspace) {
                return Err(NodeDevError::invalid(
                    "a relative path resolves against the first root; name an absolute path for another workspace",
                ));
            }
            // A relative path may not climb above the first root.
            let joined = normalize(&join(base, path));
            joined.filter(|p| is_within(p, base))
        }
        .ok_or_else(|| NodeDevError::escape(format!("'{path}' escapes its root")))?;
        if !self
            .roots
            .iter()
            .any(|r| r.workspace == ws && is_within(&abs, &r.path))
        {
            return Err(NodeDevError::escape(format!(
                "{ws}:{abs} is outside the working roots"
            )));
        }
        Ok((ws, abs))
    }

    /// Is `op` granted at `ws:path`? A path inside several roots is allowed
    /// when ANY of them allows the operation.
    pub fn check(&self, ws: &str, path: &str, op: OpKind) -> DevResult<()> {
        let inside: Vec<&WorkRoot> = self
            .roots
            .iter()
            .filter(|r| r.workspace == ws && is_within(path, &r.path))
            .collect();
        if inside.is_empty() {
            return Err(NodeDevError::escape(format!(
                "{ws}:{path} is outside the working roots"
            )));
        }
        if inside.iter().any(|r| r.allows(op)) {
            Ok(())
        } else {
            Err(NodeDevError::forbidden(format!(
                "{op:?} is not granted at {ws}:{path}"
            )))
        }
    }

    /// Narrow these roots by a grant: every root must lie inside a granted
    /// root, and keeps only the operations that root grants. Used when an
    /// agent run carries a grant the model cannot widen.
    pub fn narrowed_by(self, grant: &[WorkRoot]) -> DevResult<Self> {
        let grant = Roots::new(grant.to_vec())?;
        let mut out = Vec::new();
        for r in self.roots {
            let Some(g) = grant
                .roots
                .iter()
                .find(|g| g.workspace == r.workspace && is_within(&r.path, &g.path))
            else {
                return Err(NodeDevError::forbidden(format!(
                    "root {}:{} is outside the run's grant",
                    r.workspace, r.path
                )));
            };
            let ops: Vec<OpKind> = OpKind::ALL
                .into_iter()
                .filter(|op| r.allows(*op) && g.allows(*op))
                .collect();
            out.push(WorkRoot {
                ops: Some(ops),
                ..r
            });
        }
        Ok(Self { roots: out })
    }

    /// Resolve a target to `(workspace, path?, node_id?)` — the path is
    /// resolved (and bounded) only when present.
    pub fn target(&self, t: &Target) -> DevResult<(String, Option<String>, Option<String>)> {
        if t.path.is_none() && t.node_id.is_none() {
            return Err(NodeDevError::invalid("a target needs a path or a node_id"));
        }
        let ws = t
            .workspace
            .clone()
            .unwrap_or_else(|| self.first().workspace.clone());
        let path = match &t.path {
            Some(p) => Some(self.resolve(Some(&ws), p)?.1),
            None => None,
        };
        Ok((ws, path, t.node_id.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Roots {
        Roots::new(vec![WorkRoot {
            workspace: "apps".into(),
            path: "/libs/build-1".into(),
            ops: None,
        }])
        .unwrap()
    }

    #[test]
    fn relative_resolves_under_first_root() {
        let (ws, p) = roots().resolve(None, "views/board").unwrap();
        assert_eq!(
            (ws.as_str(), p.as_str()),
            ("apps", "/libs/build-1/views/board")
        );
    }

    #[test]
    fn dotdot_cannot_escape() {
        assert_eq!(
            roots().resolve(None, "../other").unwrap_err().code,
            "path_escape"
        );
        assert_eq!(
            roots()
                .resolve(None, "/libs/build-1/../../secrets")
                .unwrap_err()
                .code,
            "path_escape"
        );
        assert_eq!(
            roots().resolve(Some("stories"), "/x").unwrap_err().code,
            "path_escape"
        );
    }

    #[test]
    fn grant_narrows_ops() {
        let r = Roots::new(vec![WorkRoot::workspace("apps")])
            .unwrap()
            .narrowed_by(&[WorkRoot {
                workspace: "apps".into(),
                path: "/".into(),
                ops: Some(vec![OpKind::Read]),
            }])
            .unwrap();
        assert!(r.check("apps", "/x", OpKind::Read).is_ok());
        assert_eq!(
            r.check("apps", "/x", OpKind::Delete).unwrap_err().code,
            "forbidden"
        );
    }

    #[test]
    fn grant_refuses_wider_root() {
        let err = Roots::new(vec![WorkRoot::workspace("apps")])
            .unwrap()
            .narrowed_by(&[WorkRoot {
                workspace: "apps".into(),
                path: "/libs".into(),
                ops: None,
            }])
            .unwrap_err();
        assert_eq!(err.code, "forbidden");
    }

    #[test]
    fn parent_and_join() {
        assert_eq!(parent_of("/a"), "/");
        assert_eq!(parent_of("/a/b"), "/a");
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/a", "b"), "/a/b");
    }
}
