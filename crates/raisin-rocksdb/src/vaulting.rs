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

//! Auto-vaulting: a plaintext value written into a field declared
//! `encrypted: true` is sealed into the secret store, and the property is
//! replaced by the `secret://name@version` reference that stands in for it.
//!
//! # One body, every write path
//!
//! This crate has FOUR low-level node write functions — the transaction's
//! `add_node` / `put_node` and the repository's `add_impl` / `update_impl` —
//! and authorship stamping already had to be taught to all four (see CLAUDE.md,
//! "Node Revision History & Authorship"). Vaulting has the same reach and a
//! much sharper failure mode, so it lives here, at crate level, and all four
//! call [`Vaulter::vault`]. A per-layer copy is how one of them ends up not
//! vaulting, and the symptom of that is a plaintext password on disk.
//!
//! `update_property_by_path_impl` (the `queries/property.rs` single-property
//! write) needs no separate hook: it reads the node, sets one property and
//! funnels into `update_impl`.
//!
//! # Where it runs, and why exactly there
//!
//! Between the schema validator and the read cache / storage write. Every
//! neighbour of that slot is load-bearing:
//!
//! - **After validation** — the validator still sees PLAINTEXT, so its
//!   length/pattern/required constraints mean something. Run it after vaulting
//!   and every rule would be validating the string `secret://…`.
//! - **Before the read cache** — or read-your-writes inside the same
//!   transaction hands the plaintext back.
//! - **Before the node blob is written** — that is the durable record and
//!   every replica's copy of it.
//! - **Before the index writers** (property, unique, compound). This is the
//!   highest-severity one: an index entry keys on the VALUE, so a plaintext
//!   password indexed once makes `properties->>'password'::String = '<guess>'`
//!   a working oracle — permanently, because index entries carry the revision
//!   and are never rewritten by a later fix.
//!
//! # Why there is no LOCAL-write gate
//!
//! An arriving replicated node is already vaulted (its properties hold
//! references, which this module leaves untouched), and in any case the
//! replication apply path does not come through here at all:
//! `replication/application/node_operations/snapshot_ops.rs` calls
//! `apply_replicated_upsert_with_event` directly, bypassing both write layers.
//! So there is nothing to gate — a replica neither re-vaults nor needs to.
//!
//! # Fail closed
//!
//! `transaction/.../create/coercion.rs` — the sibling this module is otherwise
//! modelled on — swallows a schema-resolution failure and carries on
//! (`Err(_) => return Ok(())`). That is right for a geometry: the worst case is
//! an unindexed shape a rebuild recovers. It is WRONG here, and copying it
//! would write plaintext to disk. Not knowing whether a field is a secret must
//! refuse the write.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use raisin_core::services::element_type_resolver::ElementTypeResolver;
use raisin_core::services::encrypted_fields::{self, EncryptedFields, EncryptedFieldsCache};
use raisin_core::services::node_type_resolver::NodeTypeResolver;
use raisin_core::services::schema_lookup::{ElementTypeLookup, NodeTypeLookup};
use raisin_error::Result;
use raisin_hlc::NodeHLCState;
use raisin_models::nodes::properties::schema::secret_name_is_addressable;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::element::field_types::FieldSchemaBase;
use raisin_models::nodes::types::element::fields::base_field::is_secret as field_is_secret;
use raisin_models::nodes::Node;
use raisin_models::secret_ref::{format_secret_ref, node_field_secret_name, SecretRef};
use rocksdb::DB;

use crate::indexing::property_walk::{walk_properties, walk_properties_mut, WalkCursor};
use crate::secret_store::{SecretOwner, SecretScope, SecretStore};
use crate::RocksDBStorage;

/// Upper bound on vaulted fields per node, mirroring
/// `MAX_GEOMETRY_PATHS_PER_NODE` in `indexing/spatial_walk.rs`.
///
/// The geometry cap warns and DROPS the overflow, which is fine for an index —
/// the shape is still stored, just unqueryable. Here a dropped field is a
/// PLAINTEXT SECRET on disk, so overflow is an error and the write is refused.
pub(crate) const MAX_SECRET_PATHS_PER_NODE: usize = 64;

/// Secret versions minted within one logical write: name -> (version, hash of
/// the plaintext that version holds).
///
/// See `RocksDBTransaction::vaulted_secrets` for why a transaction carries one
/// and why the hash is part of it. The repository path passes `None`: two
/// repository calls are two logical writes with two revisions, so each SHOULD
/// mint its own version.
pub(crate) type VaultMemo = Mutex<HashMap<String, (u64, [u8; 32])>>;

