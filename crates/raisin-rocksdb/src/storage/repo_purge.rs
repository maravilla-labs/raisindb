//! Remove EVERY RocksDB key a repository owns.
//!
//! `delete_repository` used to delete the registry entry and nothing else. The
//! repository's nodes, revisions, branches, translations, embeddings, indexes
//! and jobs stayed, so a repository recreated under the same id came back
//! populated with the old one's data under the new configuration.
//!
//! # The registry below is exhaustive, and a test enforces it
//!
//! Every column family in [`crate::all_column_families`] is classified here
//! exactly once — how its keys name a repository, or why they do not. The
//! layouts are quoted from the key builders (the same ones
//! `repositories/branches/cf_registry.rs` documents for branch forks). Adding a
//! column family without deciding how a repository delete treats it fails
//! `every_column_family_is_classified_for_repository_purge`, because a CF that
//! is silently skipped here is data that silently survives a delete.
//!
//! # Prefix hygiene
//!
//! A repository is removed by `{tenant}\0{repo}\0` .. `{tenant}\0{repo}\x01`,
//! never by `{tenant}\0{repo}` alone: that would also take `{repo}2`. Keys
//! that END at the repository id (no trailing separator) are deleted by exact
//! key in addition to the range.

use std::collections::HashSet;
use std::sync::Arc;

use raisin_error::Result;
use rocksdb::{WriteBatch, DB};

use crate::cf;

