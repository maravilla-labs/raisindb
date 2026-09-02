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

//! Durable home of the per-tenant scheduling weights.
//!
//! # Why here, and why not a new column family
//!
//! This is an operator setting about a tenant, which is what `cf::REGISTRY`
//! already holds (`tenants\0{id}`, `deployments\0{tenant}\0{key}`). A new
//! column family would have to be added in four places —- `mod cf`,
//! `all_column_families()`, `BRANCH_CF_REGISTRY` and `TENANT_PREFIXED_CFS` in
//! `storage/tenant_wipe.rs`, the last of which is not test-enforced and is the
//! one that gets missed — for a keyspace of one small row per tenant.
//!
//! It is NOT stored in `TenantRegistration.metadata`, which is where a reader
//! would look first. `register_tenant` rewrites that struct wholesale, and
//! every caller in the tree passes an EMPTY metadata map, so a weight written
//! there would vanish the next time anything touched the tenant — the exact
//! silent-reset failure the persistence exists to prevent.
//!
//! # The key
//!
//! `scheduling\0weights\0{tenant}`. Two constant segments, not one: a
//! single-segment prefix could be shadowed by a tenant that happens to be named
//! after it (vmount registry rows in this same CF are keyed
//! `{tenant}\0{namespace}\0…`), and `load_all` would then try to decode a
//! foreign value. Two fixed segments make that collision impossible.
//!
//! It is also deliberately NOT under the `tenants\0` prefix: `list_tenants`
//! prefix-scans that and decodes every value it finds as a
//! `TenantRegistration`, so a row of ours living there would break tenant
//! listing outright.

use super::weights::scheduling_weights;
use crate::{cf, cf_handle, keys::KeyBuilder};
use raisin_error::{Error, Result};
use rocksdb::DB;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-tenant scheduling settings, as stored.
///
/// A struct rather than a bare integer so a second knob can be added without a
/// format migration; msgpack, matching every other `cf::REGISTRY` value (the
/// json→msgpack migration pass in `management/format_migration` walks this CF).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantSchedulingSettings {
    /// Credit per scheduling round. See `weights::MAX_TENANT_WEIGHT`.
    pub weight: u32,
}

fn weight_key(tenant_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push("scheduling")
        .push("weights")
        .push(tenant_id)
        .build()
}

fn weight_prefix() -> Vec<u8> {
    KeyBuilder::new()
        .push("scheduling")
        .push("weights")
        .build_prefix()
}

/// Persist `weight` for `tenant_id`, then publish it to the live table.
///
/// Storage first: if the write fails the caller reports the failure and the
/// running scheduler is unchanged, which is honest. The other order would leave
/// a weight in effect that no restart could reproduce.
pub fn set_weight(db: &DB, tenant_id: &str, weight: u32) -> Result<()> {
    let value = rmp_serde::to_vec(&TenantSchedulingSettings { weight })
        .map_err(|e| Error::storage(format!("Serialization error: {}", e)))?;
    let cf = cf_handle(db, cf::REGISTRY)?;
    db.put_cf(cf, weight_key(tenant_id), value)
        .map_err(|e| Error::storage(e.to_string()))?;

    scheduling_weights().set(tenant_id, weight);
    Ok(())
}

/// The stored weight for `tenant_id`, or `None` if the operator never set one.
pub fn get_weight(db: &DB, tenant_id: &str) -> Result<Option<u32>> {
    let cf = cf_handle(db, cf::REGISTRY)?;
    match db.get_cf(cf, weight_key(tenant_id)) {
        Ok(Some(bytes)) => {
            let settings: TenantSchedulingSettings = rmp_serde::from_slice(&bytes)
                .map_err(|e| Error::storage(format!("Deserialization error: {}", e)))?;
            Ok(Some(settings.weight))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(Error::storage(e.to_string())),
    }
}

/// Every stored weight, keyed by tenant.
pub fn load_all(db: &DB) -> Result<HashMap<String, u32>> {
    let prefix = weight_prefix();
    let cf = cf_handle(db, cf::REGISTRY)?;
    let mut out = HashMap::new();

    for item in db.prefix_iterator_cf(cf, &prefix) {
        let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;
        // `prefix_iterator_cf` may run past the prefix; stop, don't decode.
        if !key.starts_with(&prefix[..]) {
            break;
        }
        let tenant = match std::str::from_utf8(&key[prefix.len()..]) {
            Ok(t) => t.to_string(),
            Err(_) => continue,
        };
        let settings: TenantSchedulingSettings = match rmp_serde::from_slice(&value) {
            Ok(s) => s,
            Err(e) => {
                // One unreadable row must not cost every other tenant its
                // weight; skip it loudly and carry on.
                tracing::warn!(tenant = %tenant, error = %e, "Unreadable scheduling weight; ignoring");
                continue;
            }
        };
        out.insert(tenant, settings.weight);
    }

    Ok(out)
}

/// Load the persisted weights into the process-wide table.
///
/// Call once at startup, BEFORE the job pools are built. Skipping it is not an
/// error the operator would ever see: the schedulers come up with an empty
/// table, every tenant gets equal share, and the configuration they set is
/// simply not applied.
pub fn install_persisted_weights(db: &DB) -> Result<usize> {
    let table = load_all(db)?;
    let count = table.len();
    scheduling_weights().replace_all(table);
    tracing::info!(tenants = count, "Loaded per-tenant scheduling weights");
    Ok(count)
}
