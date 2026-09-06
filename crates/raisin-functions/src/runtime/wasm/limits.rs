// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Per-store resource limiter.
//!
//! Two jobs, and the second is why this is not `StoreLimits`: it enforces the
//! function's `max_memory_bytes`, and it REMEMBERS that it refused. A refused
//! `memory.grow` returns -1 to the guest, whose allocator then aborts, and the
//! trap that reaches us says `unreachable` — indistinguishable from any other
//! guest panic. Without [`FunctionLimiter::denied_memory_growth`] an
//! out-of-memory function would be reported as a generic runtime error.
//!
//! # The budget is STORE-wide, not per memory
//!
//! wasmtime calls [`ResourceLimiter::memory_growing`] once per linear memory,
//! with that memory's own `current`/`desired`, and a component may hold
//! several core modules with a memory each ([`FunctionLimiter::memories`]
//! allows 8). Comparing each request against the cap independently would let a
//! hand-built component reach 8 x `max_memory_bytes` while the store reported
//! one memory's worth — the semaphore's "N concurrent stores x
//! max_memory_bytes" promise (`runtime_impl.rs`) would be out by that factor.
//! So every growth is charged as a DELTA (`desired - current`) against one
//! running total, and it is the total that is capped and reported. Tables are
//! accounted the same way, for the same reason.

use wasmtime::ResourceLimiter;

/// Store-wide ceiling on table elements, across every table in the store.
///
/// Tables hold function references, not guest data, so this is a sanity
/// ceiling rather than a tenant-visible budget: at a pointer per element it
/// bounds the host allocation a malformed component can provoke at ~8 MiB.
const MAX_TABLE_ELEMENTS: usize = 1 << 20;

/// Store-scoped limiter for one function execution.
#[derive(Debug)]
pub struct FunctionLimiter {
    max_memory_bytes: usize,
    /// Sum of every linear memory's approved size, in bytes.
    memory_in_use: usize,
    peak_memory: usize,
    denied_memory_growth: bool,
    /// Sum of every table's approved element count.
    table_elements_in_use: usize,
    max_tables: usize,
}

impl FunctionLimiter {
    /// A limiter capped at `max_memory_bytes` of linear memory, summed across
    /// every memory the store's component instantiates.
    pub fn new(max_memory_bytes: usize) -> Self {
        Self {
            max_memory_bytes,
            memory_in_use: 0,
            peak_memory: 0,
            denied_memory_growth: false,
            table_elements_in_use: 0,
            max_tables: 16,
        }
    }

    /// Largest TOTAL linear-memory size this store was allowed to reach, in
    /// bytes — the sum over its memories, not the largest single one.
    pub fn peak_memory(&self) -> usize {
        self.peak_memory
    }

    /// Whether a memory growth was refused because of the cap. Load-bearing
    /// for error classification — see the module docs.
    pub fn denied_memory_growth(&self) -> bool {
        self.denied_memory_growth
    }

    /// The cap this limiter enforces, in bytes.
    pub fn max_memory_bytes(&self) -> usize {
        self.max_memory_bytes
    }

    /// Table elements approved across every table in this store.
    pub fn table_elements(&self) -> usize {
        self.table_elements_in_use
    }
}

