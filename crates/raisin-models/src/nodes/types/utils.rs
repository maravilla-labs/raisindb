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

// Regex, validation, and allowed_children_schema

use crate::errors::RaisinModelError;
use lazy_static::lazy_static;
use regex::Regex;
use schemars::{json_schema, Schema};

lazy_static! {
    pub static ref URL_FRIENDLY_NAME_REGEX: Regex =
        Regex::new(r"^[a-zA-Z]+:(?:[A-Z][a-z]*)+$").unwrap();
}

pub fn validate_allowed_children(allowed_children: &Vec<String>) -> Result<(), RaisinModelError> {
    for child in allowed_children {
        if !URL_FRIENDLY_NAME_REGEX.is_match(child) {
            let mut err = validator::ValidationError::new("invalid_allowed_child");
            err.add_param("value".into(), &child);
            let mut errors = validator::ValidationErrors::new();
            errors.add("allowed_children", err);
            return Err(RaisinModelError::Validation(errors));
        }
    }
    Ok(())
}

// Use the new Schemars 1.0 API for allowed_children_schema
pub fn allowed_children_schema(_gen: &mut schemars::SchemaGenerator) -> Schema {
    json_schema!({
        "type": "array",
        "items": {
            "type": "string",
            "pattern": "^[a-zA-Z]+:(?:[A-Z][a-z]*)+$"
        }
    })
}
