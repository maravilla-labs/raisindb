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

//! Re-vaulting a COPIED node.
//!
//! # Copy vs promotion — the distinction that decides the whole behaviour
//!
//! Both operations produce a node on the other side carrying the same
//! properties, and the secret name embeds the node id, so the two cases are
//! opposites:
//!
//! | | node id | secret name | what has to happen |
//! |---|---|---|---|
//! | cross-branch **promotion** | PRESERVED | unchanged | copy the sealed record into the target branch (`cross_branch/secrets.rs`) |
//! | **copy** (`copy_node`, `copy_node_tree`) | NEW | must change | RE-VAULT under the new name (this file) |
//!
//! Skip this and the copy's property still names the SOURCE node's secret. Two
//! nodes then share one value: deleting the source destroys the copy's password,
//! and any access decision keyed on the owning node id is answered about the
//! wrong node.
//!
//! # This is the one place the server handles plaintext outside a write path
//!
//! A new name means a new AAD-bound envelope, and only the plaintext can produce
//! one. So the value is opened and immediately re-sealed under
//! `node/{new_id}/{field.path}`. Deliberately narrow: it reads one secret at a
//! time, writes it straight back, and never returns it to a caller. Copying the
//! ciphertext instead would be wrong in a subtler way — the record would decrypt
//! fine (the crypto context binds tenant and repo, not the node), but the two
//! nodes would still be sharing one storage row.

use std::collections::HashMap;

use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::secret_ref::{format_secret_ref, node_field_secret_name, SecretRef};

use super::super::super::NodeRepositoryImpl;
use crate::indexing::property_walk::{walk_properties_mut, WalkCursor};
use crate::secret_store::{SecretOwner, SecretScope, StoredSecret};

impl NodeRepositoryImpl {
    /// Re-seal every secret the SOURCE node owned under the COPY's own name, and
    /// rewrite the copy's properties to point at it.
    ///
    /// References the source did NOT own (an operator-created
    /// `secret://stripe_key`, say) are left exactly as they are: those are
    /// shared on purpose, and re-vaulting them would fork a value the operator
    /// rotates in one place.
    ///
    /// Returns the versions minted, so the caller can capture them for
    /// replication BEFORE it writes the node.
    pub(in crate::repositories::nodes) async fn revault_copied_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        source_id: &str,
        node: &mut Node,
        actor: &str,
    ) -> Result<Vec<StoredSecret>> {
        let source_prefix = node_field_secret_name(source_id, "");
        if !carries_owned_ref(&node.properties, &source_prefix) {
            // THE FAST PATH: no keyring is asked for, so an ordinary copy on a
            // deployment with no secrets configured still works.
            return Ok(Vec::new());
        }

        let store = self.secret_store()?;
        let scope = SecretScope::new(tenant_id, repo_id, branch);
        let new_id = node.id.clone();

        let mut minted: Vec<StoredSecret> = Vec::new();
        let mut failure: Option<raisin_error::Error> = None;
        // One source name may be referenced from more than one path (an author
        // can duplicate the reference); mint once and reuse.
        let mut already: HashMap<String, String> = HashMap::new();

        walk_properties_mut(&mut node.properties, |cursor: &WalkCursor<'_>, value| {
            if failure.is_some() {
                return true;
            }
            let PropertyValue::String(raw) = value else {
                return false;
            };
            if !SecretRef::is_secret_ref(raw) {
                return false;
            }
            let Ok(reference) = SecretRef::parse(raw) else {
                return true; // not parseable: leave it verbatim, as reads do
            };
            if !reference.name.starts_with(&source_prefix) {
                return true; // not the source node's own secret — shared on purpose
            }

            if let Some(rewritten) = already.get(&reference.name) {
                *value = PropertyValue::String(rewritten.clone());
                return true;
            }

            let plaintext = match store.get(&scope, &reference.name, reference.version) {
                Ok(bytes) => bytes,
                Err(e) => {
                    failure = Some(raisin_error::Error::InvalidState(format!(
                        "cannot copy node '{source_id}': its encrypted field '{}' references \
                         secret '{}', which could not be read ({e}). Copying the reference \
                         unchanged would make the copy share the source's secret, so the copy is \
                         refused instead.",
                        cursor.path, reference.name
                    )));
                    return true;
                }
            };

            let name = node_field_secret_name(&new_id, cursor.path);
            let owner = SecretOwner::node_field(actor, &new_id, cursor.path);
            match store.put_recorded(&scope, &name, &plaintext, &owner) {
                Ok(stored) => {
                    let rewritten = format_secret_ref(&name, Some(stored.record.version));
                    already.insert(reference.name.clone(), rewritten.clone());
                    minted.push(stored);
                    *value = PropertyValue::String(rewritten);
                }
                Err(e) => failure = Some(e.into()),
            }
            true
        });

        if let Some(e) = failure {
            return Err(e);
        }
        Ok(minted)
    }
}

/// Whether any leaf holds a `secret://` reference the source node owns.
///
/// Cheap pre-check so an ordinary copy never asks for a master keyring.
fn carries_owned_ref(properties: &HashMap<String, PropertyValue>, source_prefix: &str) -> bool {
    crate::secret_store::lifecycle::secret_refs_in(properties)
        .iter()
        .any(|(_, r)| r.name.starts_with(source_prefix))
}
