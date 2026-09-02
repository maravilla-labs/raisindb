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

//! Where a tenant's scheduling weight comes from.
//!
//! The weight is the credit a tenant is granted per scheduling round, so it is
//! a RATIO and never a precedence: with weights 4 and 1, a round serves roughly
//! four of the first tenant's jobs per one of the second's, and the second
//! still advances every round. See [`super`] for why strict priority is
//! rejected.
//!
//! # Where the number comes from, and what it is allowed to mean
//!
//! An operator sets it, per tenant, through
//! `PUT /management/admin/tenants/{tenant_id}/scheduling`. RaisinDB is a
//! standalone product: it takes a bare integer and divides the pools by it. It
//! deliberately has no notion of a plan, a tier or a customer segment — whoever
//! runs the server decides what "4" means, and encoding a pricing model in this
//! crate would put a second, competing definition of that in the codebase.
//!
//! Two layers, and the split is the whole design:
//!
//! * [`SharedWeights`] is an in-memory map, and it is the ONLY thing the
//!   scheduler touches. The lookup runs under the scheduler's state `Mutex`
//!   (see [`super`]), so it cannot await and must not read storage.
//! * durability is one layer out, in [`super::weight_store`]: the write path
//!   persists first and then updates the map, and startup loads the map from
//!   storage. Without that a restart would flatten every tenant back to equal
//!   share — an operator's configuration undone with no error anywhere.
//!
//! An absent weight is [`DEFAULT_TENANT_WEIGHT`], so a server nobody has
//! configured is a pure fair-share round robin, which is the correct behaviour
//! and was the entire fix for the incident that prompted this work.

use std::collections::HashMap;
use std::sync::Arc;

/// Credit granted to a tenant per scheduling round when nothing says otherwise.
///
/// One job per round. Equal for every tenant, which is exactly fair share.
pub const DEFAULT_TENANT_WEIGHT: u32 = 1;

/// Upper bound on a tenant's weight.
///
/// A ratio is only meaningful while the low end still advances at a rate a
/// human would call progress. An unbounded weight lets one tenant serve
/// millions of jobs per turn, which is strict priority wearing a ratio's
/// clothing — the starvation the design explicitly rejects, reintroduced by
/// configuration rather than by code.
pub const MAX_TENANT_WEIGHT: u32 = 64;

/// The weight in effect for a tenant right now.
///
/// A single function so that "where does the weight come from?" has exactly one
/// answer: the process-wide table an operator writes through the scheduling
/// API, falling back to [`DEFAULT_TENANT_WEIGHT`] for a tenant nobody has
/// configured. Synchronous by construction — see [`SharedWeights`].
pub fn tenant_weight(tenant: &str) -> u32 {
    scheduling_weights().weight_for(tenant)
}

/// Resolves the per-round credit for a tenant.
///
/// Synchronous and cheap on purpose: it is called under the scheduler's state
/// lock, where an `await` is forbidden and a storage read would serialise every
/// enqueue in the process behind it.
pub trait TenantWeights: Send + Sync {
    /// Credit for `tenant`, in jobs per round.
    ///
    /// Implementations may return anything; the scheduler clamps to
    /// `1..=`[`MAX_TENANT_WEIGHT`]. Zero is clamped UP to one rather than
    /// honoured: a zero-weight queue would never be served, which is a tenant
    /// whose jobs are accepted and then never run — indistinguishable from a
    /// hang, and reported nowhere.
    fn weight_for(&self, tenant: &str) -> u32;
}

/// Every tenant gets the same turn, whatever is configured.
///
/// Not what the server runs — that is [`scheduling_weights`] — but the
/// unconfigured baseline, and what a test that wants pure round robin asks for
/// explicitly rather than by hoping the process table is empty.
#[derive(Debug, Clone, Copy, Default)]
pub struct EqualWeights;

impl TenantWeights for EqualWeights {
    fn weight_for(&self, _tenant: &str) -> u32 {
        DEFAULT_TENANT_WEIGHT
    }
}

/// A fixed table, for tests and for a future config-driven override.
#[derive(Debug, Clone, Default)]
pub struct StaticWeights {
    table: HashMap<String, u32>,
    fallback: u32,
}

impl StaticWeights {
    /// Build a table; tenants absent from it get [`DEFAULT_TENANT_WEIGHT`].
    pub fn new(table: HashMap<String, u32>) -> Self {
        Self {
            table,
            fallback: DEFAULT_TENANT_WEIGHT,
        }
    }

    /// Set the weight used for tenants absent from the table.
    pub fn with_fallback(mut self, fallback: u32) -> Self {
        self.fallback = fallback;
        self
    }
}

impl TenantWeights for StaticWeights {
    fn weight_for(&self, tenant: &str) -> u32 {
        self.table.get(tenant).copied().unwrap_or(self.fallback)
    }
}

/// Clamp a resolved weight into the range the scheduler can actually honour.
pub(super) fn clamp_weight(weight: u32) -> u32 {
    weight.clamp(1, MAX_TENANT_WEIGHT)
}

