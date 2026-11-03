// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Relation access for graph queries: ONE seam between the graph executors and
//! the relation indexes.
//!
//! # Why a seam
//!
//! The relation store has two very different access paths:
//!
//! * the **global** index (`scan_relations_global`): every relation of a type in
//!   the branch, O(total edges of that type);
//! * the **per-node** indexes (`get_outgoing_relations` /
//!   `get_incoming_relations`): the relations of ONE node, O(degree).
//!
//! `GRAPH_TABLE` used to take the first path for every hop, even when its
//! `WHERE` pinned an endpoint to a single node. This trait lets the matching
//! code choose, and lets a test hand it an instrumented graph that counts how
//! many relations each path actually touched. The count is the property the
//! pushdown exists to change, so it is the thing the tests assert.
//!
//! The production implementation, [`StorageAccess`], is a thin adapter over
//! [`Storage`]; it adds no caching and no policy.

use std::future::Future;
use std::sync::Arc;

use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::{FullRelation, RelationRef};
use raisin_models::permissions::PermissionScope;
use raisin_storage::{
    BranchScope, NodeRepository, RelationRepository, RepoScope, Storage, StorageScope,
    WorkspaceRepository,
};

use super::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// A scanned relation tuple as produced by `scan_relations_global`:
/// `(source_workspace, source_id, target_workspace, target_id, relation)`.
pub type ScannedRelation = (String, String, String, String, FullRelation);

/// A node as the relation indexes identify it: `(workspace, id)`.
pub type NodeKey = (String, String);

/// One node a query pinned down, with the type the relation records need.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SeekNode {
    pub workspace: String,
    pub id: String,
    pub node_type: String,
}

impl SeekNode {
    pub fn key(&self) -> NodeKey {
        (self.workspace.clone(), self.id.clone())
    }
}

/// Which side of a node the per-node index is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Relations whose SOURCE is the node.
    Out,
    /// Relations whose TARGET is the node.
    In,
    /// Both.
    Both,
}

/// Read access to the relation indexes and to the node facts they need.
pub trait RelationAccess: Send + Sync {
    /// The global index: every relation, optionally of one exact type.
    fn global_scan(
        &self,
        relation_type: Option<&str>,
    ) -> impl Future<Output = Result<Vec<ScannedRelation>>> + Send;

    /// Names of every workspace of the repository.
    fn workspaces(&self) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// The node's type when the node exists in `workspace`.
    fn node_type(
        &self,
        workspace: &str,
        id: &str,
    ) -> impl Future<Output = Result<Option<String>>> + Send;

    /// `(id, node_type)` of the node at `path` in `workspace`.
    fn node_by_path(
        &self,
        workspace: &str,
        path: &str,
    ) -> impl Future<Output = Result<Option<(String, String)>>> + Send;

    /// Whether the caller may read the node at `(workspace, id)`. `false` for a
    /// node that does not exist. Used to drop a pinned node the caller cannot
    /// read BEFORE any relation of it is read (see `pgq::matching::pushdown`).
    fn readable(&self, workspace: &str, id: &str) -> impl Future<Output = Result<bool>> + Send;

    /// The per-node forward index.
    fn outgoing(
        &self,
        workspace: &str,
        id: &str,
    ) -> impl Future<Output = Result<Vec<RelationRef>>> + Send;

    /// The per-node reverse index: `(source_workspace, source_id, ref)` where
    /// `ref.target_node_type` is the SOURCE's type (the reverse entry is a
    /// relation seen from the other end).
    fn incoming(
        &self,
        workspace: &str,
        id: &str,
    ) -> impl Future<Output = Result<Vec<(String, String, RelationRef)>>> + Send;
}

/// [`RelationAccess`] over a real [`Storage`].
pub struct StorageAccess<'a, S: Storage> {
    storage: &'a Arc<S>,
    tenant_id: &'a str,
    repo_id: &'a str,
    branch: &'a str,
    revision: Option<&'a HLC>,
    /// A workspace to ask about even if the repository's workspace list does
    /// not name it (the query's own workspace).
    workspace_hint: Option<&'a str>,
    /// The caller, for [`RelationAccess::readable`]; `None` reads everything
    /// (a system or internal caller, as in the scan executors).
    auth: Option<&'a AuthContext>,
}

