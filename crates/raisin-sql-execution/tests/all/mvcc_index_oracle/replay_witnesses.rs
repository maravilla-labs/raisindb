//! Stage-3 witnesses: one operation kind each, on origin A, replayed into a
//! replica. Each either converges (and stage 3 generates it) or is a named
//! expected failure (and stage 3 leaves it out until it converges).

use super::ops::{Funnel, Op, Props, TxStep};
use super::witnesses::{page, plain};

/// Witnesses whose expected failure is not tolerated in random runs (the
/// stage-3 generator avoids the shape); `expected_failures_still_fail`
/// asserts them.
pub const NEVER_TOLERATED: &[&str] = &["volatile_after_ancestor_move"];

fn base() -> Vec<Op> {
    vec![page(None, 0), page(None, 1), page(Some(0), 2)]
}

fn with(op: Op) -> Vec<Op> {
    let mut v = base();
    v.push(op);
    v
}

pub fn all() -> Vec<(&'static str, Vec<Op>)> {
    let linked = Props {
        title: 2,
        rank: Some(1),
        link: Some(1),
        card: Some((true, 0)),
    };
    vec![
        ("create", base()),
        (
            "update_tx",
            with(Op::Update {
                target: 0,
                props: linked.clone(),
                funnel: Funnel::Tx,
            }),
        ),
        (
            "update_repo",
            with(Op::Update {
                target: 0,
                props: linked.clone(),
                funnel: Funnel::Repo,
            }),
        ),
        (
            "update_sql",
            with(Op::Update {
                target: 0,
                props: linked,
                funnel: Funnel::Sql,
            }),
        ),
        ("retype", with(Op::Retype { target: 0 })),
        (
            "volatile",
            vec![
                Op::Create {
                    parent: None,
                    name: 0,
                    ty: 5,
                    props: plain(0),
                },
                Op::Volatile {
                    target: 0,
                    props: plain(1),
                },
            ],
        ),
        (
            "delete_tx_leaf",
            with(Op::Delete {
                target: 2,
                cascade: false,
                tx: true,
            }),
        ),
        (
            "delete_repo_cascade",
            with(Op::Delete {
                target: 0,
                cascade: true,
                tx: false,
            }),
        ),
        (
            "delete_repo_leaf",
            with(Op::Delete {
                target: 2,
                cascade: false,
                tx: false,
            }),
        ),
        (
            "move_tx",
            with(Op::Move {
                target: 2,
                parent: Some(1),
                tx: true,
            }),
        ),
        (
            "move_repo",
            with(Op::Move {
                target: 2,
                parent: Some(1),
                tx: false,
            }),
        ),
        ("rename", with(Op::Rename { target: 0, name: 5 })),
        (
            "move_tx_subtree",
            with(Op::Move {
                target: 0,
                parent: Some(1),
                tx: true,
            }),
        ),
        (
            "move_repo_subtree",
            with(Op::Move {
                target: 0,
                parent: Some(1),
                tx: false,
            }),
        ),
        (
            "reorder",
            with(Op::Reorder {
                target: 1,
                anchor: 0,
                before: true,
            }),
        ),
        (
            "copy",
            with(Op::Copy {
                target: 0,
                parent: Some(1),
                name: 4,
            }),
        ),
        ("translate", translate()),
        ("volatile_pair_in_one_tx", volatile_pair_in_one_tx()),
        (
            "volatile_beside_versioned_in_one_tx",
            volatile_beside_versioned_in_one_tx(),
        ),
        (
            "volatile_after_ancestor_move",
            volatile_after_ancestor_move(),
        ),
        (
            "restore",
            vec![
                page(None, 0),
                Op::Update {
                    target: 0,
                    props: plain(1),
                    funnel: Funnel::Tx,
                },
                Op::Restore { target: 0, back: 0 },
            ],
        ),
    ]
}

/// Origin A translates a node; the replica is asked in French.
pub fn translate() -> Vec<Op> {
    with(Op::Translate {
        target: 0,
        title: 1,
        hide: false,
    })
}

/// A `versionable=false` node rewritten in place after an ANCESTOR moved.
pub fn volatile_after_ancestor_move() -> Vec<Op> {
    vec![
        page(None, 0),
        Op::Create {
            parent: Some(0),
            name: 1,
            ty: 5,
            props: plain(0),
        },
        page(None, 2),
        Op::Move {
            target: 0,
            parent: Some(2),
            tx: true,
        },
        Op::Volatile {
            target: 0,
            props: plain(3),
        },
    ]
}

fn volatile(name: u8) -> Op {
    Op::Create {
        parent: None,
        name,
        ty: 5,
        props: plain(0),
    }
}

/// Two `versionable=false` nodes rewritten in place in ONE transaction, then
/// the first one again on its own. Each in-place write lands at its node's
/// own revision; one replicated op used to carry both at the newer one, so on
/// the replica the first node got a version at a revision the origin never
/// wrote it at, and its next in-place refresh landed beneath it, unseen.
pub fn volatile_pair_in_one_tx() -> Vec<Op> {
    vec![
        volatile(0),
        volatile(1),
        Op::Tx(vec![
            TxStep::Volatile {
                target: 0,
                props: plain(1),
            },
            TxStep::Volatile {
                target: 1,
                props: plain(2),
            },
        ]),
        Op::Volatile {
            target: 0,
            props: plain(3),
        },
    ]
}

/// An in-place write beside a versioned write in one transaction: the
/// versioned one lands at the transaction revision, the in-place one at its
/// node's own — not at the transaction's.
pub fn volatile_beside_versioned_in_one_tx() -> Vec<Op> {
    vec![
        volatile(0),
        page(None, 1),
        Op::Tx(vec![
            TxStep::Volatile {
                target: 0,
                props: plain(1),
            },
            TxStep::Update {
                target: 0,
                props: plain(2),
            },
        ]),
        Op::Volatile {
            target: 0,
            props: plain(3),
        },
    ]
}