/// The lowest weight an operator may set.
///
/// One, never zero. A zero-weight queue is granted no credit per round, so it
/// is served never: its jobs are ACCEPTED and then sit there. Nothing errors,
/// nothing logs, and the tenant experiences it as a product that silently
/// stopped working. [`clamp_weight`] already corrects a zero defensively, but a
/// value that arrives at the edge is a mistake being made — refuse it there so
/// the operator learns, rather than quietly storing a number that means
/// something else.
pub const MIN_TENANT_WEIGHT: u32 = 1;

/// Why a submitted weight was refused. Carries the bound so the caller can say
/// it without restating it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeightError {
    /// Below [`MIN_TENANT_WEIGHT`] — including zero, which would hang the queue.
    TooLow,
    /// Above [`MAX_TENANT_WEIGHT`] — strict priority wearing a ratio's clothing.
    TooHigh,
}

impl std::fmt::Display for WeightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLow => write!(
                f,
                "weight must be at least {MIN_TENANT_WEIGHT}; a zero-weight queue accepts jobs and never runs them"
            ),
            Self::TooHigh => write!(
                f,
                "weight must be at most {MAX_TENANT_WEIGHT}; a larger ratio starves every other tenant"
            ),
        }
    }
}

/// Check a weight an operator asked for, at the edge that accepted it.
pub fn validate_weight(weight: u32) -> std::result::Result<u32, WeightError> {
    if weight < MIN_TENANT_WEIGHT {
        Err(WeightError::TooLow)
    } else if weight > MAX_TENANT_WEIGHT {
        Err(WeightError::TooHigh)
    } else {
        Ok(weight)
    }
}

/// The live, in-memory weight table the schedulers read.
///
/// This is the ONLY thing on the scheduling hot path. The lookup happens under
/// the scheduler's state `Mutex` (see [`super`]), so it must not await and must
/// not touch storage: a RocksDB read there would serialise every enqueue in the
/// process behind one disk seek, and a read-through cache miss under that lock
/// is the same stall with worse timing. Durability lives one layer out — the
/// write path persists and then updates this map, and startup loads the map
/// from storage once.
///
/// A poisoned lock is recovered rather than unwrapped. A panic here happens
/// inside the scheduler's own lock and takes the job system down with it; a
/// weight is not worth that, and the recovered map is exactly as valid as it
/// was before (the writers only ever `insert`).
#[derive(Debug, Default)]
pub struct SharedWeights {
    table: std::sync::RwLock<HashMap<String, u32>>,
}

impl SharedWeights {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set one tenant's weight. Clamped, because a caller that skipped
    /// [`validate_weight`] must still not be able to install a zero.
    pub fn set(&self, tenant: &str, weight: u32) {
        let mut table = self.table.write().unwrap_or_else(|e| e.into_inner());
        table.insert(tenant.to_string(), clamp_weight(weight));
    }

    /// The weight explicitly set for `tenant`, if any. `None` means "nothing
    /// set", which reads as [`DEFAULT_TENANT_WEIGHT`] everywhere — an absent
    /// entry and an entry of 1 are deliberately the same behaviour.
    pub fn get(&self, tenant: &str) -> Option<u32> {
        let table = self.table.read().unwrap_or_else(|e| e.into_inner());
        table.get(tenant).copied()
    }

    /// Install a whole table, replacing what is there. Used once at startup by
    /// the loader; a partial merge would leave a weight that was removed from
    /// storage still in effect.
    pub fn replace_all(&self, entries: HashMap<String, u32>) {
        let entries = entries
            .into_iter()
            .map(|(t, w)| (t, clamp_weight(w)))
            .collect();
        let mut table = self.table.write().unwrap_or_else(|e| e.into_inner());
        *table = entries;
    }

    /// Every weight currently in effect, for diagnostics and tests.
    pub fn snapshot(&self) -> HashMap<String, u32> {
        self.table.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl TenantWeights for SharedWeights {
    fn weight_for(&self, tenant: &str) -> u32 {
        self.get(tenant).unwrap_or(DEFAULT_TENANT_WEIGHT)
    }
}

/// The process-wide table every scheduler in this process reads.
///
/// A global for the same reason the MCP client policy and the plugin capability
/// probe are: the writer is an HTTP handler in `raisin-server`, the readers are
/// schedulers constructed deep inside `JobDispatcher::new()` at a dozen call
/// sites (tests included). Threading a handle through all of them would be a
/// wide refactor whose only effect is that one of those sites eventually gets
/// missed — and a scheduler built without the table is a tenant whose
/// configured weight silently does nothing.
pub fn scheduling_weights() -> &'static Arc<SharedWeights> {
    static WEIGHTS: std::sync::OnceLock<Arc<SharedWeights>> = std::sync::OnceLock::new();
    WEIGHTS.get_or_init(|| Arc::new(SharedWeights::new()))
}
