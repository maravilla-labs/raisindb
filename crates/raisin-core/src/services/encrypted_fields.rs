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

//! Which fields of a NodeType are secrets — the gate that keeps auto-vaulting
//! off the hot path.
//!
//! # Why this exists
//!
//! Every node write has to answer "does this node carry a field declared
//! `encrypted: true`?". Resolving the NodeType to answer it would put a schema
//! read on every write. Virtually every node in a repository has no secret
//! fields at all, so the answer is almost always "no" and should cost a map
//! lookup.
//!
//! This cache stores that answer per `{tenant}:{repo}:{branch}:{node_type}`.
//!
//! # A stale `false` is a plaintext leak
//!
//! This is not an ordinary cache and must not be treated like one. The two
//! staleness directions are wildly asymmetric:
//!
//! - stale `true`  — one wasted property walk. Harmless.
//! - stale `false` — the field is **written to disk in plaintext**, silently,
//!   and no later fix recovers it: the value is already in the node blob, in the
//!   property index, in the audit log and on every replica.
//!
//! Two consequences, both load-bearing:
//!
//! 1. **No TTL.** A TTL would be a bounded plaintext-leak window, which is not a
//!    thing worth having. Entries live until something invalidates them.
//! 2. **Unknown means "walk".** Any doubt — cache miss, resolution failure,
//!    unknown type — must resolve to [`EncryptedFields::Unknown`], and callers
//!    must treat that as "assume secrets, fail closed". See
//!    [`EncryptedFields::must_examine`].
//!
//! # Invalidation, twice
//!
//! Schema reaches the database two ways, so it is dropped two ways:
//!
//! - **`Event::Schema`** — ordinary NodeType/Archetype/ElementType writes, local
//!   or replicated. Precise and immediate. (Local schema writes only started
//!   publishing this event recently; the regression test
//!   `schema_event_local_publish_test` guards it. Without that, a
//!   locally-authored nodetype would never invalidate this gate.)
//! - **`derived_cache_registry`** — replication checkpoint ingestion copies whole
//!   column families in and deliberately emits no events, so it calls
//!   `invalidate_all_derived_caches()` instead. A NodeType arriving that way
//!   would otherwise leave a stale `false` behind.
//!
//! Miss either and the failure is silent and unrecoverable, which is why both
//! are wired here rather than left to callers.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use raisin_models::nodes::properties::schema::{is_secret, PropertyValueSchema};

/// What a NodeType declares about secret fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncryptedFields {
    /// The type declares no `encrypted` field anywhere. The write path can skip
    /// vaulting entirely — no walk, no allocation.
    None,
    /// The type declares secret fields at these top-level property names.
    ///
    /// Nested paths (inside an `Element`, `Object`, `Array` or `Composite`) are
    /// not enumerable from the NodeType alone, because the enclosing
    /// element type decides them. [`EncryptedFields::needs_deep_walk`] reports
    /// whether such a field might exist.
    Some {
        /// Top-level property names declared `encrypted: true`.
        top_level: Vec<String>,
        /// Whether any property could contain a nested secret, requiring a full
        /// property-tree walk rather than direct lookups.
        deep: bool,
    },
    /// The answer is not known — the type could not be resolved, or the cache
    /// has never been populated for it.
    ///
    /// **Callers must fail closed on this**, never treat it as `None`.
    Unknown,
}

impl EncryptedFields {
    /// Whether the write path must look at this node's properties at all.
    ///
    /// `Unknown` answers `true`: not knowing is not the same as knowing there
    /// is nothing, and the cost of being wrong in that direction is a plaintext
    /// secret on disk.
    pub fn must_examine(&self) -> bool {
        !matches!(self, EncryptedFields::None)
    }

    /// Whether a full property-tree walk is required, as opposed to direct
    /// lookups of the known top-level names.
    pub fn needs_deep_walk(&self) -> bool {
        match self {
            EncryptedFields::None => false,
            EncryptedFields::Some { deep, .. } => *deep,
            // Unknown: we cannot rule out a nested secret, so walk.
            EncryptedFields::Unknown => true,
        }
    }

