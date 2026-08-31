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

//! What an adapter says it can do — the `capabilities` contract.
//!
//! # Why this lives in `raisin-models` and not in the sync engine
//!
//! There used to be TWO declarations of this struct: the engine's, in
//! `virtual_mount_sync::adapter`, and a hand-maintained "field-for-field
//! mirror" in the HTTP connection-test handler, kept because `raisin-rocksdb`
//! is an optional dependency that did not expose the type. Both serialize into
//! the ONE `capabilities` property on the `raisin:Integration` node, so
//! whichever wrote last won field-for-field — and the mirror had already
//! drifted by three fields (`accepts_content`, `move_fields`,
//! `submit_unavailable_reason`). Consequences, both observed: the Test-
//! connection panel could never show `submit_unavailable_reason` (the transport
//! deserialized into the stripped struct, dropping the key before the response
//! was built, so an operator never learned WHICH cause made an outbox
//! non-submittable), and every "Test connection" click rewrote the cached blob
//! without those keys, so the next sync's changed-check saw a difference and
//! minted an extra replicated integration-node revision for a value nobody had
//! changed.
//!
//! The duplicate is therefore deleted rather than re-synchronised: a parity
//! test only reports drift after it is written, while one declaration cannot
//! drift at all. The precedent is next door — `build_credential` was moved here
//! "so the sync engine and the HTTP connection-test handler cannot drift
//! apart", and this struct is the same problem.
//!
//! Every field defaults conservatively (`false` / `None` / empty) so an adapter
//! that omits one is treated as *not* supporting it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Typed report of what an adapter can do, returned by its `capabilities`
/// operation. Every field defaults conservatively (`false` / `None`) so an
/// adapter that omits a field is treated as *not* supporting it. The sync engine
/// caches this on the `raisin:Integration` node so the admin UI can read it
/// without invoking the adapter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub can_read: bool,
    #[serde(default)]
    pub can_write: bool,
    #[serde(default)]
    pub can_create_folders: bool,
    #[serde(default)]
    pub supports_changes: bool,
    #[serde(default)]
    pub supports_webhooks: bool,
    #[serde(default)]
    pub supports_search: bool,
    #[serde(default)]
    pub supports_push: bool,
    /// Adapter implements the optional `browse` operation (§2.10). The engine
    /// never calls `browse` — this is carried so the cached capabilities the UI
    /// reads are complete; dropping it here would silently strip the flag from
    /// every integration node the engine writes.
    #[serde(default)]
    pub supports_browse: bool,
    #[serde(default)]
    pub default_ttl: Option<u64>,
    #[serde(default)]
    pub max_file_size: Option<i64>,

    // ---- write path (§3.3 / §10 of docs/reference/virtual-node-adapters.md) ----
    //
    // All optional, all `false` / empty by default, so an adapter written before
    // the write path existed — i.e. every adapter shipped today — is correctly
    // reported as read-only rather than accidentally write-enabled.
    /// Adapter implements the `create` operation (a local create propagates).
    #[serde(default)]
    pub can_create: bool,
    /// Adapter implements the `update` operation (a local edit propagates).
    #[serde(default)]
    pub can_update: bool,
    /// Adapter implements the `delete` operation (a local delete propagates,
    /// subject to the mount's `delete_policy`).
    #[serde(default)]
    pub can_delete: bool,
    /// Adapter implements the `submit` operation — issuing a command (send a
    /// mail, RSVP) rather than mirroring an object.
    #[serde(default)]
    pub can_submit: bool,
    /// Adapter wants a node's FILE BYTES on `create` / `update`, not just its
    /// properties.
    ///
    /// Off by default, and the default is the safe one: a file-shaped provider
    /// whose adapter has not opted in mirrors metadata only, which is what
    /// every adapter did before the content channel existed. Opting in also
    /// means opting into the second shape — an object over the inline ceiling
    /// arrives as a descriptor with no bytes, and the adapter must answer with
    /// an upload URL or fail; see `write::content`.
    #[serde(default)]
    pub accepts_content: bool,
    /// The `state_only` allow-list: which node properties this provider accepts
    /// as writes. The engine has no domain knowledge (it does not know a mail
    /// body is immutable while its read flag is not), so this is how a provider
    /// explains what "writable" means for it. Empty means "nothing declared".
    #[serde(default)]
    pub mutable_fields: Vec<String>,
    /// Which of [`Self::mutable_fields`] express the object's LOCATION at the
    /// provider — the folder, the parent, the label set that IS the mailbox.
    ///
    /// A move is an `update` carrying this field and nothing more (§8), so this
    /// is what lets `move_policy` mean anything: the engine is domain-blind and
    /// cannot tell that `folder` relocates a message while `unread` does not.
    /// Empty — the default, and where every adapter shipped today is — means no
    /// declared field relocates anything, so `move_policy` has nothing to gate
    /// and the mount behaves exactly as it did before this existed.
    ///
    /// A name here that is NOT in `mutable_fields` is inert: the effective push
    /// list is the intersection of mount and adapter, and a field the adapter
    /// does not accept as a write never reaches the classification at all.
    #[serde(default)]
    pub move_fields: Vec<String>,
    /// Recommended default for a local delete: `"detach" | "trash" | "purge"`.
    /// A mount may override it; `purge` is never a default.
    #[serde(default)]
    pub default_delete_policy: Option<String>,
    /// Recommended default for a local move: `"push" | "detach" | "reject"`.
    #[serde(default)]
    pub default_move_policy: Option<String>,
    /// `delete` can soft-delete (provider trash) rather than purge.
    #[serde(default)]
    pub supports_trash: bool,
    /// `submit` can forward a provider-side idempotency key, so the engine's
    /// at-most-once attempt id is honoured end to end.
    #[serde(default)]
    pub supports_idempotency_key: bool,
    /// WHY `can_submit` is false, when an adapter can say.
    ///
    /// Carried because this struct is the only thing that survives: `resolve.rs`
    /// caches `serde_json::to_value(capabilities)` from the TYPED value onto the
    /// integration node, so a key the struct does not name is dropped before any
    /// consumer — the console included. The IMAP adapter has always answered
    /// this key (an outbox whose email provider is disabled, ambiguous, or
    /// denied by policy reports exactly which), and until it was declared here
    /// that diagnosis reached nobody: the mount showed the engine's generic
    /// "adapter does not declare can_submit" and the operator had no way to
    /// learn which of the four causes it was.
    #[serde(default)]
    pub submit_unavailable_reason: Option<String>,
}

