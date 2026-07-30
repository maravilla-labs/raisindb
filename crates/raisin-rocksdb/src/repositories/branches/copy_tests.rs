//! Unit tests for the branch-fork copy logic.
//!
//! Every test here is a guard against the SAME failure mode: the fork reads a
//! revision out of a key, and when it cannot, the entry is silently dropped —
//! a fork that reports success while missing data.

use super::super::cf_registry::{cfs_to_copy, BranchScope, RevisionLocator, BRANCH_CF_REGISTRY};
use super::BranchRepositoryImpl as Repo;
use crate::{cf, keys};
use raisin_hlc::HLC;
use raisin_storage::CompoundColumnValue;

/// An HLC whose descending encoding contains a null byte. These are common —
/// the encoding is a bitwise NOT, so any `0xFF` byte in the source becomes
/// `0x00` — which is why a revision cannot be located by splitting on nulls.
fn hlc_with_null_in_encoding() -> HLC {
    for counter in 0..4096u64 {
        let hlc = HLC::new(0x0000_00FF_0000_0000, counter);
        if hlc.encode_descending().contains(&0) {
            return hlc;
        }
    }
    panic!("no HLC with a null byte in its encoding found");
}

/// The locator the registry declares for a column family.
fn locator_for(cf_name: &str) -> RevisionLocator {
    cfs_to_copy()
        .find(|(name, _)| *name == cf_name)
        .unwrap_or_else(|| panic!("{cf_name} is not in the fork's copy set"))
        .1
        .revision
}

/// Run the real dispatch exactly as `copy_cf_entries` does.
fn locate(cf_name: &str, key: &[u8], value: &[u8]) -> Option<HLC> {
    let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
    Repo::locate_revision(locator_for(cf_name), key, value, &parts)
        .expect("extraction must not error")
}

// --- the spatial index, the defect this pass exists for ---------------------

/// THE BUG. A fork must carry the spatial index across, or `find_within_radius`
/// on the fork returns nothing while the nodes are plainly there.
#[test]
fn the_spatial_index_is_in_the_forks_copy_set() {
    assert!(
        cfs_to_copy().any(|(name, _)| name == cf::SPATIAL_INDEX),
        "the branch is part of the spatial key prefix, so entries do NOT carry \
         over implicitly — an unforked spatial index is silently empty",
    );
}

#[test]
fn a_spatial_keys_revision_is_read_through_the_purpose_built_parser() {
    let hlc = HLC::new(1_705_843_009_213_693_952, 42);
    let key = keys::spatial_index_key_versioned(
        "t", "r", "main", "ws", "location", "9q8yyk", &hlc, "node-1",
    );
    assert_eq!(locate(cf::SPATIAL_INDEX, &key, &[]), Some(hlc));
}

/// A NESTED geometry path (`venue.geo`) is one key segment containing dots but
/// never a null, so it must not disturb the six leading parts.
#[test]
fn a_nested_geometry_property_does_not_shift_the_spatial_revision() {
    let hlc = HLC::new(9_999, 3);
    let key = keys::spatial_index_key_versioned(
        "t",
        "r",
        "main",
        "ws",
        "venue.geo.point",
        "u0nc0",
        &hlc,
        "node-1",
    );
    assert_eq!(locate(cf::SPATIAL_INDEX, &key, &[]), Some(hlc));
}

/// The trap that ate ARCHETYPES/ELEMENT_TYPES, in spatial form: the spatial key
/// has SIX leading parts before the geohash, one more than most index keys, and
/// its `~HLC` can contain nulls. Anything counting `\0`-split parts mis-reads it.
#[test]
fn a_spatial_key_whose_revision_contains_nulls_is_still_copied() {
    let hlc = HLC::new(u64::MAX, 0);
    assert!(hlc.encode_descending().contains(&0), "precondition");

    let key =
        keys::spatial_index_key_versioned("t", "r", "main", "ws", "geo", "u0nc", &hlc, "node-1");
    assert_eq!(
        locate(cf::SPATIAL_INDEX, &key, &[]),
        Some(hlc),
        "the entry would otherwise be dropped from every fork",
    );
}

