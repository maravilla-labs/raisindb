//! Unit tests for the materializer: dedup, path arithmetic, size estimation.

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

use super::node_paths::{ancestor_paths, join_path, under};
use super::*;
use crate::jobs::handlers::virtual_mount_sync::config::MappedNode;

fn upsert_op(ext: &str, rel_path: &str, etag: Option<&str>) -> BatchOp {
    BatchOp::Upsert {
        rel_path: rel_path.to_string(),
        mapped: MappedNode {
            node_type: "raisin:Node".to_string(),
            name: None,
            properties: serde_json::Map::new(),
        },
        virt: VirtualMeta {
            mount_id: "m1".to_string(),
            external_id: ext.to_string(),
            etag: etag.map(str::to_string),
            synced_at: "2026-01-01T00:00:00Z".to_string(),
        },
    }
}

#[test]
fn join_and_under_paths() {
    assert_eq!(join_path("/docs", "a/b"), "/docs/a/b");
    assert_eq!(join_path("/docs", "/a"), "/docs/a");
    assert_eq!(join_path("/", "a"), "/a");
    assert!(under("/docs", "/docs/a"));
    assert!(under("/docs", "/docs"));
    assert!(!under("/docs", "/documents"));
    assert!(under("/", "/anything"));
}

#[test]
fn ancestors_exclude_self_and_root() {
    assert_eq!(
        ancestor_paths("/docs/a/b"),
        vec!["/docs".to_string(), "/docs/a".to_string()]
    );
    assert!(ancestor_paths("/docs").is_empty());
    assert!(ancestor_paths("/").is_empty());
}

#[test]
fn dedup_keeps_the_last_op_per_external_id() {
    let ops = vec![
        upsert_op("a", "a.txt", Some("v1")),
        upsert_op("b", "b.txt", Some("v1")),
        upsert_op("a", "a.txt", Some("v2")),
    ];
    let out = dedup_ops(ops, "/docs");
    assert_eq!(out.len(), 2);
    let a = out
        .iter()
        .find(|o| o.external_id() == "a")
        .expect("a survives");
    match a {
        BatchOp::Upsert { virt, .. } => assert_eq!(virt.etag.as_deref(), Some("v2")),
        _ => panic!("expected an upsert"),
    }
}

#[test]
fn create_then_delete_in_one_page_resolves_to_the_delete() {
    let ops = vec![
        upsert_op("a", "a.txt", Some("v1")),
        BatchOp::Delete {
            external_id: "a".to_string(),
        },
    ];
    let out = dedup_ops(ops, "/docs");
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], BatchOp::Delete { .. }));
}

/// Two DIFFERENT items on one path are two pieces of real content, and neither
/// may be dropped.
///
/// This used to keep the last and discard the first with a WARN that counted as
/// neither a write nor a failure — and because both ids were already in the
/// walk's `seen` set, reconcile never noticed. The two then traded places on
/// every subsequent run. Each now gets a stable suffix derived from its own
/// external id, so both materialize and neither moves again.
#[test]
fn two_items_on_one_path_are_both_kept_at_distinct_paths() {
    let paths_of = |ops: Vec<BatchOp>| -> Vec<String> {
        dedup_ops(ops, "/docs")
            .iter()
            .map(|op| match op {
                BatchOp::Upsert { rel_path, .. } => rel_path.clone(),
                _ => panic!("expected upserts"),
            })
            .collect()
    };

    let paths = paths_of(vec![
        upsert_op("a", "same.txt", Some("v1")),
        upsert_op("b", "same.txt", Some("v1")),
    ]);
    assert_eq!(paths.len(), 2, "neither item may be dropped");
    assert_ne!(paths[0], paths[1], "the collision must be broken");
    for path in &paths {
        assert!(
            path.starts_with("same-") && path.ends_with(".txt"),
            "the suffix goes before the extension, not after it: {path}"
        );
    }

    // Stable across runs: the suffix is a digest of the item's own external id,
    // so the same two items always land on the same two paths.
    assert_eq!(
        paths,
        paths_of(vec![
            upsert_op("a", "same.txt", Some("v2")),
            upsert_op("b", "same.txt", Some("v2")),
        ])
    );
}

