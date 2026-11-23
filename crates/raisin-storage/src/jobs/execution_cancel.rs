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
//! the runtime picks it up for that execution ([`scope`]) and every host call
//! made inside reads it back ([`current`]) and gives up when it fires.
//!
//! Same shape and same reason for living here as [`super::ai_usage`]: both the
//! runtime and the run executor depend on this crate, not on each other.

use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::future::Future;
use tokio_util::sync::CancellationToken;

static TOKENS: Lazy<DashMap<String, CancellationToken>> = Lazy::new(DashMap::new);

tokio::task_local! {
    static CURRENT: CancellationToken;
}

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

/// Run `fut` with `execution_id`'s token (if one is registered) as the
/// [`current`] one.
pub async fn scope<F: Future>(execution_id: &str, fut: F) -> F::Output {
    match token_for(execution_id) {
        Some(token) => CURRENT.scope(token, fut).await,
        None => fut.await,
    }
}

/// The cancellation of the execution being polled, if it has one. Readable
/// from synchronous code called during that poll (a host call).
pub fn current() -> Option<CancellationToken> {
    CURRENT.try_with(|t| t.clone()).ok()
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

    #[tokio::test]
    async fn a_registered_token_is_current_inside_its_scope_only() {
        let token = CancellationToken::new();
        let guard = register("op-1#1", token.clone());
        assert!(current().is_none());
        let seen = scope("op-1#1", async { current() }).await;
        token.cancel();
        assert!(seen.expect("current inside the scope").is_cancelled());
        assert!(scope("other#1", async { current() }).await.is_none());
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
