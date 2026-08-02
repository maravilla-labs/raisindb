//! Running a capture from a synchronous store method.
//!
//! The stores (`AdminUserStore`, `ApiKeyStore`) are synchronous, but capture is
//! `async`. The long-standing pattern here is `block_in_place` +
//! `Handle::block_on`, which preserves **capture order**: two rapid updates to
//! the same record capture in the order they were written, so the HLCs they are
//! stamped with order the same way and LWW converges to the later write.
//!
//! Both of those calls panic outside a multi-threaded runtime, though —
//! `Handle::current` with no reactor at all, `block_in_place` on a
//! current-thread runtime. That was harmless while capture was never actually
//! enabled for these stores; now that it is, a panic here would take down an
//! admin login rather than lose a replication op. So this degrades instead.

use std::future::Future;

use raisin_replication::Operation;

/// Run a capture future from synchronous code, without ever panicking.
///
/// `what` names the operation for logs. Failures are logged, never propagated:
/// the local write has already succeeded by this point, and failing it now
/// would be worse than converging late.
pub(crate) fn run_capture<F>(what: &str, fut: F)
where
    F: Future<Output = raisin_error::Result<Operation>> + Send + 'static,
{
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::error!(
            operation = what,
            "no tokio runtime available to capture a replication operation; \
             this write exists onttthis node only"
        );
        return;
    };

    match handle.runtime_flavor() {
        tokio::runtime::RuntimeFlavor::CurrentThread => {
            // Cannot block here. Spawning keeps the write replicating, but two
            // captures issued back to back may be stamped out of order, so this
            // is a fallback rather than the intended path.
            tracing::warn!(
                operation = what,
                "capturing on a current-thread runtime; capture ordering is not guaranteed"
            );
            let what = what.to_string();
            handle.spawn(async move {
                if let Err(e) = fut.await {
                    tracing::error!(operation = %what, error = %e, "failed to capture operation");
                }
            });
        }
        _ => {
            tokio::task::block_in_place(|| {
                if let Err(e) = handle.block_on(fut) {
                    tracing::error!(operation = what, error = %e, "failed to capture operation");
                }
            });
        }
    }
}