#[test]
fn index_serves_external_path_and_foreign_lookups() {
    let mut mount_owned = Node {
        id: "n1".to_string(),
        path: "/docs/a.txt".to_string(),
        ..Default::default()
    };
    mount_owned.properties.insert(
        "__mount_id".to_string(),
        PropertyValue::String("m1".to_string()),
    );
    mount_owned.properties.insert(
        "__external_id".to_string(),
        PropertyValue::String("ext-a".to_string()),
    );
    mount_owned.properties.insert(
        "__etag".to_string(),
        PropertyValue::String("v1".to_string()),
    );
    let foreign = Node {
        id: "n2".to_string(),
        path: "/docs/user.txt".to_string(),
        ..Default::default()
    };
    let outside = Node {
        id: "n3".to_string(),
        path: "/elsewhere/x.txt".to_string(),
        ..Default::default()
    };

    let idx = SyncIndex::from_nodes(vec![mount_owned, foreign, outside], "m1", "/docs", &[], &[]);

    assert_eq!(idx.by_external("ext-a").map(|n| n.id.as_str()), Some("n1"));
    assert_eq!(
        idx.by_external("ext-a")
            .and_then(|n| n.etag.clone())
            .as_deref(),
        Some("v1")
    );
    // The foreign node is visible by path — that guard depends on it.
    assert_eq!(
        idx.at_path("/docs/user.txt").map(|e| e.mount_owned),
        Some(false)
    );
    assert_eq!(
        idx.at_path("/docs/a.txt").map(|e| e.mount_owned),
        Some(true)
    );
    // Nodes outside the mount path are not indexed at all.
    assert!(idx.at_path("/elsewhere/x.txt").is_none());
    assert_eq!(idx.virtual_len(), 1);
}

#[test]
fn recording_a_write_also_marks_its_ancestor_folders_occupied() {
    let mut idx = SyncIndex::default();
    idx.record_upsert(VirtualNodeRef {
        is_command: false,
        id: "n1".to_string(),
        path: "/docs/thread-1/msg.txt".to_string(),
        external_id: "ext-a".to_string(),
        etag: Some("v1".to_string()),
        synced_secs: None,
        pushed_state: None,
        write_view: None,
    });
    assert!(idx.at_path("/docs/thread-1").is_some());
    assert_eq!(
        idx.at_path("/docs/thread-1").map(|e| e.mount_owned),
        Some(false)
    );
    assert_eq!(
        idx.at_path("/docs/thread-1/msg.txt").map(|e| e.mount_owned),
        Some(true)
    );
    assert_eq!(idx.by_external("ext-a").map(|n| n.id.as_str()), Some("n1"));

    idx.record_delete("ext-a");
    assert!(idx.by_external("ext-a").is_none());
    assert!(idx.at_path("/docs/thread-1/msg.txt").is_none());
}

#[test]
fn byte_estimate_tracks_the_property_payload() {
    let small = upsert_op("a", "a.txt", None);
    let mut big_props = serde_json::Map::new();
    big_props.insert(
        "body".to_string(),
        serde_json::Value::String("x".repeat(50_000)),
    );
    let big = BatchOp::Upsert {
        rel_path: "b.txt".to_string(),
        mapped: MappedNode {
            node_type: "raisin:Node".to_string(),
            name: None,
            properties: big_props,
        },
        virt: VirtualMeta {
            mount_id: "m1".to_string(),
            external_id: "b".to_string(),
            etag: None,
            synced_at: "2026-01-01T00:00:00Z".to_string(),
        },
    };
    assert!(estimate_op_bytes(&big) > 50_000);
    assert!(estimate_op_bytes(&small) < 1_000);
}

fn stamp_op(ext: &str, etag: Option<&str>) -> BatchOp {
    BatchOp::StampVirtual {
        node_id: format!("node-{ext}"),
        external_id: ext.to_string(),
        etag: etag.map(str::to_string),
        synced_at: "2026-01-01T00:00:00Z".to_string(),
        pushed_state: None,
        merged: None,
        adopt: false,
        node_bytes: 0,
    }
}