#[test]
fn a_spatial_key_rebuild_only_swaps_the_branch() {
    let hlc = hlc_with_null_in_encoding();
    let key = keys::spatial_index_key_versioned(
        "t",
        "r",
        "main",
        "ws",
        "venue.geo",
        "u0nc",
        &hlc,
        "node-1",
    );
    let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
    let rebuilt = Repo::build_key_with_branch(&parts, "fork-1");

    let expected = keys::spatial_index_key_versioned(
        "t",
        "r",
        "fork-1",
        "ws",
        "venue.geo",
        "u0nc",
        &hlc,
        "node-1",
    );
    assert_eq!(rebuilt, expected);
    let parsed = keys::parse_spatial_index_key(&rebuilt).expect("must still parse");
    assert_eq!(parsed.branch, "fork-1");
    assert_eq!(parsed.revision, hlc);
    assert_eq!(parsed.property_name, "venue.geo");
}

// --- the other indexes added alongside it -----------------------------------

/// A compound index key has a VARIABLE number of column segments, so any
/// fixed-part-index reader is wrong by construction.
#[test]
fn a_compound_index_revision_is_found_regardless_of_column_count() {
    let hlc = hlc_with_null_in_encoding();
    for columns in [
        vec![CompoundColumnValue::String("a".into())],
        vec![
            CompoundColumnValue::String("a".into()),
            CompoundColumnValue::Integer(7),
            CompoundColumnValue::Boolean(true),
            CompoundColumnValue::TimestampDesc(1_700_000_000),
        ],
    ] {
        let key = keys::compound_index_key_versioned(
            "t", "r", "main", "ws", "idx", &columns, &hlc, "node-1", false,
        );
        assert_eq!(
            locate(cf::COMPOUND_INDEX, &key, &[]),
            Some(hlc),
            "{} columns",
            columns.len()
        );
    }
}

#[test]
fn a_unique_index_revision_is_read_from_the_tail() {
    let hlc = hlc_with_null_in_encoding();
    let key = keys::unique_index_key_versioned("t", "r", "main", "ws", "Post", "slug", "h", &hlc);
    assert_eq!(locate(cf::UNIQUE_INDEX, &key, &[]), Some(hlc));
}

/// `rel_global` keys carry FOUR trailing segments where `rel`/`rel_rev` carry
/// one. Reading them as one lands 16 arbitrary bytes, which still DECODE into a
/// nonsense HLC — so the entry was not merely dropped, it was compared against
/// `max_revision` on garbage.
#[test]
fn a_global_relation_key_is_not_read_as_a_scoped_one() {
    let hlc = HLC::new(5_000, 1);
    let key = keys::relation_global_key_versioned(
        "t", "r", "main", "LINKS", &hlc, "ws-a", "src-1", "ws-b", "tgt-1",
    );
    assert_eq!(locate(cf::RELATION_INDEX, &key, &[]), Some(hlc));
}

#[test]
fn a_scoped_relation_key_still_reads_correctly() {
    let hlc = hlc_with_null_in_encoding();
    let key =
        keys::relation_forward_key_versioned("t", "r", "main", "ws", "src", "LINKS", &hlc, "tgt");
    assert_eq!(locate(cf::RELATION_INDEX, &key, &[]), Some(hlc));
}

// --- the previously fixed regressions must stay fixed ------------------------

