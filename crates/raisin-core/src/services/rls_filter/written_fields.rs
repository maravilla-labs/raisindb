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

    // Node fields outside the property map.
    match existing {
        Some(stored) => {
            if stored.archetype != node.archetype {
                out.insert(Written::NodeField("archetype"));
            }
            if stored.owner_id != node.owner_id {
                out.insert(Written::NodeField("owner_id"));
            }
            if stored.relations != node.relations {
                out.insert(Written::NodeField("relations"));
            }
            if stored.path != node.path || stored.name != node.name {
                out.insert(Written::NodeField("path"));
            }
            if stored.published_at != node.published_at || stored.published_by != node.published_by
            {
                out.insert(Written::NodeField("published"));
            }
        }
        None => {
            // A create names its path and type by the grant itself; an owner
            // or relations set on the new node are writes a field list
            // cannot grant.
            if node.owner_id.is_some() {
                out.insert(Written::NodeField("owner_id"));
            }
            if !node.relations.is_empty() {
                out.insert(Written::NodeField("relations"));
            }
        }
    }
    out.into_iter().collect()
}