    /// Derive the answer from a resolved NodeType's properties.
    pub fn from_resolved_properties(properties: &[PropertyValueSchema]) -> Self {
        let mut top_level = Vec::new();
        let mut deep = false;

        for prop in properties {
            if is_secret(prop) {
                if let Some(name) = &prop.name {
                    top_level.push(name.clone());
                }
                continue;
            }
            // A container property may hold a secret further down. We cannot
            // decide that here — the element type does — so flag the need for a
            // walk and let the walker consult the schema per leaf.
            if property_may_nest(prop) {
                deep = true;
            }
        }

        if top_level.is_empty() && !deep {
            EncryptedFields::None
        } else {
            EncryptedFields::Some { top_level, deep }
        }
    }
}

/// Whether a property can contain sub-properties that might be secrets.
fn property_may_nest(prop: &PropertyValueSchema) -> bool {
    use raisin_models::nodes::properties::schema::PropertyType;
    matches!(
        prop.property_type,
        PropertyType::Object
            | PropertyType::Array
            | PropertyType::Element
            | PropertyType::Composite
    )
}

/// Process-wide cache of [`EncryptedFields`], keyed
/// `"{tenant}:{repo}:{branch}:{node_type}"`.
///
/// Deliberately **not** TTL-based — see the module docs.
pub struct EncryptedFieldsCache {
    entries: RwLock<HashMap<String, EncryptedFields>>,
}

impl Default for EncryptedFieldsCache {
    fn default() -> Self {
        Self::new()
    }
}