impl<'a, S: Storage> StorageAccess<'a, S> {
    pub fn new(
        storage: &'a Arc<S>,
        tenant_id: &'a str,
        repo_id: &'a str,
        branch: &'a str,
        revision: Option<&'a HLC>,
    ) -> Self {
        Self {
            storage,
            tenant_id,
            repo_id,
            branch,
            revision,
            workspace_hint: None,
            auth: None,
        }
    }

    /// Check [`RelationAccess::readable`] against this caller.
    pub fn with_auth(mut self, auth: Option<&'a AuthContext>) -> Self {
        self.auth = auth;
        self
    }

    /// Also consider `workspace` when a node has to be located by id or path.
    pub fn with_workspace_hint(mut self, workspace: &'a str) -> Self {
        self.workspace_hint = Some(workspace);
        self
    }

    fn scope<'b>(&'b self, workspace: &'b str) -> StorageScope<'b> {
        StorageScope::new(self.tenant_id, self.repo_id, self.branch, workspace)
    }
}

fn backend(e: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Backend(e.to_string())
}

impl<S: Storage> RelationAccess for StorageAccess<'_, S> {
    async fn global_scan(&self, relation_type: Option<&str>) -> Result<Vec<ScannedRelation>> {
        self.storage
            .relations()
            .scan_relations_global(
                BranchScope::new(self.tenant_id, self.repo_id, self.branch),
                relation_type,
                self.revision,
            )
            .await
            .map_err(backend)
    }

    async fn workspaces(&self) -> Result<Vec<String>> {
        let list = self
            .storage
            .workspaces()
            .list(RepoScope::new(self.tenant_id, self.repo_id))
            .await
            .map_err(backend)?;
        let mut names: Vec<String> = list.into_iter().map(|w| w.name).collect();
        if let Some(hint) = self.workspace_hint {
            if !names.iter().any(|n| n == hint) {
                names.push(hint.to_string());
            }
        }
        Ok(names)
    }

    async fn node_type(&self, workspace: &str, id: &str) -> Result<Option<String>> {
        let node = self
            .storage
            .nodes()
            .get(self.scope(workspace), id, self.revision)
            .await
            .map_err(backend)?;
        Ok(node.map(|n| n.node_type))
    }

    async fn node_by_path(&self, workspace: &str, path: &str) -> Result<Option<(String, String)>> {
        let node = self
            .storage
            .nodes()
            .get_by_path(self.scope(workspace), path, self.revision)
            .await
            .map_err(backend)?;
        Ok(node.map(|n| (n.id, n.node_type)))
    }

    async fn readable(&self, workspace: &str, id: &str) -> Result<bool> {
        let node = self
            .storage
            .nodes()
            .get(self.scope(workspace), id, self.revision)
            .await
            .map_err(backend)?;
        let Some(node) = node else {
            return Ok(false);
        };
        let Some(auth) = self.auth else {
            return Ok(true);
        };
        let scope = PermissionScope::new(workspace, self.branch);
        Ok(super::scan_executors::helpers::rls_filter_node_graph(
            &**self.storage,
            node,
            auth,
            &scope,
            self.tenant_id,
            self.repo_id,
            self.branch,
            self.revision,
        )
        .await
        .is_some())
    }

    async fn outgoing(&self, workspace: &str, id: &str) -> Result<Vec<RelationRef>> {
        self.storage
            .relations()
            .get_outgoing_relations(self.scope(workspace), id, self.revision)
            .await
            .map_err(backend)
    }

    async fn incoming(
        &self,
        workspace: &str,
        id: &str,
    ) -> Result<Vec<(String, String, RelationRef)>> {
        self.storage
            .relations()
            .get_incoming_relations(self.scope(workspace), id, self.revision)
            .await
            .map_err(backend)
    }
}

