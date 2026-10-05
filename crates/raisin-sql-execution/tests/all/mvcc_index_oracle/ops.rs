//! The generated operation log.
//!
//! Ops carry abstract SELECTORS (`u16`), resolved against the model at apply
//! time (`sel % candidates.len()`), so a history stays meaningful as proptest
//! shrinks it: removing an op never turns a later one into an invalid request,
//! it only changes which node it lands on. An op whose precondition no longer
//! holds is skipped and logged.

use proptest::prelude::*;

/// What a write sets. Every field maps onto one property shape the indexes
/// treat differently: plain string, number, top-level reference, and a
/// reference nested inside an Element or a Composite.
#[derive(Clone, Debug)]
pub struct Props {
    pub title: u8,
    pub rank: Option<i8>,
    pub link: Option<u16>,
    /// `(composite?, target)`: a reference inside an Element/Composite block.
    pub card: Option<(bool, u16)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Funnel {
    /// `TransactionalContext::put_node` (the SQL/WS write path's layer).
    Tx,
    /// `NodeRepository::update` (the repository layer).
    Repo,
    /// `UPDATE ... SET properties = ...` through the query engine.
    Sql,
}

#[derive(Clone, Debug)]
pub enum Op {
    Create {
        parent: Option<u16>,
        name: u8,
        ty: u8,
        props: Props,
    },
    Update {
        target: u16,
        props: Props,
        funnel: Funnel,
    },
    /// Flip a Page to a Doc or back (the `list_by_type` regression shape).
    Retype {
        target: u16,
    },
    /// An update to a `versionable=false` node: overwrites in place.
    Volatile {
        target: u16,
        props: Props,
    },
    Delete {
        target: u16,
        cascade: bool,
        tx: bool,
    },
    Move {
        target: u16,
        parent: Option<u16>,
        tx: bool,
    },
    Rename {
        target: u16,
        name: u8,
    },
    Reorder {
        target: u16,
        anchor: u16,
        before: bool,
    },
    Copy {
        target: u16,
        parent: Option<u16>,
        name: u8,
    },
    Translate {
        target: u16,
        title: u8,
        hide: bool,
    },
    Restore {
        target: u16,
        back: u16,
    },
    ForkMerge(ForkSpec),
    /// Several writes in ONE transaction (`tx_ops.rs`).
    Tx(Vec<TxStep>),
}

/// One write inside an `Op::Tx`. Each node is touched at most once per
/// transaction, and a move never carries a node the transaction created or
/// wrote (the move reads the subtree from committed state).
#[derive(Clone, Debug)]
pub enum TxStep {
    /// A `versionable=false` node, rewritten in place at its OWN revision.
    Volatile { target: u16, props: Props },
    /// A versioned node, at the transaction revision.
    Update { target: u16, props: Props },
    /// A new Page appended under a parent.
    Create {
        parent: Option<u16>,
        name: u8,
        props: Props,
    },
    /// Move a subtree to the END of another parent.
    MoveInto { target: u16, parent: Option<u16> },
}

/// Fork `main`, edit both sides, merge the fork back with resolutions.
#[derive(Clone, Debug)]
pub struct ForkSpec {
    pub feature: Vec<Edit>,
    pub main: Vec<Edit>,
    /// Resolution per conflict, in conflict-id order: `true` = KeepOurs.
    pub keep_ours: Vec<bool>,
}

/// The edits allowed on either side of a fork. Structural edits (create,
/// delete) are confined per parent to ONE side at drive time, so the merged
/// sibling order is determined without emulating fractional labels.
#[derive(Clone, Debug)]
pub enum Edit {
    Update {
        target: u16,
        props: Props,
    },
    Create {
        parent: Option<u16>,
        name: u8,
        props: Props,
    },
    DeleteLeaf {
        target: u16,
    },
    /// MAIN side only: rewrite a `versionable=false` node in place.
    Volatile {
        target: u16,
        props: Props,
    },
}

pub const TITLES: [&str; 4] = ["alpha", "beta", "gamma", "delta"];

pub fn name_of(n: u8) -> String {
    format!("n{}", n % 8)
}

fn props() -> impl Strategy<Value = Props> {
    (
        0u8..4,
        proptest::option::of(-3i8..4),
        proptest::option::weighted(0.4, any::<u16>()),
        proptest::option::weighted(0.3, (any::<bool>(), any::<u16>())),
    )
        .prop_map(|(title, rank, link, card)| Props {
            title,
            rank,
            link,
            card,
        })
}

fn parent() -> impl Strategy<Value = Option<u16>> {
    proptest::option::weighted(0.75, any::<u16>())
}

fn create() -> impl Strategy<Value = Op> {
    (parent(), any::<u8>(), 0u8..6, props()).prop_map(|(parent, name, ty, props)| Op::Create {
        parent,
        name,
        ty,
        props,
    })
}

fn funnel() -> impl Strategy<Value = Funnel> {
    prop_oneof![Just(Funnel::Tx), Just(Funnel::Repo), Just(Funnel::Sql)]
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        3 => (any::<u16>(), props()).prop_map(|(target, props)| Edit::Update { target, props }),
        2 => (parent(), any::<u8>(), props())
            .prop_map(|(parent, name, props)| Edit::Create { parent, name, props }),
        1 => any::<u16>().prop_map(|target| Edit::DeleteLeaf { target }),
        1 => (any::<u16>(), props()).prop_map(|(target, props)| Edit::Volatile { target, props }),
    ]
}