impl EncryptedFieldsCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Build the cache key for a type in a branch scope.
    pub fn key(tenant_id: &str, repo_id: &str, branch: &str, node_type: &str) -> String {
        format!("{tenant_id}:{repo_id}:{branch}:{node_type}")
    }

    /// Look up a cached answer.
    ///
    /// Returns [`EncryptedFields::Unknown`] on a miss **and** if the lock is
    /// poisoned — both are "we do not know", and the caller's fail-closed path
    /// is the correct response to either.
    pub fn get(&self, key: &str) -> EncryptedFields {
        match self.entries.read() {
            Ok(map) => map.get(key).cloned().unwrap_or(EncryptedFields::Unknown),
            Err(_) => EncryptedFields::Unknown,
        }
    }

    /// Record an answer. Storing [`EncryptedFields::Unknown`] is a no-op: it
    /// carries no information and would only mask a later real answer.
    pub fn put(&self, key: String, value: EncryptedFields) {
        if matches!(value, EncryptedFields::Unknown) {
            return;
        }
        if let Ok(mut map) = self.entries.write() {
            map.insert(key, value);
        }
    }

    /// Drop every entry for one branch scope, on `Event::Schema`.
    ///
    /// The trailing `:` is load-bearing: without it, invalidating branch `main`
    /// would leave `main-backup` and `main2` untouched only by luck, and would
    /// wrongly drop nothing for a branch that merely prefixes another.
    pub fn invalidate_scope(&self, tenant_id: &str, repo_id: &str, branch: &str) {
        let prefix = format!("{tenant_id}:{repo_id}:{branch}:");
        if let Ok(mut map) = self.entries.write() {
            map.retain(|k, _| !k.starts_with(&prefix));
        }
    }

    /// Drop everything. Used by the derived-cache registry after a bulk path
    /// that wrote schema without emitting events.
    pub fn invalidate_all(&self) {
        if let Ok(mut map) = self.entries.write() {
            map.clear();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.read().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Shared handle, as the other caches in this module are shared.
pub type SharedEncryptedFieldsCache = Arc<EncryptedFieldsCache>;

/// The process-wide instance, registered with the derived-cache registry on
/// first use.
///
/// Registration happens in the initializer so it cannot get out of order with
/// population — the registry's own documented contract.
pub fn global() -> &'static SharedEncryptedFieldsCache {
    static CACHE: std::sync::OnceLock<SharedEncryptedFieldsCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let cache: SharedEncryptedFieldsCache = Arc::new(EncryptedFieldsCache::new());
        let for_registry = cache.clone();
        crate::services::derived_cache_registry::register_invalidator(move || {
            for_registry.invalidate_all();
        });
        cache
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::schema::PropertyType;

    fn prop(name: &str, ty: PropertyType, encrypted: Option<bool>) -> PropertyValueSchema {
        let yaml = format!("name: {name}\ntype: String\n");
        let mut p: PropertyValueSchema = serde_yaml::from_str(&yaml).unwrap();
        p.property_type = ty;
        p.encrypted = encrypted;
        p
    }

    #[test]
    fn a_type_with_no_secret_fields_short_circuits() {
        let props = vec![
            prop("title", PropertyType::String, None),
            prop("count", PropertyType::Number, None),
        ];
        let ef = EncryptedFields::from_resolved_properties(&props);
        assert_eq!(ef, EncryptedFields::None);
        assert!(!ef.must_examine(), "the fast path must skip the walk");
        assert!(!ef.needs_deep_walk());
    }

    #[test]
    fn a_top_level_secret_is_listed_without_requiring_a_walk() {
        let props = vec![
            prop("host", PropertyType::String, None),
            prop("password", PropertyType::String, Some(true)),
        ];
        let ef = EncryptedFields::from_resolved_properties(&props);
        assert_eq!(
            ef,
            EncryptedFields::Some {
                top_level: vec!["password".to_string()],
                deep: false
            }
        );
        assert!(ef.must_examine());
        assert!(!ef.needs_deep_walk(), "direct lookups suffice");
    }

    /// A container might hold a secret that the NodeType alone cannot enumerate,
    /// because the enclosing element type decides it.
    #[test]
    fn a_container_property_forces_a_deep_walk() {
        let props = vec![prop("hero", PropertyType::Element, None)];
        let ef = EncryptedFields::from_resolved_properties(&props);
        assert!(ef.must_examine());
        assert!(ef.needs_deep_walk());
    }

    /// The whole point of the type. Unknown must never behave like None.
    #[test]
    fn unknown_fails_closed() {
        let ef = EncryptedFields::Unknown;
        assert!(
            ef.must_examine(),
            "not knowing must never skip vaulting - that writes plaintext"
        );
        assert!(ef.needs_deep_walk());
    }

    #[test]
    fn a_miss_reads_as_unknown_not_none() {
        let cache = EncryptedFieldsCache::new();
        assert_eq!(cache.get("t:r:main:Absent"), EncryptedFields::Unknown);
    }

    #[test]
    fn storing_unknown_is_a_no_op() {
        let cache = EncryptedFieldsCache::new();
        cache.put("t:r:main:X".into(), EncryptedFields::Unknown);
        assert!(cache.is_empty(), "Unknown carries no information to cache");
    }

    #[test]
    fn round_trips_a_real_answer() {
        let cache = EncryptedFieldsCache::new();
        let key = EncryptedFieldsCache::key("t", "r", "main", "imap:Config");
        cache.put(key.clone(), EncryptedFields::None);
        assert_eq!(cache.get(&key), EncryptedFields::None);
    }

    /// A branch whose name prefixes another must not drop the other's entries.
    #[test]
    fn scope_invalidation_respects_branch_boundaries() {
        let cache = EncryptedFieldsCache::new();
        let main = EncryptedFieldsCache::key("t", "r", "main", "A");
        let main_backup = EncryptedFieldsCache::key("t", "r", "main-backup", "A");
        let other_repo = EncryptedFieldsCache::key("t", "r2", "main", "A");
        for k in [&main, &main_backup, &other_repo] {
            cache.put(k.clone(), EncryptedFields::None);
        }

        cache.invalidate_scope("t", "r", "main");

        assert_eq!(cache.get(&main), EncryptedFields::Unknown, "dropped");
        assert_eq!(
            cache.get(&main_backup),
            EncryptedFields::None,
            "a branch that merely shares a prefix must survive"
        );
        assert_eq!(cache.get(&other_repo), EncryptedFields::None);
    }

    #[test]
    fn invalidate_all_clears_every_scope() {
        let cache = EncryptedFieldsCache::new();
        cache.put(
            EncryptedFieldsCache::key("t", "r", "main", "A"),
            EncryptedFields::None,
        );
        cache.put(
            EncryptedFieldsCache::key("t2", "r2", "dev", "B"),
            EncryptedFields::None,
        );
        cache.invalidate_all();
        assert!(cache.is_empty());
    }

    /// The legacy `meta.secret` spelling reaches this gate through `is_secret`,
    /// so a shipped connector schema is not silently treated as non-secret.
    #[test]
    fn the_legacy_meta_spelling_still_reaches_the_gate() {
        let p: PropertyValueSchema =
            serde_yaml::from_str("name: password\ntype: String\nmeta:\n  secret: true\n").unwrap();
        assert_eq!(
            EncryptedFields::from_resolved_properties(&[p]),
            EncryptedFields::Some {
                top_level: vec!["password".to_string()],
                deep: false
            }
        );
    }
}
