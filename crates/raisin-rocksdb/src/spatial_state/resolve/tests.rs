//! Unit tests for the reconciliation rules.

use super::*;
fn policy(precisions: &[usize]) -> SpatialPolicy {
    SpatialPolicy {
        precisions: sorted_precisions(precisions.to_vec()),
        ..Default::default()
    }
}

fn state(p: &SpatialPolicy, phase: SpatialBuildPhase) -> SpatialIndexState {
    let mut s = SpatialIndexState::ready(p, HLC::new(1, 0));
    s.phase = phase;
    s
}

#[test]
fn write_policy_comes_from_configuration_not_from_the_state_record() {
    let indexed = policy(&[8, 7, 6]);
    let configured = policy(&[10, 9]);
    let st = state(&indexed, SpatialBuildPhase::Ready);

    let r = resolve_write_policy(configured.clone(), Some(&st));

    // The union keeps the old cells alive; the NEW precisions are present,
    // which is exactly what the old `from_local_state` never produced.
    assert_eq!(r.write.precisions, vec![10, 9, 8, 7, 6]);
    assert!(r.write.precisions.contains(&10));
    assert!(matches!(
        r.needs_build,
        Some(BuildTrigger::PolicyChanged { .. })
    ));
    assert!(r.needs_build.unwrap().rebuild());
}

#[test]
fn matching_policy_needs_no_build_and_writes_the_configured_set() {
    let configured = policy(&[9, 8]);
    let st = state(&configured, SpatialBuildPhase::Ready);
    let r = resolve_write_policy(configured.clone(), Some(&st));
    assert_eq!(r.write.precisions, configured.precisions);
    assert!(r.needs_build.is_none());
}

#[test]
fn no_state_record_writes_the_configured_set_and_asks_for_a_backfill() {
    let configured = policy(&[6, 5]);
    let r = resolve_write_policy(configured.clone(), None);
    assert_eq!(r.write.precisions, configured.precisions);
    assert_eq!(r.needs_build, Some(BuildTrigger::NeverBuilt));
    assert!(!r.needs_build.unwrap().rebuild());
}

#[test]
fn not_built_phase_asks_for_a_gap_fill_not_a_rebuild() {
    let configured = policy(&[8]);
    let st = state(&configured, SpatialBuildPhase::NotBuilt);
    let r = resolve_write_policy(configured, Some(&st));
    assert_eq!(r.needs_build, Some(BuildTrigger::MarkedNotBuilt));
    assert!(!r.needs_build.unwrap().rebuild());
}

#[test]
fn queries_use_the_old_precisions_until_the_build_starts() {
    let indexed = policy(&[8, 7]);
    let configured = policy(&[6, 5]);
    let st = state(&indexed, SpatialBuildPhase::Ready);
    match availability_in_rebuild_window(&st, &configured) {
        SpatialAvailability::Ready { precisions, .. } => assert_eq!(precisions, vec![8, 7]),
        other => panic!("expected Ready, got {:?}", other),
    }
}

#[test]
fn queries_use_the_overlap_during_a_rebuild() {
    let indexed = policy(&[9, 8, 7]);
    let configured = policy(&[8, 7, 6]);
    let st = state(&indexed, SpatialBuildPhase::Building);
    match availability_in_rebuild_window(&st, &configured) {
        SpatialAvailability::Ready { precisions, .. } => assert_eq!(precisions, vec![8, 7]),
        other => panic!("expected Ready, got {:?}", other),
    }
}

#[test]
fn a_disjoint_rebuild_fails_loudly_instead_of_answering_partially() {
    let indexed = policy(&[9, 8]);
    let configured = policy(&[5, 4]);
    let st = state(&indexed, SpatialBuildPhase::Building);
    match availability_in_rebuild_window(&st, &configured) {
        SpatialAvailability::Unusable(reason) => assert!(reason.contains("rebuilt")),
        other => panic!("expected Unusable, got {:?}", other),
    }
}

#[test]
fn a_changed_bucket_property_withdraws_the_prefilter() {
    let mut indexed = policy(&[8]);
    indexed.bucket_property = Some("floor".into());
    let mut configured = policy(&[8]);
    configured.bucket_property = Some("level".into());
    let st = state(&indexed, SpatialBuildPhase::Ready);
    match availability_in_rebuild_window(&st, &configured) {
        SpatialAvailability::Ready {
            bucket_property, ..
        } => assert_eq!(bucket_property, None),
        other => panic!("expected Ready, got {:?}", other),
    }
}

#[test]
fn cover_widens_to_extent_during_the_window() {
    let mut indexed = policy(&[8]);
    indexed.cover = SpatialCoverMode::Extent;
    let configured = policy(&[7]);
    let merged = union_policy(&configured, &indexed);
    assert_eq!(merged.cover, SpatialCoverMode::Extent);
    assert_eq!(merged.precisions, vec![8, 7]);
}