/// How one column family's keys identify a repository.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RepoScope {
    /// `{tenant}\0{repo}\0…`, and possibly exactly `{tenant}\0{repo}`.
    /// Removed by range delete plus the exact key.
    RepoPrefixed(&'static str),
    /// A leading kind segment first: `{kind}\0{tenant}\0{repo}\0…`, and/or
    /// `{tenant}\0{repo}\0…`. Small bookkeeping CFs; removed by a scan that
    /// matches either position.
    MixedLayout(&'static str),
    /// `{tenant}\0repos\0{repo}` (the registry entry itself) and anything
    /// under it; the other registry keys belong to tenants or deployments.
    Registry,
    /// `{tenant}:{repo}:{resource_type}:{name}` — colon separated.
    ColonSeparated,
    /// `{tenant}\0{job_id}` — the repository is in the stored `JobContext`,
    /// not the key. Removed per job, with its metadata and history record.
    JobByContext,
    /// `{state}:{job_id}` job queues whose VALUE names tenant and repository.
    QueuedJobByValue,
    /// Not repository data. The `&str` says why.
    NotRepoScoped(&'static str),
}

/// The exhaustive classification of every column family for a repository
/// delete. See the module docs.
pub(crate) const REPO_CF_REGISTRY: &[(&str, RepoScope)] = &[
    // ---- {tenant}\0{repo}\0{branch}\0… : the node store and its indexes ----
    (
        cf::NODES,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0nodes\0{id}\0{~rev}"),
    ),
    (
        cf::PATH_INDEX,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0path\0…"),
    ),
    (
        cf::NODE_PATH,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0node_path\0…"),
    ),
    (
        cf::PROPERTY_INDEX,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0prop…"),
    ),
    (
        cf::REFERENCE_INDEX,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0ref…"),
    ),
    (
        cf::RELATION_INDEX,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0…rel…"),
    ),
    (
        cf::ORDER_INDEX,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0…"),
    ),
    (
        cf::ORDERED_CHILDREN,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0ordered\0…"),
    ),
    (
        cf::SPATIAL_INDEX,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0geo\0…"),
    ),
    (
        cf::COMPOUND_INDEX,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0cidx…"),
    ),
    (
        cf::UNIQUE_INDEX,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0uniq\0…"),
    ),
    (
        cf::LOCALIZED_NAME_INDEX,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0lname{_of}\0…"),
    ),
    (
        cf::NODE_DELETES,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0{node_id}\0{~rev}"),
    ),
    (
        cf::EMBEDDINGS,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0…"),
    ),
    (
        cf::SECRETS,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{name}\0{~rev}"),
    ),
    (
        cf::AUDIT_LOG,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0{node}\0…"),
    ),
    (
        cf::WORKSPACE_DELTAS,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0delta\0…"),
    ),
    (
        cf::PENDING_BATCH_OPS,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{nanos}\0{uuid}"),
    ),
    (
        cf::GRAPH_CACHE,
        RepoScope::RepoPrefixed("{t}\0{r}\0graph_cache\0{branch}\0…"),
    ),
    (
        cf::GRAPH_PROJECTION,
        RepoScope::RepoPrefixed("{t}\0{r}\0graph_projection\0{branch}\0…"),
    ),
    // ---- schema ----------------------------------------------------------
    (
        cf::NODE_TYPES,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0nodetypes\0…"),
    ),
    (
        cf::ARCHETYPES,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0archetypes\0…"),
    ),
    (
        cf::ELEMENT_TYPES,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0element_types\0…"),
    ),
    // ---- translations ----------------------------------------------------
    (
        cf::TRANSLATION_DATA,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0translations\0…"),
    ),
    (
        cf::BLOCK_TRANSLATIONS,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0block_trans\0…"),
    ),
    (
        cf::TRANSLATION_INDEX,
        RepoScope::RepoPrefixed("{t}\0{r}\0translation_index\0…"),
    ),
    (
        cf::TRANSLATION_HASHES,
        RepoScope::RepoPrefixed("{t}\0{r}\0{branch}\0{ws}\0…"),
    ),
    // ---- repository-level versioning -------------------------------------
    (
        cf::WORKSPACES,
        RepoScope::RepoPrefixed("{t}\0{r}\0workspaces\0{ws}"),
    ),
    (
        cf::BRANCHES,
        RepoScope::RepoPrefixed("{t}\0{r}\0branches\0{name}"),
    ),
    (cf::TAGS, RepoScope::RepoPrefixed("{t}\0{r}\0tags\0{tag}")),
    (
        cf::REVISIONS,
        RepoScope::RepoPrefixed("{t}\0{r}\0revisions|…"),
    ),
    (
        cf::TREES,
        RepoScope::RepoPrefixed("{t}\0{r}\0trees\0{hash}"),
    ),
    (
        cf::VERSIONS,
        RepoScope::RepoPrefixed("{t}\0{r}\0versions\0{node}\0{v}"),
    ),
    (
        cf::PROCESSING_RULES,
        RepoScope::RepoPrefixed("{t}\0{r} — the whole key"),
    ),
    // The repository's own replication log. Peers learn of the delete from
    // the DeleteRepository operation captured with it, not from these.
    (
        cf::OPERATION_LOG,
        RepoScope::RepoPrefixed("{t}\0{r}\0{cluster_node}\0{seq}\0{ts}"),
    ),
    // ---- mixed layouts ---------------------------------------------------
    // The kind-first records are the `INDEX_STATUS_KINDS` below.
    (
        cf::INDEX_STATUS,
        RepoScope::MixedLayout(
            "prop_index|compound_index|spatial_index\0{t}\0{r}\0… and {t}\0{r}\0{branch}\0…",
        ),
    ),
    // ---- special ---------------------------------------------------------
    (cf::REGISTRY, RepoScope::Registry),
    (cf::SYSTEM_UPDATE_HASHES, RepoScope::ColonSeparated),
    (cf::JOB_DATA, RepoScope::JobByContext),
    (cf::JOB_METADATA, RepoScope::JobByContext),
    (cf::FULLTEXT_JOBS, RepoScope::QueuedJobByValue),
    (cf::EMBEDDING_JOBS, RepoScope::QueuedJobByValue),
    // ---- not repository data ---------------------------------------------
    (
        cf::APPLIED_OPS,
        RepoScope::NotRepoScoped(
            "applied_ops/{op_id}: this cluster node's idempotency record of which \
             operations it has applied. Keeping it is what stops a replayed \
             operation of the deleted repository from being applied again.",
        ),
    ),
    (
        cf::IDENTITIES,
        RepoScope::NotRepoScoped("tenant-wide identities"),
    ),
    (
        cf::IDENTITY_EMAIL_INDEX,
        RepoScope::NotRepoScoped("tenant-wide identity index"),
    ),
    (
        cf::SESSIONS,
        RepoScope::NotRepoScoped("tenant-wide sessions"),
    ),
    (
        cf::ADMIN_USERS,
        RepoScope::NotRepoScoped("sys\0{t}\0users\0… — tenant admins"),
    ),
    (
        cf::TENANT_AI_CONFIG,
        RepoScope::NotRepoScoped("{t} — tenant configuration"),
    ),
    (
        cf::TENANT_AUTH_CONFIG,
        RepoScope::NotRepoScoped("{t} — tenant configuration"),
    ),
    (
        cf::TENANT_EMBEDDING_CONFIG,
        RepoScope::NotRepoScoped("{t} — tenant configuration"),
    ),
    (
        cf::QUERY_EMBEDDINGS,
        RepoScope::NotRepoScoped("EMBEDDING() result cache keyed by query text, not by repository"),
    ),
];

/// What a repository purge touched.
#[derive(Debug, Clone, Default)]
pub struct RepoPurgeReport {
    /// Column families processed.
    pub cfs_purged: usize,
    /// Jobs removed (metadata, context and history).
    pub jobs_removed: usize,
    /// Column families that failed; the purge continues past them.
    pub failed: Vec<String>,
}

/// Remove every key of `(tenant, repo)` from every column family.
///
/// Idempotent. Continues past a failing column family and reports it, so one
/// bad CF cannot leave the rest of the repository behind.
pub(crate) fn purge_repository_keys(db: &Arc<DB>, tenant: &str, repo: &str) -> RepoPurgeReport {
    let mut report = RepoPurgeReport::default();
    for (name, scope) in REPO_CF_REGISTRY {
        let outcome = match scope {
            RepoScope::RepoPrefixed(_) => purge_prefixed(db, name, tenant, repo),
            RepoScope::MixedLayout(_) => purge_by_segments(db, name, tenant, repo),
            RepoScope::Registry => purge_registry(db, tenant, repo),
            RepoScope::ColonSeparated => purge_colon_separated(db, name, tenant, repo),
            RepoScope::JobByContext => {
                // JOB_DATA and JOB_METADATA are purged together, once.
                if *name == cf::JOB_DATA {
                    purge_jobs(db, tenant, repo).map(|n| report.jobs_removed += n)
                } else {
                    Ok(())
                }
            }
            RepoScope::QueuedJobByValue => purge_queued_jobs(db, name, tenant, repo),
            RepoScope::NotRepoScoped(_) => continue,
        };
        match outcome {
            Ok(()) => report.cfs_purged += 1,
            Err(e) => {
                tracing::error!(cf = %name, tenant, repo, error = %e, "repository purge failed for a column family");
                report.failed.push((*name).to_string());
            }
        }
    }
    report
}

fn handle<'a>(db: &'a DB, name: &str) -> Result<&'a rocksdb::ColumnFamily> {
    crate::cf_handle(db, name)
}

