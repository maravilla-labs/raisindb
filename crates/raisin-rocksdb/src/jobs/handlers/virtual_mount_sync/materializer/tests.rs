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

#[test]
fn dedup_keeps_the_last_op_per_resolved_path() {
    let ops = vec![
        upsert_op("a", "same.txt", Some("v1")),
        upsert_op("b", "same.txt", Some("v1")),
    ];
    let out = dedup_ops(ops, "/docs");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].external_id(), "b");
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

    let idx = SyncIndex::from_nodes(vec![mount_owned, foreign, outside], "m1", "/docs", &[]);

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
