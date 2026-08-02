// SPDX-License-Identifier: BSL-1.1

//! Graceful-shutdown plumbing for WebSocket connections.
//!
//! ## Why this exists
//!
//! `axum::serve(...).with_graceful_shutdown(...)` stops accepting new
//! connections and then waits for every *open* connection to finish — and an
//! upgraded WebSocket never finishes on its own. One idle browser tab therefore
//! held the drain open until the supervisor's timeout fired `SIGKILL`, and a
//! `SIGKILL` means RocksDB is never closed cleanly: the un-flushed WAL is
//! replayed on the *next* boot, so the unclean stop manufactures a slow start.
//!
//! The policy implemented here is: **idle sockets must not hold shutdown, but
//! in-flight work must not be cut off.** On the shutdown signal every
//! connection task stops reading new requests, waits a *bounded* grace for the
//! requests it already started (a node write, a SQL DML, a job enqueue) to
//! finish and have their responses flushed, sends a proper Close frame
//! (`1001 Going Away`, so browsers see a clean close and reconnect instead of a
//! broken pipe) and returns — releasing the connection so axum's drain can
//! complete.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use tokio::sync::Notify;

/// Close code sent to clients when the server is shutting down.
///
/// 1001 "Going Away" is the code browsers treat as "the peer is leaving";
/// clients reconnect on it instead of reporting a transport error.
pub const CLOSE_CODE_GOING_AWAY: u16 = 1001;

/// Counter of requests a connection has started but not yet finished.
///
/// Requests are dispatched onto detached tasks (see `handler::socket`), so
/// nothing else tracks them. Each spawned request holds an [`InFlightGuard`];
/// when the shutdown signal fires the connection loop waits (bounded) for the
/// count to reach zero before closing the socket.
#[derive(Debug, Default)]
pub struct InFlight {
    count: AtomicUsize,
    idle: Notify,
}

impl InFlight {
    /// Create a new, idle tracker.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register one operation as in flight. The returned guard decrements on
    /// drop, including on panic or cancellation of the task holding it.
    pub fn guard(self: &Arc<Self>) -> InFlightGuard {
        self.count.fetch_add(1, Ordering::AcqRel);
        InFlightGuard {
            inner: Arc::clone(self),
        }
    }

    /// Number of operations currently in flight.
    pub fn count(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }

    /// Wait until nothing is in flight, or until `grace` elapses.
    ///
    /// Returns `true` if the tracker went idle within the grace period,
    /// `false` if the deadline was hit with work still outstanding. The
    /// bound is the whole point: an unbounded wait is exactly the behaviour
    /// this module removes.
    pub async fn wait_idle(&self, grace: Duration) -> bool {
        tokio::time::timeout(grace, async {
            loop {
                // Arm the notification *before* re-reading the counter, or a
                // completion landing between the two would be missed and the
                // wait would stall until the grace deadline.
                let idle = self.idle.notified();
                tokio::pin!(idle);
                if self.count() == 0 {
                    return;
                }
                idle.await;
            }
        })
        .await
        .is_ok()
    }
}

/// RAII marker for one in-flight operation. See [`InFlight::guard`].
#[derive(Debug)]
pub struct InFlightGuard {
    inner: Arc<InFlight>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if self.inner.count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.idle.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wait_idle_returns_immediately_when_nothing_is_in_flight() {
        let tracker = InFlight::new();
        let start = std::time::Instant::now();
        assert!(tracker.wait_idle(Duration::from_secs(5)).await);
        assert!(start.elapsed() < Duration::from_millis(200));
    }

    #[tokio::test]
    async fn wait_idle_waits_for_an_in_flight_operation_then_returns() {
        let tracker = InFlight::new();
        let guard = tracker.guard();
        assert_eq!(tracker.count(), 1);

        let releaser = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(guard);
        });

        assert!(tracker.wait_idle(Duration::from_secs(5)).await);
        assert_eq!(tracker.count(), 0);
        releaser.await.unwrap();
    }

    #[tokio::test]
    async fn wait_idle_gives_up_at_the_grace_deadline() {
        let tracker = InFlight::new();
        let _guard = tracker.guard();
        assert!(!tracker.wait_idle(Duration::from_millis(50)).await);
        assert_eq!(tracker.count(), 1);
    }
}
