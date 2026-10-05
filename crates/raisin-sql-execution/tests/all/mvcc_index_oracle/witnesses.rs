//! Fixed, minimal histories: the witnesses of the expected-failure list and
//! the regression histories for bugs the oracle found and that were fixed.
//!
//! Selectors resolve against the model's sorted live ids (`o001`, `o002`, …
//! in creation order), so `target: 1` is the second node created.

use super::ops::{Edit, ForkSpec, Funnel, Op, Props};

pub fn plain(title: u8) -> Props {
    Props {
        title,
        rank: None,
        link: None,
        card: None,
    }
}

/// A Page named `n{name}` under the `parent`-th live node (or the root).
pub fn page(parent: Option<u16>, name: u8) -> Op {
    Op::Create {
        parent,
        name,
        ty: 0,
        props: plain(0),
    }
}

/// Every named witness: the fixed regressions first, then the expected
/// failures' witnesses. `ORACLE_WITNESS=<name>` replays one.
pub fn all() -> Vec<(&'static str, fn() -> Vec<Op>)> {
    use super::witnesses_fixed as fixed;
    use super::witnesses_review as review;
    vec![
        (
            "reorder_restamps_updated_at",
            fixed::reorder_restamps_updated_at,
        ),
        ("copy_root_appends", fixed::copy_root_appends),
        ("reorder_volatile", fixed::reorder_volatile),
        (
            "delete_keeps_reference_history",
            fixed::delete_keeps_reference_history,
        ),
        ("reference_shapes", fixed::reference_shapes),
        ("ancestor_move_path", fixed::ancestor_move_path),
        ("rename_history_paths", fixed::rename_history_paths),
        ("reorder_after_merge", fixed::reorder_after_merge),
        ("restore_reindexes", fixed::restore_reindexes),
        ("restore_after_move", review::restore_after_move),
        ("restore_after_rename", review::restore_after_rename),
        ("keep_ours_then_update", fixed::keep_ours_then_update),
        (
            "volatile_parent_has_children",
            fixed::volatile_parent_has_children,
        ),
        ("compound_after_merge", fixed::compound_after_merge),
        ("compound_plain", fixed::compound_plain),
        (
            "create_and_move_into_one_parent_in_one_tx",
            review::create_and_move_into_one_parent_in_one_tx,
        ),
        (
            "main_in_place_write_survives_merge",
            review::main_in_place_write_survives_merge,
        ),
        (
            "move_and_create_into_one_parent_in_one_tx",
            review::move_and_create_into_one_parent_in_one_tx,
        ),
        ("locale_read_at_past_revision", locale_read_at_past_revision),
        (
            "keep_ours_over_source_delete_keeps_translation",
            keep_ours_over_source_delete_keeps_translation,
        ),
        (
            "merge_rewrites_target_history",
            merge_rewrites_target_history,
        ),
        ("compound_at_past_revision", compound_at_past_revision),
    ]
}

/// A locale read at a revision before the overlay was written.
pub fn locale_read_at_past_revision() -> Vec<Op> {
    vec![
        page(None, 0),
        Op::Update {
            target: 0,
            props: plain(1),
            funnel: Funnel::Tx,
        },
        Op::Translate {
            target: 0,
            title: 2,
            hide: false,
        },
    ]
}

/// KeepOurs over "source deleted, target modified" keeps the target's node —
/// and its translation overlays.
pub fn keep_ours_over_source_delete_keeps_translation() -> Vec<Op> {
    vec![
        page(None, 0),
        page(None, 1),
        Op::Translate {
            target: 1,
            title: 1,
            hide: false,
        },
        Op::ForkMerge(ForkSpec {
            feature: vec![Edit::DeleteLeaf { target: 1 }],
            main: vec![Edit::Update {
                target: 1,
                props: plain(2),
            }],
            keep_ours: vec![true],
        }),
    ]
}

/// A merge must not change what the TARGET looked like before it.
pub fn merge_rewrites_target_history() -> Vec<Op> {
    vec![
        page(None, 0),
        Op::ForkMerge(ForkSpec {
            feature: vec![Edit::Update {
                target: 0,
                props: plain(1),
            }],
            main: vec![Edit::Create {
                parent: None,
                name: 1,
                props: plain(0),
            }],
            keep_ours: vec![true],
        }),
    ]
}

/// A typed folder listing (compound index) at a revision before an ancestor
/// rename.
pub fn compound_at_past_revision() -> Vec<Op> {
    vec![
        page(None, 0),
        page(Some(0), 1),
        page(Some(1), 2),
        // Renaming the ancestor re-keys the subtree's `__parent_path` entries.
        Op::Rename { target: 0, name: 5 },
    ]
}
