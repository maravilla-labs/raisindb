//! Fixed regression histories from the review pass over Phase 0b (see the
//! plan's items 12-18). `regression_witnesses_pass` replays each one.

use super::ops::{Edit, ForkSpec, Funnel, Op, TxStep};
use super::witnesses::{page, plain};

/// A create and a move appending under ONE parent in ONE transaction mint
/// distinct labels, in either order. The move minted from the database alone
/// and handed out the create's label (same fractional part, same transaction
/// HLC suffix); a later reorder between the two then failed.
pub fn create_and_move_into_one_parent_in_one_tx() -> Vec<Op> {
    vec![
        page(None, 0),
        page(None, 1),
        page(Some(0), 2),
        Op::Tx(vec![
            TxStep::Create {
                parent: Some(1),
                name: 3,
                props: plain(1),
            },
            TxStep::MoveInto {
                target: 1,
                parent: Some(1),
            },
        ]),
        Op::Reorder {
            target: 2,
            anchor: 0,
            before: true,
        },
    ]
}

/// As above, the move first.
pub fn move_and_create_into_one_parent_in_one_tx() -> Vec<Op> {
    vec![
        page(None, 0),
        page(None, 1),
        page(Some(0), 2),
        Op::Tx(vec![
            TxStep::MoveInto {
                target: 2,
                parent: Some(1),
            },
            TxStep::Create {
                parent: Some(1),
                name: 3,
                props: plain(1),
            },
        ]),
        Op::Reorder {
            target: 3,
            anchor: 0,
            before: true,
        },
    ]
}

/// An in-place write on `main` inside a fork window survives the merge. The
/// merge re-copied every source entry up to the source HEAD — pre-fork keys
/// included — over the target's, so the feature's stale view of the very key
/// `main` had rewritten in place (record AND property entries) won: HEAD read
/// the old title, and the new title's entry stayed live beside it.
pub fn main_in_place_write_survives_merge() -> Vec<Op> {
    vec![
        Op::Create {
            parent: None,
            name: 0,
            ty: 5,
            props: plain(0),
        },
        page(None, 1),
        Op::ForkMerge(ForkSpec {
            feature: vec![Edit::Update {
                target: 0,
                props: plain(2),
            }],
            main: vec![Edit::Volatile {
                target: 0,
                props: plain(1),
            }],
            keep_ours: vec![true],
        }),
    ]
}

/// RESTORE to a revision before the node MOVED: the historical node is read by
/// id (a by-path lookup at that revision found nothing at today's path), its
/// content comes back, and it stays where it is now.
pub fn restore_after_move() -> Vec<Op> {
    vec![
        page(None, 0),
        page(None, 1),
        Op::Update {
            target: 0,
            props: plain(1),
            funnel: Funnel::Tx,
        },
        Op::Move {
            target: 0,
            parent: Some(1),
            tx: false,
        },
        Op::Restore { target: 0, back: 0 },
    ]
}

/// RESTORE to a revision before a RENAME keeps the current name.
pub fn restore_after_rename() -> Vec<Op> {
    vec![
        page(None, 0),
        Op::Update {
            target: 0,
            props: plain(1),
            funnel: Funnel::Tx,
        },
        Op::Rename { target: 0, name: 5 },
        Op::Restore { target: 0, back: 0 },
    ]
}
