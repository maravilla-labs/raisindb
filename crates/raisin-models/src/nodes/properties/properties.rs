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

use std::collections::HashMap;

use crate::nodes::properties::value::RaisinReference;

use crate::nodes::properties::PropertyValue;

/// Wrapper over a HashMap of properties for ergonomic access helpers.

#[derive(Debug)]
pub struct Properties<'a> {
    data: &'a HashMap<String, PropertyValue>,
}

impl<'a> Properties<'a> {
    /// Create a new Properties wrapper
    pub fn new(data: &'a HashMap<String, PropertyValue>) -> Self {
        Self { data }
    }

    /// Get a nested property using dot notation
    pub fn get(&self, path: &str) -> Option<&PropertyValue> {
        let mut current: Option<&PropertyValue> = None;
        let mut current_map = self.data;

        for key in path.split('.') {
            current = current_map.get(key);
            match current {
                Some(PropertyValue::Object(obj)) => current_map = obj, // Dive into object
                Some(value) => return Some(value),                     // Return if not an object
                None => return None,                                   // Not found
            }
        }
        current
    }

    /// Get a property as a string
    pub fn get_string(&self, path: &str) -> Option<String> {
        self.get(path).and_then(|prop| match prop {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
    }

    /// Get a property as a number (f64)
    /// Converts Integer to f64 as well.
    pub fn get_number(&self, path: &str) -> Option<f64> {
        self.get(path).and_then(|prop| match prop {
            PropertyValue::Integer(n) => Some(*n as f64),
            PropertyValue::Float(n) => Some(*n),
            _ => None,
        })
    }

    /// Get a property as an exact integer (i64)
    pub fn get_integer(&self, path: &str) -> Option<i64> {
        self.get(path).and_then(|prop| match prop {
            PropertyValue::Integer(n) => Some(*n),
            _ => None,
        })
    }

    /// Get a property as a float (f64)
    pub fn get_float(&self, path: &str) -> Option<f64> {
        self.get(path).and_then(|prop| match prop {
            PropertyValue::Float(n) => Some(*n),
            PropertyValue::Integer(n) => Some(*n as f64), // Convenience conversion
            _ => None,
        })
    }

    /// Get a property as a boolean
    pub fn get_bool(&self, path: &str) -> Option<bool> {
        self.get(path).and_then(|prop| match prop {
            PropertyValue::Boolean(b) => Some(*b),
            _ => None,
        })
    }

    /// Get a property as a `RaisinReference`
    pub fn get_reference(&self, path: &str) -> Option<&RaisinReference> {
        self.get(path).and_then(|prop| match prop {
            PropertyValue::Reference(r) => Some(r),
            _ => None,
        })
    }

    /// Get a property as an array of PropertyValue.
    pub fn get_array(&self, path: &str) -> Option<&Vec<PropertyValue>> {
        self.get(path).and_then(|prop| match prop {
            PropertyValue::Array(arr) => Some(arr),
            _ => None,
        })
    }

    pub fn get_string_array(&self, path: &str) -> Option<Vec<String>> {
        self.get(path).and_then(|prop| match prop {
            PropertyValue::Array(arr) => {
                let strings: Vec<String> = arr
                    .iter()
                    .filter_map(|item| {
                        if let PropertyValue::String(s) = item {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                if strings.is_empty() {
                    None
                } else {
                    Some(strings)
                }
            }
            _ => None,
        })
    }

    pub fn get_array_owned(&self, path: &str) -> Option<Vec<PropertyValue>> {
        self.get(path).and_then(|prop| match prop {
            PropertyValue::Array(arr) => Some(arr.clone()),
            _ => None,
        })
    }

    /// Get a property as an object
    pub fn get_object(&self, path: &str) -> Option<Properties<'_>> {
        self.get(path).and_then(|prop| match prop {
            PropertyValue::Object(obj) => Some(Properties::new(obj)),
            _ => None,
        })
    }
}
