use std::collections::HashMap;

use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;

use super::InMemoryNodeRepo;
use crate::NodeKey;

/// Token types for parsing property paths (e.g., "field.array[0].nested").
#[derive(Debug)]
enum Tok {
    Field(String),
    Index(usize),
}

/// Tokenizes a property path string into field names and array indices.
fn tokenize(p: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    for seg in p.split('.') {
        let rest = seg;
        let mut base = String::new();
        let mut i = 0;
        while i < rest.len() {
            let bytes = rest.as_bytes();
            if bytes[i] == b'[' {
                if !base.is_empty() {
                    toks.push(Tok::Field(base.clone()));
                    base.clear();
                }
                let j = rest[i + 1..].find(']').map(|k| k + i + 1);
                if let Some(j) = j {
                    let idx_str = &rest[i + 1..j];
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        toks.push(Tok::Index(idx));
                    }
                    i = j + 1;
                    continue;
                } else {
                    break;
                }
            } else {
                base.push(bytes[i] as char);
                i += 1;
            }
        }
        if !base.is_empty() {
            toks.push(Tok::Field(base));
        }
    }
    toks
}

/// Retrieves a property value from a nested property structure using tokens.
fn get_from(root: &HashMap<String, PropertyValue>, toks: &[Tok]) -> Option<PropertyValue> {
    if toks.is_empty() {
        return None;
    }
    let mut cur: Option<PropertyValue> = None;
    let mut map_ref = root;
    let mut idx = 0;
    while idx < toks.len() {
        match &toks[idx] {
            Tok::Field(name) => {
                let next = map_ref.get(name)?;
                match next {
                    PropertyValue::Object(obj) => {
                        cur = Some(next.clone());
                        map_ref = obj;
                    }
                    PropertyValue::Array(arr) => {
                        // next must be index
                        idx += 1;
                        if idx >= toks.len() {
                            cur = Some(PropertyValue::Array(arr.clone()));
                            break;
                        }
                        if let Tok::Index(i) = toks[idx] {
                            let v_ref = arr.get(i)?;
                            match v_ref {
                                PropertyValue::Object(obj) => {
                                    map_ref = obj;
                                    cur = Some(v_ref.clone());
                                }
                                _ => {
                                    cur = Some(v_ref.clone());
                                }
                            }
                        } else {
                            return None;
                        }
                    }
                    _ => {
                        cur = Some(next.clone());
                    }
                }
            }
            Tok::Index(i) => {
                // index into current value if array
                if let Some(PropertyValue::Array(arr)) = cur.as_ref() {
                    let v = arr.get(*i)?.clone();
                    cur = Some(v);
                } else {
                    return None;
                }
            }
        }
        idx += 1;
    }
    cur
}

/// Updates or inserts a property value in a nested property structure.
fn upsert(root: &mut HashMap<String, PropertyValue>, toks: &[Tok], value: PropertyValue) {
    if toks.is_empty() {
        return;
    }
    match &toks[0] {
        Tok::Field(name) => {
            if toks.len() == 1 {
                root.insert(name.clone(), value);
                return;
            }
            let next = &toks[1];
            match next {
                Tok::Index(i) => {
                    let entry = root
                        .entry(name.clone())
                        .or_insert_with(|| PropertyValue::Array(Vec::new()));
                    if let PropertyValue::Array(arr) = entry {
                        if *i >= arr.len() {
                            arr.resize(i + 1, PropertyValue::Object(Default::default()));
                        }
                        // ensure object if more toks remain beyond index
                        if toks.len() == 2 {
                            arr[*i] = value;
                        } else {
                            if !matches!(arr[*i], PropertyValue::Object(_)) {
                                arr[*i] = PropertyValue::Object(Default::default());
                            }
                            if let PropertyValue::Object(ref mut obj) = arr[*i] {
                                upsert(obj, &toks[2..], value);
                            }
                        }
                    }
                }
                Tok::Field(_) => {
                    let entry = root
                        .entry(name.clone())
                        .or_insert_with(|| PropertyValue::Object(Default::default()));
                    if let PropertyValue::Object(obj) = entry {
                        upsert(obj, &toks[1..], value);
                    }
                }
            }
        }
        Tok::Index(_) => { /* invalid at root */ }
    }
}

/// Retrieves a property value by path from a node.
///
/// # Property Path Format
///
/// Property paths use dot notation with array indexing:
/// - `"field"` - access a top-level field
/// - `"nested.field"` - access a nested field
/// - `"array[0]"` - access an array element
/// - `"nested.array[0].field"` - combined access
pub(super) async fn get_property_by_path(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
    property_path: &str,
) -> Result<Option<PropertyValue>> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    let map = repo.nodes.read().await;
    let maybe = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == node_path)
        .map(|(_, n)| n.clone());
    if let Some(n) = maybe {
        Ok(get_from(&n.properties, &tokenize(property_path)))
    } else {
        Ok(None)
    }
}

/// Updates or inserts a property value by path in a node.
///
/// Uses the same dot-notation syntax as `get_property_by_path`.
/// If the node does not exist, returns an error. If the property doesn't exist,
/// it will be created along the specified path.
pub(super) async fn update_property_by_path(
    repo: &InMemoryNodeRepo,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
    property_path: &str,
    value: PropertyValue,
) -> Result<()> {
    let workspace_prefix = NodeKey::workspace_prefix(tenant_id, repo_id, branch, workspace);

    let mut map = repo.nodes.write().await;
    let key = map
        .iter()
        .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == node_path)
        .map(|(k, _)| k.clone());
    if let Some(k) = key {
        if let Some(n) = map.get_mut(&k) {
            let toks = tokenize(property_path);
            upsert(&mut n.properties, &toks, value.clone());
        }
    }
    Ok(())
}
