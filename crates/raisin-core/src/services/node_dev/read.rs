// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Typed reads: stat, list, read, diff and watch.
//!
//! Every read is bounded by the request's roots (the node's CURRENT path must
//! lie inside a root granting `read`) and filtered by the caller's row- and
//! field-level security, exactly as the node API filters it.

use std::str::FromStr;

use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_storage::{ListOptions, NodeRepository, RevisionRepository, Storage};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::root::Roots;
use super::types::*;
use super::{DevScope, NodeDevService};
use crate::services::rls_filter;
use raisin_storage::transactional::TransactionalStorage;

/// `stat` answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatResult {
    /// Whether the node exists (and is visible).
    pub exists: bool,
    /// Where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<NodeLocator>,
    /// Node type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,
    /// Archetype.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archetype: Option<String>,
    /// Child count.
    #[serde(default)]
    pub child_count: usize,
    /// The node's write counter.
    #[serde(default)]
    pub version: i32,
}

/// One `list` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListEntry {
    /// Where it is.
    pub locator: NodeLocator,
    /// Name.
    pub name: String,
    /// Node type.
    pub node_type: String,
    /// Archetype.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archetype: Option<String>,
    /// Whether it has children.
    pub has_children: bool,
}

/// `list` answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResult {
    /// The listed parent (`None` for a workspace root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<NodeLocator>,
    /// Children in editorial order.
    pub children: Vec<ListEntry>,
    /// Whether more children exist past `limit`.
    pub truncated: bool,
}

/// `read` answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResult {
    /// Where it is, with the revision read.
    pub locator: NodeLocator,
    /// Node type.
    pub node_type: String,
    /// Archetype.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archetype: Option<String>,
    /// Properties (all, or the requested subset).
    pub properties: Value,
    /// Child names in order.
    pub children: Vec<String>,
    /// Last writer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
}

/// One side of a diff: a branch and/or a revision (HLC `ts-counter`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffSide {
    /// Branch (default: the call's branch).
    #[serde(default)]
    pub branch: Option<String>,
    /// Revision (default: head).
    #[serde(default)]
    pub revision: Option<String>,
}

/// One property difference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PropertyChange {
    /// Property key.
    pub key: String,
    /// Old value (`null` when added).
    pub from: Value,
    /// New value (`null` when removed).
    pub to: Value,
}

/// `diff` answer: one node (by id) compared across revisions or branches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDiff {
    /// The node on the `from` side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<NodeLocator>,
    /// The node on the `to` side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<NodeLocator>,
    /// Whether its path differs.
    pub moved: bool,
    /// Property differences.
    pub changed: Vec<PropertyChange>,
}

/// One `watch` change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchChange {
    /// Revision that made the change.
    pub revision: String,
    /// `added` | `modified` | `deleted` | `reordered`.
    pub operation: String,
    /// The node now (path is the current one; deleted nodes keep their id).
    pub locator: NodeLocator,
}

/// `watch` answer: changes after a cursor, oldest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchResult {
    /// Changes inside the roots.
    pub changes: Vec<WatchChange>,
    /// Pass back as `since` to continue.
    pub cursor: Option<String>,
}

pub(crate) fn parse_hlc(s: &str) -> DevResult<HLC> {
    HLC::from_str(s).map_err(|e| NodeDevError::invalid(format!("revision '{s}': {e}")))
}

pub(crate) fn props_json(node: &Node) -> Map<String, Value> {
    node.properties
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(Value::Null)))
        .collect()
}

impl<S: Storage + TransactionalStorage> NodeDevService<S> {
    /// Resolve a node by id (preferred) or path, unfiltered.
    pub(crate) async fn find(
        &self,
        scope: &DevScope,
        ws: &str,
        path: Option<&str>,
        node_id: Option<&str>,
        at: Option<&HLC>,
    ) -> DevResult<Option<Node>> {
        let nodes = self.storage.nodes();
        if let Some(id) = node_id {
            return Ok(nodes.get(scope.storage(ws), id, at).await?);
        }
        match path {
            Some("/") => Ok(None),
            Some(p) => Ok(nodes.get_by_path(scope.storage(ws), p, at).await?),
            None => Ok(None),
        }
    }

