//! Tests for `capture.rs`.

use super::group_changes_by_revision;
use crate::replication::NodeChanges;
use raisin_hlc::HLC;
use std::collections::HashMap;

fn change(id: &str, rev: HLC) -> (String, NodeChanges) {
    (
        id.to_string(),
        NodeChanges::new_update(id.into(), "ws".into(), rev, None, None),
    )
}

/// Two `versionable=false` nodes refreshed in one transaction keep their
/// own revisions; a versioned write beside them keeps the transaction's.
#[test]
fn two_volatile_updates_in_one_transaction_replicate_at_their_own_revisions() {
    let (r1, r2, tx) = (HLC::new(10, 0), HLC::new(20, 0), HLC::new(30, 0));
    let changes: HashMap<_, _> = [change("v1", r1), change("v2", r2), change("n", tx)]
        .into_iter()
        .collect();
    let groups = group_changes_by_revision(&changes);
    let got: Vec<(HLC, Vec<String>)> = groups
        .iter()
        .map(|(rev, g)| (*rev, g.keys().cloned().collect()))
        .collect();
    assert_eq!(
        got,
        vec![
            (r1, vec!["v1".to_string()]),
            (r2, vec!["v2".to_string()]),
            (tx, vec!["n".to_string()]),
        ]
    );
}