/// Who and where a write is happening.
pub(crate) struct VaultScope<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    /// Resolved actor, recorded as the secret's `created_by`.
    pub actor: &'a str,
}

/// Everything vaulting needs, independent of which write layer is calling.
///
/// Held by `NodeRepositoryImpl` (the one component both write layers can
/// reach: the transaction has it as `tx.node_repo`), so the schema lookups and
/// the secret-store slot are shared rather than rebuilt per call.
#[derive(Clone)]
pub(crate) struct Vaulter {
    node_types: Arc<dyn NodeTypeLookup>,
    element_types: Arc<dyn ElementTypeLookup>,
    /// Shared with `RocksDBStorage::secret_store()`, so a store installed there
    /// (tests, or a server supplying its own keyring) is the one used here.
    secret_store: Arc<OnceLock<Arc<SecretStore>>>,
    db: Arc<DB>,
    hlc: Arc<NodeHLCState>,
}

/// Whether auto-vaulting is active. Defaults to ON; only an explicit
/// `[secrets] vaulting_enabled = false` turns it off.
static VAULTING_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Install the vaulting policy process-wide, once, at startup.
///
/// Process-wide rather than threaded through every call site for the same
/// reason `configure_mcp_client` is: the write paths that need it reach far
/// enough down the stack that plumbing a config handle to each would mean
/// several near-identical parameters, and the failure mode of one of them
/// disagreeing is silent.
pub fn configure_vaulting(enabled: bool) {
    VAULTING_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
    if !enabled {
        tracing::warn!(
            "Secret auto-vaulting is DISABLED by configuration. Fields declared `encrypted: \
             true` will be stored as plaintext. This is an escape hatch, not a supported \
             steady state."
        );
    }
}

