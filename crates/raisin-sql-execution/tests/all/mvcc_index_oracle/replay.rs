//! Stage 3: replication replay into a third storage.
//!
//! Two origins (replication capture on) share one bootstrap — origin A creates
//! the folders `/a` and `/b`, and B receives them by applying A's oplog — then
//! each runs its own generated history beneath its own folder, the two
//! alternating op by op so their revisions interleave in HLC time. Both
//! oplogs are then replayed into a replica in a random interleaving: in
//! `Causal` mode each origin's operations keep their own order (network
//! delivery), in `Permuted` mode they ARRIVE shuffled outright and pass
//! through the production `CausalDeliveryBuffer`, which releases them in an
//! order it accepts (catch-up out of order). The replica must answer every
//! template, at every revision either origin recorded, exactly as the union of
//! the two origins' models.
//!
//! The buffer only ever releases an origin's operations in that origin's
//! order, so `Permuted` never makes the replica apply two operations on one
//! node out of order. `Unbuffered` does: after the bootstrap, the shuffled
//! arrival order is applied AS IS, which is the out-of-order catch-up that
//! `apply_replicated_upsert`'s at-or-before baseline, its relabel tombstone and
//! `tombstone_superseded_by_newer` exist for. The truth is the same: HEAD is
//! order-independent, and each revision's state is that revision's.
//!
//! Every stage-3 witness passes unbuffered (`replay_witnesses_pass_out_of_order`);
//! random histories do so only on request (`ORACLE_DELIVERY=unbuffered`). Open:
//! a version whose PARENT has not arrived yet cannot be placed — a replicated
//! change names the new parent only, so the old parent of an op that arrives
//! before that parent's own create is unresolvable, and its old
//! ORDERED_CHILDREN entry stays live (`child_of_order_by___order`,
//! `list_by_parent+has_children` in the debug sweep, ~3 in 16 histories);
//! and a compound read at a past revision can still list a node that moved
//! out of the folder (~1 in 16).
//!
//! Not covered (listed in the plan): forks and merges (replicated branch ops),
//! an origin replaying a PEER's oplog onward (multi-hop), and two origins
//! writing the SAME nodes concurrently (that needs a last-writer-wins model of
//! the union; each origin here owns its own subtree).

use super::driver::Run;
use super::env::{Env, MAIN, REPO, TENANT, WS};
use super::model::{MNode, Snapshot, Tree, ROOT};
use super::ops::Op;
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::OpLogRepository;
use raisin_storage::Storage;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub enum Delivery {
    Causal,
    Permuted,
    /// Shuffled after the bootstrap and applied without the causal buffer.
    Unbuffered,
}

pub struct Replay {
    pub a: Run,
    pub b: Run,
    pub replica: Env,
    pub snaps: Vec<Snapshot>,
    pub instants: Vec<(u64, String)>,
    pub anomalies: Vec<String>,
    pub applied: usize,
}

pub fn applicator(env: &Env) -> OperationApplicator {
    OperationApplicator::new(
        env.storage.db().clone(),
        env.storage.event_bus(),
        Arc::new(env.storage.branches_impl().clone()),
    )
}

/// Every captured operation of an origin, in its own order.
pub fn oplog(env: &Env, origin: &str) -> Vec<raisin_replication::Operation> {
    let all = OpLogRepository::new(env.storage.db().clone())
        .get_all_operations(TENANT, REPO)
        .expect("oplog");
    let mut ops = all.get(origin).cloned().unwrap_or_default();
    ops.sort_by_key(|o| o.op_seq);
    ops
}

async fn bootstrap_folder(run: &mut Run, name: &str) -> String {
    let id = format!("{name}-root");
    run.tree.append(MNode {
        id: id.clone(),
        name: name.to_string(),
        parent: ROOT.to_string(),
        node_type: super::env::PAGE.to_string(),
        props: HashMap::new(),
        created: run.seq,
        updated: run.seq,
        overlay: None,
    });
    let node = Run::wire_node(&run.tree, &id);
    let tx = run.env.tx(MAIN).await;
    tx.add_node(WS, &node).await.expect("bootstrap folder");
    tx.commit().await.expect("bootstrap commit");
    id
}