fn tx_step() -> impl Strategy<Value = TxStep> {
    prop_oneof![
        2 => (any::<u16>(), props()).prop_map(|(target, props)| TxStep::Volatile { target, props }),
        2 => (any::<u16>(), props()).prop_map(|(target, props)| TxStep::Update { target, props }),
        2 => (parent(), any::<u8>(), props())
            .prop_map(|(parent, name, props)| TxStep::Create { parent, name, props }),
        2 => (any::<u16>(), parent()).prop_map(|(target, parent)| TxStep::MoveInto { target, parent }),
    ]
}

fn fork() -> impl Strategy<Value = Op> {
    (
        proptest::collection::vec(edit(), 1..5),
        proptest::collection::vec(edit(), 0..4),
        proptest::collection::vec(any::<bool>(), 4),
    )
        .prop_map(|(feature, main, keep_ours)| {
            Op::ForkMerge(ForkSpec {
                feature,
                main,
                keep_ours,
            })
        })
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => create(),
        5 => (any::<u16>(), props(), funnel())
            .prop_map(|(target, props, funnel)| Op::Update { target, props, funnel }),
        1 => any::<u16>().prop_map(|target| Op::Retype { target }),
        2 => (any::<u16>(), props()).prop_map(|(target, props)| Op::Volatile { target, props }),
        2 => (any::<u16>(), any::<bool>(), any::<bool>())
            .prop_map(|(target, cascade, tx)| Op::Delete { target, cascade, tx }),
        2 => (any::<u16>(), parent(), any::<bool>())
            .prop_map(|(target, parent, tx)| Op::Move { target, parent, tx }),
        1 => (any::<u16>(), any::<u8>()).prop_map(|(target, name)| Op::Rename { target, name }),
        3 => (any::<u16>(), any::<u16>(), any::<bool>())
            .prop_map(|(target, anchor, before)| Op::Reorder { target, anchor, before }),
        1 => (any::<u16>(), parent(), any::<u8>())
            .prop_map(|(target, parent, name)| Op::Copy { target, parent, name }),
        2 => (any::<u16>(), 0u8..4, proptest::bool::weighted(0.2))
            .prop_map(|(target, title, hide)| Op::Translate { target, title, hide }),
        1 => (any::<u16>(), any::<u16>()).prop_map(|(target, back)| Op::Restore { target, back }),
        1 => fork(),
        1 => proptest::collection::vec(tx_step(), 2..4).prop_map(Op::Tx),
    ]
}

/// A history: a few creates to give later ops something to land on, then a
/// mix. Length comes from `ORACLE_OPS` (`min..=max`, default 20..=40; the
/// plan's full range is 20..=60).
pub fn history() -> impl Strategy<Value = Vec<Op>> {
    let (lo, hi) = op_range();
    (
        proptest::collection::vec(create(), 4..=6),
        proptest::collection::vec(op(), (lo - 4)..=(hi - 6)),
    )
        .prop_map(|(mut head, tail)| {
            head.extend(tail);
            head
        })
}

fn op_range() -> (usize, usize) {
    let parse = |v: String| -> Option<(usize, usize)> {
        let (a, b) = v.split_once("..")?;
        Some((a.parse().ok()?, b.trim_start_matches('=').parse().ok()?))
    };
    std::env::var("ORACLE_OPS")
        .ok()
        .and_then(parse)
        .filter(|(a, b)| *a >= 10 && b > a)
        .unwrap_or((20, 40))
}

/// A stage-3 origin's history: as `history`, but without `versionable=false`
/// nodes. An in-place rewrite after an ancestor moved diverges on a replica
/// (expected failure `replica_in_place_after_ancestor_move`); until that is
/// fixed the replica is asked only about versioned content.
pub fn replicable_history() -> impl Strategy<Value = Vec<Op>> {
    history().prop_map(|ops| {
        ops.into_iter()
            .map(|op| match op {
                Op::Create {
                    parent,
                    name,
                    ty: 5,
                    props,
                } => Op::Create {
                    parent,
                    name,
                    ty: 0,
                    props,
                },
                Op::Volatile { target, props } => Op::Update {
                    target,
                    props,
                    funnel: Funnel::Tx,
                },
                Op::Tx(steps) => Op::Tx(
                    steps
                        .into_iter()
                        .map(|s| match s {
                            TxStep::Volatile { target, props } => TxStep::Update { target, props },
                            other => other,
                        })
                        .collect(),
                ),
                other => other,
            })
            .collect()
    })
}
