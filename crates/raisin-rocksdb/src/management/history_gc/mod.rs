//! Revision-history garbage collection.
//!
//! # Why the database grew without bound
//!
//! Every write is a new MVCC version: a new `(entity, revision)` key in
//! `nodes`, and new versions of every index row it touches (property, path,
//! reference, ordering, translation, embedding, snapshot). Nothing ever
//! removed the old ones — the repository GC was a stub, the background
//! "revision compaction" was never started, and the RocksDB-level compaction
//! that *was* running can only reclaim keys that were deleted. A key that is
//! merely old is live data to RocksDB, so compaction dropped almost nothing.
//! An overwrite deploy (every node rewritten) or a 10-minute data tick
//! therefore added a full copy of the affected nodes and indexes, forever.
//!
//! # What this removes
//!
//! For each branch, a [`HistoryRetention`] policy yields a *cutoff* revision.
//! Within each group of versions of one entity, GC keeps:
//!
//! * every version newer than the cutoff;
//! * the newest version at or before the cutoff — the state as of the cutoff,
//!   which every read at a retained revision resolves to;
//! * the newest version at or before each *pinned* revision: tag revisions and
//!   branch fork points (`created_from`), so tags stay readable and merges keep
//!   their merge base.
//!
//! Everything else is deleted. Reads at the head, at any revision after the
//! cutoff, and at any pinned revision return exactly what they did before.
//! Only reads between pins before the cutoff — "time travel" past the
//! retention window — see the cutoff state instead.
//!
//! Where the group is the unit a reader resolves (a node id, a path), an
//! entity whose only surviving version is a tombstone is removed entirely:
//! "absent" and "deleted" read the same. See
//! [`layout::GcTarget::drop_orphan_tombstones`] for why index families keep it.
//!
//! Afterwards the touched column families are compacted with a forced
//! bottommost pass, so the space is actually returned rather than waiting for
//! RocksDB to pick the files up on its own.
//!
//! # What else a run reclaims
//!
//! * Job results persisted before results were bounded are bounded in place
//!   (`GcOptions::bound_job_results`).
//! * Upload blobs nothing mentions any more — superseded package `.rap`
//!   files, assets whose history was pruned by an earlier run — are swept
//!   from the binary store ([`run_gc_and_sweep_blobs`]).
//!
//! # Reading the numbers
//!
//! `bytes_deleted` is logical (uncompressed key + value), so it is not the
//! SST bytes returned: most pruned versions are small index rows. Versions
//! still inside the retention window are counted as `versions_retained` —
//! when a database's history is younger than the window, that is where it
//! is, and a run reclaims little by design.

mod blobs;
pub mod collapse;
mod layout;
mod node_delete_translations;
mod pins;
pub mod retention;
mod sweep;
#[cfg(test)]
mod tests;
mod translation_floor;

pub use retention::{HistoryRetention, ALL_BRANCHES};

use crate::{cf, cf_handle, RocksDBStorage};
use layout::{GcTarget, Located, GC_TARGETS};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

/// Deletes per write batch.
const BATCH_DELETES: usize = 20_000;

/// A chunk (keys sharing everything before the revision) larger than this is
/// left untouched rather than held in memory. Keeping is always safe.
const MAX_CHUNK_ENTRIES: usize = 2_000_000;

/// What to collect and how.
#[derive(Debug, Clone)]
pub struct GcOptions {
    /// Report what would be removed without removing anything.
    pub dry_run: bool,
    /// Policy for branches with no stored policy of their own.
    pub default_retention: HistoryRetention,
    /// Use this policy for every branch in scope, ignoring stored ones.
    pub retention_override: Option<HistoryRetention>,
    /// Restrict to one tenant (and optionally one repository of it).
    pub tenant: Option<String>,
    pub repo: Option<String>,
    /// Nothing younger than this is ever pruned, whatever the policy says —
    /// in-flight reads and jobs hold revisions from the last few minutes.
    pub min_age: Duration,
    /// Also purge the replication operation log (only correct when
    /// replication is off: nothing will ever read it).
    pub purge_oplog: bool,
    /// Also delete finished jobs older than this.
    pub job_retention: Option<Duration>,
    /// Report binary blobs that only pruned versions referenced.
    pub collect_orphaned_blobs: bool,
    /// Bound the stored results of finished jobs written before results were
    /// bounded (see `jobs::metadata_store::MAX_PERSISTED_RESULT_BYTES`).
    pub bound_job_results: bool,
    /// Also delete every blob the binary store lists that nothing in the
    /// database mentions, not just the ones pruned history named.
    pub sweep_unreferenced_blobs: bool,
    /// A listed blob younger than this is never swept: an upload is stored
    /// before the node that names it is written.
    pub blob_min_age: Duration,
    /// Compact the touched column families afterwards.
    pub compact: bool,
}

impl Default for GcOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            default_retention: HistoryRetention::KEEP_ALL,
            retention_override: None,
            tenant: None,
            repo: None,
            min_age: Duration::from_secs(600),
            purge_oplog: false,
            job_retention: None,
            collect_orphaned_blobs: true,
            bound_job_results: true,
            sweep_unreferenced_blobs: true,
            blob_min_age: Duration::from_secs(24 * 3600),
            compact: true,
        }
    }
}

