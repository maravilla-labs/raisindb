// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! How a WebAssembly component honours [`ExecutionPolicy::Deterministic`].
//!
//! The policy itself is generic (QuickJS and Starlark enforce it too); this is
//! the component model's implementation of it:
//!
//! - every `host.call` answers `Err` ([`HostPolicy::DenyAll`]);
//! - WASI's clocks are frozen at zero and its random sources are a fixed byte
//!   cycle, so neither time nor entropy can make two calls on the same input
//!   differ (guest hash maps seed from `wasi:random` too);
//! - no args, env, preopens or captured output.
//!
//! [`ExecutionPolicy::Deterministic`]: crate::types::ExecutionPolicy::Deterministic
//! [`HostPolicy::DenyAll`]: super::bindings::HostPolicy::DenyAll

use std::time::Duration;

use wasmtime_wasi::{Deterministic, HostMonotonicClock, HostWallClock, WasiCtxBuilder};

use super::bindings::HostPolicy;
use crate::types::ExecutionPolicy;

/// A clock that never moves: a deterministic call must not observe time.
struct FrozenClock;

impl HostWallClock for FrozenClock {
    fn resolution(&self) -> Duration {
        Duration::from_nanos(1)
    }
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

impl HostMonotonicClock for FrozenClock {
    fn resolution(&self) -> u64 {
        1
    }
    fn now(&self) -> u64 {
        0
    }
}

/// A WASI context with no ambient nondeterminism: frozen clocks, fixed
/// "random" bytes and seed, no args, env, preopens or captured output.
pub(super) fn pure_wasi() -> wasmtime_wasi::WasiCtx {
    const BYTES: [u8; 8] = [0x52, 0x41, 0x49, 0x53, 0x49, 0x4e, 0x44, 0x42];
    WasiCtxBuilder::new()
        .wall_clock(FrozenClock)
        .monotonic_clock(FrozenClock)
        .secure_random(Deterministic::new(BYTES.to_vec()))
        .insecure_random(Deterministic::new(BYTES.to_vec()))
        .insecure_random_seed(0)
        .build()
}

/// The host-call policy a generic execution policy maps to.
pub(super) fn host_policy(policy: ExecutionPolicy) -> HostPolicy {
    if policy.allows_host_calls() {
        HostPolicy::Allow
    } else {
        HostPolicy::DenyAll
    }
}
