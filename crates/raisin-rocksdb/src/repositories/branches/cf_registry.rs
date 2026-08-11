//! The single, exhaustive table of how every column family relates to branch
//! scoping — and therefore what a branch fork must do with it.
//!
//! # Why this exists
//!
//! `copy_branch_indexes` used to carry a hand-written list of column families.
//! Twice that list silently fell behind reality:
//!
//! 1. `ARCHETYPES` / `ELEMENT_TYPES` were missing, which broke branch-based
//!    publish until it was root-caused.
//! 2. `SPATIAL_INDEX` was missing, so a spatial query on a fork returned zero
//!    rows while the nodes themselves were plainly there.
//!
//! Both are the same failure: a new branch-scoped CF was added and nobody
//! remembered the copier. The list was *implicit* — a CF that was never
//! mentioned was simply not copied, and nothing said so.
//!
//! This table makes the decision **explicit and exhaustive**. Every CF in
//! [`crate::all_column_families`] MUST appear here exactly once, classified as
//! one of:
//!
//! - [`BranchScope::NotBranchScoped`] — the key has no branch component, so
//!   there is nothing to copy.
//! - [`BranchScope::Copied`] — branch-scoped; the fork copies it.
//! - [`BranchScope::SkippedOnPurpose`] — branch-scoped, but deliberately not
//!   copied, with the reason recorded inline.
//!
//! [`exhaustiveness`](self::tests) is enforced by a unit test that walks
//! `all_column_families()`. Adding a CF constant without classifying it here
//! FAILS THE BUILD'S TESTS, which is the point: the next person cannot forget,
//! because forgetting is no longer silent.

use crate::cf;

/// Where in a key the 16-byte descending HLC revision lives.
///
/// A `~HLC` is a bitwise-NOT encoding, so it **may contain null bytes**. Any
/// locator that indexes into `key.split(b'\0')` is therefore unreliable; the
/// variants here all locate the revision positionally instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RevisionLocator {
    /// The key ENDS with the revision — take the last 16 bytes.
    Tail,
    /// The revision is followed by `n` null-separated, null-free trailing
    /// segments (node ids, workspace names). Walk back `n` nulls from the end
    /// of the key, then take the 16 bytes before that null.
    BeforeTrailingSegments(usize),
    /// `RELATION_INDEX` has two shapes with different trailing-segment counts:
    /// `rel` / `rel_rev` end with one id, `rel_global` ends with four segments.
    RelationIndex,
    /// `SPATIAL_INDEX` — use the purpose-built `parse_spatial_index_key`.
    SpatialKey,
    /// Schema CFs (`NODE_TYPES` / `ARCHETYPES` / `ELEMENT_TYPES`): the main
    /// shape ends with the revision, while the `*_versions` shape stores a
    /// descending HLC in the VALUE.
    SchemaOrVersionValue,
}

/// Everything the copier needs to know to move one column family onto a fork.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CopyPlan {
    /// Human name used in the copier's log line.
    pub display_name: &'static str,
    /// Where the revision lives, so the copy can honour `max_revision`.
    pub revision: RevisionLocator,
}

