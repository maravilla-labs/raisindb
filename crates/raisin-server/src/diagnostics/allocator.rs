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

//! jemalloc counters and process-level memory, read from inside the process.
//!
//! # Why these specific numbers
//!
//! The point of this module is to tell three very different causes of RSS
//! growth apart without changing any code first. They have disjoint signatures:
//!
//! | cause | `allocated` | `resident` / `retained` | RocksDB properties |
//! |---|---|---|---|
//! | a Rust-heap leak (a map that never evicts) | climbs | climbs | flat |
//! | allocator retention / fragmentation | **flat** | climbs | flat |
//! | RocksDB (table readers, memtables) | flat | climbs | climb with it |
//!
//! `allocated` is the live Rust heap. It is the discriminator: a genuine leak
//! moves it, and pure allocator retention does not. RocksDB's C++ heap does not
//! go through Rust's global allocator at all, so it shows up in process RSS and
//! in the RocksDB properties but never in `allocated` — which is why
//! [`super::rocksdb_props`] must be sampled alongside this.
//!
//! Everything here is `None` off Linux: `tikv-jemallocator` is only built with
//! the `stats` feature on the Linux target (see `Cargo.toml`), so a macOS dev
//! build genuinely cannot answer. That is expected, not a failure.

use serde::Serialize;

/// A single allocator + process memory sample.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AllocatorStats {
    /// Bytes allocated by the application and not yet freed — the live Rust
    /// heap. **The leak discriminator**: allocator retention alone leaves this
    /// flat.
    pub allocated_bytes: Option<u64>,
    /// Bytes in active pages. Slightly above `allocated` by definition.
    pub active_bytes: Option<u64>,
    /// Bytes of physical memory jemalloc has mapped. Tracks RSS closely; the
    /// gap between this and `allocated` IS the retention/fragmentation.
    pub resident_bytes: Option<u64>,
    /// Bytes in the virtual address space jemalloc has mapped.
    pub mapped_bytes: Option<u64>,
    /// Bytes of virtual memory retained (mapped but returned to the OS as
    /// unused). Rising `retained` with flat `allocated` is the fragmentation
    /// signature.
    pub retained_bytes: Option<u64>,
    /// Bytes jemalloc uses for its own metadata.
    pub metadata_bytes: Option<u64>,

    /// Process resident set size, from the OS rather than the allocator.
    /// Compare against `resident_bytes`: a large gap means the growth is
    /// **outside** the Rust allocator (RocksDB, Tantivy mmaps, thread stacks).
    pub process_rss_bytes: Option<u64>,
    /// Process virtual size, from the OS.
    pub process_vsz_bytes: Option<u64>,

    /// Why the allocator fields are empty, when they are.
    pub unavailable_reason: Option<String>,
}

/// Sample jemalloc's counters and the process's memory.
pub fn sample() -> AllocatorStats {
    let mut stats = AllocatorStats {
        process_rss_bytes: process_rss_bytes(),
        process_vsz_bytes: process_vsz_bytes(),
        ..Default::default()
    };
    read_jemalloc(&mut stats);
    stats
}

#[cfg(target_os = "linux")]
fn read_jemalloc(stats: &mut AllocatorStats) {
    use tikv_jemalloc_ctl::{epoch, stats as jstats};

    // jemalloc's statistics are cached and only refreshed when the epoch is
    // advanced. Skip this and every sample returns the values from process
    // start — a flat line that looks like "no growth" and is simply stale.
    if let Err(e) = epoch::advance() {
        stats.unavailable_reason = Some(format!("jemalloc epoch advance failed: {e}"));
        return;
    }

    stats.allocated_bytes = jstats::allocated::read().ok().map(|v| v as u64);
    stats.active_bytes = jstats::active::read().ok().map(|v| v as u64);
    stats.resident_bytes = jstats::resident::read().ok().map(|v| v as u64);
    stats.mapped_bytes = jstats::mapped::read().ok().map(|v| v as u64);
    stats.retained_bytes = jstats::retained::read().ok().map(|v| v as u64);
    stats.metadata_bytes = jstats::metadata::read().ok().map(|v| v as u64);
}

#[cfg(not(target_os = "linux"))]
fn read_jemalloc(stats: &mut AllocatorStats) {
    stats.unavailable_reason = Some(
        "jemalloc stats are compiled in on the Linux target only; \
         this build cannot read them"
            .to_string(),
    );
}

/// Resident set size from `/proc/self/statm` (field 2, in pages).
#[cfg(target_os = "linux")]
fn process_rss_bytes() -> Option<u64> {
    statm_field(1).map(|pages| pages * page_size())
}

/// Virtual size from `/proc/self/statm` (field 1, in pages).
#[cfg(target_os = "linux")]
fn process_vsz_bytes() -> Option<u64> {
    statm_field(0).map(|pages| pages * page_size())
}

#[cfg(target_os = "linux")]
fn statm_field(index: usize) -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    statm.split_whitespace().nth(index)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn page_size() -> u64 {
    // SAFETY: `sysconf` with a valid name is always safe; a negative return is
    // handled below.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size > 0 {
        size as u64
    } else {
        4096
    }
}

#[cfg(not(target_os = "linux"))]
fn process_rss_bytes() -> Option<u64> {
    None
}

#[cfg(not(target_os = "linux"))]
fn process_vsz_bytes() -> Option<u64> {
    None
}

/// Dump jemalloc's full human-readable statistics.
///
/// This is the measurement that distinguishes "retained but free" from
/// "genuinely held" — per-arena dirty/muzzy page counts and bin fragmentation.
/// Before this existed the only way to obtain it was
/// `_RJEM_MALLOC_CONF=stats_print:true` plus a deliberate process stop, i.e.
/// an outage.
#[cfg(target_os = "linux")]
pub fn malloc_stats_text() -> Result<String, String> {
    use std::ffi::c_void;
    use std::os::raw::c_char;

    unsafe extern "C" fn collect(opaque: *mut c_void, chunk: *const c_char) {
        // SAFETY: `opaque` is the `&mut String` handed to `malloc_stats_print`
        // below, and `chunk` is a NUL-terminated C string owned by jemalloc for
        // the duration of this call.
        unsafe {
            let out = &mut *(opaque as *mut String);
            if let Ok(text) = std::ffi::CStr::from_ptr(chunk).to_str() {
                out.push_str(text);
            }
        }
    }

    let mut out = String::new();
    // SAFETY: `collect` matches the expected callback ABI and `out` outlives
    // the call. A NULL `opts` selects jemalloc's default (full) report.
    unsafe {
        tikv_jemalloc_sys::malloc_stats_print(
            Some(collect),
            &mut out as *mut String as *mut c_void,
            std::ptr::null(),
        );
    }

    if out.is_empty() {
        Err("jemalloc produced no statistics output".to_string())
    } else {
        Ok(out)
    }
}

#[cfg(not(target_os = "linux"))]
pub fn malloc_stats_text() -> Result<String, String> {
    Err("jemalloc stats are compiled in on the Linux target only".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sampling must never panic and must always say something, even where the
    /// allocator cannot be read — a diagnostics endpoint that 500s during an
    /// incident is worse than useless.
    #[test]
    fn sampling_always_returns_something() {
        let stats = sample();
        if stats.allocated_bytes.is_none() {
            assert!(
                stats.unavailable_reason.is_some(),
                "an empty sample must explain itself"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_reports_live_heap_and_rss() {
        let stats = sample();
        assert!(stats.allocated_bytes.unwrap_or(0) > 0);
        assert!(stats.process_rss_bytes.unwrap_or(0) > 0);
    }
}
