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

//! `NodeType.immutable`: once a node of an immutable type is created, its
//! `properties` can never be changed again. Structural writes — path,
//! parent, `node_type`, `published_at`, etc. — remain allowed, and delete
//! remains allowed (delete is a tombstone, not a property rewrite). This is
//! deliberately scoped to `properties` only: [`reject_if_immutable`] is a
//! no-op unless the old and new property maps actually differ.
//!
//! # One check, every write path
//!
//! Mirrors [`crate::vaulting`]'s reach: this crate has four low-level node
//! write functions (transaction's `add_node`/`put_node`, repository's
//! `add_impl`/`update_impl`), plus the cross-branch promotion upsert. Only
//! the UPDATE-capable ones need this check — `add_node`/`add_impl` are
//! CREATE-only, so there is no prior state to protect. Call
//! [`reject_if_immutable`] once the existing node's `NodeType` is resolved,
//! after it is confirmed to be an update (not a create).
//!
//! # Fail open on an unresolved type
//!
//! Unlike vaulting (which must fail closed, since a missed encrypted field is
//! a permanent plaintext leak), an unresolvable `NodeType` here means
//! "proceed" — this matches the existing `allowed_children` check in
//! `add_node.rs`, which "treats a parent it cannot resolve as no constraint
//! rather than an error". The overwhelming majority of types are not
//! immutable, so callers should look the type up, and only call
//! [`reject_if_immutable`] when the lookup actually returned `Some`; a
//! lookup failure or `None` should simply skip the call.
//!
//! # Replication
//!
//! Deliberately NOT enforced on the replication apply path, for the same
//! reason vaulting isn't re-run there: an arriving node was already accepted
//! by its origin peer under that peer's own policy, and there is no recovery
//! story for rejecting an already-committed revision mid-apply.

use std::collections::HashMap;

use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::NodeType;

/// Reject a property-changing write to an immutable node.
///
/// No-op unless `node_type.immutable == Some(true)` AND `old_properties !=
/// new_properties`. Callers only invoke this when `node_type` was
/// successfully resolved for an UPDATE (not a create).
pub fn reject_if_immutable(
    node_type: &NodeType,
    node_id: &str,
    old_properties: &HashMap<String, PropertyValue>,
    new_properties: &HashMap<String, PropertyValue>,
) -> Result<()> {
    if node_type.immutable == Some(true) && old_properties != new_properties {
        return Err(raisin_error::Error::Conflict(format!(
            "Node '{}' is immutable (NodeType '{}'); property update rejected",
            node_id, node_type.name
        )));
    }
    Ok(())
}
