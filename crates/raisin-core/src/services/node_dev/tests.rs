// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Pure tests: envelopes validate against the contract, digests are stable,
//! and record paths cannot be steered. Storage-backed behaviour (moves,
//! conflicts, replays) is tested against RocksDB in
//! `raisin-rocksdb/tests/all/node_dev_test.rs`.

use raisin_agent_contract::validate_tool_result;
use serde_json::json;

use super::changeset_plan::digest_of;
use super::changeset_store::{changeset_id, check_id, record_path};
use super::envelope::*;
use super::*;

fn loc(path: &str, id: &str) -> NodeLocator {
    NodeLocator {
        repository: "r".into(),
        branch: "main".into(),
        workspace: "ws".into(),
        path: path.into(),
        node_id: Some(id.into()),
        revision: Some(raisin_agent_contract::tool_result::Revision {
            value: "abc".into(),
            alg: NODE_REVISION_ALG.into(),
        }),
    }
}

fn receipt() -> Receipt {
    Receipt {
        changeset_id: "0".repeat(32),
        repository: "r".into(),
        branch: "main".into(),
        committed_revision: Some("1-0".into()),
        ops: vec![
            OpReceipt {
                index: 0,
                action: OpAction::Moved,
                old: Some(loc("/a", "n1")),
                new: Some(loc("/b/a", "n1")),
                changed_properties: vec![],
                created_descendants: vec![],
                deleted_descendants: vec![],
                moved_descendants: vec![MovedEntry {
                    from_path: "/a/c".into(),
                    to: loc("/b/a/c", "n2"),
                }],
                rewritten_references: vec![],
            },
            OpReceipt {
                index: 1,
                action: OpAction::Deleted,
                old: Some(loc("/x", "n3")),
                new: None,
                changed_properties: vec![],
                created_descendants: vec![],
                deleted_descendants: vec![],
                moved_descendants: vec![],
                rewritten_references: vec![],
            },
        ],
        replayed: false,
    }
}

#[test]
fn receipt_envelope_is_a_valid_tool_result() {
    let env = receipt_envelope("op-1", &receipt(), 0, "node");
    validate_tool_result(&env).expect("valid");
    assert_eq!(env.writes.len(), 3, "root move, descendant move, delete");
    assert!(env.writes.iter().filter(|w| w.from.is_some()).count() == 2);
    let primaries = env
        .artifact_refs
        .iter()
        .filter(|a| a.role == raisin_agent_contract::tool_result::ArtifactRole::Primary)
        .count();
    assert_eq!(primaries, 1);
}

#[test]
fn error_and_conflict_envelopes_validate() {
    let err = NodeDevError::forbidden("nope");
    validate_tool_result(&error_envelope("op", &err)).expect("valid");
    let c = Conflict {
        index: 0,
        code: "stale_revision".into(),
        message: "changed".into(),
        expected: Some("x".into()),
        actual: Some(loc("/a", "n1")),
    };
    let env = conflict_envelope("op", "id", &[c], "sha256:0");
    validate_tool_result(&env).expect("valid");
    assert_eq!(env.suggested_next_actions.len(), 1);
}

#[test]
fn digest_is_stable_and_sensitive() {
    let ops = vec![ChangeOp::Delete {
        target: Target::path("/a"),
        expected_revision: None,
        recursive: false,
    }];
    let planned = |rev: &str| PlannedOp {
        index: 0,
        action: OpAction::Deleted,
        workspace: "ws".into(),
        node_id: Some("n1".into()),
        before: Some(NodeLocator {
            revision: Some(raisin_agent_contract::tool_result::Revision {
                value: rev.into(),
                alg: NODE_REVISION_ALG.into(),
            }),
            ..loc("/a", "n1")
        }),
        after_path: None,
        node_type: "t".into(),
        changed_properties: vec![],
        moved_descendants: vec![],
        descendants: vec![],
        referrers: vec![],
    };
    let a = digest_of(&ops, &[planned("r1")], &[]);
    assert_eq!(a, digest_of(&ops, &[planned("r1")], &[]));
    assert_ne!(
        a,
        digest_of(&ops, &[planned("r2")], &[]),
        "a changed node changes the digest"
    );
}

#[test]
fn changeset_ids_are_keyed_and_safe() {
    let s = DevScope::new("t", "r", "main");
    let a = changeset_id(&s, "u1", Some("k"));
    assert_eq!(a, changeset_id(&s, "u1", Some("k")), "same key, same id");
    assert_ne!(a, changeset_id(&s, "u2", Some("k")), "keys are per caller");
    check_id(&a).unwrap();
    assert!(check_id("../../etc").is_err());
    assert_eq!(record_path(&a), format!("/changesets/{}/{a}", &a[..2]));
}

#[test]
fn ops_round_trip_as_json() {
    let v = json!([
        {"op": "create", "path": "views/board", "node_type": "raisin:Folder", "properties": {"title": "Board"}},
        {"op": "move", "target": {"path": "/a"}, "to_parent": {"path": "/"}, "expected_revision": "abc"},
        {"op": "patch", "target": {"node_id": "n1"}, "set": {"x": 1}, "unset": ["y"]},
        {"op": "delete", "target": {"path": "/z"}, "recursive": true}
    ]);
    let ops: Vec<ChangeOp> = serde_json::from_value(v).unwrap();
    assert_eq!(ops.len(), 4);
}
