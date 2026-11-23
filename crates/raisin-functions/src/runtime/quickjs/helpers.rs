// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Helper functions for the QuickJS runtime.
//!
//! Provides async-to-sync bridging utilities.

/// Default timeout for external HTTP fetch requests (2 minutes).
///
/// Used by the W3C `fetch()` polyfill backend (`api_fetch.rs`). The
/// `raisin.http.fetch` timeout lives in the shared registry invoker
/// (`bindings/methods/http.rs`) so both runtimes get it.
pub(super) const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Run an async function in a blocking context.
///
/// This is used to bridge between the synchronous JS runtime and async Rust code.
/// Uses tokio's block_in_place to avoid blocking the async runtime.
pub(super) fn run_async_blocking<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}

/// Run an async function in a blocking context with a timeout.
///
/// Used for external HTTP fetch requests to prevent indefinite blocking when
/// the target service is unresponsive. Returns an error if the future does not
/// complete within the timeout duration.
pub(super) fn run_async_blocking_with_timeout<F, T>(
    future: F,
    timeout: std::time::Duration,
) -> std::result::Result<T, raisin_error::Error>
where
    F: std::future::Future<Output = T>,
{
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            tokio::time::timeout(timeout, future).await.map_err(|_| {
                raisin_error::Error::Internal(format!(
                    "HTTP request timed out after {}s",
                    timeout.as_secs()
                ))
            })
        })
    })
}