/// A stamp is charged the size of the node it re-writes, not the size of the
/// metadata it amends.
///
/// This is what stops a drain over an existing mailbox — `state_only` switched
/// on, where every node diverges and is pushed and stamped — from packing a
/// whole run's mail bodies into ONE transaction, and so one `ApplyRevision`
/// past the 10 MB transport frame cap.
#[test]
fn a_stamp_is_charged_the_whole_node_it_rewrites() {
    let stamp = BatchOp::StampVirtual {
        node_id: "node-a".to_string(),
        external_id: "a".to_string(),
        etag: Some("e1".to_string()),
        synced_at: "2026-01-01T00:00:00Z".to_string(),
        pushed_state: None,
        merged: None,
        adopt: false,
        node_bytes: 30_000,
    };
    assert!(estimate_op_bytes(&stamp) > 30_000);
    // 500 stamps of a 30 KB mail node must blow the 4 MiB default budget many
    // times over — i.e. flush repeatedly instead of committing as one batch.
    assert!(estimate_op_bytes(&stamp) * 500 > 4 * 1024 * 1024);
}

/// An upsert re-states the node whole, so it wins over a stamp for the same
/// item — in EITHER order.
///
/// Both directions matter and they fail differently. Keeping a later stamp
/// would drop the upsert entirely, because a stamp writes no mapper output:
/// the item's remote change would vanish and only its metadata would land.
/// Keeping an earlier stamp would re-apply metadata the upsert has already
/// rewritten, stamping a push's etag over the newer one the provider just
/// reported — the one way this design can lose a remote change.
#[test]
fn an_upsert_supersedes_a_stamp_for_the_same_item() {
    let out = dedup_ops(
        vec![
            stamp_op("a", Some("pushed")),
            upsert_op("a", "a.txt", Some("v2")),
        ],
        "/docs",
    );
    assert_eq!(out.len(), 1);
    match &out[0] {
        BatchOp::Upsert { virt, .. } => assert_eq!(virt.etag.as_deref(), Some("v2")),
        other => panic!("expected the upsert to survive, got {other:?}"),
    }

    let out = dedup_ops(
        vec![
            upsert_op("a", "a.txt", Some("v2")),
            stamp_op("a", Some("pushed")),
        ],
        "/docs",
    );
    assert_eq!(out.len(), 1);
    assert!(
        matches!(&out[0], BatchOp::Upsert { .. }),
        "an authoritative op never loses to a stamp"
    );
}

/// Stamps collapse per external id like everything else, and a stamp for an
/// item with no other op survives — the ordinary case, since the write drain
/// flushes ahead of the read phases.
#[test]
fn stamps_collapse_per_external_id_and_survive_alone() {
    let out = dedup_ops(
        vec![
            stamp_op("a", Some("e1")),
            stamp_op("b", Some("e1")),
            stamp_op("a", Some("e2")),
        ],
        "/docs",
    );
    assert_eq!(out.len(), 2);
    let a = out.iter().find(|o| o.external_id() == "a").unwrap();
    match a {
        BatchOp::StampVirtual { etag, .. } => assert_eq!(etag.as_deref(), Some("e2")),
        other => panic!("expected a stamp, got {other:?}"),
    }
}

/// The converge check, in isolation: absent evidence is NOT divergence.
#[test]
fn a_write_view_distinguishes_unseeded_from_diverged() {
    let fields = vec!["unread".to_string()];
    let mut node = Node {
        id: "n1".to_string(),
        path: "/docs/a".to_string(),
        ..Default::default()
    };
    node.properties
        .insert("unread".to_string(), PropertyValue::Boolean(true));

    // No `__pushed_state` at all: unseeded, and NOT reported as diverged-only.
    let view = write_view_of(&node, &fields).expect("a watched mount gets a view");
    assert!(view.is_unseeded());

    // Stamped with the same value: converged.
    node.properties.insert(
        PUSHED_STATE_PROP.to_string(),
        serde_json::from_value(serde_json::json!({ "unread": true })).unwrap(),
    );
    let view = write_view_of(&node, &fields).unwrap();
    assert!(!view.is_unseeded());
    assert!(!view.diverges(&fields));

    // Local edit: diverged.
    node.properties
        .insert("unread".to_string(), PropertyValue::Boolean(false));
    assert!(write_view_of(&node, &fields).unwrap().diverges(&fields));

    // A mount that watches nothing carries no view at all.
    assert!(write_view_of(&node, &[]).is_none());
}

