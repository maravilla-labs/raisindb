//! The schema-derived index definitions a write needs, cached OFF the apply
//! hot path (plan Phase 8 step 3).
//!
//! A COMPOUND entry needs the node type's compound declarations (merged along
//! its `extends` chain) and a UNIQUE claim needs the type's `unique: true`
//! property names. Both are a NodeType read — async, and on the replication
//! apply path a read that must not happen inside a batch (the deadlock rule,
//! `jobs/handlers/fulltext/batch.rs:152-161`). So the local write paths
//! [`resolve`] (async, before any batch lock) and the apply path only
//! [`peek`]s: a type that is not cached there is a COLD definition, which the
//! caller answers by marking the workspace's compound indexes `NotBuilt` and
//! requesting a local build (`cold.rs`) — never by skipping silently.
//!
//! **Invalidation.** Every NodeType write (local: `node_types` repository;
//! replicated: the applicator's schema arm), `Event::Schema` and a merge that
//! copied NodeType versions re-resolve the branch from storage
//! (`refresh.rs`), and the cache is registered with `derived_cache_registry`
//! (database-scoped, see `cache.rs`), because a checkpoint ingest copies
//! NodeType records and emits no event. A
//! resolve races an invalidation through a generation counter: an answer read
//! before an invalidation is never stored after it. A refresh that finds a
//! cached type's compound declarations CHANGED marks those indexes `NotBuilt`
//! (entries written under the old declaration cannot be trusted) and requests
//! a build — on every node, replicas included.
//!
//! Keyed by the DATABASE PATH as well as `{tenant, repo, branch, type}`: the
//! cache is process-wide, and two databases in one process (tests, an embedded
//! host) may declare the same type differently.

use super::cache;
use raisin_error::Result;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_storage::{BranchScope, NodeTypeRepository};
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;

/// What one node type contributes to the schema-driven indexes.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TypeIndexDefs {
    /// Compound declarations, own and inherited (most-derived wins by name).
    pub compound: Vec<CompoundIndexDefinition>,
    /// `unique: true` property names of the type itself.
    pub unique: Vec<String>,
}

/// The cached definitions of `node_type`, if resolved since the last
/// invalidation. Never reads storage — safe on the apply path.
pub fn peek(db: &DB, scope: BranchScope<'_>, node_type: &str) -> Option<Arc<TypeIndexDefs>> {
    cache::get(&cache::branch_key(db, scope), node_type)
}

/// The definitions of `node_type`, cache-first. An unknown type resolves to
/// none (no type, no schema-driven entries) and is cached as such.
pub async fn resolve<R: NodeTypeRepository>(
    db: &DB,
    repo: &R,
    scope: BranchScope<'_>,
    node_type: &str,
) -> Result<Arc<TypeIndexDefs>> {
    let key = cache::branch_key(db, scope);
    if let Some(hit) = cache::get(&key, node_type) {
        return Ok(hit);
    }
    let seen = cache::snapshot(&key);
    let defs = Arc::new(load(repo, scope, node_type).await?);
    cache::store(&key, node_type, defs.clone(), seen);
    Ok(defs)
}

/// Every stored NodeType version, not just those at or below the branch
/// HEAD: the cache answers what the NEXT write indexes under, and on a replica
/// a NodeType op lands before the op that advances HEAD over it — a
/// HEAD-bounded read would cache the declaration it replaces.
const NEWEST: raisin_hlc::HLC = raisin_hlc::HLC::new(u64::MAX, u64::MAX);