fn storage_err(e: impl std::fmt::Display) -> raisin_error::Error {
    raisin_error::Error::storage(e.to_string())
}

/// `{tenant}\0{repo}` — the bare repository key, with no trailing separator.
fn repo_key(tenant: &str, repo: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(tenant.len() + repo.len() + 1);
    k.extend_from_slice(tenant.as_bytes());
    k.push(0);
    k.extend_from_slice(repo.as_bytes());
    k
}

fn purge_range(db: &DB, name: &str, lo: &[u8], hi: &[u8]) -> Result<()> {
    let cf = handle(db, name)?;
    db.delete_range_cf(cf, lo, hi).map_err(storage_err)?;
    db.compact_range_cf(cf, Some(lo), Some(hi));
    Ok(())
}

fn purge_prefixed(db: &DB, name: &str, tenant: &str, repo: &str) -> Result<()> {
    let base = repo_key(tenant, repo);
    let mut lo = base.clone();
    lo.push(0);
    let mut hi = base.clone();
    hi.push(1);
    purge_range(db, name, &lo, &hi)?;
    db.delete_cf(handle(db, name)?, &base).map_err(storage_err)
}

/// Keys whose `\0`-separated segments hold `tenant, repo` either first or
/// right after a leading kind segment.
fn purge_by_segments(db: &DB, name: &str, tenant: &str, repo: &str) -> Result<()> {
    let cf = handle(db, name)?;
    let mut batch = WriteBatch::default();
    let mut it = db.raw_iterator_cf(cf);
    it.seek_to_first();
    while it.valid() {
        if let Some(key) = it.key() {
            let mut parts = key.split(|b| *b == 0);
            let first = parts.next();
            let second = parts.next();
            let third = parts.next();
            let matches = (first == Some(tenant.as_bytes()) && second == Some(repo.as_bytes()))
                || (second == Some(tenant.as_bytes()) && third == Some(repo.as_bytes()));
            if matches {
                batch.delete_cf(cf, key);
            }
        }
        it.next();
    }
    it.status().map_err(storage_err)?;
    db.write(batch).map_err(storage_err)
}

/// The leading kind segments of the KIND-FIRST state records in a
/// [`RepoScope::MixedLayout`] column family (`cf::INDEX_STATUS`):
/// `{kind}\0{tenant}\0{repo}\0…`. Every other record there is tenant-first.
///
/// A new kind-first record kind MUST be added here, or a tenant wipe leaves it
/// behind: a whole-tenant wipe cannot match "any first segment, tenant second"
/// without also matching another tenant's repository that happens to share
/// this tenant's name.
pub(crate) const INDEX_STATUS_KINDS: &[&str] = &["prop_index", "compound_index", "spatial_index"];