/// Per-column-family outcome.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CfGcStats {
    pub keys_scanned: u64,
    pub versions_deleted: u64,
    pub bytes_deleted: u64,
    /// Superseded versions kept because they are newer than the cutoff (or
    /// pinned): history the retention policy still holds.
    pub versions_retained: u64,
    pub bytes_retained: u64,
    pub live_sst_bytes_before: u64,
    pub live_sst_bytes_after: u64,
}

/// The cutoff GC applied to one branch.
#[derive(Debug, Clone, Serialize)]
pub struct BranchGcPlan {
    pub tenant: String,
    pub repo: String,
    pub branch: String,
    pub retention: HistoryRetention,
    /// `None` when the policy keeps everything on this branch.
    pub cutoff: Option<String>,
    pub pinned_revisions: usize,
}

/// Everything a GC run did (or, dry, would do).
#[derive(Debug, Clone, Default, Serialize)]
pub struct GcReport {
    pub dry_run: bool,
    pub branches: Vec<BranchGcPlan>,
    pub column_families: BTreeMap<String, CfGcStats>,
    pub versions_deleted: u64,
    /// Logical bytes (key + value) of the deleted versions.
    pub bytes_deleted: u64,
    pub oplog_entries_deleted: u64,
    pub oplog_bytes_deleted: u64,
    /// Superseded versions the policy still retains, across column families.
    pub versions_retained: u64,
    pub bytes_retained: u64,
    pub jobs_deleted: u64,
    /// Oversized stored job results bounded in place, and the bytes saved.
    pub job_results_bounded: u64,
    pub job_result_bytes_saved: u64,
    /// Blob keys that only pruned versions referenced. The caller owns the
    /// binary store and deletes them.
    pub orphaned_blobs: Vec<String>,
    /// Live SST bytes across every column family before and after.
    pub live_sst_bytes_before: u64,
    pub live_sst_bytes_after: u64,
    pub duration_ms: u64,
}

impl GcReport {
    /// SST bytes the run returned to the filesystem.
    pub fn reclaimed_sst_bytes(&self) -> u64 {
        self.live_sst_bytes_before
            .saturating_sub(self.live_sst_bytes_after)
    }
}

/// Cutoff and pins for one scope (a branch, or a whole repository for the
/// repository-wide snapshot rows).
#[derive(Debug, Clone)]
pub(crate) struct ScopePlan {
    pub cutoff: HLC,
    /// Pinned revisions strictly older than `cutoff`, newest first.
    pub pins: Vec<HLC>,
}

/// Which versions of one entity survive.
///
/// `revs` are the entity's versions NEWEST FIRST, `tomb[i]` whether version
/// `i` is a tombstone. Returns a keep-flag per version.
pub(crate) fn select_survivors(
    revs: &[HLC],
    tomb: &[bool],
    plan: &ScopePlan,
    drop_orphan_tombstones: bool,
) -> Vec<bool> {
    let mut keep: Vec<bool> = revs.iter().map(|r| *r > plan.cutoff).collect();
    for point in std::iter::once(&plan.cutoff).chain(plan.pins.iter()) {
        if let Some(i) = revs.iter().position(|r| r <= point) {
            keep[i] = true;
        }
    }
    if drop_orphan_tombstones {
        // Oldest survivors that are tombstones hide nothing any more: every
        // older version is already gone, so "deleted" and "absent" read alike.
        while let Some(i) = keep.iter().rposition(|k| *k) {
            if tomb[i] {
                keep[i] = false;
            } else {
                break;
            }
        }
    }
    keep
}

