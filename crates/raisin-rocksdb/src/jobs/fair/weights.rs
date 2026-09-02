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
//! # There is no tier model in this codebase yet, and this does not invent one
//!
//! `raisin-context/src/tier.rs` declares `ServiceTier` and a `TierProvider`
//! trait, but nothing implements or installs one: no surface maps a tenant id
//! to a tier, and the one place a tier could be stored today
//! (`TenantRegistration.metadata`) is rewritten wholesale on every boot by
//! `register_tenant`, so a value written there would silently disappear.
//!
//! Inventing a schema here would be worse than having none — it would put a
//! second, competing definition of "what plan is this tenant on" in the
//! codebase, and this crate is the wrong place to decide that. So
//! [`tenant_weight`] returns [`DEFAULT_TENANT_WEIGHT`] for everyone, which
//! makes the scheduler a pure fair-share round robin: every tenant gets an
//! equal turn, which is the correct behaviour in the absence of tiers and is
//! already the entire fix for the incident that prompted this work.
//!
//! The seam for the day tiers exist is [`TenantWeights`] — one trait, one
//! method. Wire a real provider into [`super::FairScheduler`] and nothing else
//! changes.

use std::collections::HashMap;

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

/// The weight for a tenant, in the absence of any tier model.
///
/// A single function so that "where does the weight come from?" has exactly one
/// answer, and so the day a tier model lands there is exactly one site to
/// change.
///
/// TODO(tiers): resolve from a real plan/tier source once one exists. The
/// intended shape is `raisin_context::TierProvider` — but it needs (a) an
/// implementation, (b) a durable per-tenant tier that survives
/// `register_tenant` overwriting `TenantRegistration.metadata`, and (c) a
/// synchronous lookup or a cache, because the scheduler resolves the weight
/// while holding its state lock and must never await there.
pub fn tenant_weight(_tenant: &str) -> u32 {
    DEFAULT_TENANT_WEIGHT
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

/// Every tenant gets the same turn. The production default today.
#[derive(Debug, Clone, Copy, Default)]
pub struct EqualWeights;

impl TenantWeights for EqualWeights {
    fn weight_for(&self, tenant: &str) -> u32 {
        tenant_weight(tenant)
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
