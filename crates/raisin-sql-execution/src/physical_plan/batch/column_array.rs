//! Columnar array storage for typed column data.
//!
//! [`ColumnArray`] stores values of a specific type in a columnar format,
//! using `Option<T>` to represent NULL values with type safety.

use raisin_models::nodes::properties::value::{
    Composite, DateTimeTimestamp, Element, RaisinReference, RaisinUrl, Resource,
};
use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

/// A columnar array storing values of a specific type
///
/// Each variant contains a vector of Option<T> to support NULL values.
/// Using Option<T> provides:
/// 1. Idiomatic Rust null handling
/// 2. Type safety
/// 3. Clear semantics without separate null bitmaps
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnArray {
    /// Column of timestamps (seconds since epoch)
    Date(Vec<Option<DateTimeTimestamp>>),
    /// Column of boolean values
    Boolean(Vec<Option<bool>>),
    /// Column of integer values (i64)
    Integer(Vec<Option<i64>>),
    /// Column of float values (f64)
    Float(Vec<Option<f64>>),
    /// Column of string values
    String(Vec<Option<String>>),
    /// Column of URL values with rich metadata
    Url(Vec<Option<RaisinUrl>>),
    /// Column of references to other nodes
    Reference(Vec<Option<RaisinReference>>),
    /// Column of resource metadata
    Resource(Vec<Option<Resource>>),
    /// Column of composite structures
    Composite(Vec<Option<Composite>>),
    /// Column of elements
    Element(Vec<Option<Element>>),
    /// Column of vector embeddings
    Vector(Vec<Option<Vec<f32>>>),
    /// Column of arrays (nested PropertyValue arrays)
    Array(Vec<Option<Vec<PropertyValue>>>),
    /// Column of objects (nested HashMaps)
    Object(Vec<Option<HashMap<String, PropertyValue>>>),
}

impl ColumnArray {
    /// Get the number of elements in this column
    pub fn len(&self) -> usize {
        match self {
            ColumnArray::Date(v) => v.len(),
            ColumnArray::Boolean(v) => v.len(),
            ColumnArray::Integer(v) => v.len(),
            ColumnArray::Float(v) => v.len(),
            ColumnArray::String(v) => v.len(),
            ColumnArray::Url(v) => v.len(),
            ColumnArray::Reference(v) => v.len(),
            ColumnArray::Resource(v) => v.len(),
            ColumnArray::Composite(v) => v.len(),
            ColumnArray::Element(v) => v.len(),
            ColumnArray::Vector(v) => v.len(),
            ColumnArray::Array(v) => v.len(),
            ColumnArray::Object(v) => v.len(),
        }
    }

    /// Check if this column is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the value at the specified row index
    pub fn get(&self, index: usize) -> Option<PropertyValue> {
        match self {
            ColumnArray::Date(v) => v.get(index)?.as_ref().map(|dt| PropertyValue::Date(*dt)),
            ColumnArray::Boolean(v) => v.get(index)?.as_ref().map(|b| PropertyValue::Boolean(*b)),
            ColumnArray::Integer(v) => v.get(index)?.as_ref().map(|n| PropertyValue::Integer(*n)),
            ColumnArray::Float(v) => v.get(index)?.as_ref().map(|n| PropertyValue::Float(*n)),
            ColumnArray::String(v) => v
                .get(index)?
                .as_ref()
                .map(|s| PropertyValue::String(s.clone())),
            ColumnArray::Url(v) => v
                .get(index)?
                .as_ref()
                .map(|u| PropertyValue::Url(u.clone())),
            ColumnArray::Reference(v) => v
                .get(index)?
                .as_ref()
                .map(|r| PropertyValue::Reference(r.clone())),
            ColumnArray::Resource(v) => v
                .get(index)?
                .as_ref()
                .map(|r| PropertyValue::Resource(r.clone())),
            ColumnArray::Composite(v) => v
                .get(index)?
                .as_ref()
                .map(|c| PropertyValue::Composite(c.clone())),
            ColumnArray::Element(v) => v
                .get(index)?
                .as_ref()
                .map(|e| PropertyValue::Element(e.clone())),
            ColumnArray::Vector(v) => v
                .get(index)?
                .as_ref()
                .map(|vec| PropertyValue::Vector(vec.clone())),
            ColumnArray::Array(v) => v
                .get(index)?
                .as_ref()
                .map(|arr| PropertyValue::Array(arr.clone())),
            ColumnArray::Object(v) => v
                .get(index)?
                .as_ref()
                .map(|obj| PropertyValue::Object(obj.clone())),
        }
    }