/// Read and merge one type's definitions (the ONE inheritance walk for
/// compound declarations: parent first, so a derived declaration of the same
/// NAME replaces it; a missing or cyclic parent ends the walk).
pub(super) async fn load<R: NodeTypeRepository>(
    repo: &R,
    scope: BranchScope<'_>,
    node_type: &str,
) -> Result<TypeIndexDefs> {
    const MAX_DEPTH: usize = 20;
    let Some(own) = repo.get(scope, node_type, Some(&NEWEST)).await? else {
        return Ok(TypeIndexDefs::default());
    };
    let unique = crate::repositories::nodes::extract_unique_property_names(&own);
    let mut chain: Vec<Vec<CompoundIndexDefinition>> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut current: Option<NodeType> = Some(own);
    while let Some(nt) = current {
        if chain.len() >= MAX_DEPTH || !seen.insert(nt.name.clone()) {
            break;
        }
        chain.push(nt.compound_indexes.clone().unwrap_or_default());
        current = match nt.extends.as_deref() {
            Some(parent) if !parent.is_empty() => {
                repo.get(scope, parent, Some(&NEWEST)).await.ok().flatten()
            }
            _ => None,
        };
    }
    let mut compound: Vec<CompoundIndexDefinition> = Vec::new();
    for level in chain.into_iter().rev() {
        for idx in level {
            match compound.iter_mut().find(|e| e.name == idx.name) {
                Some(existing) => *existing = idx,
                None => compound.push(idx),
            }
        }
    }
    Ok(TypeIndexDefs { compound, unique })
}

pub use super::refresh::{fresh_branch, refresh_branch, warm_branch};

/// The index NAMES whose declaration differs between `before` and `after`:
/// added, removed, or changed (by `definition_hash`). The ONE comparison —
/// the local NodeType write and the cache refresh both mark these `NotBuilt`.
pub fn changed_index_names(
    before: &[CompoundIndexDefinition],
    after: &[CompoundIndexDefinition],
) -> Vec<String> {
    let hash = |defs: &[CompoundIndexDefinition], name: &str| {
        defs.iter()
            .find(|d| d.name == name)
            .map(CompoundIndexDefinition::definition_hash)
    };
    let mut names: Vec<String> = before
        .iter()
        .chain(after)
        .map(|d| d.name.clone())
        .filter(|name| hash(before, name) != hash(after, name))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Drop every cached type of one branch (a branch created or deleted under
/// its name, a refresh that failed).
pub fn invalidate_branch(db: &DB, tenant_id: &str, repo_id: &str, branch: &str) {
    cache::invalidate(&cache::branch_key(
        db,
        BranchScope::new(tenant_id, repo_id, branch),
    ));
}

pub use super::cache::{invalidate_all, invalidate_database};

/// The definitions of a known set of node types, resolved up front so the
/// write that uses them can run synchronously (under a batch lock, or on the
/// apply path).
#[derive(Debug, Default, Clone)]
pub struct DefsSet(HashMap<String, Arc<TypeIndexDefs>>);

impl DefsSet {
    /// Resolve every type in `types` (local write paths; async, cache-first).
    pub async fn resolve<R: NodeTypeRepository>(
        db: &DB,
        repo: &R,
        scope: BranchScope<'_>,
        types: &[&str],
    ) -> Result<Self> {
        let mut out = HashMap::new();
        for name in types {
            if !out.contains_key(*name) {
                out.insert(name.to_string(), resolve(db, repo, scope, name).await?);
            }
        }
        Ok(Self(out))
    }

    /// Every type in `types` from the cache, or `None` when ANY is cold (the
    /// apply path: a partial set would skip one side's entries silently).
    pub fn peek<'t>(
        db: &DB,
        scope: BranchScope<'_>,
        types: impl IntoIterator<Item = &'t str>,
    ) -> Option<Self> {
        let mut out = HashMap::new();
        for name in types {
            if !out.contains_key(name) {
                out.insert(name.to_string(), peek(db, scope, name)?);
            }
        }
        Some(Self(out))
    }

    /// A set built from known definitions (tests, a build that resolved them).
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, Arc<TypeIndexDefs>)>) -> Self {
        Self(pairs.into_iter().collect())
    }

    /// The compound declarations of `node_type`. A type the set was not built
    /// for has none — callers build the set from exactly the types they write.
    pub fn compound(&self, node_type: &str) -> &[CompoundIndexDefinition] {
        match self.0.get(node_type) {
            Some(defs) => &defs.compound,
            None => {
                debug_assert!(false, "DefsSet missing type {node_type}");
                &[]
            }
        }
    }

    /// The `unique: true` property names of `node_type`.
    pub fn unique(&self, node_type: &str) -> &[String] {
        self.0
            .get(node_type)
            .map(|d| d.unique.as_slice())
            .unwrap_or(&[])
    }

    /// Whether any type in the set declares a compound index.
    pub fn any_compound(&self) -> bool {
        self.0.values().any(|d| !d.compound.is_empty())
    }
}
