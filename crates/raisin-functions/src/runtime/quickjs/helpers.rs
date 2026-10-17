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

/// Run an async host call in a blocking context, bounded by the execution's
/// wall-clock `deadline`.
///
/// A host call is awaited INSIDE the interpreter's poll, so neither the tokio
/// timeout around the whole execution (it only fires between polls) nor the
/// interrupt handler (it only runs while bytecode executes) can cut a call
/// that never returns — a silent SMTP server, a stuck provider. The call
/// itself therefore carries the remaining execution time; when it elapses the
/// future is dropped (cancelling the I/O) and the function gets a timeout
/// error. The interrupt handler, whose deadline has passed by then, ends the
/// execution as a timeout at the next bytecode.
pub(super) fn run_async_blocking_until<F, T>(
    future: F,
    deadline: std::time::Instant,
    what: &str,
) -> std::result::Result<T, raisin_error::Error>
where
    F: std::future::Future<Output = T>,
{
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            tokio::time::timeout(remaining, future).await.map_err(|_| {
                raisin_error::Error::Internal(format!(
                    "function timed out during host call `{what}`"
                ))
            })
        })
    })
}