/// The relations of `nodes` read from the per-node indexes, in the SAME tuple
/// shape the global scan produces, so a caller can swap one for the other.
///
/// Types are filtered by `accept`. A relation that touches two of `nodes`
/// (a self-loop, or an edge between two pinned nodes) appears ONCE, exactly as
/// it does in the global index.
pub async fn node_relations<A: RelationAccess>(
    access: &A,
    nodes: &[SeekNode],
    side: Side,
    accept: &(dyn Fn(&str) -> bool + Send + Sync),
) -> Result<Vec<ScannedRelation>> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<ScannedRelation> = Vec::new();
    let mut push = |t: ScannedRelation| {
        let key = (
            t.4.relation_type.clone(),
            t.0.clone(),
            t.1.clone(),
            t.2.clone(),
            t.3.clone(),
        );
        if seen.insert(key) {
            out.push(t);
        }
    };

    for node in nodes {
        if matches!(side, Side::Out | Side::Both) {
            for rel in access.outgoing(&node.workspace, &node.id).await? {
                if !accept(&rel.relation_type) {
                    continue;
                }
                let full = FullRelation::from_source_and_ref(
                    node.id.clone(),
                    node.workspace.clone(),
                    node.node_type.clone(),
                    &rel,
                );
                push((
                    node.workspace.clone(),
                    node.id.clone(),
                    rel.workspace.clone(),
                    rel.target.clone(),
                    full,
                ));
            }
        }
        if matches!(side, Side::In | Side::Both) {
            for (src_ws, src_id, rel) in access.incoming(&node.workspace, &node.id).await? {
                if !accept(&rel.relation_type) {
                    continue;
                }
                // The reverse entry is the relation seen from the other end: its
                // `target*` fields describe the SOURCE.
                let as_ref = RelationRef::new(
                    node.id.clone(),
                    node.workspace.clone(),
                    node.node_type.clone(),
                    rel.relation_type.clone(),
                    rel.weight,
                );
                let full = FullRelation::from_source_and_ref(
                    src_id.clone(),
                    src_ws.clone(),
                    rel.target_node_type.clone(),
                    &as_ref,
                );
                push((
                    src_ws,
                    src_id,
                    node.workspace.clone(),
                    node.id.clone(),
                    full,
                ));
            }
        }
    }

    // Deterministic: the per-node index is ordered by type then revision, which
    // is not an order a caller can rely on.
    out.sort_by(|a, b| {
        (&a.4.relation_type, &a.0, &a.1, &a.2, &a.3).cmp(&(
            &b.4.relation_type,
            &b.0,
            &b.1,
            &b.2,
            &b.3,
        ))
    });
    Ok(out)
}

/// Find every `(workspace, id)` an id names, with its node type.
///
/// An id is not unique across workspaces, so every workspace is asked. Returns
/// an empty list when the node exists nowhere.
pub async fn resolve_ids<A: RelationAccess>(
    access: &A,
    ids: &[String],
    workspaces: Option<&[String]>,
) -> Result<Vec<SeekNode>> {
    let all;
    let candidates: &[String] = match workspaces {
        Some(w) => w,
        None => {
            all = access.workspaces().await?;
            &all
        }
    };
    let mut found = Vec::new();
    for ws in candidates {
        for id in ids {
            if let Some(node_type) = access.node_type(ws, id).await? {
                found.push(SeekNode {
                    workspace: ws.clone(),
                    id: id.clone(),
                    node_type,
                });
            }
        }
    }
    Ok(found)
}

/// Find every node a path names, one per workspace that has it.
pub async fn resolve_paths<A: RelationAccess>(
    access: &A,
    paths: &[String],
    workspaces: Option<&[String]>,
) -> Result<Vec<SeekNode>> {
    let all;
    let candidates: &[String] = match workspaces {
        Some(w) => w,
        None => {
            all = access.workspaces().await?;
            &all
        }
    };
    let mut found = Vec::new();
    for ws in candidates {
        for path in paths {
            if let Some((id, node_type)) = access.node_by_path(ws, path).await? {
                found.push(SeekNode {
                    workspace: ws.clone(),
                    id,
                    node_type,
                });
            }
        }
    }
    Ok(found)
}

