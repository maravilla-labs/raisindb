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

//! Model usage of one function execution, collected while it runs.
//!
//! An agent's TOOL may call a model itself (a tool that drafts and checks an
//! answer). Those calls are made inside the function runtime, which knows
//! nothing about the conversation the tool serves, so they reached no cost
//! record and no budget. The caller that does know (the agent run's tool
//! executor) [`track`]s the execution, the runtime's AI callback [`add`]s to
//! it, and the caller [`TrackedUsage::take`]s the totals and records them.
//!
//! Keyed by the execution id the SERVER mints for that call (the run's
//! `"{op_id}#{attempt}"`); no client or function names it. Adding is the only
//! thing the runtime can do with a key: the totals are read only through the
//! guard [`track`] returned. The guard removes its entry when dropped, so a
//! panic, a timeout or an early return cannot leave one behind.
//!
//! Sits here, not in raisin-functions, because both the runtime
//! (raisin-functions) and the run executor (raisin-rocksdb) depend on this
//! crate and not on each other, like the conversation broadcaster.

use dashmap::DashMap;
use once_cell::sync::Lazy;

/// One model call's usage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelCall {
    /// Model id as the provider reported it.
    pub model: String,
    /// Input (prompt) tokens.
    pub input_tokens: u64,
    /// Output (completion) tokens.
    pub output_tokens: u64,
}

/// What an execution's model calls used, in call order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageTotals {
    /// Each call.
    pub calls: Vec<ModelCall>,
}

impl UsageTotals {
    /// Input tokens over all calls.
    pub fn input_tokens(&self) -> u64 {
        self.calls.iter().map(|c| c.input_tokens).sum()
    }
    /// Output tokens over all calls.
    pub fn output_tokens(&self) -> u64 {
        self.calls.iter().map(|c| c.output_tokens).sum()
    }
    /// Input + output over all calls.
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens() + self.output_tokens()
    }
}

static TRACKED: Lazy<DashMap<String, UsageTotals>> = Lazy::new(DashMap::new);

/// Collect the model usage of execution `execution_id` until the guard drops.
/// An id already tracked is not taken over: the second guard collects nothing
/// and leaves the first one's entry alone.
pub fn track(execution_id: &str) -> TrackedUsage {
    let owns = match TRACKED.entry(execution_id.to_string()) {
        dashmap::mapref::entry::Entry::Occupied(_) => false,
        dashmap::mapref::entry::Entry::Vacant(v) => {
            v.insert(UsageTotals::default());
            true
        }
    };
    TrackedUsage {
        execution_id: execution_id.to_string(),
        owns,
    }
}

/// Whether `execution_id` is being tracked (the runtime wraps its AI
/// callback only then).
pub fn is_tracked(execution_id: &str) -> bool {
    TRACKED.contains_key(execution_id)
}

/// Add one model call to `execution_id`, if it is tracked; otherwise nothing.
pub fn add(execution_id: &str, call: ModelCall) {
    if let Some(mut totals) = TRACKED.get_mut(execution_id) {
        totals.calls.push(call);
    }
}

/// The handle on one tracked execution. Dropping it ends the tracking.
#[derive(Debug)]
pub struct TrackedUsage {
    execution_id: String,
    owns: bool,
}

impl TrackedUsage {
    /// The usage so far, and the end of the tracking.
    pub fn take(self) -> UsageTotals {
        if !self.owns {
            return UsageTotals::default();
        }
        TRACKED
            .remove(&self.execution_id)
            .map(|(_, t)| t)
            .unwrap_or_default()
    }
}

impl Drop for TrackedUsage {
    fn drop(&mut self) {
        if self.owns {
            TRACKED.remove(&self.execution_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(model: &str, i: u64, o: u64) -> ModelCall {
        ModelCall {
            model: model.into(),
            input_tokens: i,
            output_tokens: o,
        }
    }

    #[test]
    fn a_tracked_execution_collects_its_calls() {
        let g = track("ai-usage-test-1#0");
        assert!(is_tracked("ai-usage-test-1#0"));
        add("ai-usage-test-1#0", call("m", 100, 20));
        add("ai-usage-test-1#0", call("m", 50, 5));
        let t = g.take();
        assert_eq!(t.calls.len(), 2);
        assert_eq!(t.input_tokens(), 150);
        assert_eq!(t.output_tokens(), 25);
        assert_eq!(t.total_tokens(), 175);
        assert!(!is_tracked("ai-usage-test-1#0"), "take ends the tracking");
    }

    #[test]
    fn an_untracked_id_collects_nothing() {
        add("ai-usage-test-2#0", call("m", 100, 20));
        assert!(!is_tracked("ai-usage-test-2#0"));
        let g = track("ai-usage-test-2#0");
        assert_eq!(g.take(), UsageTotals::default(), "nothing from before");
    }

    #[test]
    fn dropping_the_guard_removes_the_entry_even_on_panic() {
        let _ = std::panic::catch_unwind(|| {
            let _g = track("ai-usage-test-3#0");
            add("ai-usage-test-3#0", call("m", 1, 1));
            panic!("tool blew up");
        });
        assert!(!is_tracked("ai-usage-test-3#0"));
        drop(track("ai-usage-test-4#0"));
        assert!(!is_tracked("ai-usage-test-4#0"));
    }

    #[test]
    fn a_second_tracker_of_the_same_id_cannot_take_or_end_the_first() {
        let first = track("ai-usage-test-5#0");
        let second = track("ai-usage-test-5#0");
        add("ai-usage-test-5#0", call("m", 10, 1));
        assert_eq!(second.take(), UsageTotals::default());
        assert!(is_tracked("ai-usage-test-5#0"), "the first still owns it");
        assert_eq!(first.take().total_tokens(), 11);
    }
}
