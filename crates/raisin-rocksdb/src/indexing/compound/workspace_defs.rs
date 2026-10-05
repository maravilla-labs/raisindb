//! WORKSPACE-owned compound declarations (plan Phase 13e), as the writers and
//! the builds see them.
//!
//! A workspace index covers EVERY node of its workspace, whatever its type,
//! so the one writer (`writer.rs`), the delete tombstoner and the builds add
//! these to the node type's declarations for every node they index — no
//! caller has to remember to.
//!
//! **No cold state.** Unlike a NodeType (an async resolve the apply path must
//! not perform), a workspace record is one synchronous point read from
//! `cf::WORKSPACES`, safe on the apply path and under the commit lock. So the
//! declarations are always known: [`current`] reads the record's BYTES on
//! every call and decodes only when they differ from what it decoded last.
//! That is what makes the cache correct for every writer of the record — the
//! repository, the replication applicator, a transaction, a backup import, a
//! checkpoint ingest — without an invalidation hook in each: it detects the
//! change from the data.
//!
//! **Declaration changes fail the index closed.** Whenever the bytes changed
//! (and on the first read in a process), the workspace's state records are
//! RECONCILED against the declarations: a workspace index whose record is not
//! `NotBuilt` and that is no longer declared, or declared with a different
//! hash, is marked `NotBuilt` (generation advanced, so a build in flight under
//! the old declaration loses its compare-and-set) and a local build is
//! requested for a changed one. The reconcile runs BEFORE the read returns,
//! so no entry is written under the new declaration while a record still
//! vouches for the old one. Removal matters as much as change: an index that
//! is no longer declared stops being maintained, and a record still saying
//! `Ready` would be trusted again the moment the same declaration returns.
//!
//! The reconcile runs before entries are DERIVED under the new declaration,
//! but a write derives when it stages and commits later. So the reader also
//! keeps a process-wide sequence of observed changes ([`change_seq`]): a
//! write records it before reading, and its commit asks [`changed_since`]
//! (`property_delta::staged_declarations`). And a build stamps `Ready` only
//! while the stored record still declares what it built
//! (`compound_state::build_cas`, through [`stored`], which never reconciles).

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_models::workspace::Workspace;
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

/// `{database path, tenant, repo, workspace}`.
type Key = (String, String, String, String);

struct Entry {
    /// The record bytes the declarations were decoded from (`None`: no record).
    bytes: Option<Vec<u8>>,
    defs: Arc<[CompoundIndexDefinition]>,
    /// [`change_seq`] when this process last saw the declarations CHANGE
    /// (0: unchanged since it first read them).
    changed_at: u64,
}

/// Process-wide sequence of observed declaration changes (any workspace).
static CHANGES: AtomicU64 = AtomicU64::new(0);

fn cache() -> &'static RwLock<HashMap<Key, Entry>> {
    static CACHE: OnceLock<RwLock<HashMap<Key, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The compound indexes `workspace` owns (stored names, owner stamped), as of
/// the record stored now.
pub fn current(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
) -> Result<Arc<[CompoundIndexDefinition]>> {
    let cf = crate::cf_handle(db, crate::cf::WORKSPACES)?;
    let record_key = crate::keys::workspace_key(tenant_id, repo_id, workspace);
    let stored = db
        .get_pinned_cf(cf, &record_key)
        .map_err(|e| Error::storage(format!("Failed to read workspace record: {e}")))?;
    let key: Key = (
        db.path().to_string_lossy().into_owned(),
        tenant_id.to_string(),
        repo_id.to_string(),
        workspace.to_string(),
    );
    let previous = match cache().read() {
        Ok(cache) => match cache.get(&key) {
            Some(entry) if entry.bytes.as_deref() == stored.as_deref() => {
                return Ok(entry.defs.clone());
            }
            Some(entry) => Some((entry.defs.clone(), entry.changed_at)),
            None => None,
        },
        Err(_) => None,
    };
    let defs: Arc<[CompoundIndexDefinition]> =
        Arc::from(decode(stored.as_deref(), tenant_id, repo_id, workspace)?);
    reconcile(db, tenant_id, repo_id, workspace, &defs)?;
    // A first read has no history to compare with: nothing staged in this
    // process can predate it (every writer reads before it stages).
    let changed_at = match previous {
        Some((before, at)) if before[..] == defs[..] => at,
        Some(_) => CHANGES.fetch_add(1, Ordering::SeqCst) + 1,
        None => 0,
    };
    if let Ok(mut cache) = cache().write() {
        cache.insert(
            key,
            Entry {
                bytes: stored.as_deref().map(<[u8]>::to_vec),
                defs: defs.clone(),
                changed_at,
            },
        );
    }
    Ok(defs)
}

