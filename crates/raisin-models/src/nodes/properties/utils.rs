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

/// The key that tags a decimal VALUE on the wire.
///
/// `raisin:` rather than BSON's `$`, because this codebase already tags
/// value-shaped maps that way — `raisin:ref` / `raisin:workspace` for a
/// Reference, `raisin:url` for a Url — and `Decimal` is now the same kind of
/// thing. It also keeps well clear of the `$` sigil, which is load-bearing one
/// level up: `is_reserved_property_key` is `starts_with('$')` and strips such
/// keys out of `properties` entirely. Different namespace, no functional
/// collision, but a reader should not have to work out which level they are at.
pub const DECIMAL_TAG: &str = "raisin:decimal";

/// Write a decimal as `{"raisin:decimal": "19.90"}` in EVERY format.
///
/// A decimal used to serialize as a bare string (`rust_decimal` is built
/// `serde-str`), which made it byte-identical to a `String` — so the untagged
/// ladder, which tries `Decimal` before `String`, claimed every numeric-looking
/// string on the way out of storage and `"05"` came back as `5`.
///
/// The fix is to stop throwing the information away at the ENCODER. One tagged
/// form for both MessagePack and JSON, deliberately NOT a MessagePack ext type
/// and NOT a tuple:
///
/// * An ext type cannot be read back at all. `PropertyValue` is untagged, so
///   serde buffers through `Content` via `deserialize_any`, while rmp-serde's
///   ext handling lives in `deserialize_newtype_struct` — which an untagged
///   enum never calls. Measured: a hand-built ext value gives "data did not
///   match any variant of untagged enum PropertyValue".
/// * A one-element tuple `["19.90"]` is what `Date` did, and it is why
///   `days_of_week: [3]` was read as three nanoseconds. A single-element array
///   is also just an array; this variant sits ahead of `Array`, so it would
///   claim genuine one-string arrays for exactly the same reason.
///
/// A map survives `deserialize_any` buffering intact, reads identically in both
/// formats (so nothing has to consult `is_human_readable`, which the untagged
/// `ContentDeserializer` reports as true even for MessagePack), and travels
/// through every transport — including replication, which re-encodes with
/// `to_vec_named` and would have dropped an ext type.
pub fn serialize_tagged_decimal<S>(
    value: &rust_decimal::Decimal,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    // BINARY ONLY. `is_human_readable()` is false for MessagePack and true for
    // JSON, and it is reliable HERE because serialization goes straight to the
    // real serializer — it is only the untagged DEserializer that cannot see it
    // (serde buffers through `Content`, which reports human-readable always).
    //
    // The tag is a STORAGE concern. Storage is MessagePack, and that is the only
    // place the untagged ladder ever reads a value back, so it is the only place
    // the ambiguity can bite. JSON is a rendering surface: it keeps emitting a
    // bare string, which is the API contract every consumer already implements
    // by hand (`PropertyValue::Decimal(d) => d.to_string()`, in 14 places), and
    // JSON INPUT never reaches the ladder because `PropertyValue::from_json`
    // converts scalars without guessing. Tagging JSON would have changed what
    // every API client sees — caught by the end-to-end test, which read back
    // `{"raisin:string": "76133"}` instead of `"76133"`.
    if serializer.is_human_readable() {
        return serializer.serialize_str(&value.to_string());
    }
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(1))?;
    map.serialize_entry(DECIMAL_TAG, &value.to_string())?;
    map.end()
}

/// The key that tags a STRING whose spelling a decimal would steal.
///
/// Tagging `Decimal` alone does not fix the defect, and this is the subtlety
/// that matters: a `String` still serializes as a bare str, and a bare str is
/// still ambiguous, so `String("76133")` written today would come back a
/// `Decimal` tomorrow. Only the AMBIGUOUS strings are tagged — the ones a
/// decimal would claim — so `"hello"` stays a plain str and nothing bloats.
pub const STRING_TAG: &str = "raisin:string";

/// Would the legacy bare-string rule read this back as a decimal?
///
/// Exactly the test in [`deserialize_tagged_decimal`]'s `visit_str`, so the two
/// cannot drift: a string is at risk precisely when a decimal parsed from it
/// spells itself back identically.
pub fn string_is_decimal_ambiguous(raw: &str) -> bool {
    raw.parse::<rust_decimal::Decimal>()
        .is_ok_and(|d| d.to_string() == raw)
}

