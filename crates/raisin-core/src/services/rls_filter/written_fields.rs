//! What a write writes, for field-level write grants (`write_grants`).
//!
//! Properties by name (engine-owned `$…` / `__…` keys exempt), TRANSLATIONS
//! by the property key they translate (an overlay of `title` is a write of
//! `title`), and every other client-writable node field — archetype, owner,
//! relations, path / name (a move or rename), publication — as a node field
//! that a field list cannot grant. An update writes what it CHANGES, a create
//! what it SETS.

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use std::collections::BTreeSet;

/// One thing a write writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Written {
    /// A top-level property (or its translation).
    Property(String),
    /// A node field outside the property map.
    NodeField(&'static str),
}

fn is_engine_owned(property: &str) -> bool {
    property.starts_with('$') || property.starts_with("__")
}

fn differs(a: &PropertyValue, b: &PropertyValue) -> bool {
    a != b && serde_json::to_value(a).ok() != serde_json::to_value(b).ok()
}

/// Keys of `new` that differ from `old` (both directions), as names.
fn changed_keys<'a>(
    old: Option<&'a std::collections::HashMap<String, PropertyValue>>,
    new: &'a std::collections::HashMap<String, PropertyValue>,
) -> impl Iterator<Item = String> + 'a {
    let added_or_changed = new
        .iter()
        .filter(move |(k, v)| old.and_then(|o| o.get(*k)).is_none_or(|o| differs(o, v)))
        .map(|(k, _)| k.clone());
    let removed = old
        .into_iter()
        .flat_map(|o| o.keys())
        .filter(move |k| !new.contains_key(*k))
        .cloned();
    added_or_changed.chain(removed)
}

/// The property keys a locale overlay carries; `None` when it is not an
/// object of properties (then the whole translation counts as one field).
fn overlay_keys(v: &PropertyValue) -> Option<Vec<String>> {
    match v {
        PropertyValue::Object(map) => Some(map.keys().cloned().collect()),
        _ => None,
    }
}

