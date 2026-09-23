// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Wire types of the node-development surface: locators, revisions, roots,
//! grants and the error shape. Everything here is plain serde so the HTTP
//! routes, the function bindings and the SDKs speak exactly these shapes.

use raisin_agent_contract::tool_result::{Locator, Revision};
use raisin_models::nodes::Node;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The revision scheme of a single node: changes on every write to the node
/// (its `version` counter and `updated_at`) and on every move (its path).
pub const NODE_REVISION_ALG: &str = "raisin.node/1";

/// The revision scheme of a whole branch: its head HLC.
pub const BRANCH_REVISION_ALG: &str = "hlc";

/// A canonical, absolute node locator.
///
/// `repository/branch/workspace/path` is where the node is now; `node_id` is
/// what it IS (stable across moves); `revision` is what the reader saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeLocator {
    /// Repository.
    pub repository: String,
    /// Branch.
    pub branch: String,
    /// Workspace.
    pub workspace: String,
    /// Absolute path inside the workspace.
    pub path: String,
    /// Stable node id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// The node revision observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<Revision>,
}

impl NodeLocator {
    /// Locator of a node as read now.
    pub fn of(repo: &str, branch: &str, workspace: &str, node: &Node) -> Self {
        Self {
            repository: repo.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            path: node.path.clone(),
            node_id: Some(node.id.clone()),
            revision: Some(node_revision(node)),
        }
    }

    /// The contract locator (`raisin.tool-result/1`).
    pub fn contract(&self) -> Locator {
        Locator {
            repository: Some(self.repository.clone()),
            branch: Some(self.branch.clone()),
            workspace: self.workspace.clone(),
            path: self.path.clone(),
            node_id: self.node_id.clone(),
        }
    }
}

/// The node revision: a short digest of identity, location and write counter.
pub fn node_revision(node: &Node) -> Revision {
    let mut h = Sha256::new();
    h.update(node.id.as_bytes());
    h.update([0x1f]);
    h.update(node.path.as_bytes());
    h.update([0x1f]);
    h.update(node.version.to_string().as_bytes());
    h.update([0x1f]);
    if let Some(at) = node.updated_at {
        h.update(at.to_rfc3339().as_bytes());
    }
    let digest = h.finalize();
    Revision {
        value: hex(&digest[..12]),
        alg: NODE_REVISION_ALG.to_string(),
    }
}

/// Lower-case hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// An expected revision: the full object, or just its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExpectedRevision {
    /// `{ value, alg }`.
    Full(Revision),
    /// `"value"`.
    Value(String),
}

impl ExpectedRevision {
    /// The value to compare.
    pub fn value(&self) -> &str {
        match self {
            Self::Full(r) => &r.value,
            Self::Value(v) => v,
        }
    }
}

/// Operation classes a root can grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    /// stat, list, read, diff, watch.
    Read,
    /// Create nodes.
    Create,
    /// Patch properties.
    Update,
    /// Delete nodes (and their subtrees).
    Delete,
    /// Move and rename (the source).
    Move,
    /// Copy (the destination is checked as `create`).
    Copy,
}

impl OpKind {
    /// Every kind.
    pub const ALL: [OpKind; 6] = [
        OpKind::Read,
        OpKind::Create,
        OpKind::Update,
        OpKind::Delete,
        OpKind::Move,
        OpKind::Copy,
    ];
}

/// A working root: a subtree of one workspace, with the operations allowed
/// inside it. A request's roots are its whole world — nothing outside them
/// can be named, and a relative path resolves against the FIRST root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRoot {
    /// Workspace.
    pub workspace: String,
    /// Absolute subtree path (`/` for the whole workspace).
    #[serde(default = "slash")]
    pub path: String,
    /// Allowed operations; absent means all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ops: Option<Vec<OpKind>>,
}

fn slash() -> String {
    "/".to_string()
}

impl WorkRoot {
    /// Whole workspace, every operation.
    pub fn workspace(ws: &str) -> Self {
        Self {
            workspace: ws.to_string(),
            path: "/".to_string(),
            ops: None,
        }
    }

    /// Does this root allow `op`?
    pub fn allows(&self, op: OpKind) -> bool {
        self.ops.as_ref().is_none_or(|ops| ops.contains(&op))
    }
}

/// Where an operation points: a path (relative to the first root, or
/// absolute), a node id, or both (the id wins, the path is a hint).
///
/// On the wire a bare string is a path, everywhere a target appears — the
/// top-level `target` of a read and every changeset op's `target`/`source`
/// alike — so a caller never has to know which positions accept the short form.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "TargetWire")]
pub struct Target {
    /// Workspace; defaults to the first root's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Node id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

/// The accepted wire forms of a [`Target`].
#[derive(Deserialize)]
#[serde(untagged)]
enum TargetWire {
    Path(String),
    Full {
        #[serde(default)]
        workspace: Option<String>,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        node_id: Option<String>,
    },
}

impl From<TargetWire> for Target {
    fn from(w: TargetWire) -> Self {
        match w {
            TargetWire::Path(p) => Target::path(&p),
            TargetWire::Full {
                workspace,
                path,
                node_id,
            } => Target {
                workspace,
                path,
                node_id,
            },
        }
    }
}

impl Target {
    /// By path.
    pub fn path(p: &str) -> Self {
        Self {
            path: Some(p.to_string()),
            ..Self::default()
        }
    }
    /// By id.
    pub fn id(id: &str) -> Self {
        Self {
            node_id: Some(id.to_string()),
            ..Self::default()
        }
    }
}

/// A refused call. Conflicts on stale revisions are NOT errors — they are a
/// changeset result — so this carries only what the caller must fix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDevError {
    /// HTTP-ish status (400, 403, 404, 409, 422, 503).
    pub status: u16,
    /// Stable machine code.
    pub code: String,
    /// Human message.
    pub message: String,
}

impl NodeDevError {
    /// Build one.
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.to_string(),
            message: message.into(),
        }
    }
    /// 400.
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::new(400, "invalid_request", msg)
    }
    /// 403, outside the roots.
    pub fn escape(msg: impl Into<String>) -> Self {
        Self::new(403, "path_escape", msg)
    }
    /// 403, not granted.
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(403, "forbidden", msg)
    }
    /// 404.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(404, "not_found", msg)
    }
}

impl std::fmt::Display for NodeDevError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for NodeDevError {}

impl From<raisin_error::Error> for NodeDevError {
    fn from(e: raisin_error::Error) -> Self {
        use raisin_error::Error as E;
        match &e {
            E::NotFound(m) => Self::not_found(m.clone()),
            E::PermissionDenied(m) | E::Forbidden(m) | E::Unauthorized(m) => {
                Self::forbidden(m.clone())
            }
            E::Validation(m) => Self::new(422, "validation_failed", m.clone()),
            E::AlreadyExists(m) | E::Conflict(m) => Self::new(409, "conflict", m.clone()),
            _ => Self::new(500, "internal", e.to_string()),
        }
    }
}

/// Result alias.
pub type DevResult<T> = std::result::Result<T, NodeDevError>;
