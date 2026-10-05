//! Fixed regression histories for bugs the oracle found and that were fixed.
//! `regression_witnesses_pass` replays each one; it must answer cleanly.

use super::ops::{Edit, ForkSpec, Funnel, Op, Props};
use super::witnesses::{page, plain};

/// A reorder re-stamps `updated_at`; ORDER BY updated_at must still see it.
pub fn reorder_restamps_updated_at() -> Vec<Op> {
    vec![
        page(None, 0),
        page(None, 1),
        Op::Reorder {
            target: 1,
            anchor: 0,
            before: true,
        },
    ]
}

/// A copied subtree root is APPENDED under its new parent, with
/// `order_key == ORDERED_CHILDREN label`.
pub fn copy_root_appends() -> Vec<Op> {
    vec![
        page(None, 0),
        page(Some(0), 1),
        page(None, 2),
        Op::Copy {
            target: 1,
            parent: None,
            name: 3,
        },
        Op::Copy {
            target: 0,
            parent: Some(2),
            name: 4,
        },
    ]
}

/// Reordering a `versionable=false` node re-stamps `updated_at` like any other.
pub fn reorder_volatile() -> Vec<Op> {
    vec![
        Op::Create {
            parent: None,
            name: 0,
            ty: 5,
            props: plain(0),
        },
        page(None, 1),
        Op::Reorder {
            target: 0,
            anchor: 0,
            before: false,
        },
    ]
}

/// Deleting a node must not erase its reference entries at earlier revisions.
pub fn delete_keeps_reference_history() -> Vec<Op> {
    vec![
        page(None, 0),
        Op::Create {
            parent: None,
            name: 1,
            ty: 0,
            props: Props {
                title: 1,
                rank: None,
                link: Some(0),
                card: Some((true, 0)),
            },
        },
        Op::Create {
            parent: None,
            name: 2,
            ty: 0,
            props: Props {
                title: 1,
                rank: None,
                link: None,
                card: Some((false, 0)),
            },
        },
        Op::Delete {
            target: 1,
            cascade: true,
            tx: true,
        },
    ]
}

/// One referrer per reference shape: top-level, in an Element, in a Composite.
pub fn reference_shapes() -> Vec<Op> {
    let with = |name: u8, link: Option<u16>, card: Option<(bool, u16)>| Op::Create {
        parent: None,
        name,
        ty: 0,
        props: Props {
            title: 1,
            rank: None,
            link,
            card,
        },
    };
    vec![
        page(None, 0),
        with(1, Some(0), None),
        with(2, None, Some((false, 0))),
        with(3, None, Some((true, 0))),
    ]
}

/// Moving an ancestor changes every descendant's path, at every depth.
pub fn ancestor_move_path() -> Vec<Op> {
    vec![
        page(None, 0),
        page(Some(0), 1),
        page(Some(1), 2),
        page(None, 3),
        Op::Update {
            target: 2,
            props: plain(2),
            funnel: Funnel::Tx,
        },
        Op::Move {
            target: 0,
            parent: Some(3),
            tx: false,
        },
        Op::Copy {
            target: 0,
            parent: None,
            name: 5,
        },
    ]
}

/// A rename must not rewrite the paths descendants had BEFORE it.
pub fn rename_history_paths() -> Vec<Op> {
    let ty = |name: u8, parent: Option<u16>, ty: u8| Op::Create {
        parent,
        name,
        ty,
        props: plain(0),
    };
    vec![
        ty(7, None, 5),
        ty(7, Some(0), 0),
        ty(7, Some(1), 5),
        page(None, 2),
        Op::Rename { target: 0, name: 5 },
        Op::Volatile {
            target: 0,
            props: plain(3),
        },
    ]
}

/// A node created on `main` while a fork is open, reordered after the merge.
pub fn reorder_after_merge() -> Vec<Op> {
    vec![
        page(None, 0),
        page(Some(0), 1),
        Op::ForkMerge(ForkSpec {
            feature: vec![Edit::Update {
                target: 0,
                props: plain(1),
            }],
            main: vec![Edit::Create {
                parent: None,
                name: 2,
                props: plain(2),
            }],
            keep_ours: vec![true],
        }),
        Op::Reorder {
            target: 2,
            anchor: 0,
            before: false,
        },
    ]
}

/// RESTORE writes the historical content back, and every index follows it.
pub fn restore_reindexes() -> Vec<Op> {
    vec![
        page(None, 0),
        page(None, 1),
        Op::Update {
            target: 0,
            props: plain(1),
            funnel: Funnel::Tx,
        },
        Op::Restore { target: 0, back: 0 },
    ]
}

/// KeepOurs over a conflict, then an ordinary update: nothing the source side
/// indexed may survive.
pub fn keep_ours_then_update() -> Vec<Op> {
    let linked = |title: u8| Props {
        title,
        rank: Some(1),
        link: Some(1),
        card: None,
    };
    vec![
        page(None, 0),
        page(None, 1),
        Op::ForkMerge(ForkSpec {
            feature: vec![Edit::Update {
                target: 0,
                props: linked(1),
            }],
            main: vec![Edit::Update {
                target: 0,
                props: plain(2),
            }],
            keep_ours: vec![true],
        }),
        Op::Update {
            target: 0,
            props: plain(3),
            funnel: Funnel::Sql,
        },
        Op::Delete {
            target: 1,
            cascade: true,
            tx: false,
        },
    ]
}

/// `has_children` of a `versionable=false` parent that gained a child.
pub fn volatile_parent_has_children() -> Vec<Op> {
    vec![
        page(None, 0),
        Op::Create {
            parent: Some(0),
            name: 1,
            ty: 5,
            props: plain(0),
        },
        page(None, 3),
        // Move o003 under the childless volatile o002, then rewrite o002 in
        // place.
        Op::Move {
            target: 2,
            parent: Some(1),
            tx: false,
        },
        Op::Volatile {
            target: 0,
            props: plain(1),
        },
    ]
}

/// A typed folder listing that includes a child the merge brought in.
pub fn compound_after_merge() -> Vec<Op> {
    vec![
        page(None, 0),
        Op::ForkMerge(ForkSpec {
            feature: vec![Edit::Create {
                parent: None,
                name: 1,
                props: plain(0),
            }],
            main: vec![],
            keep_ours: vec![true],
        }),
    ]
}

/// Two typed root children, no merge.
pub fn compound_plain() -> Vec<Op> {
    vec![page(None, 0), page(None, 1)]
}
