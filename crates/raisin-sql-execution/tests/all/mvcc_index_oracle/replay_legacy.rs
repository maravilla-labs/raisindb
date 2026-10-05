//! Replication over records written before `Node.order_key` was stamped.
//!
//! Every node the transaction path wrote before that fix carries
//! `order_key == ""` in its blob — on the origin and on every replica, since
//! the replicated snapshot carried the empty value. The oracle's histories
//! only ever create nodes after the fix, so this seeds the legacy state by
//! hand: replicate, blank the replica's stored `order_key`, then replicate a
//! reorder of that node.

use super::driver::Run;
use super::env::{Env, MAIN, REPO, TENANT, WS};
use super::ops::Op;
use super::replay::{applicator, oplog};
use super::witnesses::page;

/// Rewrite every stored version of `id` on `env` with an empty `order_key`.
fn blank_order_key(env: &Env, id: &str) {
    let db = env.storage.db();
    let cf = db.cf_handle(raisin_rocksdb::cf::NODES).expect("nodes cf");
    let prefix = format!("{TENANT}\0{REPO}\0{MAIN}\0{WS}\0nodes\0{id}\0").into_bytes();
    let rows: Vec<(Box<[u8]>, Box<[u8]>)> = db
        .prefix_iterator_cf(&cf, &prefix)
        .flatten()
        .take_while(|(k, _)| k.starts_with(&prefix))
        .collect();
    let mut blanked = 0;
    for (key, value) in rows {
        if key.len() != prefix.len() + 16 || raisin_rocksdb::keys::is_tombstone_value(&value) {
            continue;
        }
        let (mut node, _) = raisin_rocksdb::decode_node_blob(&value).expect("node blob");
        node.order_key = String::new();
        let blob = rmp_serde::to_vec_named(&node).expect("encode");
        db.put_cf(&cf, &key, blob).expect("rewrite");
        blanked += 1;
    }
    assert!(blanked > 0, "no stored version of {id} to blank");
}

/// A replicated reorder of a node whose stored `order_key` is empty must
/// still tombstone its old label. The replica read the old label from the
/// blob, found "", decided nothing was relabelled, and kept the node live at
/// BOTH labels: `ORDER BY __order` and `ORDER BY __order DESC` then disagreed.
#[test]
fn replicated_reorder_of_a_node_with_an_empty_stored_order_key_tombstones_its_old_label() {
    let rt = super::runtime();
    let outcome = rt.block_on(async {
        let mut a = Run::new(Env::new(Some("node-a")).await).await;
        a.apply(&page(None, 0)).await;
        a.apply(&page(None, 1)).await;

        let replica = Env::new(None).await;
        let to_c = applicator(&replica);
        let first = oplog(&a.env, "node-a");
        for op in &first {
            to_c.apply_operation(op).await.expect("replica apply");
        }
        blank_order_key(&replica, "o002");

        a.apply(&Op::Reorder {
            target: 1,
            anchor: 0,
            before: true,
        })
        .await;
        let all = oplog(&a.env, "node-a");
        for op in &all[first.len()..] {
            to_c.apply_operation(op).await.expect("replica apply");
        }

        // HEAD only: the blanked versions below it are legacy by construction.
        let head = a.snaps.last().expect("snapshot").head;
        let mismatches =
            super::check_snaps(&replica, &a.snaps, &a.instants, |s| s.head == head).await;
        super::verdict(&a, &mismatches)
    });
    if let Err(report) = outcome {
        panic!("{report}");
    }
}
