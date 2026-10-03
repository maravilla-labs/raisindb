use super::*;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_storage::{NodeRepository, StorageScope};
use raisin_storage_memory::InMemoryStorage;
use std::collections::HashMap;

async fn create_test_node(
    storage: &InMemoryStorage,
    workspace: &str,
    id: &str,
    path: &str,
    properties: HashMap<String, PropertyValue>,
) {
    let node = Node {
        id: id.to_string(),
        name: path.trim_start_matches('/').to_string(),
        path: path.to_string(),
        node_type: "test:Content".to_string(),
        properties,
        version: 1,
        workspace: Some(workspace.to_string()),
        ..Default::default()
    };
    let scope = StorageScope::new("default", "default", "main", workspace);
    storage
        .nodes()
        .create(scope, node, raisin_storage::CreateNodeOptions::default())
        .await
        .unwrap();
}

fn resolver(storage: &Arc<InMemoryStorage>) -> ReferenceResolver<InMemoryStorage> {
    ReferenceResolver::new(storage.clone(), "default", "default", "main", HLC::now())
}

fn str_props(pairs: &[(&str, &str)]) -> HashMap<String, PropertyValue> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), PropertyValue::String(v.to_string())))
        .collect()
}

fn ref_prop(id: &str, workspace: &str) -> PropertyValue {
    PropertyValue::Reference(RaisinReference {
        id: id.into(),
        workspace: workspace.into(),
        path: String::new(),
    })
}

fn json_ref(id: &str, workspace: &str) -> serde_json::Value {
    serde_json::json!({"raisin:ref": id, "raisin:workspace": workspace, "raisin:path": ""})
}

#[test]
fn test_node_to_json_value() {
    let node = Node {
        id: "test-id".to_string(),
        name: "Test Node".to_string(),
        path: "/test".to_string(),
        node_type: "test:Content".to_string(),
        properties: str_props(&[("bio", "Test bio")]),
        ..Default::default()
    };
    let json = node_to_json_value(&node);
    assert_eq!(json["id"], "test-id");
    assert_eq!(json["name"], "Test Node");
    assert_eq!(json["path"], "/test");
    assert_eq!(json["node_type"], "test:Content");
    assert_eq!(json["bio"], "Test bio");
}

/// References nested in blocks, arrays and a second workspace resolve in one
/// pass, a missing target is kept verbatim, and a repeated id is fetched once
/// but inlined everywhere it appears.
#[tokio::test]
async fn test_resolve_json_nested_blocks() {
    let storage = Arc::new(InMemoryStorage::default());
    let img = str_props(&[("alt", "A plane"), ("extracted_text", "long")]);
    create_test_node(&storage, "assets", "img", "/hero.jpg", img).await;
    let tag = str_props(&[("title", "News")]);
    create_test_node(&storage, "tags", "tag", "/news", tag).await;

    let doc = serde_json::json!({
        "title": "Home",
        "content": [
            {"element_type": "x:Hero", "uuid": "h1", "image": json_ref("img", "assets")},
            {"element_type": "x:Gallery", "uuid": "g1",
             "items": [json_ref("img", "assets"), json_ref("gone", "assets")]}
        ],
        "tags": [json_ref("tag", "tags")]
    });

    let resolver = resolver(&storage);
    let out = resolver
        .resolve_json("stories", &doc, 1, None)
        .await
        .unwrap();

    assert_eq!(out["title"], "Home");
    assert_eq!(out["content"][0]["image"]["path"], "/hero.jpg");
    assert_eq!(out["content"][0]["image"]["alt"], "A plane");
    assert_eq!(out["content"][1]["items"][0]["id"], "img");
    assert_eq!(out["content"][1]["items"][1]["raisin:ref"], "gone");
    assert_eq!(out["tags"][0]["title"], "News");
    assert_eq!(out["content"][0]["element_type"], "x:Hero");
    // img, gone, tag: three distinct targets, three reads.
    assert_eq!(resolver.memo.stats().reads, 3);
}