    /// Create an empty column of a specific type with pre-allocated capacity
    pub(crate) fn with_capacity(capacity: usize, variant: &ColumnArray) -> Self {
        match variant {
            ColumnArray::Date(_) => ColumnArray::Date(Vec::with_capacity(capacity)),
            ColumnArray::Boolean(_) => ColumnArray::Boolean(Vec::with_capacity(capacity)),
            ColumnArray::Integer(_) => ColumnArray::Integer(Vec::with_capacity(capacity)),
            ColumnArray::Float(_) => ColumnArray::Float(Vec::with_capacity(capacity)),
            ColumnArray::String(_) => ColumnArray::String(Vec::with_capacity(capacity)),
            ColumnArray::Url(_) => ColumnArray::Url(Vec::with_capacity(capacity)),
            ColumnArray::Reference(_) => ColumnArray::Reference(Vec::with_capacity(capacity)),
            ColumnArray::Resource(_) => ColumnArray::Resource(Vec::with_capacity(capacity)),
            ColumnArray::Composite(_) => ColumnArray::Composite(Vec::with_capacity(capacity)),
            ColumnArray::Element(_) => ColumnArray::Element(Vec::with_capacity(capacity)),
            ColumnArray::Vector(_) => ColumnArray::Vector(Vec::with_capacity(capacity)),
            ColumnArray::Array(_) => ColumnArray::Array(Vec::with_capacity(capacity)),
            ColumnArray::Object(_) => ColumnArray::Object(Vec::with_capacity(capacity)),
        }
    }

    /// Push a value to this column
    pub(crate) fn push(&mut self, value: Option<PropertyValue>) {
        // Handle None values first
        if value.is_none() {
            self.push_none();
            return;
        }

        // Handle Some values with type matching
        let value = value.unwrap();
        match self {
            ColumnArray::Date(v) => {
                if let PropertyValue::Date(dt) = value {
                    v.push(Some(dt));
                } else {
                    v.push(None); // Type mismatch
                }
            }
            ColumnArray::Boolean(v) => {
                if let PropertyValue::Boolean(b) = value {
                    v.push(Some(b));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::Integer(v) => {
                if let PropertyValue::Integer(n) = value {
                    v.push(Some(n));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::Float(v) => {
                if let PropertyValue::Float(n) = value {
                    v.push(Some(n));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::String(v) => {
                if let PropertyValue::String(s) = value {
                    v.push(Some(s));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::Url(v) => {
                if let PropertyValue::Url(u) = value {
                    v.push(Some(u));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::Reference(v) => {
                if let PropertyValue::Reference(r) = value {
                    v.push(Some(r));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::Resource(v) => {
                if let PropertyValue::Resource(r) = value {
                    v.push(Some(r));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::Composite(v) => {
                if let PropertyValue::Composite(c) = value {
                    v.push(Some(c));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::Element(v) => {
                if let PropertyValue::Element(e) = value {
                    v.push(Some(e));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::Vector(v) => {
                if let PropertyValue::Vector(vec) = value {
                    v.push(Some(vec));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::Array(v) => {
                if let PropertyValue::Array(arr) = value {
                    v.push(Some(arr));
                } else {
                    v.push(None);
                }
            }
            ColumnArray::Object(v) => {
                if let PropertyValue::Object(obj) = value {
                    v.push(Some(obj));
                } else {
                    v.push(None);
                }
            }
        }
    }

    /// Push a None value to this column
    fn push_none(&mut self) {
        match self {
            ColumnArray::Date(v) => v.push(None),
            ColumnArray::Boolean(v) => v.push(None),
            ColumnArray::Integer(v) => v.push(None),
            ColumnArray::Float(v) => v.push(None),
            ColumnArray::String(v) => v.push(None),
            ColumnArray::Url(v) => v.push(None),
            ColumnArray::Reference(v) => v.push(None),
            ColumnArray::Resource(v) => v.push(None),
            ColumnArray::Composite(v) => v.push(None),
            ColumnArray::Element(v) => v.push(None),
            ColumnArray::Vector(v) => v.push(None),
            ColumnArray::Array(v) => v.push(None),
            ColumnArray::Object(v) => v.push(None),
        }
    }
}