/// Drive both origins, then replay into a replica.
pub async fn replay(ops_a: &[Op], ops_b: &[Op], delivery: Delivery, seed: u64) -> Replay {
    let mut a = Run::new(Env::new(Some("node-a")).await).await;
    let mut b = Run::new(Env::new(Some("node-b")).await).await;
    a.id_prefix = "a";
    b.id_prefix = "b";
    let root_a = bootstrap_folder(&mut a, "a").await;
    let root_b = bootstrap_folder(&mut a, "b").await;
    let boot = oplog(&a.env, "node-a");
    let to_b = applicator(&b.env);
    for op in &boot {
        if let Err(e) = to_b.apply_operation(op).await {
            b.anomalies
                .push(format!("bootstrap apply on B failed: {e}"));
        }
    }
    b.tree = a.tree.clone();
    a.scope = Some(root_a);
    b.scope = Some(root_b);
    // Fresh `main` snapshots now that the bootstrap is in place.
    for run in [&mut a, &mut b] {
        let head = run.env.head(MAIN).await;
        // The replica's truth starts once both folders exist.
        run.snaps.clear();
        run.snaps.push(Snapshot {
            branch: MAIN.to_string(),
            head,
            op: 0,
            tree: run.tree.clone(),
            tainted: Default::default(),
            retro: false,
            overlay_gap: Default::default(),
            replica: false,
        });
    }
    for i in 0..ops_a.len().max(ops_b.len()) {
        if let Some(op) = ops_a.get(i) {
            a.apply(op).await;
            b.seq = a.seq;
        }
        if let Some(op) = ops_b.get(i) {
            b.apply(op).await;
            a.seq = b.seq;
        }
    }
    super::model::apply_taint(&mut a.snaps, &a.writes);
    super::model::apply_taint(&mut b.snaps, &b.writes);

    // Replay into the replica. Its compound index is built at bootstrap; its
    // index definitions are warmed the way a replica's boot sweep does, so
    // the apply path maintains COMPOUND and UNIQUE entries inline (plan Phase
    // 8 step 3) and the compound templates are answered FROM THE INDEX on
    // the replica, not by a scan behind a `NotBuilt` mark.
    let replica = Env::new(None).await;
    {
        use raisin_storage::Storage;
        raisin_rocksdb::indexing::compound::defs::warm_branch(
            replica.storage.db(),
            replica.storage.node_types(),
            raisin_storage::BranchScope::new(TENANT, REPO, MAIN),
        )
        .await
        .expect("warm the replica's index definitions");
    }
    let (log_a, log_b) = (oplog(&a.env, "node-a"), oplog(&b.env, "node-b"));
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    if std::env::var("ORACLE_DUMP").is_ok() {
        for op in log_a.iter().chain(&log_b) {
            let t = format!("{:?}", op.op_type);
            eprintln!(
                "  OP {} #{} {}",
                op.cluster_node_id,
                op.op_seq,
                &t[..t.len().min(160)]
            );
        }
    }
    let order = interleave(&log_a, &log_b, boot.len(), delivery, &mut rng);
    let to_c = applicator(&replica);
    let mut anomalies = Vec::new();
    // Permuted arrival goes through the production causal-delivery buffer,
    // which holds an operation until its causal predecessors have been
    // delivered — the replica applies what it releases, in that order.
    let mut causal =
        raisin_replication::CausalDeliveryBuffer::new(raisin_replication::VectorClock::new(), None);
    for op in order.iter().cloned() {
        let ready = match delivery {
            Delivery::Causal | Delivery::Unbuffered => vec![op],
            Delivery::Permuted => causal.deliver(op),
        };
        for op in &ready {
            if let Err(e) = to_c.apply_operation(op).await {
                anomalies.push(format!("replica apply of {:?} failed: {e}", op.op_id));
            }
        }
    }
    if !causal.is_empty() {
        anomalies.push(format!(
            "causal buffer still holds {} operation(s) after delivery",
            causal.buffer_size()
        ));
    }
    // Every replicated write was warm, so the replica's compound index is
    // still `Ready` — the compound templates below are index-served there.
    if let Ok(Some(state)) = raisin_rocksdb::compound_state::read_state(
        replica.storage.db(),
        TENANT,
        REPO,
        MAIN,
        WS,
        "folder_time",
    ) {
        if !matches!(
            state.phase,
            raisin_storage::compound::CompoundBuildPhase::Ready
        ) {
            anomalies.push(format!(
                "replica compound index left {:?}: a replicated write did not maintain it",
                state.phase
            ));
        }
    }
    let applied = order.len();
    let (snaps, instants) = union(&a, &b);
    Replay {
        a,
        b,
        replica,
        snaps,
        instants,
        anomalies,
        applied,
    }
}

