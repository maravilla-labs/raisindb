//! The pre-frontier `resolve_json`, verbatim in behaviour: rewrite the whole
//! document once per level, memo keyed by id. The oracle the new engine's
//! output is compared against.

use serde_json::Value;
use std::collections::HashMap;

fn legacy_collect(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(id) = map.get("raisin:ref").and_then(Value::as_str) {
                out.push(id.to_string());
                return;
            }
            map.values().for_each(|v| legacy_collect(v, out));
        }
        Value::Array(items) => items.iter().for_each(|v| legacy_collect(v, out)),
        _ => {}
    }
}

fn legacy_replace(value: &mut Value, fetched: &HashMap<String, Option<Value>>) {
    match value {
        Value::Object(map) => {
            if let Some(id) = map.get("raisin:ref").and_then(Value::as_str) {
                if let Some(Some(node)) = fetched.get(id) {
                    *value = node.clone();
                }
                return;
            }
            map.values_mut().for_each(|v| legacy_replace(v, fetched));
        }
        Value::Array(items) => items.iter_mut().for_each(|v| legacy_replace(v, fetched)),
        _ => {}
    }
}

/// The pre-frontier `resolve_json`: rewrite the whole document once per level.
pub(super) fn legacy_resolve(value: &Value, depth: u32, nodes: &HashMap<String, Value>) -> Value {
    let mut current = value.clone();
    let mut fetched: HashMap<String, Option<Value>> = HashMap::new();
    for _ in 0..depth {
        let mut wanted = Vec::new();
        legacy_collect(&current, &mut wanted);
        if wanted.is_empty() {
            break;
        }
        let mut resolvable = false;
        for id in wanted {
            let node = fetched
                .entry(id.clone())
                .or_insert_with(|| nodes.get(&id).cloned());
            resolvable |= node.is_some();
        }
        if !resolvable {
            break;
        }
        legacy_replace(&mut current, &fetched);
    }
    current
}