/// A column family's relationship to branch scoping.
#[derive(Debug, Clone, Copy)]
pub(crate) enum BranchScope {
    /// No branch component in the key. The `&str` records the key shape that
    /// was checked, so the claim can be re-verified without re-deriving it.
    NotBranchScoped(&'static str),
    /// Branch-scoped; copied on fork.
    Copied(CopyPlan),
    /// Branch-scoped but deliberately not copied. The `&str` is the reason.
    SkippedOnPurpose(&'static str),
}

const fn copied(display_name: &'static str, revision: RevisionLocator) -> BranchScope {
    BranchScope::Copied(CopyPlan {
        display_name,
        revision,
    })
}

/// The exhaustive classification of every column family.
///
/// Key layouts are quoted from the builder that produces them; part indices are
/// 0-based over `\0`-separated segments. Every `Copied` entry has its branch at
/// part index **2** — the copier's key rewrite swaps exactly that segment, so a
/// CF whose branch sits elsewhere cannot be added here without teaching the
/// copier about it first (see `GRAPH_PROJECTION` below).
pub(crate) const BRANCH_CF_REGISTRY: &[(&str, BranchScope)] = &[
    // ---- branch-scoped, copied on fork -------------------------------------
    // {tenant}\0{repo}\0{branch}\0{ws}\0nodes\0{node_id}\0{~rev}
    (cf::NODES, copied("nodes", RevisionLocator::Tail)),
    // {tenant}\0{repo}\0{branch}\0{ws}\0path\0{path}\0{~rev}
    (cf::PATH_INDEX, copied("path_index", RevisionLocator::Tail)),
    // {tenant}\0{repo}\0{branch}\0{ws}\0node_path\0{node_id}\0{~rev}
    (cf::NODE_PATH, copied("node_path", RevisionLocator::Tail)),
    // {tenant}\0{repo}\0{branch}\0{ws}\0prop{_pub}\0{name}\0{value}\0{~rev}\0{node_id}
    (
        cf::PROPERTY_INDEX,
        copied("property_index", RevisionLocator::BeforeTrailingSegments(1)),
    ),
    // fwd: …\0ref{_pub}\0{node_id}\0{prop_path}\0{~rev}
    // rev: …\0ref_rev{_pub}\0{ws}\0{path}\0{src_id}\0{prop_path}\0{~rev}
    (
        cf::REFERENCE_INDEX,
        copied("reference_index", RevisionLocator::Tail),
    ),
    // fwd/rev: …\0rel{_rev}\0{id}\0{type}\0{~rev}\0{id}
    // global:  {tenant}\0{repo}\0{branch}\0rel_global\0{type}\0{~rev}\0{ws}\0{id}\0{ws}\0{id}
    (
        cf::RELATION_INDEX,
        copied("relation_index", RevisionLocator::RelationIndex),
    ),
    // …\0ordered\0{parent_id}\0{order_label}\0{~rev}\0{child_id}
    (
        cf::ORDERED_CHILDREN,
        copied(
            "ordered_children",
            RevisionLocator::BeforeTrailingSegments(1),
        ),
    ),
    // {tenant}\0{repo}\0{branch}\0nodetypes\0{name}\0{~rev} (+ *_versions in value)
    (
        cf::NODE_TYPES,
        copied("node_types", RevisionLocator::SchemaOrVersionValue),
    ),
    (
        cf::ARCHETYPES,
        copied("archetypes", RevisionLocator::SchemaOrVersionValue),
    ),
    (
        cf::ELEMENT_TYPES,
        copied("element_types", RevisionLocator::SchemaOrVersionValue),
    ),
    // …\0translations|trans_meta\0{node_id}\0{locale}\0{~rev}
    (
        cf::TRANSLATION_DATA,
        copied("translation_data", RevisionLocator::Tail),
    ),
    // …\0block_trans\0{node_id}\0{block_uuid}\0{locale}\0{~rev}
    (
        cf::BLOCK_TRANSLATIONS,
        copied("block_translations", RevisionLocator::Tail),
    ),
    // …\0geo\0{property}\0{geohash}\0{~rev}\0{node_id}
    // Added after a fork silently produced an EMPTY spatial index, so
    // find_within_radius on the fork returned nothing.
    (
        cf::SPATIAL_INDEX,
        copied("spatial_index", RevisionLocator::SpatialKey),
    ),
    // …\0cidx{_pub}\0{index_name}\0{col}…\0{~rev}\0{node_id} — a VARIABLE number
    // of column segments, so only a from-the-tail locator works.
    (
        cf::COMPOUND_INDEX,
        copied("compound_index", RevisionLocator::BeforeTrailingSegments(1)),
    ),
    // …\0uniq\0{node_type}\0{property}\0{value_hash}\0{~rev}
    (
        cf::UNIQUE_INDEX,
        copied("unique_index", RevisionLocator::Tail),
    ),
    // v2:     …\0{embedder_hash}\0{kind}\0{source_id}\0{chunk_idx}\0{~rev}
    // legacy: …\0{node_id}\0{~rev}
    (cf::EMBEDDINGS, copied("embeddings", RevisionLocator::Tail)),
    // ---- branch-scoped, deliberately NOT copied ----------------------------
    (
        cf::ORDER_INDEX,
        BranchScope::SkippedOnPurpose(
            "Denormalized child list written only by the index-rebuild job; \
             ORDERED_CHILDREN is the authoritative copy and IS forked.",
        ),
    ),
    (
        cf::TRANSLATION_HASHES,
        BranchScope::SkippedOnPurpose(
            "Staleness fingerprints with no revision in the key. Absence means \
             'recompute', never a wrong read.",
        ),
    ),
    (
        cf::WORKSPACE_DELTAS,
        BranchScope::SkippedOnPurpose(
            "Per-branch uncommitted working state; a fresh fork has no deltas \
             by definition.",
        ),
    ),
    (
        cf::AUDIT_LOG,
        BranchScope::SkippedOnPurpose(
            "Copying would fabricate audit entries attributing actions to a \
             branch on which they never happened.",
        ),
    ),
    // {tenant}\0{repo}\0{branch}\0{name}\0{~rev}
    //
    // Copied, and it MUST be: a node's `secret://` reference is branch-agnostic
    // (the name embeds the node id, which survives a fork), while the store is
    // branch-scoped. Skip the copy and every vaulted field on the fork points at
    // a secret that does not exist there — and because reads return the
    // reference without resolving it, the fork looks healthy until someone
    // actually reveals one.
    //
    // `Tail` is correct because the descending HLC is the last 16 bytes and
    // `{name}` is null-free.
    (cf::SECRETS, copied("secrets", RevisionLocator::Tail)),
    (
        cf::GRAPH_CACHE,
        BranchScope::SkippedOnPurpose(
            "Derived precomputation cache; a miss recomputes. Also keys the \
             branch at part 3, which the copier's part-2 rewrite cannot handle.",
        ),
    ),
    (
        cf::GRAPH_PROJECTION,
        BranchScope::SkippedOnPurpose(
            "Derived adjacency (node list, edge list, weights, stale flag), NOT \
             configuration: a projection CONFIG is a `raisin:GraphAlgorithmConfig` \
             NODE, so it forks with the nodes. `recompute_for_branch` does a full \
             build when the load misses, so a miss is never an empty answer — \
             copying would only spend fork time on an edge list the next \
             recompute regenerates. (It also keys the branch at part 3, which the \
             copier's part-2 rewrite could not handle anyway.) Proven by \
             `branch_fork_index_copies_test::\
             a_fork_inherits_graph_configs_but_not_the_derived_projection`.",
        ),
    ),
    (
        cf::INDEX_STATUS,
        BranchScope::SkippedOnPurpose(
            "Lazy-indexing watermark. Absence means 'never indexed', so the \
             fork re-indexes rather than trusting a stale mark.",
        ),
    ),
    (
        cf::PENDING_BATCH_OPS,
        BranchScope::SkippedOnPurpose(
            "In-flight aggregator operations. Copying would re-execute another \
             branch's pending writes against the fork.",
        ),
    ),
    // ---- not branch-scoped: no branch component in the key -----------------
    (
        cf::WORKSPACES,
        BranchScope::NotBranchScoped("{tenant}\0{repo}\0workspaces\0{ws_id}"),
    ),
    (
        cf::BRANCHES,
        BranchScope::NotBranchScoped("{tenant}\0{repo}\0branches\0{name} — branch is DATA here"),
    ),
    (
        cf::TAGS,
        BranchScope::NotBranchScoped("{tenant}\0{repo}\0tags\0{tag}"),
    ),
    (
        cf::REVISIONS,
        BranchScope::NotBranchScoped("{tenant}\0{repo}\0revisions|…\0… — repo-scoped"),
    ),
    (
        cf::TREES,
        BranchScope::NotBranchScoped("{tenant}\0{repo}\0trees\0{content_hash}"),
    ),
    (
        cf::REGISTRY,
        BranchScope::NotBranchScoped(
            "{tenant}\0repos\0{repo} | tenants\0{tenant} | deployments\0…",
        ),
    ),
    (
        cf::VERSIONS,
        BranchScope::NotBranchScoped("{tenant}\0{repo}\0versions\0{node_id}\0{version}"),
    ),
    (
        cf::TRANSLATION_INDEX,
        BranchScope::NotBranchScoped(
            "{tenant}\0{repo}\0translation_index\0{locale}\0{~rev}\0{node_id} — repo-wide",
        ),
    ),
    (
        cf::PROCESSING_RULES,
        BranchScope::NotBranchScoped("{tenant}\0{repo}"),
    ),
    (
        cf::SYSTEM_UPDATE_HASHES,
        BranchScope::NotBranchScoped("{tenant}:{repo}:{resource_type}:{name} — colon separated"),
    ),
    (
        cf::OPERATION_LOG,
        BranchScope::NotBranchScoped("{tenant}\0{repo}\0{cluster_node}\0{seq}\0{ts}"),
    ),
    (
        cf::APPLIED_OPS,
        BranchScope::NotBranchScoped("applied_ops/{op_id} — per-cluster-node state"),
    ),
    (
        cf::IDENTITIES,
        BranchScope::NotBranchScoped("{tenant}\0identities\0{identity_id}"),
    ),
    (
        cf::IDENTITY_EMAIL_INDEX,
        BranchScope::NotBranchScoped("{tenant}\0identity_email\0{email}"),
    ),
    (
        cf::SESSIONS,
        BranchScope::NotBranchScoped("{tenant}\0sessions|identity_sessions\0…"),
    ),
    (
        cf::ADMIN_USERS,
        BranchScope::NotBranchScoped("sys\0{tenant}\0users\0{username}"),
    ),
    (
        cf::JOB_DATA,
        BranchScope::NotBranchScoped("{tenant}\0{job_id}"),
    ),
    (
        cf::JOB_METADATA,
        BranchScope::NotBranchScoped("{tenant}\0{job_id}"),
    ),
    (
        cf::FULLTEXT_JOBS,
        BranchScope::NotBranchScoped("{state}:{job_id}"),
    ),
    (
        cf::EMBEDDING_JOBS,
        BranchScope::NotBranchScoped("{state}:{job_id}"),
    ),
    (
        cf::QUERY_EMBEDDINGS,
        BranchScope::NotBranchScoped("EMBEDDING() result cache, keyed by query text"),
    ),
    (
        cf::TENANT_AI_CONFIG,
        BranchScope::NotBranchScoped("{tenant}"),
    ),
    (
        cf::TENANT_AUTH_CONFIG,
        BranchScope::NotBranchScoped("{tenant}"),
    ),
    (
        cf::TENANT_EMBEDDING_CONFIG,
        BranchScope::NotBranchScoped("{tenant}"),
    ),
];

/// The column families a branch fork copies, in copy order.
pub(crate) fn cfs_to_copy() -> impl Iterator<Item = (&'static str, CopyPlan)> {
    BRANCH_CF_REGISTRY
        .iter()
        .filter_map(|(name, scope)| match scope {
            BranchScope::Copied(plan) => Some((*name, *plan)),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// THE GUARD. Every column family the database defines must be classified
    /// here. Add a `cf::` constant without deciding whether a branch fork must
    /// copy it and this test fails — which is precisely the failure that
    /// `ARCHETYPES`/`ELEMENT_TYPES` and then `SPATIAL_INDEX` got away with
    /// silently, each time surfacing much later as missing data on a fork.
    #[test]
    fn every_column_family_is_classified_for_branch_forking() {
        let classified: HashSet<&str> = BRANCH_CF_REGISTRY.iter().map(|(n, _)| *n).collect();
        let unclassified: Vec<&str> = crate::all_column_families()
            .into_iter()
            .filter(|name| !classified.contains(name))
            .collect();

        assert!(
            unclassified.is_empty(),
            "column families are not classified in BRANCH_CF_REGISTRY: {unclassified:?}.\n\
             Decide, explicitly, whether a branch fork must copy each one:\n\
             - key has no branch segment      -> BranchScope::NotBranchScoped\n\
             - branch-scoped, must be forked  -> BranchScope::Copied(..)\n\
             - branch-scoped, must NOT be     -> BranchScope::SkippedOnPurpose(reason)\n\
             Verify by READING the key builder, not by the CF's name.",
        );
    }

    /// The registry must not describe column families that do not exist, and
    /// must not list one twice (a duplicate would copy it twice).
    #[test]
    fn the_registry_contains_no_unknown_or_duplicate_column_families() {
        let real: HashSet<&str> = crate::all_column_families().into_iter().collect();
        let mut seen: HashSet<&str> = HashSet::new();
        for (name, _) in BRANCH_CF_REGISTRY {
            assert!(real.contains(name), "{name} is not a real column family");
            assert!(seen.insert(name), "{name} is listed twice in the registry");
        }
    }

    /// Regression pin: the two CFs whose omission has already caused a
    /// production defect, plus the ones added alongside them, must stay in the
    /// copy set. A refactor that drops one should fail loudly here.
    #[test]
    fn the_indexes_whose_omission_broke_forks_are_copied() {
        let copied: HashSet<&str> = cfs_to_copy().map(|(n, _)| n).collect();
        for required in [
            cf::ARCHETYPES,
            cf::ELEMENT_TYPES,
            cf::SPATIAL_INDEX,
            cf::COMPOUND_INDEX,
            cf::UNIQUE_INDEX,
            cf::NODES,
            cf::PATH_INDEX,
            // A fork that drops secrets still LOOKS healthy: reads return the
            // `secret://` reference without resolving it, so every vaulted field
            // renders normally right up until something tries to reveal one.
            cf::SECRETS,
        ] {
            assert!(copied.contains(required), "{required} must be forked");
        }
    }
}