    /// Resolve and authorize a read of one node; `Ok(None)` when absent or
    /// invisible (the two are indistinguishable on purpose).
    pub(crate) async fn read_node(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        roots: &Roots,
        target: &Target,
    ) -> DevResult<Option<(String, Node)>> {
        let (ws, path, id) = roots.target(target)?;
        let Some(node) = self
            .find(scope, &ws, path.as_deref(), id.as_deref(), None)
            .await?
        else {
            return Ok(None);
        };
        roots.check(&ws, &node.path, OpKind::Read)?;
        Ok(rls_filter::filter_node(node, auth, &scope.permission(&ws)).map(|n| (ws, n)))
    }

    /// `stat`.
    pub async fn stat(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        roots: &Roots,
        target: &Target,
    ) -> DevResult<StatResult> {
        let Some((ws, node)) = self.read_node(scope, auth, roots, target).await? else {
            return Ok(StatResult {
                exists: false,
                locator: None,
                node_type: None,
                archetype: None,
                child_count: 0,
                version: 0,
            });
        };
        Ok(StatResult {
            exists: true,
            locator: Some(NodeLocator::of(&scope.repo, &scope.branch, &ws, &node)),
            node_type: Some(node.node_type.clone()),
            archetype: node.archetype.clone(),
            child_count: node.children.len(),
            version: node.version,
        })
    }

    /// `read`, optionally projecting to `keys`.
    pub async fn read(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        roots: &Roots,
        target: &Target,
        keys: Option<&[String]>,
    ) -> DevResult<ReadResult> {
        let (ws, node) = self
            .read_node(scope, auth, roots, target)
            .await?
            .ok_or_else(|| NodeDevError::not_found("node not found"))?;
        let mut props = props_json(&node);
        if let Some(keys) = keys {
            props.retain(|k, _| keys.iter().any(|w| w == k));
        }
        Ok(ReadResult {
            locator: NodeLocator::of(&scope.repo, &scope.branch, &ws, &node),
            node_type: node.node_type.clone(),
            archetype: node.archetype.clone(),
            properties: Value::Object(props),
            children: node.children.clone(),
            updated_by: node.updated_by.clone(),
        })
    }

    /// `list` the children of a node (or of a workspace root: path `/`).
    pub async fn list(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        roots: &Roots,
        target: &Target,
        limit: usize,
    ) -> DevResult<ListResult> {
        let (ws, path, id) = roots.target(target)?;
        let (parent, parent_path) = if path.as_deref() == Some("/") && id.is_none() {
            roots.check(&ws, "/", OpKind::Read)?;
            (None, "/".to_string())
        } else {
            let (_, node) = self
                .read_node(scope, auth, roots, target)
                .await?
                .ok_or_else(|| NodeDevError::not_found("node not found"))?;
            let p = node.path.clone();
            (
                Some(NodeLocator::of(&scope.repo, &scope.branch, &ws, &node)),
                p,
            )
        };
        let children = self
            .storage
            .nodes()
            .list_children(scope.storage(&ws), &parent_path, ListOptions::for_api())
            .await?;
        let perm = scope.permission(&ws);
        let visible: Vec<Node> = children
            .into_iter()
            .filter(|n| roots.check(&ws, &n.path, OpKind::Read).is_ok())
            .filter_map(|n| rls_filter::filter_node(n, auth, &perm))
            .collect();
        let truncated = visible.len() > limit;
        let children = visible
            .into_iter()
            .take(limit)
            .map(|n| ListEntry {
                locator: NodeLocator::of(&scope.repo, &scope.branch, &ws, &n),
                name: n.name.clone(),
                node_type: n.node_type.clone(),
                archetype: n.archetype.clone(),
                has_children: n.has_children.unwrap_or(!n.children.is_empty()),
            })
            .collect();
        Ok(ListResult {
            parent,
            children,
            truncated,
        })
    }