/// Remove every kind-first record of `tenant` (all repositories) from a
/// mixed-layout column family: one range per kind,
/// `{kind}\0{tenant}\0` .. `{kind}\0{tenant}\x01`. The tenant-first records
/// are the caller's ordinary `{tenant}\0` range.
pub(crate) fn purge_mixed_layout_tenant(db: &DB, name: &str, tenant: &str) -> Result<()> {
    for kind in INDEX_STATUS_KINDS {
        let mut lo = Vec::with_capacity(kind.len() + tenant.len() + 2);
        lo.extend_from_slice(kind.as_bytes());
        lo.push(0);
        lo.extend_from_slice(tenant.as_bytes());
        let mut hi = lo.clone();
        lo.push(0);
        hi.push(1);
        purge_range(db, name, &lo, &hi)?;
    }
    Ok(())
}

/// The column families a [`RepoScope::MixedLayout`] classifies.
pub(crate) fn mixed_layout_cfs() -> impl Iterator<Item = &'static str> {
    REPO_CF_REGISTRY
        .iter()
        .filter(|(_, scope)| matches!(scope, RepoScope::MixedLayout(_)))
        .map(|(name, _)| *name)
}

fn purge_registry(db: &DB, tenant: &str, repo: &str) -> Result<()> {
    let key = crate::keys::repository_key(tenant, repo);
    let mut lo = key.clone();
    lo.push(0);
    let mut hi = key.clone();
    hi.push(1);
    purge_range(db, cf::REGISTRY, &lo, &hi)?;
    db.delete_cf(handle(db, cf::REGISTRY)?, &key)
        .map_err(storage_err)
}

fn purge_colon_separated(db: &DB, name: &str, tenant: &str, repo: &str) -> Result<()> {
    let lo = format!("{tenant}:{repo}:").into_bytes();
    let hi = format!("{tenant}:{repo};").into_bytes();
    purge_range(db, name, &lo, &hi)
}

/// Every job whose stored context names this repository, with its metadata
/// and history record. Returns how many were removed.
fn purge_jobs(db: &Arc<DB>, tenant: &str, repo: &str) -> Result<usize> {
    let cf_data = handle(db, cf::JOB_DATA)?;
    let prefix = crate::keys::job_tenant_prefix(tenant);
    let mut job_ids = Vec::new();
    for item in crate::prefix_scan(db, cf_data, &prefix) {
        let (key, value) = item.map_err(storage_err)?;
        let Ok(context) = rmp_serde::from_slice::<raisin_storage::jobs::JobContext>(&value) else {
            continue;
        };
        if context.repo_id == repo {
            let id = String::from_utf8_lossy(&key[prefix.len()..]).to_string();
            job_ids.push(raisin_storage::jobs::JobId::from_string(id));
        }
    }
    let metadata = crate::jobs::JobMetadataStore::new(db.clone());
    for job_id in &job_ids {
        metadata.delete(tenant, job_id)?;
    }
    Ok(job_ids.len())
}

/// Queued fulltext / embedding jobs of this repository.
fn purge_queued_jobs(db: &DB, name: &str, tenant: &str, repo: &str) -> Result<()> {
    let cf = handle(db, name)?;
    let mut batch = WriteBatch::default();
    let mut it = db.raw_iterator_cf(cf);
    it.seek_to_first();
    while it.valid() {
        if let (Some(key), Some(value)) = (it.key(), it.value()) {
            let owner = if name == cf::FULLTEXT_JOBS {
                rmp_serde::from_slice::<raisin_storage::FullTextIndexJob>(value)
                    .ok()
                    .map(|j| (j.tenant_id, j.repo_id))
            } else {
                rmp_serde::from_slice::<raisin_embeddings::EmbeddingJob>(value)
                    .ok()
                    .map(|j| (j.tenant_id, j.repo_id))
            };
            if owner.is_some_and(|(t, r)| t == tenant && r == repo) {
                batch.delete_cf(cf, key);
            }
        }
        it.next();
    }
    it.status().map_err(storage_err)?;
    db.write(batch).map_err(storage_err)
}

/// The column families the registry names, for the exhaustiveness test.
#[cfg(test)]
fn classified() -> HashSet<&'static str> {
    REPO_CF_REGISTRY.iter().map(|(name, _)| *name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE GUARD: a column family nobody classified is data a repository
    /// delete silently leaves behind.
    #[test]
    fn every_column_family_is_classified_for_repository_purge() {
        let classified = classified();
        let missing: Vec<&str> = crate::all_column_families()
            .into_iter()
            .filter(|cf| !classified.contains(cf))
            .collect();
        assert!(
            missing.is_empty(),
            "column families not classified in REPO_CF_REGISTRY: {missing:?}. Read the key \
             builder and decide how a repository delete removes its keys (or why it must not)."
        );
        let mut seen = HashSet::new();
        for (name, _) in REPO_CF_REGISTRY {
            assert!(
                crate::all_column_families().contains(name),
                "{name} is not a real CF"
            );
            assert!(seen.insert(*name), "{name} is listed twice");
        }
    }
}