/// Write a string, tagging it only when a decimal would otherwise steal it.
pub fn serialize_guarded_string<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    // Bare in JSON (a rendering surface, and its input never reaches the
    // ladder), tagged only in the binary form that storage actually uses — see
    // `serialize_tagged_decimal` for why the flag is trustworthy here.
    if serializer.is_human_readable() || !string_is_decimal_ambiguous(value) {
        return serializer.serialize_str(value);
    }
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(1))?;
    map.serialize_entry(STRING_TAG, value)?;
    map.end()
}

/// Read a string in either form: a bare str, or the tag above.
///
/// Strict for the same reason the decimal tag is: exactly one entry, exactly
/// this key, value a string. Anything else belongs to `Object`.
pub fn deserialize_guarded_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{Error as _, MapAccess, Visitor};
    use std::fmt;

    struct StringVisitor;

    impl<'de> Visitor<'de> for StringVisitor {
        type Value = String;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "a string, or a {STRING_TAG} map")
        }

        fn visit_str<E: serde::de::Error>(self, raw: &str) -> Result<Self::Value, E> {
            Ok(raw.to_string())
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let Some(key) = map.next_key::<String>()? else {
                return Err(M::Error::custom("empty map is not a string"));
            };
            if key != STRING_TAG {
                return Err(M::Error::custom("not a string tag"));
            }
            let raw: String = map.next_value()?;
            if map.next_key::<String>()?.is_some() {
                return Err(M::Error::custom("string tag must be the only key"));
            }
            Ok(raw)
        }
    }

    deserializer.deserialize_any(StringVisitor)
}

/// Read a decimal in either form: the tagged map, or a legacy bare string.
///
/// TAGGED (`{"raisin:decimal": "19.90"}`) is what everything written from now
/// on looks like, and it is unambiguous.
///
/// LEGACY (a bare `"19.90"`) is every value written before the tag existed.
/// Those bytes are genuinely ambiguous — a decimal and a string were spelled
/// identically — so the only honest reading is the losslessness heuristic:
/// accept the variant ONLY if the decimal renders back to exactly the string it
/// came from. `"19.90"` does and is read as a decimal; `"05"` renders as `5`,
/// `"1e3"` as `1000`, `"+7"` as `7`, and anything past 96 bits does not parse,
/// so each of those falls through to `String` and keeps its spelling. That is
/// today's behaviour, preserved exactly, so no stored value changes meaning.
///
/// STRICTNESS IS THE WHOLE RISK OF THE MAP FORM. This accepts a map ONLY when
/// it has exactly one entry, whose key is exactly [`DECIMAL_TAG`] and whose
/// value is a string. Anything looser — the key among others, or any
/// single-key map — would start eating ordinary objects that must reach the
/// `Object` fallback, trading one silent misclassification for another.
pub fn deserialize_tagged_decimal<'de, D>(
    deserializer: D,
) -> Result<rust_decimal::Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{Error as _, MapAccess, Visitor};
    use std::fmt;

    struct DecimalVisitor;

    impl<'de> Visitor<'de> for DecimalVisitor {
        type Value = rust_decimal::Decimal;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(
                f,
                "a {DECIMAL_TAG} map, or a decimal spelled exactly as it parses"
            )
        }

        /// The legacy form. See the losslessness rule above.
        fn visit_str<E: serde::de::Error>(self, raw: &str) -> Result<Self::Value, E> {
            let parsed: rust_decimal::Decimal =
                raw.parse().map_err(|_| E::custom("not a decimal"))?;
            if parsed.to_string() != raw {
                // Not a refusal of the VALUE — a refusal of this VARIANT, so the
                // untagged ladder moves on and the string stays a string.
                return Err(E::custom("decimal does not round-trip to its own spelling"));
            }
            Ok(parsed)
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let Some(key) = map.next_key::<String>()? else {
                return Err(M::Error::custom("empty map is not a decimal"));
            };
            if key != DECIMAL_TAG {
                return Err(M::Error::custom("not a decimal tag"));
            }
            let raw: String = map.next_value()?;
            // EXACTLY one entry: a second key means this is somebody's object
            // that happens to start with our tag, and it belongs to `Object`.
            if map.next_key::<String>()?.is_some() {
                return Err(M::Error::custom("decimal tag must be the only key"));
            }
            raw.parse()
                .map_err(|_| M::Error::custom("tagged value is not a decimal"))
        }
    }

    deserializer.deserialize_any(DecimalVisitor)
}