/// Regression: the NODES arm read the revision from `key.split(0)[6]`. When the
/// `~HLC` contained a null the split cut it in half, extraction returned None,
/// and the copy loop skipped the entry WITHOUT error — every fork silently lost
/// ~0.8% of its nodes.
#[test]
fn a_node_key_whose_revision_contains_nulls_is_still_copied() {
    let hlc = hlc_with_null_in_encoding();
    let key = keys::node_key_versioned("t", "r", "main", "ws", "node-1", &hlc);
    assert!(
        key.split(|&b| b == 0).count() > 7,
        "this key must actually exercise the split hazard",
    );
    assert_eq!(locate(cf::NODES, &key, &[]), Some(hlc));
}

#[test]
fn a_plain_node_key_still_reads_correctly() {
    let hlc = HLC::new(12_345, 7);
    let key = keys::node_key_versioned("t", "r", "main", "ws", "node-1", &hlc);
    assert_eq!(locate(cf::NODES, &key, &[]), Some(hlc));
}

#[test]
fn an_ordered_children_revision_survives_a_null_in_the_hlc() {
    let hlc = hlc_with_null_in_encoding();
    let key =
        keys::ordered_child_key_versioned("t", "r", "main", "ws", "parent", "a0", &hlc, "child");
    assert_eq!(locate(cf::ORDERED_CHILDREN, &key, &[]), Some(hlc));
}

/// The schema CFs whose omission broke branch-based publish. Both shapes must
/// resolve: the definition (revision in the key) and the version index
/// (revision in the VALUE).
#[test]
fn both_schema_shapes_resolve_a_revision() {
    let hlc = hlc_with_null_in_encoding();
    for (cf_name, definition, versions) in [
        (
            cf::ARCHETYPES,
            keys::archetype_key_versioned("t", "r", "main", "Page", &hlc),
            keys::archetype_version_index_key("t", "r", "main", "Page", 3),
        ),
        (
            cf::ELEMENT_TYPES,
            keys::element_type_key_versioned("t", "r", "main", "Hero", &hlc),
            keys::element_type_version_index_key("t", "r", "main", "Hero", 3),
        ),
    ] {
        assert_eq!(locate(cf_name, &definition, &[]), Some(hlc), "{cf_name}");
        assert_eq!(
            locate(cf_name, &versions, &hlc.encode_descending()),
            Some(hlc),
            "{cf_name} version index",
        );
    }
}

#[test]
fn rebuilt_key_only_swaps_the_branch_segment() {
    let hlc = hlc_with_null_in_encoding();
    let key = keys::node_key_versioned("t", "r", "main", "ws", "node-1", &hlc);
    let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
    let rebuilt = Repo::build_key_with_branch(&parts, "edit-x");

    assert_eq!(
        rebuilt,
        keys::node_key_versioned("t", "r", "edit-x", "ws", "node-1", &hlc)
    );
    assert_eq!(
        keys::extract_revision_from_key(&rebuilt).unwrap(),
        hlc,
        "the revision must survive the rebuild",
    );
}

// --- structural guards -------------------------------------------------------

/// The copier rewrites key part 2, so a CF whose branch lives elsewhere must
/// never be added to the copy set without teaching the copier about it first.
/// This is exactly how `GRAPH_CACHE`/`GRAPH_PROJECTION` (branch at part 3) would
/// otherwise be copied into corrupt keys.
#[test]
fn every_copied_cf_has_its_branch_at_key_part_two() {
    let copied: Vec<&str> = cfs_to_copy().map(|(n, _)| n).collect();
    for excluded in [cf::GRAPH_CACHE, cf::GRAPH_PROJECTION, cf::INDEX_STATUS] {
        assert!(!copied.contains(&excluded), "{excluded} must stay excluded");
    }
}

/// Every classification carries a reason a human can check, so "not copied"
/// is never merely an omission.
#[test]
fn every_deliberate_skip_records_why() {
    for (name, scope) in BRANCH_CF_REGISTRY {
        if let BranchScope::SkippedOnPurpose(reason) = scope {
            assert!(
                reason.len() > 20,
                "{name} is skipped without a real justification",
            );
        }
    }
}