/// The declarations in a stored workspace record (`None`: no record) — THE
/// one decode, shared by [`current`] and [`stored`].
fn decode(
    bytes: Option<&[u8]>,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
) -> Result<Vec<CompoundIndexDefinition>> {
    let Some(bytes) = bytes else {
        return Ok(Vec::new());
    };
    let ws: Workspace = rmp_serde::from_slice(bytes).map_err(|e| {
        // Unknown declarations: refuse rather than index without them.
        Error::storage(format!(
            "workspace record {tenant_id}/{repo_id}/{workspace} cannot be decoded \
             ({e}); its compound indexes are unknown"
        ))
    })?;
    Ok(ws.owned_compound_indexes())
}

/// The declarations stored NOW, read and decoded without the cache and
/// WITHOUT reconciling — for the compound state transitions, which run under
/// the transition lock the reconcile takes (`compound_state::marker`).
pub fn stored(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
) -> Result<Vec<CompoundIndexDefinition>> {
    let cf = crate::cf_handle(db, crate::cf::WORKSPACES)?;
    let record_key = crate::keys::workspace_key(tenant_id, repo_id, workspace);
    let bytes = db
        .get_pinned_cf(cf, &record_key)
        .map_err(|e| Error::storage(format!("Failed to read workspace record: {e}")))?;
    decode(bytes.as_deref(), tenant_id, repo_id, workspace)
}

/// The current position of the declaration-change sequence. A write records
/// it BEFORE it reads the declarations it derives entries from, and asks
/// [`changed_since`] at its commit.
pub fn change_seq() -> u64 {
    CHANGES.load(Ordering::SeqCst)
}

/// Whether `workspace`'s declarations changed after `seq` ([`change_seq`])
/// — reading the record now, so a change no reader has seen yet counts (and
/// is reconciled on the way).
pub fn changed_since(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
    seq: u64,
) -> Result<bool> {
    current(db, tenant_id, repo_id, workspace)?;
    let key: Key = (
        db.path().to_string_lossy().into_owned(),
        tenant_id.to_string(),
        repo_id.to_string(),
        workspace.to_string(),
    );
    Ok(match cache().read() {
        Ok(cache) => cache.get(&key).is_none_or(|entry| entry.changed_at > seq),
        // Unknown: assume changed (fail closed costs a rebuild, never a row).
        Err(_) => true,
    })
}

/// [`current`] for the workspace an [`IndexCtx`](crate::indexing::IndexCtx)
/// writes into.
pub fn for_ctx(
    db: &DB,
    ctx: &crate::indexing::IndexCtx<'_>,
) -> Result<Arc<[CompoundIndexDefinition]>> {
    current(db, ctx.tenant_id, ctx.repo_id, ctx.workspace)
}

/// Fail closed every workspace-index state record of `workspace` (any
/// branch) that `declared` no longer vouches for, and request a build for
/// each one still declared.
fn reconcile(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
    declared: &[CompoundIndexDefinition],
) -> Result<()> {
    // Marked on the RECORD, under the transition lock; state readers are
    // per-statement store instances, so the next plan reads the mark.
    let rebuild = crate::compound_state::reconcile_workspace_declarations(
        db, tenant_id, repo_id, workspace, declared,
    )?;
    for branch in rebuild {
        super::cold::request_build(db, tenant_id, repo_id, &branch, workspace);
    }
    Ok(())
}

/// Called right after a workspace record write (repository, replicated
/// apply): reconcile now, instead of at the next node write — so a build in
/// flight under the old declaration loses its compare-and-set as early as
/// possible. Failures are logged: the record write succeeded, and the next
/// reader reconciles again (the cache was not updated).
pub fn declarations_written(db: &DB, tenant_id: &str, repo_id: &str, workspace: &str) {
    if let Err(e) = current(db, tenant_id, repo_id, workspace) {
        tracing::warn!(
            tenant = %tenant_id,
            repo = %repo_id,
            workspace = %workspace,
            error = %e,
            "could not reconcile workspace compound declarations after a workspace write"
        );
    }
}

/// Drop every cached workspace (tests; a full invalidation).
pub fn invalidate_all() {
    if let Ok(mut cache) = cache().write() {
        cache.clear();
    }
}