/// A's bootstrap first (B's ops causally follow it), then the rest.
fn interleave<T: Clone>(
    a: &[T],
    b: &[T],
    boot: usize,
    delivery: Delivery,
    rng: &mut impl Rng,
) -> Vec<T> {
    let mut out: Vec<T> = a[..boot.min(a.len())].to_vec();
    let (mut ra, mut rb): (Vec<T>, Vec<T>) = (a[boot.min(a.len())..].to_vec(), b.to_vec());
    match delivery {
        Delivery::Causal => {
            ra.reverse();
            rb.reverse();
            while !ra.is_empty() || !rb.is_empty() {
                let take_a = rb.is_empty() || (!ra.is_empty() && rng.gen_bool(0.5));
                out.push(if take_a { ra.pop() } else { rb.pop() }.expect("non-empty"));
            }
        }
        Delivery::Permuted | Delivery::Unbuffered => {
            let mut rest: Vec<T> = ra.into_iter().chain(rb).collect();
            rest.shuffle(rng);
            out.extend(rest);
        }
    }
    out
}

/// The replica's truth at every revision either origin recorded: A's state
/// below `/a`, B's below `/b`, each as of that revision.
fn union(a: &Run, b: &Run) -> (Vec<Snapshot>, Vec<(u64, String)>) {
    let at = |run: &Run, head: &raisin_hlc::HLC| -> Option<Snapshot> {
        run.snaps
            .iter()
            .filter(|s| s.branch == MAIN && s.head <= *head)
            .last()
            .cloned()
    };
    let mut heads: Vec<raisin_hlc::HLC> = a
        .snaps
        .iter()
        .chain(&b.snaps)
        .filter(|s| s.branch == MAIN)
        .map(|s| s.head)
        .collect();
    heads.sort();
    heads.dedup();
    let mut out = Vec::new();
    for (i, head) in heads.iter().enumerate() {
        let (Some(sa), Some(sb)) = (at(a, head), at(b, head)) else {
            continue;
        };
        let mut tree = Tree::default();
        let roots = [a.scope.clone().unwrap(), b.scope.clone().unwrap()];
        for (side, root) in [(&sa.tree, &roots[0]), (&sb.tree, &roots[1])] {
            for id in side.subtree(root) {
                tree.nodes.insert(id.clone(), side.nodes[&id].clone());
                if let Some(kids) = side.children.get(&id) {
                    tree.children.insert(id.clone(), kids.clone());
                }
            }
        }
        if let Some(kids) = sa.tree.children.get(ROOT) {
            tree.children.insert(ROOT.to_string(), kids.clone());
        }
        let mut tainted = sa.tainted.clone();
        tainted.extend(sb.tainted.iter().cloned());
        out.push(Snapshot {
            branch: MAIN.to_string(),
            head: *head,
            op: i,
            tree,
            tainted,
            retro: false,
            overlay_gap: Default::default(),
            replica: true,
        });
    }
    let mut instants: Vec<(u64, String)> = a.instants.iter().chain(&b.instants).cloned().collect();
    instants.sort();
    (out, instants)
}