/// What `node` writes over `existing` (`None`: a create), sorted, deduplicated.
pub(super) fn written_fields(existing: Option<&Node>, node: &Node) -> Vec<Written> {
    let mut out: BTreeSet<Written> = BTreeSet::new();
    let empty = std::collections::HashMap::new();

    // Properties.
    for k in changed_keys(existing.map(|s| &s.properties), &node.properties) {
        if !is_engine_owned(&k) {
            out.insert(Written::Property(k));
        }
    }

    // Translations, by the property key they translate.
    let old_tr = existing
        .and_then(|s| s.translations.as_ref())
        .unwrap_or(&empty);
    let new_tr = node.translations.as_ref().unwrap_or(&empty);
    for locale in changed_keys(Some(old_tr), new_tr) {
        let (old_v, new_v) = (old_tr.get(&locale), new_tr.get(&locale));
        let keys = match (old_v.map(overlay_keys), new_v.map(overlay_keys)) {
            (Some(None), _) | (_, Some(None)) => None,
            (o, n) => {
                let o: Vec<String> = o.flatten().unwrap_or_default();
                let n: Vec<String> = n.flatten().unwrap_or_default();
                // Keys present on either side whose value differs.
                let get = |v: Option<&PropertyValue>, k: &str| match v {
                    Some(PropertyValue::Object(m)) => m.get(k).cloned(),
                    _ => None,
                };
                Some(
                    o.into_iter()
                        .chain(n)
                        .filter(|k| match (get(old_v, k), get(new_v, k)) {
                            (Some(a), Some(b)) => differs(&a, &b),
                            _ => true,
                        })
                        .collect::<Vec<_>>(),
                )
            }
        };
        match keys {
            Some(keys) => {
                for k in keys.into_iter().filter(|k| !is_engine_owned(k)) {
                    out.insert(Written::Property(k));
                }
            }
            None => {
                out.insert(Written::NodeField("translations"));
            }
        }
    }

    // Every other field of the node, decided EXPLICITLY. The destructuring
    // has no `..`: a field added to `Node` fails to compile here until it is
    // classified (property write, node-field write, or server-stamped).
    let Node {
        id: _,           // identity: an update is addressed by it
        name,            // placement (with path / parent)
        path,            // placement
        node_type,       // node field
        archetype,       // node field
        properties: _,   // property writes (above)
        children: _,     // derived from the tree, server-side
        order_key: _,    // server-assigned (ORDERED_CHILDREN label)
        has_children: _, // derived
        parent,          // placement
        version: _,      // server-stamped edit counter
        created_at: _,   // server-stamped
        updated_at: _,   // server-stamped
        published_at,    // node field
        published_by,    // node field
        updated_by: _,   // server-stamped
        created_by: _,   // server-stamped
        translations: _, // property writes by key (above)
        tenant_id: _,    // scope, not content
        workspace: _,    // scope, not content
        owner_id,        // node field
        relations,       // node field
    } = node;
    match existing {
        Some(stored) => {
            if stored.path != *path || stored.name != *name || stored.parent != *parent {
                out.insert(Written::NodeField("path"));
            }
            if stored.node_type != *node_type {
                out.insert(Written::NodeField("node_type"));
            }
            if stored.archetype != *archetype {
                out.insert(Written::NodeField("archetype"));
            }
            if stored.published_at != *published_at || stored.published_by != *published_by {
                out.insert(Written::NodeField("published"));
            }
            if stored.owner_id != *owner_id {
                out.insert(Written::NodeField("owner_id"));
            }
            if stored.relations != *relations {
                out.insert(Written::NodeField("relations"));
            }
        }
        None => {
            // A create's path and type are what the grant itself matched
            // (its path pattern and node_types); everything else it sets is
            // a write a field list cannot grant.
            let _ = (name, path, parent, node_type);
            if archetype.is_some() {
                out.insert(Written::NodeField("archetype"));
            }
            if published_at.is_some() || published_by.is_some() {
                out.insert(Written::NodeField("published"));
            }
            if owner_id.is_some() {
                out.insert(Written::NodeField("owner_id"));
            }
            if !relations.is_empty() {
                out.insert(Written::NodeField("relations"));
            }
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::{written_fields, Written};
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::nodes::Node;

    fn base() -> Node {
        Node {
            id: "n".into(),
            name: "a".into(),
            path: "/f/a".into(),
            parent: Some("f".into()),
            node_type: "x:A".into(),
            ..Default::default()
        }
    }

    fn node_fields(existing: Option<&Node>, node: &Node) -> Vec<&'static str> {
        written_fields(existing, node)
            .into_iter()
            .filter_map(|w| match w {
                Written::NodeField(f) => Some(f),
                Written::Property(_) => None,
            })
            .collect()
    }

    /// Every Node field, changed on its own in an UPDATE: what it counts as.
    #[test]
    fn every_node_field_is_classified_on_update() {
        let stored = base();
        let cases: Vec<(&str, Box<dyn Fn(&mut Node)>, Option<&str>)> = vec![
            ("name", Box::new(|n| n.name = "b".into()), Some("path")),
            ("path", Box::new(|n| n.path = "/g/a".into()), Some("path")),
            (
                "parent",
                Box::new(|n| n.parent = Some("g".into())),
                Some("path"),
            ),
            (
                "node_type",
                Box::new(|n| n.node_type = "x:B".into()),
                Some("node_type"),
            ),
            (
                "archetype",
                Box::new(|n| n.archetype = Some("t".into())),
                Some("archetype"),
            ),
            (
                "published_at",
                Box::new(|n| n.published_at = Some(chrono::Utc::now())),
                Some("published"),
            ),
            (
                "published_by",
                Box::new(|n| n.published_by = Some("u".into())),
                Some("published"),
            ),
            (
                "owner_id",
                Box::new(|n| n.owner_id = Some("u".into())),
                Some("owner_id"),
            ),
            (
                "relations",
                Box::new(|n| {
                    n.relations = vec![serde_json::from_value(serde_json::json!({
                        "target": "t", "workspace": "w"
                    }))
                    .unwrap()]
                }),
                Some("relations"),
            ),
            // Server-stamped / derived / scope: never a client write.
            ("id", Box::new(|n| n.id = "other".into()), None),
            (
                "children",
                Box::new(|n| n.children = vec!["c".into()]),
                None,
            ),
            ("order_key", Box::new(|n| n.order_key = "a0".into()), None),
            (
                "has_children",
                Box::new(|n| n.has_children = Some(true)),
                None,
            ),
            ("version", Box::new(|n| n.version = 9), None),
            (
                "created_at",
                Box::new(|n| n.created_at = Some(chrono::Utc::now())),
                None,
            ),
            (
                "updated_at",
                Box::new(|n| n.updated_at = Some(chrono::Utc::now())),
                None,
            ),
            (
                "updated_by",
                Box::new(|n| n.updated_by = Some("u".into())),
                None,
            ),
            (
                "created_by",
                Box::new(|n| n.created_by = Some("u".into())),
                None,
            ),
            (
                "tenant_id",
                Box::new(|n| n.tenant_id = Some("t".into())),
                None,
            ),
            (
                "workspace",
                Box::new(|n| n.workspace = Some("w".into())),
                None,
            ),
        ];
        for (field, change, expected) in &cases {
            let mut n = stored.clone();
            change(&mut n);
            let got = node_fields(Some(&stored), &n);
            assert_eq!(
                got,
                expected.into_iter().copied().collect::<Vec<_>>(),
                "{field}"
            );
        }
        // properties and translations are property writes (by key).
        let mut n = stored.clone();
        n.properties
            .insert("p".into(), PropertyValue::String("v".into()));
        assert_eq!(
            written_fields(Some(&stored), &n),
            vec![Written::Property("p".into())]
        );
    }

    /// A create: path and type are matched by the grant; archetype,
    /// publication, owner and relations are node-field writes.
    #[test]
    fn create_counts_archetype_publication_owner_and_relations() {
        assert!(node_fields(None, &base()).is_empty());
        let mut n = base();
        n.archetype = Some("t".into());
        n.published_at = Some(chrono::Utc::now());
        n.owner_id = Some("u".into());
        assert_eq!(
            node_fields(None, &n),
            vec!["archetype", "owner_id", "published"]
        );
    }
}