/// An in-memory graph that counts what each access path touches.
///
/// The counters are the point: the pushdown exists to change how many relations
/// a query reads, and a result-only test cannot see that.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    pub struct FakeGraph {
        /// `(workspace, id)` -> node type.
        pub nodes: HashMap<NodeKey, String>,
        /// Live relations: `(src_ws, src_id, tgt_ws, tgt_id, type)`.
        pub relations: Vec<(String, String, String, String, String)>,
        /// Relations the global index handed out.
        pub global_read: AtomicUsize,
        /// Relations the per-node indexes handed out.
        pub node_read: AtomicUsize,
        /// Per-node index calls.
        pub node_calls: AtomicUsize,
        /// Global scans started.
        pub global_scans: AtomicUsize,
        /// Nodes the caller may not read.
        pub hidden: std::collections::HashSet<NodeKey>,
    }

    impl FakeGraph {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn node(&mut self, ws: &str, id: &str, ty: &str) -> &mut Self {
            self.nodes.insert((ws.into(), id.into()), ty.into());
            self
        }

        pub fn rel(&mut self, ws: &str, from: &str, to: &str, ty: &str) -> &mut Self {
            self.relations
                .push((ws.into(), from.into(), ws.into(), to.into(), ty.into()));
            self
        }

        pub fn hide(&mut self, ws: &str, id: &str) -> &mut Self {
            self.hidden.insert((ws.into(), id.into()));
            self
        }

        pub fn type_of(&self, ws: &str, id: &str) -> String {
            self.nodes
                .get(&(ws.to_string(), id.to_string()))
                .cloned()
                .unwrap_or_default()
        }

        pub fn reset(&self) {
            self.global_read.store(0, Ordering::SeqCst);
            self.node_read.store(0, Ordering::SeqCst);
            self.node_calls.store(0, Ordering::SeqCst);
            self.global_scans.store(0, Ordering::SeqCst);
        }

        pub fn read_total(&self) -> usize {
            self.global_read.load(Ordering::SeqCst) + self.node_read.load(Ordering::SeqCst)
        }
    }

    impl RelationAccess for FakeGraph {
        async fn global_scan(&self, relation_type: Option<&str>) -> Result<Vec<ScannedRelation>> {
            self.global_scans.fetch_add(1, Ordering::SeqCst);
            let out: Vec<ScannedRelation> = self
                .relations
                .iter()
                .filter(|r| relation_type.is_none_or(|t| t == r.4))
                .map(|(sw, s, tw, t, ty)| {
                    (
                        sw.clone(),
                        s.clone(),
                        tw.clone(),
                        t.clone(),
                        FullRelation::new(
                            s.clone(),
                            sw.clone(),
                            self.type_of(sw, s),
                            t.clone(),
                            tw.clone(),
                            self.type_of(tw, t),
                            ty.clone(),
                            None,
                        ),
                    )
                })
                .collect();
            self.global_read.fetch_add(out.len(), Ordering::SeqCst);
            Ok(out)
        }

        async fn workspaces(&self) -> Result<Vec<String>> {
            let mut w: Vec<String> = self.nodes.keys().map(|(w, _)| w.clone()).collect();
            w.sort();
            w.dedup();
            Ok(w)
        }

        async fn node_type(&self, workspace: &str, id: &str) -> Result<Option<String>> {
            Ok(self
                .nodes
                .get(&(workspace.to_string(), id.to_string()))
                .cloned())
        }

        async fn node_by_path(
            &self,
            workspace: &str,
            path: &str,
        ) -> Result<Option<(String, String)>> {
            // Test paths are `/<id>`.
            let id = path.trim_start_matches('/');
            Ok(self
                .node_type(workspace, id)
                .await?
                .map(|t| (id.to_string(), t)))
        }

        async fn readable(&self, workspace: &str, id: &str) -> Result<bool> {
            let key = (workspace.to_string(), id.to_string());
            Ok(self.nodes.contains_key(&key) && !self.hidden.contains(&key))
        }

        async fn outgoing(&self, workspace: &str, id: &str) -> Result<Vec<RelationRef>> {
            self.node_calls.fetch_add(1, Ordering::SeqCst);
            let out: Vec<RelationRef> = self
                .relations
                .iter()
                .filter(|r| r.0 == workspace && r.1 == id)
                .map(|r| {
                    RelationRef::new(
                        r.3.clone(),
                        r.2.clone(),
                        self.type_of(&r.2, &r.3),
                        r.4.clone(),
                        None,
                    )
                })
                .collect();
            self.node_read.fetch_add(out.len(), Ordering::SeqCst);
            Ok(out)
        }

        async fn incoming(
            &self,
            workspace: &str,
            id: &str,
        ) -> Result<Vec<(String, String, RelationRef)>> {
            self.node_calls.fetch_add(1, Ordering::SeqCst);
            let out: Vec<(String, String, RelationRef)> = self
                .relations
                .iter()
                .filter(|r| r.2 == workspace && r.3 == id)
                .map(|r| {
                    (
                        r.0.clone(),
                        r.1.clone(),
                        // Seen from the other end: `target*` describe the SOURCE.
                        RelationRef::new(
                            r.1.clone(),
                            r.0.clone(),
                            self.type_of(&r.0, &r.1),
                            r.4.clone(),
                            None,
                        ),
                    )
                })
                .collect();
            self.node_read.fetch_add(out.len(), Ordering::SeqCst);
            Ok(out)
        }
    }
}
