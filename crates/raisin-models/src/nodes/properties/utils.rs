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

// Utility functions, regex, and deserialization helpers

use lazy_static::lazy_static;
use regex::Regex;
use schemars::{json_schema, Schema};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::errors::RaisinModelError;

use super::schema::PropertyType;
use super::schema::PropertyValueSchema;
use super::value::{RaisinReference, RaisinUrl};

lazy_static! {
    pub static ref URL_FRIENDLY_NAME_REGEX: Regex =
        Regex::new(r"^[a-z_]+$").expect("invalid URL_FRIENDLY_NAME_REGEX pattern");
}

pub fn validate_allow_additional_properties(
    schema: &PropertyValueSchema,
) -> Result<(), RaisinModelError> {
    if let Some(allow_additional) = schema.allow_additional_properties {
        if schema.property_type != PropertyType::Object && allow_additional {
            return Err(RaisinModelError::Other(
                "allow_additional_properties_must_be_false_when_not_object".to_string(),
            ));
        }
    }
    Ok(())
}

pub fn allow_additional_properties_schema(_gen: &mut schemars::SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "properties": {
            "property_type": { "const": "Object" },
            "allow_additional_properties": { "type": "boolean" }
        },
        "required": ["property_type"]
    })
}

pub fn deserialize_raisin_reference<'de, D>(
    deserializer: D,
) -> Result<RaisinReference, RaisinModelError>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer).map_err(RaisinModelError::from_serde)?;

    // Handle JSON object format: {"raisin:ref": "...", "raisin:workspace": "...", "raisin:path": "..."}
    if let Value::Object(ref map) = value {
        // raisin:path is optional - it will be auto-populated during INSERT/UPDATE
        // if raisin:ref contains a path (starts with '/')

        if map.contains_key("raisin:ref") && map.contains_key("raisin:workspace") {
            return RaisinReference::deserialize(value).map_err(RaisinModelError::from_serde);
        }
    }

    // Handle MessagePack tuple format: ["id", "workspace", "path"] or ["id", "workspace"]
    // This happens because rmp_serde::to_vec serializes structs as arrays (field order)
    //
    // STRICT VALIDATION: Only accept if `id` looks like a real node reference.
    // This prevents plain string arrays like ["test", "integration"] from being
    // incorrectly deserialized as RaisinReference (issue: keywords bug).
    //
    // Valid reference ids:
    // - UUIDs: contain hyphens (e.g., "550e8400-e29b-41d4-a716-446655440000")
    // - Nanoids: 21+ characters (e.g., "V1StGXR8_Z5jdHi6B-myT")
    // - Paths: start with "/" (e.g., "/content/articles/my-post")
    // if let Value::Array(ref arr) = value {
    //     if arr.len() >= 2 && arr.len() <= 3 {
    //         if let (Some(id), Some(workspace)) = (arr[0].as_str(), arr[1].as_str()) {
    //             // Strict validation: id must look like a node reference
    //             let is_uuid_or_nanoid = id.contains('-') || id.len() >= 21;
    //             let is_path = id.starts_with('/');

    //             if is_uuid_or_nanoid || is_path {
    //                 let path = arr
    //                     .get(2)
    //                     .and_then(|v| v.as_str())
    //                     .unwrap_or("")
    //                     .to_string();
    //                 return Ok(RaisinReference {
    //                     id: id.to_string(),
    //                     workspace: workspace.to_string(),
    //                     path,
    //                 });
    //             }
    //             // If id is a short string without hyphens (like "test"), it's NOT a reference
    //         }
    //     }
    // }

    Err(RaisinModelError::Other("Not a RaisinReference".to_string()))
}

pub fn deserialize_raisin_url<'de, D>(deserializer: D) -> Result<RaisinUrl, RaisinModelError>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer).map_err(RaisinModelError::from_serde)?;
    if let Value::Object(ref map) = value {
        if map.contains_key("raisin:url") {
            return RaisinUrl::deserialize(value).map_err(RaisinModelError::from_serde);
        }
    }
    Err(RaisinModelError::Other("Not a RaisinUrl".to_string()))
}