fn hlc_at_ms(ms: u64) -> HLC {
    HLC::new(ms, u64::MAX)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn storage_err(e: impl std::fmt::Display) -> Error {
    Error::storage(e.to_string())
}

/// Every `{tenant}\0{repo}\0{kind}\0{name}` record of a column family, decoded.
fn scan_repo_records<T: serde::de::DeserializeOwned>(
    db: &DB,
    cf_name: &str,
    kind: &[u8],
    scope: Option<&[u8]>,
) -> Result<Vec<(String, String, T)>> {
    let cf = cf_handle(db, cf_name)?;
    let mut it = db.raw_iterator_cf(cf);
    match scope {
        Some(p) => it.seek(p),
        None => it.seek_to_first(),
    }
    let mut out = Vec::new();
    while it.valid() {
        let (Some(key), Some(value)) = (it.key(), it.value()) else {
            break;
        };
        if let Some(p) = scope {
            if !key.starts_with(p) {
                break;
            }
        }
        let parts: Vec<&[u8]> = key.splitn(4, |b| *b == 0).collect();
        if parts.len() == 4 && parts[2] == kind {
            if let Ok(record) = rmp_serde::from_slice::<T>(value) {
                out.push((
                    String::from_utf8_lossy(parts[0]).into_owned(),
                    String::from_utf8_lossy(parts[1]).into_owned(),
                    record,
                ));
            }
        }
        it.next();
    }
    Ok(out)
}

/// The scope prefix the options restrict the run to.
fn scope_prefix(opts: &GcOptions) -> Option<Vec<u8>> {
    let tenant = opts.tenant.as_ref()?;
    let mut p = tenant.as_bytes().to_vec();
    p.push(0);
    if let Some(repo) = &opts.repo {
        p.extend_from_slice(repo.as_bytes());
        p.push(0);
    }
    Some(p)
}

/// The `n`-th newest revision committed on each branch of a repository.
fn nth_newest_revisions(
    db: &DB,
    tenant: &str,
    repo: &str,
    wanted: &HashMap<String, u64>,
) -> Result<HashMap<String, HLC>> {
    let mut found = HashMap::new();
    if wanted.is_empty() {
        return Ok(found);
    }
    let cf = cf_handle(db, cf::REVISIONS)?;
    let prefix = crate::keys::KeyBuilder::new()
        .push(tenant)
        .push(repo)
        .push("revisions")
        .build_prefix();
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut it = db.raw_iterator_cf(cf);
    it.seek(&prefix);
    while it.valid() {
        let (Some(key), Some(value)) = (it.key(), it.value()) else {
            break;
        };
        if !key.starts_with(&prefix) {
            break;
        }
        if let Ok(meta) = rmp_serde::from_slice::<raisin_storage::RevisionMeta>(value) {
            if let Some(n) = wanted.get(&meta.branch) {
                if !found.contains_key(&meta.branch) {
                    let c = counts.entry(meta.branch.clone()).or_insert(0);
                    *c += 1;
                    if *c >= *n {
                        found.insert(meta.branch.clone(), meta.revision);
                        if found.len() == wanted.len() {
                            break;
                        }
                    }
                }
            }
        }
        it.next();
    }
    Ok(found)
}

struct Plans {
    /// `{tenant}\0{repo}\0{branch}` -> plan
    branch: HashMap<Vec<u8>, ScopePlan>,
    /// `{tenant}\0{repo}\0{branch}` -> the branch HEAD when the plan was made
    /// (caps the translation history floor, `translation_floor.rs`).
    heads: HashMap<Vec<u8>, HLC>,
    /// `{tenant}\0{repo}` -> plan (only when EVERY branch of the repo has one)
    repo: HashMap<Vec<u8>, ScopePlan>,
    report: Vec<BranchGcPlan>,
}

fn build_plans(db: &DB, opts: &GcOptions) -> Result<Plans> {
    let scope = scope_prefix(opts);
    let branches: Vec<(String, String, raisin_context::Branch)> =
        scan_repo_records(db, cf::BRANCHES, b"branches", scope.as_deref())?;
    let tags: Vec<(String, String, raisin_context::Tag)> =
        scan_repo_records(db, cf::TAGS, b"tags", scope.as_deref())?;

    // Pins per repository: every tag and every fork point, on every branch —
    // a tag does not record its branch, so it protects them all.
    let mut pins: HashMap<(String, String), Vec<HLC>> = HashMap::new();
    for (t, r, tag) in &tags {
        pins.entry((t.clone(), r.clone()))
            .or_default()
            .push(tag.revision);
    }
    for (t, r, b) in &branches {
        if let Some(from) = b.created_from {
            pins.entry((t.clone(), r.clone())).or_default().push(from);
        }
    }
    // Merge commits and their second parents: after a second merge the
    // divergence base is the earlier merge (or the source head it merged),
    // and the three-way merge reads both branches AT that revision.
    for (t, r) in pins::repositories(&branches) {
        let merges = pins::merge_revisions(db, &t, &r)?;
        pins.entry((t, r)).or_default().extend(merges);
    }

    let now = now_ms();
    let safety = hlc_at_ms(now.saturating_sub(opts.min_age.as_millis() as u64));

    // Resolve policies, grouped per repository for the revision-count lookup.
    let mut per_repo: BTreeMap<(String, String), Vec<(String, HistoryRetention)>> = BTreeMap::new();
    for (t, r, b) in &branches {
        let policy = match opts.retention_override {
            Some(p) => p,
            None => retention::resolve_policy(db, t, r, &b.name, opts.default_retention)?,
        };
        per_repo
            .entry((t.clone(), r.clone()))
            .or_default()
            .push((b.name.clone(), policy));
    }

    let mut plans = Plans {
        branch: HashMap::new(),
        heads: branches
            .iter()
            .map(|(t, r, b)| (format!("{t}\0{r}\0{}", b.name).into_bytes(), b.head))
            .collect(),
        repo: HashMap::new(),
        report: Vec::new(),
    };

    for ((tenant, repo), branch_policies) in per_repo {
        let wanted: HashMap<String, u64> = branch_policies
            .iter()
            .filter_map(|(b, p)| p.keep_revisions.map(|n| (b.clone(), n.max(1))))
            .collect();
        let nth = nth_newest_revisions(db, &tenant, &repo, &wanted)?;

        let mut repo_pins = pins
            .remove(&(tenant.clone(), repo.clone()))
            .unwrap_or_default();
        repo_pins.sort_by(|a, b| b.cmp(a));
        repo_pins.dedup();

        let mut repo_cutoff: Option<HLC> = None;
        let mut every_branch_planned = true;

        for (branch, policy) in branch_policies {
            let by_time = policy
                .keep_days
                .map(|d| hlc_at_ms(now.saturating_sub(u64::from(d) * 86_400_000)));
            // A branch with fewer than N revisions keeps them all.
            let by_count = match policy.keep_revisions {
                Some(_) => Some(nth.get(&branch).copied()),
                None => None,
            };
            let cutoff = match (by_time, by_count) {
                (None, None) => None,
                (_, Some(None)) => None,
                (Some(t), None) => Some(t),
                (None, Some(Some(c))) => Some(c),
                // Either limit keeping a revision keeps it: the OLDER cutoff.
                (Some(t), Some(Some(c))) => Some(t.min(c)),
            }
            .map(|c| c.min(safety));

            let pins_below: Vec<HLC> = match cutoff {
                Some(c) => repo_pins.iter().copied().filter(|p| *p < c).collect(),
                None => Vec::new(),
            };
            plans.report.push(BranchGcPlan {
                tenant: tenant.clone(),
                repo: repo.clone(),
                branch: branch.clone(),
                retention: policy,
                cutoff: cutoff.map(|c| c.to_string()),
                pinned_revisions: pins_below.len(),
            });

            match cutoff {
                Some(c) => {
                    repo_cutoff = Some(repo_cutoff.map_or(c, |rc| rc.min(c)));
                    let key = format!("{tenant}\0{repo}\0{branch}").into_bytes();
                    plans.branch.insert(
                        key,
                        ScopePlan {
                            cutoff: c,
                            pins: pins_below,
                        },
                    );
                }
                None => every_branch_planned = false,
            }
        }

        if every_branch_planned {
            if let Some(c) = repo_cutoff {
                plans.repo.insert(
                    format!("{tenant}\0{repo}").into_bytes(),
                    ScopePlan {
                        cutoff: c,
                        pins: repo_pins.into_iter().filter(|p| *p < c).collect(),
                    },
                );
            }
        }
    }

    Ok(plans)
}

/// One version held while its chunk is open.
struct Version {
    key: Vec<u8>,
    rev: HLC,
    tomb: bool,
    size: u64,
    /// Kept only where blob references are tracked.
    value: Option<Vec<u8>>,
}

/// Keys sharing everything before the revision, grouped by what follows it.
struct Chunk {
    prefix: Vec<u8>,
    scope_end: usize,
    groups: HashMap<Vec<u8>, Vec<Version>>,
    entries: usize,
    overflow: bool,
}

/// Mutable state of one column-family pass.
struct Pass<'a> {
    db: &'a DB,
    cf: &'a rocksdb::ColumnFamily,
    target: &'a GcTarget,
    plans: &'a HashMap<Vec<u8>, ScopePlan>,
    dry_run: bool,
    track_blobs: bool,
    batch: WriteBatch,
    pending: usize,
    stats: CfGcStats,
    /// Blob keys named by pruned versions (`id -> key`).
    blob_candidates: &'a mut HashMap<String, String>,
    /// Keys pruned in a dry run, so the blob reference pass can skip them.
    pruned_keys: &'a mut HashSet<Vec<u8>>,
    /// Branch scopes (`{tenant}\0{repo}\0{branch}`) where a translation
    /// version was deleted (`translation_floor.rs`).
    translation_scopes: &'a mut HashSet<Vec<u8>>,
}

