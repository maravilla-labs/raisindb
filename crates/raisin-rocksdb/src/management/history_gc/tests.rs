use super::*;

fn h(ts: u64) -> HLC {
    HLC::new(ts, 0)
}

fn plan(cutoff: u64, pins: &[u64]) -> ScopePlan {
    ScopePlan {
        cutoff: h(cutoff),
        pins: pins.iter().map(|p| h(*p)).collect(),
    }
}

#[test]
fn keeps_everything_after_the_cutoff_and_the_state_at_it() {
    // versions newest first: 9, 7, 5, 3, 1 — cutoff 6
    let revs = [h(9), h(7), h(5), h(3), h(1)];
    let keep = select_survivors(&revs, &[false; 5], &plan(6, &[]), false);
    assert_eq!(keep, vec![true, true, true, false, false]);
}

#[test]
fn a_pin_keeps_the_version_visible_at_it() {
    let revs = [h(9), h(7), h(5), h(3), h(1)];
    // tag at 4 sees version 3; tag at 2 sees version 1
    let keep = select_survivors(&revs, &[false; 5], &plan(6, &[4, 2]), false);
    assert_eq!(keep, vec![true, true, true, true, true]);
    let keep = select_survivors(&revs, &[false; 5], &plan(6, &[4]), false);
    assert_eq!(keep, vec![true, true, true, true, false]);
}

#[test]
fn a_cutoff_after_every_version_keeps_only_the_newest() {
    let revs = [h(5), h(3), h(1)];
    let keep = select_survivors(&revs, &[false; 3], &plan(100, &[]), false);
    assert_eq!(keep, vec![true, false, false]);
}

#[test]
fn a_deleted_entity_disappears_only_where_allowed() {
    // deleted at 5, live before
    let revs = [h(5), h(3), h(1)];
    let tomb = [true, false, false];
    let keep = select_survivors(&revs, &tomb, &plan(100, &[]), true);
    assert_eq!(keep, vec![false, false, false], "absent == deleted");
    let keep = select_survivors(&revs, &tomb, &plan(100, &[]), false);
    assert_eq!(
        keep,
        vec![true, false, false],
        "index families keep the tombstone"
    );
}

#[test]
fn a_tombstone_that_still_hides_a_pinned_version_stays() {
    // deleted at 5, live at 3, tag at 4 needs version 3 — and the tombstone
    // must stay, or a read at HEAD would see version 3 again.
    let revs = [h(5), h(3)];
    let tomb = [true, false];
    let keep = select_survivors(&revs, &tomb, &plan(100, &[4]), true);
    assert_eq!(keep, vec![true, true]);
}

#[test]
fn a_recent_tombstone_is_kept_while_older_live_versions_remain() {
    // deleted at 9 (after the cutoff), live at 3 (the state at the cutoff)
    let revs = [h(9), h(3)];
    let tomb = [true, false];
    let keep = select_survivors(&revs, &tomb, &plan(6, &[]), true);
    assert_eq!(keep, vec![true, true]);
}