/// The `local_wins` pre-merge, in isolation: which fields survive, which
/// converge, and how ABSENCE travels through both maps.
#[test]
fn preserve_pending_edits_keeps_edits_and_their_divergence() {
    use super::write_view::{preserve_pending_edits, WriteView};
    use serde_json::{json, Map, Value};

    let fields = vec!["unread".to_string(), "folder".to_string()];
    let obj = |v: Value| match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    };

    // Pending `unread` edit (local true, pushed false); `folder` converged.
    let live = WriteView {
        watched: obj(json!({ "unread": true, "folder": "inbox" })),
        pushed: Some(obj(json!({ "unread": false, "folder": "inbox" }))),
    };

    // The incoming item still says false, and moves the folder.
    let incoming = obj(json!({ "unread": false, "folder": "archive", "subject": "hi" }));
    let (merged, baseline) =
        preserve_pending_edits(&incoming, &live, &fields).expect("unread is pending");
    // The pending field keeps the local value; everything else — the
    // non-diverged watched field AND the unwatched property — follows the item.
    assert_eq!(
        Value::Object(merged),
        json!({ "unread": true, "folder": "archive", "subject": "hi" })
    );
    // The baseline keeps the OLD entry for the pending field only.
    assert_eq!(
        Value::Object(baseline),
        json!({ "unread": false, "folder": "archive" })
    );

    // An item already carrying the local value has nothing pending: the caller
    // must take the ordinary reseed path, which is the convergence.
    let matching = obj(json!({ "unread": true, "folder": "inbox" }));
    assert!(preserve_pending_edits(&matching, &live, &fields).is_none());

    // No local edit at all: same answer, ordinary path.
    let converged = WriteView {
        watched: obj(json!({ "unread": false, "folder": "inbox" })),
        pushed: Some(obj(json!({ "unread": false, "folder": "inbox" }))),
    };
    let moved = obj(json!({ "unread": false, "folder": "archive" }));
    assert!(preserve_pending_edits(&moved, &converged, &fields).is_none());
}

/// Absence is load-bearing on both sides of the merge: a first edit of a field
/// the provider never reported must stay diverged (baseline entry REMOVED, not
/// nulled), and a local delete of a field must not be resurrected by the
/// incoming value.
#[test]
fn preserve_pending_edits_treats_absence_as_an_answer() {
    use super::write_view::{preserve_pending_edits, WriteView};
    use serde_json::{json, Map, Value};

    let fields = vec!["unread".to_string()];
    let obj = |v: Value| match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    };

    // First edit of a never-reported field: local present, baseline lacks it.
    let live = WriteView {
        watched: obj(json!({ "unread": true })),
        pushed: Some(Map::new()),
    };
    let incoming = obj(json!({ "unread": false }));
    let (merged, baseline) =
        preserve_pending_edits(&incoming, &live, &fields).expect("first edit is pending");
    assert_eq!(Value::Object(merged), json!({ "unread": true }));
    assert!(
        !baseline.contains_key("unread"),
        "the baseline must stay ABSENT — storing the incoming value (or null) \
         would make the first edit look already-pushed"
    );

    // Unseeded node (no `__pushed_state` at all): same rule.
    let unseeded = WriteView {
        watched: obj(json!({ "unread": true })),
        pushed: None,
    };
    let (_, baseline) =
        preserve_pending_edits(&incoming, &unseeded, &fields).expect("unseeded edit is pending");
    assert!(!baseline.contains_key("unread"));

    // Local DELETE of a watched field the provider still reports: the merge
    // must not resurrect it.
    let deleted = WriteView {
        watched: Map::new(),
        pushed: Some(obj(json!({ "unread": false }))),
    };
    let (merged, baseline) =
        preserve_pending_edits(&incoming, &deleted, &fields).expect("the delete is pending");
    assert!(
        !merged.contains_key("unread"),
        "keeping the incoming value would resurrect a locally-deleted field"
    );
    assert_eq!(Value::Object(baseline), json!({ "unread": false }));
}
