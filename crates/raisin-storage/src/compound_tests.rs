use super::*;
use raisin_models::nodes::properties::schema::{CompoundColumnType, CompoundIndexColumn};

fn def(name: &str, cols: &[(&str, CompoundColumnType)]) -> CompoundIndexDefinition {
    CompoundIndexDefinition {
        name: name.to_string(),
        columns: cols
            .iter()
            .map(|(p, t)| CompoundIndexColumn {
                property: p.to_string(),
                column_type: t.clone(),
                ascending: None,
            })
            .collect(),
        has_order_column: false,
        owner_node_type: None,
    }
}

#[test]
fn a_ready_record_matching_its_declaration_is_ready() {
    let d = def("idx", &[("status", CompoundColumnType::String)]);
    let state = CompoundIndexState::ready(&d, HLC::new(7, 0));
    assert!(state.availability_for(&d).is_ready());
}

/// The case the record exists for. Entries in the keyspace were written
/// under the OLD columns; reading them through the new layout is silent
/// corruption, so a changed declaration must read as unusable.
#[test]
fn a_changed_declaration_makes_the_build_unusable() {
    let built = def("idx", &[("status", CompoundColumnType::String)]);
    let state = CompoundIndexState::ready(&built, HLC::new(7, 0));

    let reordered = def(
        "idx",
        &[
            ("buyer", CompoundColumnType::String),
            ("status", CompoundColumnType::String),
        ],
    );
    match state.availability_for(&reordered) {
        CompoundAvailability::Unusable(reason) => {
            assert!(reason.contains("different declaration"), "{reason}");
        }
        other => panic!("expected Unusable, got {other:?}"),
    }
}

/// Column ORDER is identity: `(a, b)` and `(b, a)` produce different key
/// bytes, so they must not share a fingerprint.
#[test]
fn column_order_changes_the_fingerprint() {
    let ab = def(
        "idx",
        &[
            ("a", CompoundColumnType::String),
            ("b", CompoundColumnType::String),
        ],
    );
    let ba = def(
        "idx",
        &[
            ("b", CompoundColumnType::String),
            ("a", CompoundColumnType::String),
        ],
    );
    assert_ne!(ab.definition_hash(), ba.definition_hash());
}

/// A type change alters the ENCODING (`Integer` is big-endian bytes,
/// `String` is UTF-8), so it must invalidate the build too.
#[test]
fn column_type_changes_the_fingerprint() {
    let as_string = def("idx", &[("qty", CompoundColumnType::String)]);
    let as_int = def("idx", &[("qty", CompoundColumnType::Integer)]);
    assert_ne!(as_string.definition_hash(), as_int.definition_hash());
}

/// A rebuild CLEARS the keyspace before writing, so mid-build there is no
/// complete generation to serve — unlike spatial, where `Building` stays
/// queryable against the older entries.
#[test]
fn a_building_record_is_not_queryable() {
    let d = def("idx", &[("status", CompoundColumnType::String)]);
    let mut state = CompoundIndexState::ready(&d, HLC::new(7, 0));
    state.phase = CompoundBuildPhase::Building;
    assert!(!state.availability_for(&d).is_ready());
}

#[test]
fn an_unsupported_record_version_is_unusable() {
    let d = def("idx", &[("status", CompoundColumnType::String)]);
    let mut state = CompoundIndexState::ready(&d, HLC::new(7, 0));
    state.v = CompoundIndexState::VERSION + 1;
    assert!(matches!(
        state.availability_for(&d),
        CompoundAvailability::Unusable(_)
    ));
}

/// A build keeps no history below the HEAD it read: a read pinned below that
/// floor must not be served by the index (time travel before a rebuild).
#[test]
fn a_read_below_the_build_floor_is_unusable() {
    let d = def("idx", &[("status", CompoundColumnType::String)]);
    let ready = CompoundIndexState::ready(&d, HLC::new(7, 0)).availability_for(&d);
    assert!(ready.clone().at_revision(None).is_ready(), "HEAD read");
    assert!(ready.clone().at_revision(Some(&HLC::new(7, 0))).is_ready());
    assert!(ready.clone().at_revision(Some(&HLC::new(9, 0))).is_ready());
    assert!(!ready.at_revision(Some(&HLC::new(6, 5))).is_ready());
}

/// An older record format is a format upgrade, not a stale mark.
#[test]
fn an_older_record_version_is_a_format_upgrade() {
    let d = def("idx", &[("status", CompoundColumnType::String)]);
    let mut state = CompoundIndexState::ready(&d, HLC::new(7, 0));
    assert!(!state.is_format_upgrade());
    state.v = CompoundIndexState::VERSION - 1;
    assert!(state.is_format_upgrade());
}