/// Whether auto-vaulting is currently active.
pub fn vaulting_enabled() -> bool {
    VAULTING_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

impl Vaulter {
    pub(crate) fn new(
        node_types: Arc<dyn NodeTypeLookup>,
        element_types: Arc<dyn ElementTypeLookup>,
        secret_store: Arc<OnceLock<Arc<SecretStore>>>,
        db: Arc<DB>,
        hlc: Arc<NodeHLCState>,
    ) -> Self {
        Self {
            node_types,
            element_types,
            secret_store,
            db,
            hlc,
        }
    }

    /// Seal every plaintext value sitting in a declared-secret field, rewriting
    /// the property to a `secret://` reference.
    ///
    /// Returns `Ok(())` untouched for the ~99.9% of writes whose NodeType
    /// declares no secret field at all — that case costs one map lookup and no
    /// walk.
    pub(crate) async fn vault(
        &self,
        scope: VaultScope<'_>,
        node: &mut Node,
        memo: Option<&VaultMemo>,
    ) -> Result<()> {
        let fields = self.resolve_gate(node, &scope).await?;

        // THE FAST PATH. `EncryptedFields::None` is the only value that skips
        // the walk; `Unknown` deliberately does not (see the gate's docs).
        if !fields.must_examine() {
            return Ok(());
        }

        // The operator kill switch, checked AFTER the gate so a disabled
        // deployment does not pay for a check on every write, and so the warning
        // below fires only for nodes that would actually have been vaulted
        // rather than on every write in the system.
        //
        // Disabling this stores declared-secret values as PLAINTEXT. It exists
        // because this sits on the core write path and fails closed, so a defect
        // here refuses writes rather than degrading them, and an operator needs a
        // way out at 3am that is not a rollback. It is deliberately loud: silence
        // is how a temporary "just for tonight" becomes permanent.
        if !vaulting_enabled() {
            tracing::warn!(
                node_id = %node.id,
                node_type = %node.node_type,
                tenant = %scope.tenant_id,
                repo = %scope.repo_id,
                "SECRET VAULTING IS DISABLED ([secrets] vaulting_enabled = false): this node \
                 declares encrypted field(s) and their values are being stored AS PLAINTEXT, in \
                 the node blob and in the property index. Re-enabling does not retroactively seal \
                 anything already written this way."
            );
            return Ok(());
        }

        let top_level: HashSet<&str> = match &fields {
            EncryptedFields::Some { top_level, .. } => {
                top_level.iter().map(|s| s.as_str()).collect()
            }
            // Unknown reaching here means the type could not be resolved but
            // the node carries no plaintext candidate; the walk finds none.
            _ => HashSet::new(),
        };
        for name in &top_level {
            refuse_unaddressable_secret_name(name, &node.node_type, "property")?;
        }

        // Element-declared secrets are only reachable through a full walk, and
        // the element types have to be resolved BEFORE it: the walk is
        // synchronous and resolution is not.
        let element_secrets = if fields.needs_deep_walk() {
            self.resolve_element_secrets(node, &scope).await?
        } else {
            HashMap::new()
        };

        let secret_scope = SecretScope::new(scope.tenant_id, scope.repo_id, scope.branch);

        // The store is asked for lazily, on the FIRST leaf that actually needs
        // sealing. `must_examine()` is true for any type that merely has a
        // container property (a secret could be nested in one), which is a
        // great many ordinary types; requiring a master keyring for all of them
        // would make an unconfigured deployment unable to write them at all.
        let mut store: Option<Arc<SecretStore>> = None;

        let node_id = node.id.clone();
        let actor = scope.actor.to_string();
        let mut failure: Option<raisin_error::Error> = None;
        let mut vaulted = 0usize;

        let deep = fields.needs_deep_walk();
        let mut vault_leaf = |cursor: &WalkCursor<'_>, value: &mut PropertyValue| -> bool {
            if failure.is_some() {
                return true; // stop doing work; the error is already fatal
            }
            if !is_declared_secret(cursor, &top_level, &element_secrets) {
                return false;
            }

            let plaintext = match value {
                // Already a reference: LEAVE IT ALONE. A client reads a
                // reference and writes it back unchanged on every
                // read-modify-write, so re-vaulting here would mint a version
                // per round trip. Conversely plaintext arriving means a human
                // typed a new value, so it ALWAYS mints — comparing against the
                // stored value would mean decrypting it, which the write path
                // must never do.
                PropertyValue::String(s) if SecretRef::is_secret_ref(s) => return true,
                PropertyValue::String(s) => s.clone(),
                // A field declared secret but absent carries nothing to seal.
                PropertyValue::Null => return true,
                other => {
                    failure = Some(raisin_error::Error::Validation(format!(
                        "field '{}' on node '{}' is declared encrypted, so it must hold a string \
                         (or be absent), but it holds {:?}; refusing the write rather than \
                         storing it in plaintext",
                        cursor.path,
                        node_id,
                        std::mem::discriminant(&*other),
                    )));
                    return true;
                }
            };

            vaulted += 1;
            if vaulted > MAX_SECRET_PATHS_PER_NODE {
                failure = Some(raisin_error::Error::Validation(format!(
                    "node '{node_id}' carries more than {MAX_SECRET_PATHS_PER_NODE} encrypted \
                     fields; refusing the write (dropping the overflow, as the geometry walker \
                     does, would write those secrets in plaintext)"
                )));
                return true;
            }

            let name = node_field_secret_name(&node_id, cursor.path);
            let hash: [u8; 32] = *blake3::hash(plaintext.as_bytes()).as_bytes();

            let memoized = memo.and_then(|m| {
                m.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&name)
                    .filter(|(_, seen)| seen == &hash)
                    .map(|(version, _)| *version)
            });

            let version = match memoized {
                Some(v) => v,
                None => {
                    let store = match store.clone() {
                        Some(s) => s,
                        None => match lazy_secret_store(&self.secret_store, &self.db, &self.hlc) {
                            Ok(s) => {
                                store = Some(s.clone());
                                s
                            }
                            Err(e) => {
                                failure = Some(e);
                                return true;
                            }
                        },
                    };
                    let owner = SecretOwner::node_field(actor.clone(), &node_id, cursor.path);
                    match store.put(&secret_scope, &name, plaintext.as_bytes(), &owner) {
                        Ok((_, version)) => {
                            if let Some(m) = memo {
                                m.lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .insert(name.clone(), (version, hash));
                            }
                            version
                        }
                        Err(e) => {
                            failure = Some(e.into());
                            return true;
                        }
                    }
                }
            };

            *value = PropertyValue::String(format_secret_ref(&name, Some(version)));
            true
        };

        if deep {
            walk_properties_mut(&mut node.properties, vault_leaf);
        } else {
            // No container can hide a secret, so the declared names are
            // addressable directly — no traversal, no allocation per leaf.
            for name in &top_level {
                if let Some(value) = node.properties.get_mut(*name) {
                    let cursor = WalkCursor {
                        path: name,
                        enclosing_element_type: None,
                    };
                    vault_leaf(&cursor, value);
                }
            }
        }

        if let Some(e) = failure {
            return Err(e);
        }

        if vaulted > 0 {
            tracing::debug!(
                node_id = %node.id,
                fields = vaulted,
                "vaulted encrypted fields"
            );
        }

        Ok(())
    }

    /// Read the gate, resolving and caching the NodeType on a miss.
    ///
    /// A resolution failure is NOT swallowed: it returns an error whenever the
    /// node carries anything that could be a secret.
    async fn resolve_gate(&self, node: &Node, scope: &VaultScope<'_>) -> Result<EncryptedFields> {
        let cache = encrypted_fields::global();
        let key = EncryptedFieldsCache::key(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            &node.node_type,
        );

        let cached = cache.get(&key);
        if !matches!(cached, EncryptedFields::Unknown) {
            return Ok(cached);
        }

        // Deliberately the unpinned `resolve`: the cache key has no workspace
        // in it, and resolving per workspace under a workspace-blind key would
        // let one workspace's answer overwrite another's. Of the two ways that
        // can be wrong, this is the safe one — branch-latest sees every field
        // the newest revision declares secret, so a pinned older revision can
        // only cause OVER-vaulting (harmless). The reverse could cache a
        // `false` that leaks.
        let resolver = NodeTypeResolver::<RocksDBStorage>::from_lookup(
            self.node_types.clone(),
            scope.tenant_id.to_string(),
            scope.repo_id.to_string(),
            scope.branch.to_string(),
        );

        match resolver.resolve(&node.node_type).await {
            Ok(resolved) => {
                let fields =
                    EncryptedFields::from_resolved_properties(&resolved.resolved_properties);
                cache.put(key, fields.clone());
                Ok(fields)
            }
            // DETERMINATE absence. No NodeType exists, so no schema declares
            // `encrypted: true`, so there is nothing to vault — this is an
            // answer, not a failure, and refusing here would make every node of
            // an unregistered type unwritable.
            //
            // Deliberately NOT cached. `Event::Schema` does fire on NodeType
            // CREATION (`node_types/crud.rs` publishes `SchemaChange::Created`,
            // guarded by `schema_event_local_publish_test`), so an invalidation
            // would arrive — but caching an absence buys nothing on the hot
            // path (a registered type caches its real answer on first use) and
            // costs the entire stale-`false` risk if that one event is ever
            // missed. An unregistered type simply re-resolves.
            Err(raisin_error::Error::NotFound(_)) => Ok(EncryptedFields::None),

            // INDETERMINATE. The type may exist and may declare secrets, and we
            // could not read it — a storage fault, a cycle, an inheritance
            // depth blowout. FAIL CLOSED: the geometry sibling returns Ok here,
            // and doing that would write whatever the client sent straight to
            // disk and to the property index.
            Err(e) => {
                if carries_a_possible_secret(node) {
                    Err(raisin_error::Error::Validation(format!(
                        "cannot determine whether node type '{}' declares encrypted fields ({}), \
                         and node '{}' carries string properties that might be secrets; refusing \
                         the write rather than risking plaintext at rest",
                        node.node_type, e, node.id
                    )))
                } else {
                    // Nothing string-shaped anywhere in the tree, so there is
                    // nothing a secret could be hiding in.
                    Ok(EncryptedFields::None)
                }
            }
        }
    }

    /// For every element type reachable in this node's property tree, the set
    /// of field names it declares secret.
    async fn resolve_element_secrets(
        &self,
        node: &Node,
        scope: &VaultScope<'_>,
    ) -> Result<HashMap<String, HashSet<String>>> {
        let mut names: HashSet<String> = HashSet::new();
        {
            // The selector never claims a leaf (it always returns None), so the
            // walk descends everywhere; the element types are recorded through
            // a side channel. Same shape the walker's own tests use.
            let seen = std::cell::RefCell::new(HashSet::new());
            walk_properties(&node.properties, |cursor: &WalkCursor<'_>, _v| {
                if let Some(et) = cursor.enclosing_element_type {
                    seen.borrow_mut().insert(et.to_string());
                }
                None::<&str>
            });
            names.extend(seen.into_inner());
        }

        let mut out = HashMap::new();
        if names.is_empty() {
            return Ok(out);
        }

        let resolver = ElementTypeResolver::<RocksDBStorage>::from_lookup(
            self.element_types.clone(),
            scope.tenant_id.to_string(),
            scope.repo_id.to_string(),
            scope.branch.to_string(),
        );

        for name in names {
            // Fail closed here too: an element type present in the data but
            // unresolvable means its secret fields are unknown.
            let resolved = resolver.resolve(&name).await.map_err(|e| {
                raisin_error::Error::Validation(format!(
                    "cannot determine whether element type '{name}' declares encrypted fields \
                     ({e}); refusing the write on node '{}' rather than risking plaintext at rest",
                    node.id
                ))
            })?;
            let secret_fields: HashSet<String> = resolved
                .resolved_fields
                .iter()
                .filter(|f| field_is_secret(f.base()))
                .map(|f| f.base().name.clone())
                .collect();
            for field in &secret_fields {
                refuse_unaddressable_secret_name(field, &name, "field")?;
            }
            out.insert(name, secret_fields);
        }

        Ok(out)
    }
}

