// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Reference types for zero-copy access to PropertyValue data.

use rust_decimal::Decimal;
use std::collections::HashMap;

use super::value::{
    Composite, DateTimeTimestamp, Element, GeoJson, PropertyValue, RaisinReference, RaisinUrl,
    Resource,
};

/// A reference to a property value (avoids cloning for read-only access)
///
/// This enum mirrors `PropertyValue` but holds references instead of owned values.
/// Use this when you only need to inspect a value without taking ownership.
///
/// # Performance
///
/// Using references instead of cloning avoids:
/// - String cloning (O(n) allocation)
/// - HashMap cloning (O(n) allocation)
/// - Vec cloning (O(n) allocation)
///
/// Copy types (bool, i64, f64) are returned by value since copying is cheaper than
/// dereferencing in most cases.
///
/// # Example
///
/// ```rust,ignore
/// // Instead of cloning:
/// if let Some(PropertyValue::String(s)) = column.get(index) {
///     println!("{}", s);  // s is owned, String was cloned
/// }
///
/// // Use PropertyValueRef for read-only access:
/// if let Some(PropertyValueRef::String(s)) = column.get_ref(index) {
///     println!("{}", s);  // s is &str, no clone
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValueRef<'a> {
    /// Null value
    Null,
    /// Reference to a timestamp
    Date(&'a DateTimeTimestamp),
    /// Boolean value (Copy type, returned by value)
    Boolean(bool),
    /// Integer value (Copy type, returned by value)
    Integer(i64),
    /// Float value (Copy type, returned by value)
    Float(f64),
    /// Decimal value (Copy type via ref)
    Decimal(&'a Decimal),
    /// Reference to a string
    String(&'a str),
    /// Reference to a URL
    Url(&'a RaisinUrl),
    /// Reference to a node reference
    Reference(&'a RaisinReference),
    /// Reference to a resource
    Resource(&'a Resource),
    /// Reference to a composite
    Composite(&'a Composite),
    /// Reference to an element
    Element(&'a Element),
    /// Reference to a vector embedding
    Vector(&'a [f32]),
    /// Reference to a geometry
    Geometry(&'a GeoJson),
    /// Reference to an array
    Array(&'a [PropertyValue]),
    /// Reference to an object
    Object(&'a HashMap<String, PropertyValue>),
}

impl<'a> PropertyValueRef<'a> {
    /// Convert this reference to an owned PropertyValue (clones the data)
    pub fn to_owned(&self) -> PropertyValue {
        match self {
            PropertyValueRef::Null => PropertyValue::Null,
            PropertyValueRef::Date(dt) => PropertyValue::Date((*dt).clone()),
            PropertyValueRef::Boolean(b) => PropertyValue::Boolean(*b),
            PropertyValueRef::Integer(n) => PropertyValue::Integer(*n),
            PropertyValueRef::Float(n) => PropertyValue::Float(*n),
            PropertyValueRef::Decimal(d) => PropertyValue::Decimal(**d),
            PropertyValueRef::String(s) => PropertyValue::String((*s).to_string()),
            PropertyValueRef::Url(u) => PropertyValue::Url((*u).clone()),
            PropertyValueRef::Reference(r) => PropertyValue::Reference((*r).clone()),
            PropertyValueRef::Resource(r) => PropertyValue::Resource((*r).clone()),
            PropertyValueRef::Composite(c) => PropertyValue::Composite((*c).clone()),
            PropertyValueRef::Element(e) => PropertyValue::Element((*e).clone()),
            PropertyValueRef::Vector(v) => PropertyValue::Vector(v.to_vec()),
            PropertyValueRef::Geometry(g) => PropertyValue::Geometry((*g).clone()),
            PropertyValueRef::Array(a) => PropertyValue::Array(a.to_vec()),
            PropertyValueRef::Object(o) => PropertyValue::Object((*o).clone()),
        }
    }

    /// Create a reference from a PropertyValue
    pub fn from_value(value: &'a PropertyValue) -> Self {
        match value {
            PropertyValue::Null => PropertyValueRef::Null,
            PropertyValue::Date(dt) => PropertyValueRef::Date(dt),
            PropertyValue::Boolean(b) => PropertyValueRef::Boolean(*b),
            PropertyValue::Integer(n) => PropertyValueRef::Integer(*n),
            PropertyValue::Float(n) => PropertyValueRef::Float(*n),
            PropertyValue::Decimal(d) => PropertyValueRef::Decimal(d),
            PropertyValue::String(s) => PropertyValueRef::String(s.as_str()),
            PropertyValue::Url(u) => PropertyValueRef::Url(u),
            PropertyValue::Reference(r) => PropertyValueRef::Reference(r),
            PropertyValue::Resource(r) => PropertyValueRef::Resource(r),
            PropertyValue::Composite(c) => PropertyValueRef::Composite(c),
            PropertyValue::Element(e) => PropertyValueRef::Element(e),
            PropertyValue::Vector(v) => PropertyValueRef::Vector(v.as_slice()),
            PropertyValue::Geometry(g) => PropertyValueRef::Geometry(g),
            PropertyValue::Array(a) => PropertyValueRef::Array(a.as_slice()),
            PropertyValue::Object(o) => PropertyValueRef::Object(o),
        }
    }

    /// Check if this is a null value
    pub fn is_null(&self) -> bool {
        matches!(self, PropertyValueRef::Null)
    }

    /// Check if this is a boolean value
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            PropertyValueRef::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Check if this is an integer value
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            PropertyValueRef::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// Check if this is a float value
    pub fn as_float(&self) -> Option<f64> {
        match self {
            PropertyValueRef::Float(n) => Some(*n),
            _ => None,
        }
    }

    /// Get as string reference if this is a string value
    pub fn as_str(&self) -> Option<&'a str> {
        match self {
            PropertyValueRef::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get as object reference if this is an object value
    pub fn as_object(&self) -> Option<&'a HashMap<String, PropertyValue>> {
        match self {
            PropertyValueRef::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Get as array reference if this is an array value
    pub fn as_array(&self) -> Option<&'a [PropertyValue]> {
        match self {
            PropertyValueRef::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Get as vector reference if this is a vector value
    pub fn as_vector(&self) -> Option<&'a [f32]> {
        match self {
            PropertyValueRef::Vector(v) => Some(v),
            _ => None,
        }
    }
}