impl Pass<'_> {
    fn flush_batch(&mut self) -> Result<()> {
        if self.pending > 0 {
            let batch = std::mem::take(&mut self.batch);
            self.db.write(batch).map_err(storage_err)?;
            self.pending = 0;
        }
        Ok(())
    }

    fn close_chunk(&mut self, chunk: Chunk) -> Result<()> {
        if chunk.overflow {
            tracing::warn!(
                cf = self.target.cf,
                entries = chunk.entries,
                "history GC: chunk too large to hold, left untouched"
            );
            return Ok(());
        }
        let Some(plan) = self.plans.get(&chunk.prefix[..chunk.scope_end]) else {
            return Ok(());
        };
        let translation_cf = matches!(
            self.target.cf,
            cf::TRANSLATION_DATA | cf::BLOCK_TRANSLATIONS
        );
        let node_chunk = node_delete_translations::NodeChunk::parse(self.target.cf, &chunk.prefix);
        for (_, mut versions) in chunk.groups {
            if versions.len() < 2 && !(self.target.drop_orphan_tombstones && versions[0].tomb) {
                continue;
            }
            versions.sort_by(|a, b| b.rev.cmp(&a.rev));
            let revs: Vec<HLC> = versions.iter().map(|v| v.rev).collect();
            let tomb: Vec<bool> = versions.iter().map(|v| v.tomb).collect();
            let mut keep = select_survivors(&revs, &tomb, plan, self.target.drop_orphan_tombstones);
            if node_chunk.is_some() {
                node_delete_translations::keep_generation_starts(&tomb, &mut keep);
            }
            for (i, (v, kept)) in versions.into_iter().zip(keep).enumerate() {
                if kept {
                    if i > 0 {
                        self.stats.versions_retained += 1;
                        self.stats.bytes_retained += v.size;
                    }
                    continue;
                }
                self.stats.versions_deleted += 1;
                self.stats.bytes_deleted += v.size;
                if translation_cf
                    && !self
                        .translation_scopes
                        .contains(&chunk.prefix[..chunk.scope_end])
                {
                    self.translation_scopes
                        .insert(chunk.prefix[..chunk.scope_end].to_vec());
                }
                if let Some(value) = &v.value {
                    if !v.tomb {
                        blobs::exact_keys_in_msgpack(value, self.blob_candidates);
                    }
                }
                if self.dry_run {
                    if self.track_blobs {
                        self.pruned_keys.insert(v.key);
                    }
                } else {
                    if let (true, Some(node)) = (v.tomb, &node_chunk) {
                        // Same batch as the tombstone's delete (flushed only
                        // after it): the read rule's evidence never vanishes
                        // before its replacement lands.
                        let next_record = i.checked_sub(1).map(|newer| revs[newer]);
                        self.pending += node.materialize(
                            self.db,
                            &mut self.batch,
                            &v.rev,
                            next_record.as_ref(),
                        )?;
                    }
                    self.batch.delete_cf(self.cf, &v.key);
                    self.pending += 1;
                    if self.pending >= BATCH_DELETES {
                        self.flush_batch()?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Prune one column family.
fn prune_cf(
    db: &DB,
    target: &GcTarget,
    plans: &Plans,
    opts: &GcOptions,
    blob_candidates: &mut HashMap<String, String>,
    pruned_keys: &mut HashSet<Vec<u8>>,
    translation_scopes: &mut HashSet<Vec<u8>>,
) -> Result<CfGcStats> {
    let cf = cf_handle(db, target.cf)?;
    let plan_map = if target.branch_scoped {
        &plans.branch
    } else {
        &plans.repo
    };
    let mut pass = Pass {
        db,
        cf,
        target,
        plans: plan_map,
        dry_run: opts.dry_run,
        track_blobs: opts.collect_orphaned_blobs
            && matches!(target.cf, cf::NODES | cf::REVISIONS | cf::TRANSLATION_DATA),
        batch: WriteBatch::default(),
        pending: 0,
        stats: CfGcStats::default(),
        blob_candidates,
        pruned_keys,
        translation_scopes,
    };
    if plan_map.is_empty() {
        return Ok(pass.stats);
    }

    let scope = scope_prefix(opts);
    let mut read_opts = rocksdb::ReadOptions::default();
    read_opts.fill_cache(false);
    let mut it = db.raw_iterator_cf_opt(cf, read_opts);
    match &scope {
        Some(p) => it.seek(p),
        None => it.seek_to_first(),
    }

    // Open chunks form a chain of prefixes of the current key (a chunk whose
    // keys all start with `prefix\0` is contiguous, but a longer chunk such
    // as `…\0{id}\0adj` can sit INSIDE `…\0{id}`'s range).
    let mut open: Vec<Chunk> = Vec::new();

    while it.valid() {
        let (Some(key), Some(value)) = (it.key(), it.value()) else {
            break;
        };
        if let Some(p) = &scope {
            if !key.starts_with(p) {
                break;
            }
        }
        pass.stats.keys_scanned += 1;

        // Close every open chunk this key is no longer inside.
        while let Some(top) = open.last() {
            let inside = key.len() > top.prefix.len()
                && key.starts_with(&top.prefix)
                && key[top.prefix.len()] == 0;
            if inside {
                break;
            }
            let chunk = open.pop().expect("non-empty");
            pass.close_chunk(chunk)?;
        }

        if let Some(loc) = layout::locate(target, key) {
            // Scopes without a plan keep everything; don't hold their keys.
            if plan_map.contains_key(&key[..loc.scope_end]) {
                add_version(&mut open, key, value, &loc, pass.track_blobs)?;
            }
        }
        it.next();
    }
    it.status().map_err(storage_err)?;
    while let Some(chunk) = open.pop() {
        pass.close_chunk(chunk)?;
    }
    pass.flush_batch()?;
    Ok(pass.stats)
}

fn add_version(
    open: &mut Vec<Chunk>,
    key: &[u8],
    value: &[u8],
    loc: &Located,
    keep_value: bool,
) -> Result<()> {
    let chunk_bytes = loc.chunk(key);
    let rev = HLC::decode_descending(loc.revision(key)).map_err(storage_err)?;
    let version = Version {
        key: key.to_vec(),
        rev,
        tomb: crate::keys::is_tombstone_value(value),
        size: (key.len() + value.len()) as u64,
        value: keep_value.then(|| value.to_vec()),
    };
    let idx = match open.iter().rposition(|c| c.prefix == chunk_bytes) {
        Some(i) => i,
        None => {
            open.push(Chunk {
                prefix: chunk_bytes.to_vec(),
                scope_end: loc.scope_end,
                groups: HashMap::new(),
                entries: 0,
                overflow: false,
            });
            open.len() - 1
        }
    };
    let chunk = &mut open[idx];
    chunk.entries += 1;
    if chunk.overflow {
        return Ok(());
    }
    if chunk.entries > MAX_CHUNK_ENTRIES {
        chunk.overflow = true;
        chunk.groups.clear();
        return Ok(());
    }
    chunk
        .groups
        .entry(loc.tail(key).to_vec())
        .or_default()
        .push(version);
    Ok(())
}

/// Column families derived from node versions and rebuilt from them. A blob
/// mentioned only here is named by no node version: the row is stale. Object
/// values used to be indexed under an unstable encoding, so a superseded
/// value's tombstone missed its row and the row never went away — on one dev
/// database that kept 162 replaced package archives (3.2 GB) alive.
const DERIVED_INDEXES: &[&str] = &[
    cf::PATH_INDEX,
    cf::NODE_PATH,
    cf::PROPERTY_INDEX,
    cf::REFERENCE_INDEX,
    cf::RELATION_INDEX,
    cf::ORDERED_CHILDREN,
    cf::SPATIAL_INDEX,
    cf::COMPOUND_INDEX,
    cf::UNIQUE_INDEX,
    cf::EMBEDDINGS,
];

/// Which blob candidates (by id) are still mentioned by surviving data: any
/// key or value in every column family except the [`DERIVED_INDEXES`]. A
/// tombstone row is not a mention either — it says the value is gone.
fn blobs_still_referenced<V>(
    db: &DB,
    candidates: &HashMap<String, V>,
    pruned_keys: &HashSet<Vec<u8>>,
) -> Result<HashSet<String>> {
    let mut seen = HashSet::new();
    if candidates.is_empty() {
        return Ok(seen);
    }
    for cf_name in crate::all_column_families() {
        if DERIVED_INDEXES.contains(&cf_name) {
            continue;
        }
        let Some(cf) = db.cf_handle(cf_name) else {
            continue;
        };
        let mut read_opts = rocksdb::ReadOptions::default();
        read_opts.fill_cache(false);
        let mut it = db.raw_iterator_cf_opt(cf, read_opts);
        it.seek_to_first();
        while it.valid() {
            if let (Some(key), Some(value)) = (it.key(), it.value()) {
                if !crate::keys::is_tombstone_value(value) && !pruned_keys.contains(key) {
                    blobs::mentioned_ids(key, candidates, &mut seen);
                    blobs::mentioned_ids(value, candidates, &mut seen);
                }
            }
            it.next();
        }
        it.status().map_err(storage_err)?;
    }
    Ok(seen)
}

fn live_sst_bytes(db: &DB, cf_name: &str) -> u64 {
    db.cf_handle(cf_name)
        .and_then(|cf| {
            db.property_int_value_cf(cf, "rocksdb.live-sst-files-size")
                .ok()
                .flatten()
        })
        .unwrap_or(0)
}

fn total_live_sst_bytes(db: &DB) -> u64 {
    crate::all_column_families()
        .into_iter()
        .map(|c| live_sst_bytes(db, c))
        .sum()
}

/// Compact a whole column family down through the bottommost level, which is
/// where deleted keys and their tombstones are finally dropped.
pub fn compact_column_family(db: &DB, cf_name: &str) {
    if let Some(cf) = db.cf_handle(cf_name) {
        let mut opts = rocksdb::CompactOptions::default();
        opts.set_bottommost_level_compaction(rocksdb::BottommostLevelCompaction::Force);
        opts.set_change_level(false);
        db.compact_range_cf_opt(cf, None::<&[u8]>, None::<&[u8]>, &opts);
    }
}

/// Delete every replication operation-log entry (in scope).
fn purge_oplog(db: &DB, opts: &GcOptions) -> Result<(u64, u64)> {
    let cf = cf_handle(db, cf::OPERATION_LOG)?;
    let scope = scope_prefix(opts);
    let mut it = db.raw_iterator_cf(cf);
    match &scope {
        Some(p) => it.seek(p),
        None => it.seek_to_first(),
    }
    let (mut n, mut bytes) = (0u64, 0u64);
    let mut batch = WriteBatch::default();
    let mut pending = 0usize;
    while it.valid() {
        let (Some(key), Some(value)) = (it.key(), it.value()) else {
            break;
        };
        if let Some(p) = &scope {
            if !key.starts_with(p) {
                break;
            }
        }
        n += 1;
        bytes += (key.len() + value.len()) as u64;
        if !opts.dry_run {
            batch.delete_cf(cf, key);
            pending += 1;
            if pending >= BATCH_DELETES {
                db.write(std::mem::take(&mut batch)).map_err(storage_err)?;
                pending = 0;
            }
        }
        it.next();
    }
    it.status().map_err(storage_err)?;
    if pending > 0 {
        db.write(batch).map_err(storage_err)?;
    }
    Ok((n, bytes))
}

/// Run history GC. Blocking: call it from `spawn_blocking`.
pub fn run_history_gc(storage: &RocksDBStorage, opts: &GcOptions) -> Result<GcReport> {
    run_history_gc_on_db(storage.db(), Some(storage), opts)
}

/// [`run_history_gc`] against a bare `DB` (job cleanup needs the storage).
pub(crate) fn run_history_gc_on_db(
    db: &DB,
    storage: Option<&RocksDBStorage>,
    opts: &GcOptions,
) -> Result<GcReport> {
    if opts.repo.is_some() && opts.tenant.is_none() {
        return Err(Error::Validation(
            "history GC: a repository scope needs a tenant".to_string(),
        ));
    }
    let started = Instant::now();
    // Retention keeps the NEWEST version at or below its cutoff and deletes
    // the older ones; run-collapse keeps the OLDEST of a run and deletes the
    // newer twin. Interleaved, both commit and the group is empty: hold the
    // database against collapse slices for the whole run (a dry run deletes
    // nothing and needs no hold).
    let _pruning = (!opts.dry_run).then(|| crate::management::cf_exclusion::enter_pruner(db));
    let mut report = GcReport {
        dry_run: opts.dry_run,
        ..Default::default()
    };
    report.live_sst_bytes_before = total_live_sst_bytes(db);

    let plans = build_plans(db, opts)?;
    report.branches = plans.report.clone();

    let mut blob_candidates: HashMap<String, String> = HashMap::new();
    let mut pruned_keys: HashSet<Vec<u8>> = HashSet::new();
    let mut translation_scopes: HashSet<Vec<u8>> = HashSet::new();
    let mut touched: Vec<&'static str> = Vec::new();

    for target in GC_TARGETS {
        let before = live_sst_bytes(db, target.cf);
        let mut stats = prune_cf(
            db,
            target,
            &plans,
            opts,
            &mut blob_candidates,
            &mut pruned_keys,
            &mut translation_scopes,
        )?;
        stats.live_sst_bytes_before = before;
        if stats.versions_deleted > 0 {
            touched.push(target.cf);
            tracing::info!(
                cf = target.cf,
                scanned = stats.keys_scanned,
                deleted = stats.versions_deleted,
                bytes = stats.bytes_deleted,
                dry_run = opts.dry_run,
                "history GC: column family pruned"
            );
        }
        report.versions_deleted += stats.versions_deleted;
        report.bytes_deleted += stats.bytes_deleted;
        report.versions_retained += stats.versions_retained;
        report.bytes_retained += stats.bytes_retained;
        report.column_families.insert(target.cf.to_string(), stats);
    }

    if !opts.dry_run {
        translation_floor::record(db, &plans, &translation_scopes)?;
    }

    if opts.purge_oplog {
        let (n, bytes) = purge_oplog(db, opts)?;
        report.oplog_entries_deleted = n;
        report.oplog_bytes_deleted = bytes;
        if n > 0 {
            touched.push(cf::OPERATION_LOG);
        }
    }

    if let (true, Some(storage)) = (opts.bound_job_results, storage) {
        // Callers poll a finished job's full result from the in-memory
        // registry for a few minutes; the stored copy is for history only.
        let finished_before = chrono::Utc::now() - chrono::Duration::minutes(10);
        let (n, saved) = storage.job_metadata_store().bound_stored_results(
            opts.tenant.as_deref(),
            finished_before,
            opts.dry_run,
        )?;
        report.job_results_bounded = n;
        report.job_result_bytes_saved = saved;
        if n > 0 && !touched.contains(&cf::JOB_METADATA) {
            touched.push(cf::JOB_METADATA);
        }
    }

    if let (Some(retention), Some(storage)) = (opts.job_retention, storage) {
        if !opts.dry_run {
            let cutoff = chrono::Utc::now()
                - chrono::Duration::from_std(retention).unwrap_or(chrono::Duration::days(1));
            let deleted = match &opts.tenant {
                Some(t) => storage
                    .job_metadata_store()
                    .cleanup_old_jobs_for_tenant(t, cutoff)?,
                None => storage.job_metadata_store().cleanup_old_jobs(cutoff)?,
            };
            report.jobs_deleted = deleted as u64;
            if deleted > 0 {
                for c in [cf::JOB_METADATA, cf::JOB_DATA] {
                    if !touched.contains(&c) {
                        touched.push(c);
                    }
                }
            }
        }
    }

    if opts.collect_orphaned_blobs && !blob_candidates.is_empty() {
        let referenced = blobs_still_referenced(db, &blob_candidates, &pruned_keys)?;
        let mut orphaned: Vec<String> = blob_candidates
            .into_iter()
            .filter(|(id, _)| !referenced.contains(id))
            .map(|(_, key)| key)
            .collect();
        orphaned.sort();
        report.orphaned_blobs = orphaned;
    }

    if opts.compact && !opts.dry_run {
        for cf_name in &touched {
            compact_column_family(db, cf_name);
        }
    }

    for (name, stats) in report.column_families.iter_mut() {
        stats.live_sst_bytes_after = live_sst_bytes(db, name);
    }
    report.live_sst_bytes_after = total_live_sst_bytes(db);
    report.duration_ms = started.elapsed().as_millis() as u64;

    tracing::info!(
        dry_run = opts.dry_run,
        versions_deleted = report.versions_deleted,
        logical_bytes = report.bytes_deleted,
        oplog_entries = report.oplog_entries_deleted,
        jobs = report.jobs_deleted,
        orphaned_blobs = report.orphaned_blobs.len(),
        sst_before = report.live_sst_bytes_before,
        sst_after = report.live_sst_bytes_after,
        duration_ms = report.duration_ms,
        "history GC finished"
    );
    Ok(report)
}

/// Outcome of a GC run including the binary-store sweep.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GcRunOutcome {
    #[serde(flatten)]
    pub report: GcReport,
    /// Blobs the store listed; `None` when the sweep did not run (disabled, or
    /// a backend that cannot enumerate what it owns).
    pub blobs_listed: Option<u64>,
    /// Listed blobs nothing mentions (deleted unless dry), and their bytes.
    pub unreferenced_blobs: u64,
    pub unreferenced_blob_bytes: u64,
    pub blobs_deleted: u64,
    /// Bytes of the deleted blobs, where the store reports a size.
    pub blob_bytes_deleted: u64,
    pub blob_delete_errors: u64,
}

/// Options for a scheduled or manual run, from the storage's configuration.
pub fn configured_options(storage: &RocksDBStorage) -> GcOptions {
    let config = storage.config();
    GcOptions {
        default_retention: config.history_retention,
        // Nothing reads the operation log unless replication is on.
        purge_oplog: !config.replication_enabled,
        job_retention: Some(Duration::from_secs(
            config.job_retention_hours.max(1) as u64 * 3600,
        )),
        ..GcOptions::default()
    }
}

/// Run history GC off the async runtime, then delete the blobs nothing
/// references any more (skipped on a dry run): the ones only pruned history
/// named, and — with [`GcOptions::sweep_unreferenced_blobs`] — every listed
/// blob no key or value mentions.
pub async fn run_gc_and_sweep_blobs<B>(
    storage: std::sync::Arc<RocksDBStorage>,
    bin: &B,
    opts: GcOptions,
) -> Result<GcRunOutcome>
where
    B: raisin_binary::BinaryStorage + ?Sized,
{
    let dry_run = opts.dry_run;
    let gc_storage = storage.clone();
    let gc_opts = opts.clone();
    let report = tokio::task::spawn_blocking(move || run_history_gc(&gc_storage, &gc_opts))
        .await
        .map_err(|e| Error::storage(format!("history GC task failed: {e}")))??;

    let mut outcome = GcRunOutcome {
        report,
        ..Default::default()
    };

    // key -> size, where known
    let mut doomed: BTreeMap<String, Option<u64>> = outcome
        .report
        .orphaned_blobs
        .iter()
        .map(|k| (k.clone(), None))
        .collect();

    if opts.sweep_unreferenced_blobs {
        match bin.list_blobs().await {
            Ok(Some(listed)) => {
                outcome.blobs_listed = Some(listed.len() as u64);
                let found = sweep::unreferenced(storage, listed, &opts).await?;
                for blob in found {
                    outcome.unreferenced_blobs += 1;
                    outcome.unreferenced_blob_bytes += blob.size;
                    doomed.insert(blob.key, Some(blob.size));
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "history GC: could not list blobs; sweep skipped"),
        }
    }

    if dry_run {
        return Ok(outcome);
    }
    for (key, size) in doomed {
        let size = match size {
            Some(size) => size,
            // Size first; a key that no longer resolves is already gone (or
            // was never this store's), so there is nothing to delete.
            None => match bin.get_stream(&key).await {
                Ok((size, _stream)) => size,
                Err(_) => continue,
            },
        };
        match bin.delete(&key).await {
            Ok(()) => {
                outcome.blobs_deleted += 1;
                outcome.blob_bytes_deleted += size;
            }
            Err(e) => {
                outcome.blob_delete_errors += 1;
                tracing::warn!(key = %key, error = %e, "history GC: could not delete orphaned blob");
            }
        }
    }
    if outcome.blobs_deleted > 0 {
        tracing::info!(
            blobs = outcome.blobs_deleted,
            bytes = outcome.blob_bytes_deleted,
            "history GC: deleted orphaned blobs"
        );
    }
    Ok(outcome)
}

/// Run the maintenance pass every `storage.config().maintenance_interval_secs`
/// (no-op when 0). The first pass starts one interval after boot, capped at
/// ten minutes, so start-up is not competing with a full scan.
pub fn spawn_maintenance<B>(
    storage: std::sync::Arc<RocksDBStorage>,
    bin: std::sync::Arc<B>,
) -> Option<tokio::task::JoinHandle<()>>
where
    B: raisin_binary::BinaryStorage + Send + Sync + ?Sized + 'static,
{
    let interval_secs = storage.config().maintenance_interval_secs;
    if interval_secs == 0 {
        tracing::info!("Storage maintenance disabled (maintenance_interval_secs = 0)");
        return None;
    }
    let interval = Duration::from_secs(interval_secs);
    tracing::info!(
        interval_secs,
        retention = ?storage.config().history_retention,
        job_retention_hours = storage.config().job_retention_hours,
        "Storage maintenance scheduled"
    );
    Some(tokio::spawn(async move {
        tokio::time::sleep(interval.min(Duration::from_secs(600))).await;
        loop {
            let opts = configured_options(&storage);
            match run_gc_and_sweep_blobs(storage.clone(), bin.as_ref(), opts).await {
                Ok(o) => tracing::info!(
                    versions_deleted = o.report.versions_deleted,
                    oplog_entries_deleted = o.report.oplog_entries_deleted,
                    jobs_deleted = o.report.jobs_deleted,
                    blobs_deleted = o.blobs_deleted,
                    blob_bytes_deleted = o.blob_bytes_deleted,
                    reclaimed_sst_bytes = o.report.reclaimed_sst_bytes(),
                    "Storage maintenance pass complete"
                ),
                Err(e) => tracing::error!(error = %e, "Storage maintenance pass failed"),
            }
            tokio::time::sleep(interval).await;
        }
    }))
}