impl ResourceLimiter for FunctionLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        // `current` is THIS memory's current size (wasmtime 47
        // `vm/memory.rs:346,652`: `0 -> minimum` at instantiation, then
        // `old_byte_size -> new_byte_size`), so the delta is what the store is
        // being asked to hold on top of everything already approved.
        let delta = desired.saturating_sub(current);
        let requested_total = self.memory_in_use.saturating_add(delta);

        if requested_total > self.max_memory_bytes || maximum.is_some_and(|m| desired > m) {
            self.denied_memory_growth = true;
            return Ok(false);
        }

        self.memory_in_use = requested_total;
        self.peak_memory = self.peak_memory.max(requested_total);
        Ok(true)
    }

    // NOTE: `memory_grow_failed` is deliberately NOT implemented to refund the
    // charge. wasmtime also calls it on a path where `memory_growing` was
    // never consulted (`vm/memory.rs:640-647`, growth beyond the memory type's
    // own limits), so a refund there would return budget that was never spent
    // — and under-counting is the direction that lets a store exceed its cap.
    // Over-counting after a failed OS allocation only makes the limiter
    // stricter for the rest of one execution, which then ends.

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let delta = desired.saturating_sub(current);
        let requested_total = self.table_elements_in_use.saturating_add(delta);
        if requested_total > MAX_TABLE_ELEMENTS || maximum.is_some_and(|m| desired > m) {
            return Ok(false);
        }
        self.table_elements_in_use = requested_total;
        Ok(true)
    }

    fn instances(&self) -> usize {
        // CORE instances, not components: one component is several of them
        // (the guest module, wit-bindgen's adapter, wasi-libc's) and a jco
        // build has more still. A store only ever instantiates ONE component,
        // so this is a sanity ceiling, not the isolation boundary.
        64
    }

    fn tables(&self) -> usize {
        self.max_tables
    }

    fn memories(&self) -> usize {
        // A component may hold more than one core module, each with its own
        // memory. That is why the byte cap above is charged store-wide: this
        // number bounds how many memories exist, never how much they hold.
        8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_beyond_the_cap_is_refused_and_remembered() {
        let mut limiter = FunctionLimiter::new(4 * 1024 * 1024);
        assert!(limiter.memory_growing(0, 1024 * 1024, None).unwrap());
        assert_eq!(limiter.peak_memory(), 1024 * 1024);
        assert!(!limiter.denied_memory_growth());

        assert!(!limiter
            .memory_growing(1024 * 1024, 8 * 1024 * 1024, None)
            .unwrap());
        assert!(limiter.denied_memory_growth());
        // A refusal must not move the peak: the growth did not happen.
        assert_eq!(limiter.peak_memory(), 1024 * 1024);
    }

    #[test]
    fn an_engine_supplied_maximum_is_honoured_too() {
        let mut limiter = FunctionLimiter::new(usize::MAX);
        assert!(!limiter.memory_growing(0, 1024, Some(512)).unwrap());
        assert!(limiter.denied_memory_growth());
    }

    #[test]
    fn several_memories_share_one_budget() {
        const MIB: usize = 1024 * 1024;
        let mut limiter = FunctionLimiter::new(4 * MIB);

        // Four core modules, each instantiating a 1 MiB memory: the store is
        // now full even though no single memory is near the cap.
        for _ in 0..4 {
            assert!(limiter.memory_growing(0, MIB, None).unwrap());
        }
        assert_eq!(limiter.peak_memory(), 4 * MIB);
        assert!(!limiter.denied_memory_growth());

        // A fifth memory of the same size — trivially under the cap on its own
        // — is refused, because the STORE has no room.
        assert!(!limiter.memory_growing(0, MIB, None).unwrap());
        assert!(limiter.denied_memory_growth());
        assert_eq!(limiter.peak_memory(), 4 * MIB);
    }

    #[test]
    fn a_growth_is_charged_as_a_delta_not_a_size() {
        const MIB: usize = 1024 * 1024;
        let mut limiter = FunctionLimiter::new(4 * MIB);

        assert!(limiter.memory_growing(0, MIB, None).unwrap());
        // The same memory grows 1 -> 3 MiB: two more, not three.
        assert!(limiter.memory_growing(MIB, 3 * MIB, None).unwrap());
        assert_eq!(limiter.peak_memory(), 3 * MIB);
        // So exactly 1 MiB is left for a second memory.
        assert!(limiter.memory_growing(0, MIB, None).unwrap());
        assert_eq!(limiter.peak_memory(), 4 * MIB);
        assert!(!limiter.memory_growing(0, 1, None).unwrap());
    }

    #[test]
    fn tables_share_one_element_budget_too() {
        let mut limiter = FunctionLimiter::new(usize::MAX);
        assert!(limiter
            .table_growing(0, MAX_TABLE_ELEMENTS - 1, None)
            .unwrap());
        assert_eq!(limiter.table_elements(), MAX_TABLE_ELEMENTS - 1);
        // A second table asking for two more elements than remain.
        assert!(!limiter.table_growing(0, 2, None).unwrap());
        assert!(limiter.table_growing(0, 1, None).unwrap());
        assert_eq!(limiter.table_elements(), MAX_TABLE_ELEMENTS);
    }
}