/// Build the secret store on first use, sharing one instance per database.
///
/// # Why this can fail, and why that is the point
///
/// The store needs a master keyring. A deployment that declares no `encrypted`
/// field never reaches this call, so it never needs one. Once a NodeType DOES
/// declare a secret, an unconfigured keyring must be an ERROR here, never a
/// fallback: carrying on without a store writes the plaintext to the node blob
/// and to the property index, which no later fix recovers.
pub(crate) fn lazy_secret_store(
    slot: &Arc<OnceLock<Arc<SecretStore>>>,
    db: &Arc<DB>,
    hlc: &Arc<NodeHLCState>,
) -> Result<Arc<SecretStore>> {
    if let Some(store) = slot.get() {
        return Ok(store.clone());
    }
    let keys = raisin_crypto::EnvKeyProvider::shared().map_err(|e| {
        raisin_error::Error::InvalidState(format!(
            "the secret store needs a master keyring, and none is configured: {e}"
        ))
    })?;
    let store = Arc::new(SecretStore::with_hlc(
        db.clone(),
        Arc::new(raisin_crypto::SecretBox::with_keyring(keys)),
        hlc.clone(),
    ));
    // A racing caller may have won; keep whichever instance landed so every
    // caller shares one HLC.
    let _ = slot.set(store);
    Ok(slot
        .get()
        .expect("the slot is set on the line above")
        .clone())
}

