//! The byte length of a value's JSON rendering — `serde_json::to_vec(
//! &serde_json::to_value(value)).len()`, the budget's count of inlined bytes —
//! without building the rendering (plan Phase 13d).
//!
//! Exact, not approximate, so the budget errors at the same document as
//! before. Rendering through `to_value` first matters in two places a direct
//! `to_writer` would disagree: a vector's `f32`s are widened to `f64` (and
//! printed at that width), and an element whose content repeats
//! `element_type` / `uuid` renders that key ONCE (a map). Values this does
//! not compute directly (dates, decimals, URLs, resources, geometries) are
//! rendered for real.

use raisin_models::nodes::properties::value::Element;
use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

struct Count(usize);

impl std::io::Write for Count {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// How long `serde_json` writes `value` (a string, a number).
fn written<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    let mut count = Count(0);
    match serde_json::to_writer(&mut count, value) {
        Ok(()) => count.0,
        Err(_) => 0,
    }
}

fn float(f: f64) -> usize {
    if f.is_finite() {
        written(&f)
    } else {
        4 // null
    }
}

/// `serde_json::to_vec(&serde_json::to_value(value)).len()`.
pub(super) fn json_len(value: &PropertyValue) -> usize {
    match value {
        PropertyValue::Null => 4,
        PropertyValue::Boolean(b) => {
            if *b {
                4
            } else {
                5
            }
        }
        PropertyValue::Integer(i) => written(i),
        PropertyValue::Float(f) => float(*f),
        PropertyValue::String(s) => written(s.as_str()),
        PropertyValue::Vector(floats) => {
            list(floats.len(), floats.iter().map(|f| float(f64::from(*f))))
        }
        PropertyValue::Array(items) => list(items.len(), items.iter().map(json_len)),
        PropertyValue::Object(map) => object(map.iter().map(|(k, v)| (k.as_str(), json_len(v)))),
        PropertyValue::Reference(r) => {
            let mut entries = vec![
                ("raisin:ref", written(r.id.as_str())),
                ("raisin:workspace", written(r.workspace.as_str())),
            ];
            if !r.path.is_empty() {
                entries.push(("raisin:path", written(r.path.as_str())));
            }
            object(entries.into_iter())
        }
        PropertyValue::Element(element) => element_len(element),
        PropertyValue::Composite(composite) => object(
            [
                ("uuid", written(composite.uuid.as_str())),
                (
                    "items",
                    list(
                        composite.items.len(),
                        composite.items.iter().map(element_len),
                    ),
                ),
            ]
            .into_iter(),
        ),
        other => serde_json::to_value(other)
            .ok()
            .and_then(|json| serde_json::to_vec(&json).ok())
            .map_or(0, |bytes| bytes.len()),
    }
}

/// An element renders as ONE map: `element_type`, `uuid` (when set), then
/// its content — a content key repeating either replaces it.
fn element_len(element: &Element) -> usize {
    let content: &HashMap<String, PropertyValue> = &element.content;
    let mut own: Vec<(&str, usize)> = Vec::with_capacity(2);
    if !content.contains_key("element_type") {
        own.push(("element_type", written(element.element_type.as_str())));
    }
    if !element.uuid.is_empty() && !content.contains_key("uuid") {
        own.push(("uuid", written(element.uuid.as_str())));
    }
    object(
        own.into_iter()
            .chain(content.iter().map(|(k, v)| (k.as_str(), json_len(v)))),
    )
}

/// `[a,b,…]` around `n` items of the given lengths.
fn list(n: usize, lengths: impl Iterator<Item = usize>) -> usize {
    2 + n.saturating_sub(1) + lengths.sum::<usize>()
}

/// `{"k":v,…}` around entries of the given key and value lengths. Keys are
/// distinct by construction (a map's; `element_len` drops its own keys that
/// a content key repeats).
fn object<'a>(entries: impl Iterator<Item = (&'a str, usize)>) -> usize {
    let mut n: usize = 0;
    let mut total: usize = 2;
    for (key, len) in entries {
        n += 1;
        total += written(key) + 1 + len;
    }
    total + n.saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::value::Composite;
    use raisin_models::nodes::properties::RaisinReference;

    #[test]
    fn equals_the_rendered_length() {
        let s = |v: &str| PropertyValue::String(v.into());
        let reference = PropertyValue::Reference(RaisinReference {
            id: "n\"1".into(),
            workspace: "ws".into(),
            path: "/é".into(),
        });
        let content: HashMap<String, PropertyValue> = [
            ("title".to_string(), s("t\n")),
            ("uuid".to_string(), s("override")),
            ("n".to_string(), PropertyValue::Float(0.1)),
        ]
        .into_iter()
        .collect();
        let element = Element {
            uuid: "u".into(),
            element_type: "hero".into(),
            content,
        };
        let values = vec![
            PropertyValue::Null,
            PropertyValue::Boolean(false),
            PropertyValue::Integer(-42),
            PropertyValue::Float(1e300),
            PropertyValue::Float(f64::NAN),
            PropertyValue::Vector(vec![0.1, 1.0, f32::INFINITY]),
            reference.clone(),
            PropertyValue::Element(element.clone()),
            PropertyValue::Composite(Composite {
                uuid: "c".into(),
                items: vec![element.clone(), element],
            }),
            PropertyValue::Array(vec![]),
            PropertyValue::Object(HashMap::new()),
            PropertyValue::Object(
                [
                    ("a".to_string(), reference),
                    (
                        "ü".to_string(),
                        PropertyValue::Array(vec![s("x"), PropertyValue::Integer(1)]),
                    ),
                    (
                        "d".to_string(),
                        PropertyValue::Decimal("1.50".parse().unwrap()),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
        ];
        for value in values {
            let expected = serde_json::to_vec(&serde_json::to_value(&value).unwrap())
                .unwrap()
                .len();
            assert_eq!(json_len(&value), expected, "{value:?}");
        }
    }
}
