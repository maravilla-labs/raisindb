//! Plan Phase 13d: RESOLVE over stored values equals RESOLVE over JSON.
//!
//! The oracle is the pre-13d resolver's walk and inline, verbatim, over JSON:
//! documents and targets rendered with `serde_json`, references found and
//! replaced in the rendering, the result read back with `from_json` (what the
//! SQL projection did). It is fed the SAME fetch decisions — each target's
//! node rendered by `node_to_json_value_with_fields` when the caller may read
//! it, nothing when it is missing or denied — and must agree with
//! `resolve_values_many(..).into_json_round_trip()` on the document AND on the
//! budget's occurrence and byte totals.

use super::scope::reader_of;
use super::*;
use raisin_models::nodes::properties::value::{Composite, Element, Resource};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

// ---- The oracle: the JSON resolver as it was ---------------------------------

fn as_reference(map: &Map<String, Value>) -> Option<(Option<&str>, &str)> {
    let locator = map.get("raisin:ref")?.as_str()?;
    let workspace = map
        .get("raisin:workspace")
        .and_then(Value::as_str)
        .filter(|ws| !ws.is_empty());
    Some((workspace, locator))
}

fn collect(value: &Value, out: &mut Vec<(String, String)>, ws: &str) {
    match value {
        Value::Object(map) => {
            if let Some((w, l)) = as_reference(map) {
                out.push((w.unwrap_or(ws).to_string(), l.to_string()));
                return;
            }
            map.values().for_each(|v| collect(v, out, ws));
        }
        Value::Array(items) => items.iter().for_each(|v| collect(v, out, ws)),
        _ => {}
    }
}

type Nodes = HashMap<(String, String), Value>;

/// The frontier walk: which targets `depth` levels reach.
fn gather(
    doc: &Value,
    depth: u32,
    ws: &str,
    nodes: &Nodes,
) -> HashMap<(String, String), Option<Value>> {
    let mut resolved = HashMap::new();
    let mut queued = HashSet::new();
    let mut frontier = Vec::new();
    let mut first = Vec::new();
    collect(doc, &mut first, ws);
    for t in first {
        if queued.insert(t.clone()) {
            frontier.push(t);
        }
    }
    let mut level = 1;
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for target in frontier.drain(..) {
            let found = nodes.get(&target).cloned();
            if level < depth {
                if let Some(node) = &found {
                    let mut refs = Vec::new();
                    collect(node, &mut refs, ws);
                    for child in refs {
                        if queued.insert(child.clone()) {
                            next.push(child);
                        }
                    }
                }
            }
            resolved.insert(target, found);
        }
        frontier = next;
        level += 1;
    }
    resolved
}

struct Oracle<'a> {
    ws: &'a str,
    resolved: &'a HashMap<(String, String), Option<Value>>,
    occurrences: usize,
    bytes: usize,
}

impl Oracle<'_> {
    fn inline(&mut self, value: &mut Value, remaining: u32) {
        match value {
            Value::Object(map) => {
                if let Some((w, l)) = as_reference(map) {
                    if remaining == 0 {
                        return;
                    }
                    let key = (w.unwrap_or(self.ws).to_string(), l.to_string());
                    let Some(Some(target)) = self.resolved.get(&key) else {
                        return;
                    };
                    let mut expanded = target.clone();
                    self.occurrences += 1;
                    self.bytes += serde_json::to_vec(target).unwrap().len();
                    if remaining > 1 {
                        self.inline(&mut expanded, remaining - 1);
                    }
                    *value = expanded;
                    return;
                }
                map.values_mut().for_each(|v| self.inline(v, remaining));
            }
            Value::Array(items) => items.iter_mut().for_each(|v| self.inline(v, remaining)),
            _ => {}
        }
    }
}

