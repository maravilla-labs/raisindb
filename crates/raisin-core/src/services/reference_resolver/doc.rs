//! The document RESOLVE walks: a [`PropertyValue`] read straight from the
//! stored records (plan Phase 13d).
//!
//! RESOLVE used to convert every document and every target node to
//! `serde_json::Value`, walk and inline that, and hand the projection a JSON
//! document to convert back — 2.2× the storage floor, all of it JSON. It now
//! walks the values themselves. Its output is defined as before: the value
//! whose JSON rendering (`serde_json::to_value`) is the document the JSON
//! resolver produced. That holds because rendering is compositional — a
//! container renders each child where it sits — PROVIDED the walker sees a
//! reference exactly where the JSON walker saw one, and can put a target
//! wherever the JSON walker put one. This module is those two guarantees:
//!
//! - [`reference`]: is this value, rendered, an object with a string
//!   `raisin:ref`? (A reference; a plain object or an element whose
//!   `raisin:ref` renders as a string.)
//! - [`make_walkable`]: the two places a typed value cannot hold a target —
//!   a composite ITEM that is itself a reference, and a resource's
//!   `metadata` map that is one — are re-spelled as the plain structure they
//!   render to before anything is walked.

use super::walk::RawRef;
use raisin_models::nodes::properties::PropertyValue;
use std::borrow::Cow;
use std::collections::HashMap;

pub(super) const REF_KEY: &str = "raisin:ref";
pub(super) const WORKSPACE_KEY: &str = "raisin:workspace";

/// `Some(raw)` when `value` renders as a reference object.
pub(super) fn reference(value: &PropertyValue) -> Option<(Option<Cow<'_, str>>, Cow<'_, str>)> {
    match value {
        PropertyValue::Reference(r) => Some((
            (!r.workspace.is_empty()).then_some(Cow::Borrowed(r.workspace.as_str())),
            Cow::Borrowed(r.id.as_str()),
        )),
        PropertyValue::Object(map) => reference_in(map),
        // An element renders flat: its content keys sit beside
        // `element_type` and `uuid`.
        PropertyValue::Element(element) => reference_in(&element.content),
        _ => None,
    }
}

/// [`reference`] for a map rendered as an object.
pub(super) fn reference_in(
    map: &HashMap<String, PropertyValue>,
) -> Option<(Option<Cow<'_, str>>, Cow<'_, str>)> {
    let locator = json_str(map.get(REF_KEY)?)?;
    let workspace = map
        .get(WORKSPACE_KEY)
        .and_then(json_str)
        .filter(|ws| !ws.is_empty());
    Some((workspace, locator))
}

/// The string a value renders as, when it renders as a JSON string.
fn json_str(value: &PropertyValue) -> Option<Cow<'_, str>> {
    match value {
        PropertyValue::String(s) => Some(Cow::Borrowed(s.as_str())),
        PropertyValue::Date(_) | PropertyValue::Decimal(_) => serde_json::to_value(value)
            .ok()
            .and_then(|json| json.as_str().map(|s| Cow::Owned(s.to_string()))),
        _ => None,
    }
}

impl RawRef {
    /// The reference `value` renders as, if any.
    pub(super) fn of(value: &PropertyValue) -> Option<RawRef> {
        reference(value).map(|(workspace, locator)| RawRef {
            workspace: workspace.map(Cow::into_owned),
            locator: locator.into_owned(),
        })
    }
}

/// Visit every value rendered as a member of `value`'s rendering — what the
/// JSON walker recursed into. A reference is a leaf: callers check
/// [`reference`] first.
pub(super) fn for_each_child(value: &PropertyValue, f: &mut dyn FnMut(&PropertyValue)) {
    match value {
        PropertyValue::Object(map) => map.values().for_each(f),
        PropertyValue::Array(items) => items.iter().for_each(f),
        PropertyValue::Element(element) => element.content.values().for_each(f),
        PropertyValue::Composite(composite) => composite
            .items
            .iter()
            .flat_map(|item| item.content.values())
            .for_each(f),
        PropertyValue::Resource(resource) => {
            resource.metadata.iter().flatten().for_each(|(_, v)| f(v))
        }
        _ => {}
    }
}

/// Mutable twin of [`for_each_child`].
pub(super) fn for_each_child_mut(value: &mut PropertyValue, f: &mut dyn FnMut(&mut PropertyValue)) {
    match value {
        PropertyValue::Object(map) => map.values_mut().for_each(f),
        PropertyValue::Array(items) => items.iter_mut().for_each(f),
        PropertyValue::Element(element) => element.content.values_mut().for_each(f),
        PropertyValue::Composite(composite) => composite
            .items
            .iter_mut()
            .flat_map(|item| item.content.values_mut())
            .for_each(f),
        PropertyValue::Resource(resource) => resource
            .metadata
            .iter_mut()
            .flatten()
            .for_each(|(_, v)| f(v)),
        _ => {}
    }
}

/// Re-spell, as the plain structure it renders to, every value that holds a
/// reference in a position no typed value can hold a target in.
pub(super) fn make_walkable(value: &mut PropertyValue) {
    let respell = match value {
        PropertyValue::Composite(composite) => composite
            .items
            .iter()
            .any(|item| reference_in(&item.content).is_some()),
        PropertyValue::Resource(resource) => resource
            .metadata
            .as_ref()
            .is_some_and(|metadata| reference_in(metadata).is_some()),
        _ => false,
    };
    if respell {
        if let Ok(json) = serde_json::to_value(&*value) {
            *value = structural(json);
        }
    }
    if reference(value).is_none() {
        for_each_child_mut(value, &mut make_walkable);
    }
}

/// A JSON value as the value tree it is, classifying nothing: objects stay
/// objects, arrays arrays, strings strings — so it renders back to itself.
pub(super) fn structural(json: serde_json::Value) -> PropertyValue {
    use serde_json::Value;
    match json {
        Value::Null => PropertyValue::Null,
        Value::Bool(b) => PropertyValue::Boolean(b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => PropertyValue::Integer(i),
            None => PropertyValue::Float(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => PropertyValue::String(s),
        Value::Array(items) => PropertyValue::Array(items.into_iter().map(structural).collect()),
        Value::Object(map) => {
            PropertyValue::Object(map.into_iter().map(|(k, v)| (k, structural(v))).collect())
        }
    }
}