/// `fields` keeps the identity members plus the named properties only.
#[tokio::test]
async fn test_resolve_json_fields_projection() {
    let storage = Arc::new(InMemoryStorage::default());
    let img = str_props(&[("alt", "A plane"), ("extracted_text", "long")]);
    create_test_node(&storage, "assets", "img", "/hero.jpg", img).await;

    let fields = vec!["alt".to_string(), "missing".to_string()];
    let doc = serde_json::json!({"image": json_ref("img", "assets")});
    let out = resolver(&storage)
        .resolve_json("stories", &doc, 1, Some(&fields))
        .await
        .unwrap();

    let image = out["image"].as_object().unwrap();
    let mut keys: Vec<&str> = image.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["alt", "id", "name", "node_type", "path"]);
}

/// Depth 2 resolves references inside resolved nodes, a single reference
/// resolves to the node itself, and a cycle is bounded by the depth.
#[tokio::test]
async fn test_resolve_json_depth_and_cycles() {
    let storage = Arc::new(InMemoryStorage::default());
    let mut a = str_props(&[("title", "A")]);
    a.insert("next".into(), ref_prop("b", "test"));
    let mut b = str_props(&[("title", "B")]);
    b.insert("next".into(), ref_prop("a", "test"));
    create_test_node(&storage, "test", "a", "/a", a).await;
    create_test_node(&storage, "test", "b", "/b", b).await;

    let resolver = resolver(&storage);
    let one = resolver
        .resolve_json("test", &json_ref("a", ""), 1, None)
        .await
        .unwrap();
    assert_eq!(one["title"], "A");
    assert_eq!(one["next"]["raisin:ref"], "b");

    let two = resolver
        .resolve_json("test", &json_ref("a", ""), 2, None)
        .await
        .unwrap();
    assert_eq!(two["next"]["title"], "B");

    // A cycle nests until the depth runs out, and then stops as a reference.
    let three = resolver
        .resolve_json("test", &json_ref("a", ""), 3, None)
        .await
        .unwrap();
    assert_eq!(three["next"]["next"]["title"], "A");
    assert_eq!(three["next"]["next"]["next"]["raisin:ref"], "b");

    // Three calls, two nodes: each read once for the whole "statement".
    assert_eq!(resolver.memo.stats().reads, 2);
}

/// An asset used on the page AND on a teased child page is inlined in both
/// places, from a single read.
#[tokio::test]
async fn test_resolve_json_shared_reference_across_levels() {
    let storage = Arc::new(InMemoryStorage::default());
    let img = str_props(&[("alt", "A plane")]);
    create_test_node(&storage, "assets", "img", "/hero.jpg", img).await;
    let mut child = str_props(&[("title", "Child")]);
    child.insert("image".into(), ref_prop("img", "assets"));
    create_test_node(&storage, "stories", "child", "/child", child).await;

    let doc = serde_json::json!({
        "hero": json_ref("img", "assets"),
        "teaser": json_ref("child", "stories"),
    });
    let resolver = resolver(&storage);
    let out = resolver
        .resolve_json("stories", &doc, 2, None)
        .await
        .unwrap();
    assert_eq!(out["hero"]["alt"], "A plane");
    assert_eq!(out["teaser"]["image"]["alt"], "A plane");
    assert_eq!(resolver.memo.stats().reads, 2);
}

/// A reference that names its target by path — as translation overlays write
/// them — resolves like one by id.
#[tokio::test]
async fn test_resolve_json_reference_by_path() {
    let storage = Arc::new(InMemoryStorage::default());
    let img = str_props(&[("alt", "A plane")]);
    create_test_node(&storage, "assets", "img", "/uploads/hero.jpg", img).await;

    let doc = serde_json::json!({
        "background": {"raisin:ref": "/uploads/hero.jpg", "raisin:workspace": "assets"}
    });
    let out = resolver(&storage)
        .resolve_json("stories", &doc, 1, None)
        .await
        .unwrap();
    assert_eq!(out["background"]["id"], "img");
    assert_eq!(out["background"]["alt"], "A plane");
}

mod scope;