impl Capabilities {
    /// Conservative fallback used when an adapter has no `capabilities`
    /// operation, returns a non-object, or throws: assume read-only and nothing
    /// else. This keeps the sync running (reads still work) while advertising no
    /// write/change/webhook support to the UI.
    pub fn fallback() -> Self {
        Self {
            can_read: true,
            ..Self::default()
        }
    }

    /// Which of the operations a `mirror` write mode needs this adapter does
    /// NOT declare, in a stable order suitable for an operator-facing message.
    ///
    /// `mirror` means "the node **is** the remote object", so create, update and
    /// delete all have to propagate; `can_write` is the umbrella flag an adapter
    /// sets to say it writes at all. Empty means every needed op is declared.
    ///
    /// `needs_create` comes from the MOUNT, not the adapter: the engine issues
    /// `create` only for a mount that opted a node type into local creation, so
    /// an adapter that updates and deletes is a complete mirror for every mount
    /// that does not. Demanding the capability unconditionally refused correct
    /// adapters for an op they would never be called with.
    pub fn missing_mirror_ops(
        &self,
        needs_create: bool,
        propagates_deletes: bool,
    ) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.can_write {
            missing.push("can_write");
        }
        if needs_create && !self.can_create {
            missing.push("can_create");
        }
        if !self.can_update {
            missing.push("can_update");
        }
        // Only from a mount that actually deletes upstream. A `detach` mount
        // unhooks the node locally and never calls the adapter, so demanding
        // the capability refused mounts for an operation they are configured
        // never to perform — see the note at the call site in `resolve_mirror`.
        if propagates_deletes && !self.can_delete {
            missing.push("can_delete");
        }
        missing
    }

    /// Which of the operations a `state_only` write mode needs this adapter
    /// does NOT declare, in a stable order suitable for an operator message.
    ///
    /// `state_only` pushes a declared allow-list of fields onto an existing
    /// remote object, so it needs `update` and nothing else — no create, no
    /// delete. Reusing [`Self::missing_mirror_ops`] here would refuse a perfectly
    /// good mail mount for lacking a `delete` it will never call.
    pub fn missing_state_only_ops(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.can_write {
            missing.push("can_write");
        }
        if !self.can_update {
            missing.push("can_update");
        }
        missing
    }

    /// Which of the operations a `submit` write mode needs this adapter does
    /// NOT declare.
    ///
    /// `submit` issues a COMMAND — it neither mirrors nor patches a remote
    /// object — so `can_update`, `can_create` and `can_delete` say nothing about
    /// it and requiring them would refuse a perfectly good outbox for lacking
    /// operations it will never call. `can_write` is still required: it is the
    /// umbrella flag that says this adapter changes anything at the provider at
    /// all, and an adapter that sets `can_submit` without it has contradicted
    /// itself rather than opted in.
    ///
    /// Note what is NOT here: [`Self::supports_idempotency_key`]. An adapter
    /// that cannot forward a provider-side key is still a usable outbox — the
    /// engine's at-most-once protocol is built on the durable `queued ->
    /// sending` claim, not on the provider cooperating, precisely because
    /// almost none of them do (Graph's `sendMail` has no such key, and SMTP has
    /// nothing at all). Requiring it would refuse every real adapter.
    pub fn missing_submit_ops(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.can_write {
            missing.push("can_write");
        }
        if !self.can_submit {
            missing.push("can_submit");
        }
        missing
    }

    /// Parse an adapter's `capabilities` return value. Returns `None` when the
    /// value is null/absent or is not a decodable capabilities object, so the
    /// caller can substitute [`Capabilities::fallback`].
    pub fn from_adapter_value(value: &Value) -> Option<Self> {
        if value.is_null() {
            return None;
        }
        serde_json::from_value(value.clone()).ok()
    }
}
