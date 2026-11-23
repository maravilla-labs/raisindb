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

//! Cancelling a function execution from outside it.
//!
//! An agent run's operation (a model turn, a tool) is a function execution,
//! and a stop must reach the I/O that execution is waiting on — otherwise the
//! provider request keeps running, and keeps being paid for, after the person
//! pressed stop. The caller that owns the cancellation (the run's operation
//! executor) [`register`]s its token under the execution id the SERVER minted;
//! the runtime looks it up for that execution ([`token_for`]) and hands it to
//! its host gateway, whose every call gives up when it fires.
//!
//! Same shape and same reason for living here as [`super::ai_usage`]: both the
//! runtime and the run executor depend on this crate, not on each other.

use dashmap::DashMap;
use once_cell::sync::Lazy;
use tokio_util::sync::CancellationToken;

static TOKENS: Lazy<DashMap<String, CancellationToken>> = Lazy::new(DashMap::new);

/// Make `token` the cancellation of execution `execution_id` until the guard
/// drops. An id already registered is not taken over.
pub fn register(execution_id: &str, token: CancellationToken) -> Registered {
    let owns = match TOKENS.entry(execution_id.to_string()) {
        dashmap::mapref::entry::Entry::Occupied(_) => false,
        dashmap::mapref::entry::Entry::Vacant(v) => {
            v.insert(token);
            true
        }
    };
    Registered {
        execution_id: execution_id.to_string(),
        owns,
    }
}

/// The token registered for `execution_id`, if any.
pub fn token_for(execution_id: &str) -> Option<CancellationToken> {
    TOKENS.get(execution_id).map(|t| t.clone())
}

/// The handle on one registration. Dropping it removes the registration.
#[derive(Debug)]
pub struct Registered {
    execution_id: String,
    owns: bool,
}

impl Drop for Registered {
    fn drop(&mut self) {
        if self.owns {
            TOKENS.remove(&self.execution_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_token_is_found_until_its_guard_drops() {
        let token = CancellationToken::new();
        let guard = register("op-1#1", token.clone());
        token.cancel();
        assert!(token_for("op-1#1").expect("registered").is_cancelled());
        assert!(token_for("other#1").is_none());
        drop(guard);
        assert!(token_for("op-1#1").is_none());
    }

    #[test]
    fn a_second_registration_does_not_take_over() {
        let first = CancellationToken::new();
        let _g1 = register("op-2#1", first.clone());
        let g2 = register("op-2#1", CancellationToken::new());
        drop(g2);
        first.cancel();
        assert!(token_for("op-2#1").unwrap().is_cancelled());
    }
}