// ---- The fixture -------------------------------------------------------------

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn obj(entries: Vec<(&str, PropertyValue)>) -> PropertyValue {
    PropertyValue::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn element(kind: &str, content: Vec<(&str, PropertyValue)>) -> Element {
    Element {
        uuid: format!("u-{kind}"),
        element_type: kind.to_string(),
        content: content
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

fn props(value: PropertyValue) -> HashMap<String, PropertyValue> {
    match value {
        PropertyValue::Object(map) => map,
        _ => unreachable!(),
    }
}

/// Nested references, blocks (an element and a composite item that ARE
/// references among them), arrays, a cycle, a path reference, a missing and
/// a denied target, typed values that render specially.
async fn fixture() -> (Arc<InMemoryStorage>, PropertyValue) {
    let storage = Arc::new(InMemoryStorage::default());
    let date: PropertyValue =
        serde_json::from_value(serde_json::json!("2026-10-05T12:00:00Z")).unwrap();
    let a = obj(vec![
        ("title", s("A")),
        ("next", ref_prop("b", "open")),
        ("when", date.clone()),
        ("price", PropertyValue::Decimal("19.90".parse().unwrap())),
        ("embedding", PropertyValue::Vector(vec![0.1, 0.25])),
        ("type", s("article")),
    ]);
    let b = obj(vec![
        ("title", s("B")),
        ("back", ref_prop("a", "open")), // a cycle
        (
            "hero",
            PropertyValue::Element(element("hero", vec![("img", ref_prop("c", "open"))])),
        ),
        ("nan", PropertyValue::Float(f64::NAN)),
    ]);
    let c = obj(vec![("alt", s("C")), ("secret", ref_prop("s1", "secret"))]);
    create_test_node(&storage, "open", "a", "/a", props(a)).await;
    create_test_node(&storage, "open", "b", "/b", props(b)).await;
    create_test_node(&storage, "open", "c", "/c", props(c)).await;
    create_test_node(&storage, "secret", "s1", "/s1", str_props(&[("pii", "x")])).await;

    let as_ref_element = element(
        "link",
        vec![("raisin:ref", s("c")), ("raisin:workspace", s("open"))],
    );
    let resource: Resource = serde_json::from_value(serde_json::json!({
        "uuid": "r1", "name": null, "size": 3, "mime_type": "image/png", "url": null,
        "metadata": {"raisin:ref": "a", "raisin:workspace": "open"},
        "is_loaded": null, "is_external": null,
        "created_at": "2026-10-05T12:00:00Z", "updated_at": "2026-10-05T12:00:00Z"
    }))
    .unwrap();
    let doc = obj(vec![
        ("title", s("Page")),
        ("lead", ref_prop("a", "")), // default workspace
        (
            "by_path",
            obj(vec![
                ("raisin:ref", s("/c")),
                ("raisin:workspace", s("open")),
            ]),
        ),
        ("missing", ref_prop("nope", "open")),
        ("denied", ref_prop("s1", "secret")),
        (
            "blocks",
            PropertyValue::Composite(Composite {
                uuid: "c1".into(),
                items: vec![
                    element(
                        "text",
                        vec![("body", s("x")), ("see", ref_prop("b", "open"))],
                    ),
                    as_ref_element.clone(),
                ],
            }),
        ),
        ("aside", PropertyValue::Element(as_ref_element)),
        ("file", PropertyValue::Resource(resource)),
        (
            "list",
            PropertyValue::Array(vec![ref_prop("b", "open"), PropertyValue::Integer(3), date]),
        ),
    ]);
    (storage, doc)
}

/// Every node the reader may read, rendered as the JSON resolver rendered it.
async fn readable(storage: &Arc<InMemoryStorage>, fields: Option<&[String]>) -> Nodes {
    let mut nodes = Nodes::new();
    for id in ["a", "b", "c"] {
        let scope = raisin_storage::StorageScope::new("default", "default", "main", "open");
        let node = storage.nodes().get(scope, id, None).await.unwrap().unwrap();
        let json = node_to_json_value_with_fields(&node, fields);
        nodes.insert(("open".into(), node.path.clone()), json.clone());
        nodes.insert(("open".into(), node.id.clone()), json);
    }
    nodes
}

#[tokio::test]
async fn values_resolve_exactly_as_json_did() {
    let (storage, doc) = fixture().await;
    let fields_list = vec!["title".to_string(), "next".to_string(), "hero".to_string()];
    for fields in [None, Some(fields_list.as_slice())] {
        let nodes = readable(&storage, fields).await;
        for depth in 0..=4 {
            let resolver = resolver(&storage).with_auth(Some(reader_of("open")));
            let new = resolver
                .resolve_values_many("open", vec![doc.clone()], depth, fields)
                .await
                .unwrap()
                .pop()
                .unwrap()
                .into_json_round_trip();

            let mut old = serde_json::to_value(&doc).unwrap();
            let resolved = gather(&old, depth, "open", &nodes);
            let mut oracle = Oracle {
                ws: "open",
                resolved: &resolved,
                occurrences: 0,
                bytes: 0,
            };
            oracle.inline(&mut old, depth);
            let old = PropertyValue::from_json(&old);

            let same = new == old || format!("{new:?}") == format!("{old:?}");
            assert!(
                same,
                "depth {depth}, fields {fields:?}:\n new {new:?}\n old {old:?}"
            );
            assert!(depth == 0 || oracle.occurrences > 2, "the fixture inlines");
            let stats = resolver.memo.stats();
            assert_eq!(stats.occurrences, oracle.occurrences, "depth {depth}");
            assert_eq!(stats.bytes, oracle.bytes, "depth {depth}");
        }
    }
}
