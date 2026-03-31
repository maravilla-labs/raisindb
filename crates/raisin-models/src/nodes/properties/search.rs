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

use super::properties::Properties;
use super::value::PropertyValue;

impl<'a> Properties<'a> {
    /// Iteratively search for an object with a matching "id" field starting from a given base path.
    /// Returns a Properties wrapper for the matching object, if found.
    pub fn find_object_by_id(&self, base_path: &str, id: &str) -> Option<Properties<'_>> {
        let starting = self.get(base_path)?;
        let mut stack = vec![starting];
        while let Some(value) = stack.pop() {
            match value {
                PropertyValue::Object(obj) => {
                    let props = Properties::new(obj);
                    if let Some(current_id) = props.get_string("id") {
                        if current_id == id {
                            return Some(props);
                        }
                    }
                    for (_k, v) in obj.iter() {
                        stack.push(v);
                    }
                }
                PropertyValue::Array(arr) => {
                    for v in arr {
                        stack.push(v);
                    }
                }
                _ => {}
            }
        }
        None
    }
}
