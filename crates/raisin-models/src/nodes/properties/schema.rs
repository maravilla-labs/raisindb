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

// PropertyValueSchema, PropertyType, and validation functions

use crate::nodes::properties::spatial_policy::SpatialPropertySchema;
use crate::nodes::properties::value::PropertyValue;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Index types available for properties and node types
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
pub enum IndexType {
    /// Tantivy full-text search with language support and stemming
    Fulltext,
    /// HNSW vector embeddings for AI-powered semantic search
    Vector,
    /// RocksDB property_index CF for exact-match lookups
    Property,
    /// RocksDB spatial_index CF for geohash-cell proximity lookups.
    ///
    /// **Documentation only.** Spatial indexing is driven by the runtime property
    /// *type*: any `PropertyValue::Geometry` is indexed with no opt-in, and
    /// listing (or omitting) `Spatial` here does not change that. The entry
    /// exists for symmetry with the other index types and so a schema can state
    /// the intent; configure the index itself via
    /// [`PropertyValueSchema::spatial`].
    Spatial,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct PropertyValueSchema {
    #[validate(regex(path = "*crate::nodes::properties::utils::URL_FRIENDLY_NAME_REGEX"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type", alias = "property_type")]
    pub property_type: PropertyType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<PropertyValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<HashMap<String, PropertyValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<HashMap<String, PropertyValueSchema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<PropertyValueSchema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<PropertyValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, PropertyValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_translatable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[validate(custom = "validate_allow_additional_properties")]
    pub allow_additional_properties: Option<bool>,
    /// Which indexes this property should be included in
    /// Default: None (property is not indexed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<Vec<IndexType>>,
    /// Spatial indexing configuration for a `Geometry`-valued property.
    ///
    /// This tunes an index that already exists rather than enabling one: geometry
    /// properties are indexed automatically by value type. `None` inherits the
    /// workspace defaults and then the server constants — see
    /// [`resolve_spatial_policy`](crate::nodes::properties::spatial_policy::resolve_spatial_policy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial: Option<SpatialPropertySchema>,
    /// The value is a secret: it is moved into the secret store on write and the
    /// stored property holds a `secret://…` reference instead of the plaintext.
    ///
    /// This is enforced by the server at the write layer, so no transport can
    /// bypass it. Reads return the reference; they never resolve it.
    ///
    /// A first-class field rather than a `meta` key because `meta` is
    /// free-form and uninterpreted, and this one changes what gets stored.
    /// The legacy `meta.secret: true` spelling is still honoured on read — see
    /// [`is_secret`] — so already-shipped schemas keep working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted: Option<bool>,
}

/// Whether a truthy boolean `meta.<key>` flag is present.
///
/// THE one reader of a `meta` boolean flag. There were three — this function,
/// [`crate::nodes::types::element::fields::base_field::is_secret`] and
/// `integrations::config::meta_flag` — each re-deciding whether the string
/// `"true"` counts (it does not). For an ordinary UI hint a disagreement is
/// cosmetic; for `meta.secret` one reader saying "no" writes the value to disk
/// in plaintext, so they are collapsed into this.
///
/// Takes the map rather than a schema so both shapes that carry `meta`
/// ([`PropertyValueSchema`] and `FieldTypeSchema`) feed the same code.
pub fn meta_bool(meta: Option<&HashMap<String, PropertyValue>>, key: &str) -> bool {
    matches!(
        meta.and_then(|m| m.get(key)),
        Some(PropertyValue::Boolean(true))
    )
}

/// Whether a property schema declares its value a secret.
///
/// Checks the first-class [`PropertyValueSchema::encrypted`] field, then falls
/// back to the legacy `meta.secret: true` convention that shipped connector
/// schemas still use. One reader, so the two spellings cannot drift.
pub fn is_secret(schema: &PropertyValueSchema) -> bool {
    if let Some(flag) = schema.encrypted {
        return flag;
    }
    meta_bool(schema.meta.as_ref(), "secret")
}

/// Whether a secret field's NAME can be addressed unambiguously.
///
/// The property walker's path format joins segments with `.` and has no
/// escaping, so a property literally named `a.b` is indistinguishable from key
/// `b` inside object `a`. For an index entry that ambiguity is a known,
/// tolerated limitation. For a SECRET it is not: the vault path becomes the
/// secret's storage name, so two different fields could collide on one secret,
/// or a rewrite could land on the wrong leaf — either way a value ends up
/// somewhere its author did not intend.
///
/// Enforced at the write layer, where it refuses the write, rather than being
/// silently tolerated.
pub fn secret_name_is_addressable(name: &str) -> bool {
    !name.contains('.')
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub enum PropertyType {
    #[serde(alias = "string")]
    String,
    #[serde(alias = "number")]
    Number,
    #[serde(alias = "boolean")]
    Boolean,
    #[serde(alias = "array")]
    Array,
    #[serde(alias = "object")]
    Object,
    #[serde(alias = "date")]
    Date,
    #[serde(alias = "url")]
    URL,
    #[serde(alias = "reference")]
    Reference,
    #[serde(alias = "nodetype", alias = "nodeType")]
    NodeType,
    #[serde(alias = "element")]
    Element,
    #[serde(alias = "composite")]
    Composite,
    #[serde(alias = "resource")]
    Resource,
    #[serde(alias = "geometry")]
    Geometry,
}

/// Compound index definition for efficient multi-column queries.
///
/// A compound index combines multiple property columns into a single index key,
/// enabling efficient queries that filter on multiple columns and/or need ordered results.
///
/// Example: An index on `(node_type, category, created_at DESC)` enables queries like:
/// ```sql
/// SELECT * FROM content
/// WHERE node_type = 'news:Article' AND properties->>'category' = 'business'
/// ORDER BY created_at DESC
/// LIMIT 10
/// ```
/// to execute in O(LIMIT) time instead of scanning all matching nodes.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
pub struct CompoundIndexDefinition {
    /// Unique name for this index (used in key encoding)
    pub name: String,

    /// Columns in order (leading columns for equality, trailing for ordering)
    pub columns: Vec<CompoundIndexColumn>,

    /// If true, the last column is used for ordering (created_at, updated_at)
    #[serde(default)]
    pub has_order_column: bool,
}

/// A column in a compound index definition.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
pub struct CompoundIndexColumn {
    /// Property name.
    /// Use system property names for node metadata:
    /// - `__node_type` for node_type
    /// - `__created_at` for created_at
    /// - `__updated_at` for updated_at
    ///   Use regular property names (e.g., `category`, `status`) for JSON properties.
    pub property: String,

    /// For ordering columns: sort direction (true = ASC, false = DESC).
    /// Only applicable when this is the last column and `has_order_column` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ascending: Option<bool>,

    /// Column type hint for proper key encoding.
    /// Timestamps need special encoding for sortable keys.
    pub column_type: CompoundColumnType,
}

impl CompoundIndexDefinition {
    /// Bump when the meaning of a declaration changes such that entries written
    /// under the old reading are no longer valid, even for an identical
    /// `columns` list.
    pub const NORMALIZER_VERSION: u32 = 1;

    /// A stable fingerprint of everything that determines the KEY BYTES this
    /// index produces.
    ///
    /// Persisted alongside the built entries so a changed declaration is
    /// detectable. It has to be, because an index name addresses a
    /// workspace-global keyspace: change a column and the new entries land in
    /// the SAME keyspace as the old ones, interleaved and mutually
    /// unintelligible. Comparing this hash is what lets the planner answer
    /// "stale" instead of quietly reading both.
    ///
    /// FNV-1a over a canonical encoding, hand-rolled for the same reason
    /// `SpatialPolicy::policy_hash` is: `DefaultHasher` is explicitly NOT
    /// stable across std versions, and this value outlives the process that
    /// wrote it.
    ///
    /// Column ORDER is part of the identity — `(status, created_at)` and
    /// `(created_at, status)` are different indexes — so the columns are hashed
    /// in sequence, never as a set.
    pub fn definition_hash(&self) -> u64 {
        let mut h = crate::nodes::properties::spatial_policy::Fnv::new();
        h.write_u32(Self::NORMALIZER_VERSION);
        h.write_bytes(self.name.as_bytes());
        h.write_u32(u32::from(self.has_order_column));
        h.write_u32(self.columns.len() as u32);
        for column in &self.columns {
            h.write_bytes(column.property.as_bytes());
            // Length-prefix-free encodings let `("ab","c")` collide with
            // `("a","bc")`; the discriminator below plus the per-column type
            // keeps the stream unambiguous.
            h.write_u32(column.property.len() as u32);
            h.write_u32(match column.column_type {
                CompoundColumnType::String => 0,
                CompoundColumnType::Integer => 1,
                CompoundColumnType::Timestamp => 2,
                CompoundColumnType::Boolean => 3,
            });
            // `Option<bool>`: absent and present-false are different
            // declarations, so they must not hash alike.
            h.write_u32(match column.ascending {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            });
        }
        h.finish()
    }
}

/// Type hint for compound index column encoding.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, JsonSchema)]
pub enum CompoundColumnType {
    /// String values (node_type, category, etc.)
    String,
    /// Integer values
    Integer,
    /// Timestamp values (created_at, updated_at) - encoded for sortability
    Timestamp,
    /// Boolean values
    Boolean,
}

#[cfg(test)]
mod secret_flag_tests {
    use super::*;

    fn from_yaml(yaml: &str) -> PropertyValueSchema {
        serde_yaml::from_str(yaml).expect("schema should deserialize")
    }

    /// The first-class spelling, which is what the server enforces.
    #[test]
    fn encrypted_true_marks_the_property_secret() {
        assert!(is_secret(&from_yaml(
            "name: password\ntype: String\nencrypted: true\n"
        )));
    }

    /// Shipped connector schemas (e.g. the imap-adapter's `password` field) use
    /// `meta.secret`, and must keep working — the `meta` map is the only place a
    /// hint could live before `encrypted` existed.
    #[test]
    fn legacy_meta_secret_is_still_honoured() {
        assert!(is_secret(&from_yaml(
            "name: password\ntype: String\nmeta:\n  secret: true\n"
        )));
    }

    /// The explicit field wins, in BOTH directions — including `encrypted:
    /// false` overriding a stale `meta.secret: true`, which is the only way to
    /// un-mark a field without editing the meta bag.
    #[test]
    fn the_first_class_field_takes_precedence_over_meta() {
        assert!(is_secret(&from_yaml(
            "name: p\ntype: String\nencrypted: true\nmeta:\n  secret: false\n"
        )));
        assert!(!is_secret(&from_yaml(
            "name: p\ntype: String\nencrypted: false\nmeta:\n  secret: true\n"
        )));
    }

    /// Absent means not secret. Stated as a test because the fail-closed rule
    /// applies to schema *resolution* failures, not to a schema that resolved
    /// fine and simply declares nothing.
    #[test]
    fn absent_flag_is_not_secret() {
        assert!(!is_secret(&from_yaml("name: title\ntype: String\n")));
        assert!(!is_secret(&from_yaml(
            "name: title\ntype: String\nmeta:\n  label: Title\n"
        )));
    }

    /// `meta.secret` must be a real boolean — the string "true" is not a flag.
    /// Mirrors `meta_flag`'s behaviour so the two readers cannot disagree.
    #[test]
    fn a_stringy_meta_secret_does_not_count() {
        assert!(!is_secret(&from_yaml(
            "name: p\ntype: String\nmeta:\n  secret: \"true\"\n"
        )));
    }

    /// `encrypted` must survive a YAML round trip. `PropertyValueSchema`
    /// silently DROPS unknown top-level keys (which is why hints live in
    /// `meta`), so a field that failed to deserialize would read as "not
    /// secret" and store the value in plaintext, with no error anywhere.
    #[test]
    fn encrypted_survives_a_round_trip() {
        let schema = from_yaml("name: password\ntype: String\nencrypted: true\n");
        let round_tripped: PropertyValueSchema =
            serde_yaml::from_str(&serde_yaml::to_string(&schema).unwrap()).unwrap();
        assert_eq!(round_tripped.encrypted, Some(true));
        assert!(is_secret(&round_tripped));
    }
}

#[cfg(test)]
mod compound_index_shape_tests {
    use super::*;

    /// The shorthand two skill files documented — `columns: ["a", "b"]` — does
    /// not deserialize, and the failure takes the WHOLE NodeType down, not just
    /// the index.
    ///
    /// Pinned here because the docs teach the YAML and the struct is what
    /// enforces it: `CompoundIndexColumn` is a struct with a REQUIRED
    /// `column_type` (no `#[serde(default)]`, no untagged/from-string variant),
    /// so a bare string is `invalid type: string, expected struct
    /// CompoundIndexColumn` — the same class of trap as the designer-format
    /// `TemplatableNumber` bug.
    #[test]
    fn a_bare_string_column_is_not_a_compound_column() {
        let err = serde_yaml::from_str::<CompoundIndexDefinition>(
            r#"
name: folder_time
columns: ["__parent_path", "__created_at"]
has_order_column: true
"#,
        )
        .expect_err("the string shorthand must not silently parse");
        assert!(
            err.to_string().contains("CompoundIndexColumn"),
            "unexpected error: {err}"
        );
    }

    /// The form the docs must teach: every column an object with an explicit
    /// `column_type`, spelled exactly as the enum variant.
    #[test]
    fn the_object_form_with_an_explicit_column_type_parses() {
        let def: CompoundIndexDefinition = serde_yaml::from_str(
            r#"
name: mail_folder_recent
columns:
  - property: __parent_path
    column_type: String
  - property: __created_at
    column_type: Timestamp
has_order_column: true
"#,
        )
        .expect("the object form is the supported shape");
        assert_eq!(def.columns.len(), 2);
        assert_eq!(def.columns[1].column_type, CompoundColumnType::Timestamp);
        assert!(def.has_order_column);
    }

    /// `column_type` has no default: omitting it is an error, not a String
    /// column. Worth pinning separately — a `#[serde(default)]` added later
    /// would make every mistyped column silently a String, which the writer
    /// then refuses to index (it requires (String, PropertyValue::String)) and
    /// the node drops out of the index with only a debug log.
    #[test]
    fn column_type_is_required() {
        assert!(serde_yaml::from_str::<CompoundIndexDefinition>(
            r#"
name: x
columns:
  - property: status
has_order_column: false
"#,
        )
        .is_err());
    }
}