/// Whether the node holds any string value at all, at any depth.
///
/// Only consulted on the fail-closed path, so its cost never touches a normal
/// write.
fn carries_a_possible_secret(node: &Node) -> bool {
    if node.properties.is_empty() {
        return false;
    }
    !walk_properties(&node.properties, |_, v| match v {
        PropertyValue::String(s) => Some(s.as_str()),
        _ => None,
    })
    .is_empty()
}

/// Refuse a declared-secret name the path format cannot address unambiguously.
///
/// The walker joins path segments with `.` and has no escaping, so a field
/// named `a.b` collides with key `b` inside container `a`. Ordinary indexes
/// tolerate that ambiguity; a secret cannot, because the path becomes the
/// secret's storage NAME — two fields could collide on one secret, or a rewrite
/// could land on the wrong leaf.
///
/// Enforced here, at the write layer, because there is no validation hook on
/// the NodeType/ElementType write path today. That makes it unbypassable (every
/// write funnels through this) at the cost of surfacing late: the author learns
/// at the first node write, not at schema save.
fn refuse_unaddressable_secret_name(name: &str, declared_by: &str, kind: &str) -> Result<()> {
    if secret_name_is_addressable(name) {
        return Ok(());
    }
    Err(raisin_error::Error::Validation(format!(
        "'{declared_by}' declares the encrypted {kind} '{name}', whose name contains a '.'. \
         Property paths are dot-joined and have no escaping, so that name is ambiguous against a \
         nested path and cannot address one secret unambiguously. Rename the {kind}."
    )))
}

/// Whether the value the walk is currently offering sits in a declared-secret
/// field.
///
/// Two sources, matching the two places a declaration can live:
///
/// - a top-level NodeType property, addressed by its bare name;
/// - a field of the nearest enclosing element type, addressed by the LAST path
///   segment.
///
/// The last-segment match is an over-approximation: a value nested inside a
/// plain `Object` inside an element shares the element's cursor, so
/// `hero.meta.token` matches an element field named `token`. That direction is
/// deliberate — over-vaulting encrypts something the author did not ask to
/// encrypt, which is visible and reversible; under-vaulting writes a secret in
/// plaintext, which is neither.
fn is_declared_secret(
    cursor: &WalkCursor<'_>,
    top_level: &HashSet<&str>,
    element_secrets: &HashMap<String, HashSet<String>>,
) -> bool {
    match cursor.enclosing_element_type {
        None => !cursor.path.contains('.') && top_level.contains(cursor.path),
        Some(element_type) => {
            let leaf = cursor.path.rsplit('.').next().unwrap_or(cursor.path);
            element_secrets
                .get(element_type)
                .is_some_and(|fields| fields.contains(leaf))
        }
    }
}