    async fn side(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        ws: &str,
        id: &str,
        side: &DiffSide,
    ) -> DevResult<Option<(NodeLocator, Node)>> {
        let branch = side.branch.clone().unwrap_or_else(|| scope.branch.clone());
        let s = DevScope::new(&scope.tenant, &scope.repo, &branch);
        let at = side.revision.as_deref().map(parse_hlc).transpose()?;
        let node = self.find(&s, ws, None, Some(id), at.as_ref()).await?;
        Ok(node
            .and_then(|n| rls_filter::filter_node(n, auth, &s.permission(ws)))
            .map(|n| (NodeLocator::of(&s.repo, &s.branch, ws, &n), n)))
    }

    /// `diff` one node between two revisions and/or branches.
    pub async fn diff(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        roots: &Roots,
        target: &Target,
        from: &DiffSide,
        to: &DiffSide,
    ) -> DevResult<NodeDiff> {
        let (ws, path, id) = roots.target(target)?;
        let id = match id {
            Some(id) => id,
            None => self
                .find(scope, &ws, path.as_deref(), None, None)
                .await?
                .map(|n| n.id)
                .ok_or_else(|| NodeDevError::not_found("node not found"))?,
        };
        let a = self.side(scope, auth, &ws, &id, from).await?;
        let b = self.side(scope, auth, &ws, &id, to).await?;
        for (loc, _) in a.iter().chain(b.iter()) {
            roots.check(&ws, &loc.path, OpKind::Read)?;
        }
        let pa = a.as_ref().map(|(_, n)| props_json(n)).unwrap_or_default();
        let pb = b.as_ref().map(|(_, n)| props_json(n)).unwrap_or_default();
        Ok(NodeDiff {
            moved: matches!((&a, &b), (Some((x, _)), Some((y, _))) if x.path != y.path),
            changed: property_changes(&pa, &pb),
            from: a.map(|(l, _)| l),
            to: b.map(|(l, _)| l),
        })
    }

    /// `watch`: node changes inside the roots committed after `since`
    /// (exclusive), oldest first. Poll with the returned cursor.
    pub async fn watch(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        roots: &Roots,
        since: Option<&str>,
        limit: usize,
    ) -> DevResult<WatchResult> {
        let after = match since {
            Some(s) => parse_hlc(s)?,
            None => HLC::new(0, 0),
        };
        let revs = self
            .storage
            .revisions()
            .list_revisions_since(&scope.tenant, &scope.repo, &after, limit.max(1))
            .await?;
        let mut changes = Vec::new();
        let mut cursor = since.map(str::to_string);
        for meta in revs {
            cursor = Some(meta.revision.to_string());
            if meta.branch != scope.branch {
                continue;
            }
            for c in &meta.changed_nodes {
                if c.translation_locale.is_some()
                    || !roots.all().iter().any(|r| r.workspace == c.workspace)
                {
                    continue;
                }
                // A deleted node is read as it was just before the delete.
                let deleted = format!("{:?}", c.operation).eq_ignore_ascii_case("deleted");
                let at = if deleted {
                    meta.parent
                } else {
                    Some(meta.revision)
                };
                let Some(at) = at else { continue };
                let node = self
                    .find(scope, &c.workspace, None, Some(&c.node_id), Some(&at))
                    .await?;
                let Some(node) = node
                    .filter(|n| roots.check(&c.workspace, &n.path, OpKind::Read).is_ok())
                    .and_then(|n| {
                        rls_filter::filter_node(n, auth, &scope.permission(&c.workspace))
                    })
                else {
                    continue;
                };
                changes.push(WatchChange {
                    revision: meta.revision.to_string(),
                    operation: format!("{:?}", c.operation).to_lowercase(),
                    locator: NodeLocator::of(&scope.repo, &scope.branch, &c.workspace, &node),
                });
            }
        }
        Ok(WatchResult { changes, cursor })
    }
}

/// Property-level difference of two property maps, sorted by key.
pub fn property_changes(a: &Map<String, Value>, b: &Map<String, Value>) -> Vec<PropertyChange> {
    let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter_map(|k| {
            let (x, y) = (a.get(k), b.get(k));
            (x != y).then(|| PropertyChange {
                key: k.clone(),
                from: x.cloned().unwrap_or(Value::Null),
                to: y.cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}
