//! The causal-stability watermark: below it, nothing will be inserted again.
//!
//! **Single node**: the branch HEAD, bounded by `min_age` of wall clock. HEAD
//! alone is not enough: a transaction allocates its revision before it
//! commits, and the commit path's monotonic guard lets one that committed
//! after HEAD moved past it land BELOW HEAD. `min_age` (default ten minutes,
//! the same floor retention GC uses for in-flight work) covers that window.
//!
//! **Cluster**: the watermark would be the minimum HLC every peer has applied
//! and acknowledged. Nothing records that today — a push `Ack` is logged and
//! dropped (`raisin-replication/src/coordinator/push.rs`), and
//! `raisin_replication::PeerWatermarks` is an in-memory op-sequence map that
//! no server path feeds. A replicated op older than any local guess may still
//! arrive (a peer that was down, a catch-up), and it would land inside a run
//! collapse already shortened. So in cluster mode collapse REFUSES to run.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use std::time::Duration;

/// The refusal message for cluster mode.
pub const CLUSTER_REFUSAL: &str = "collapse_runs refused: this node replicates, and there is no \
     trustworthy cluster-wide causal-stability watermark (peer acknowledgements are not \
     persisted), so an older replicated op could still land inside a collapsed run";

/// The watermark for one branch: `min(head, now - min_age, cap)`; `None` when
/// the branch has no HEAD (nothing to collapse). Errors in cluster mode.
pub fn collapse_watermark(
    head: Option<HLC>,
    now_ms: u64,
    min_age: Duration,
    cap: Option<HLC>,
    cluster_mode: bool,
) -> Result<Option<HLC>> {
    if cluster_mode {
        return Err(Error::Validation(CLUSTER_REFUSAL.to_string()));
    }
    let Some(head) = head else {
        return Ok(None);
    };
    let aged = HLC::new(now_ms.saturating_sub(min_age.as_millis() as u64), u64::MAX);
    let mut watermark = head.min(aged);
    if let Some(cap) = cap {
        watermark = watermark.min(cap);
    }
    Ok(Some(watermark))
}

/// Milliseconds since the epoch.
pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_mode_refuses() {
        let err = collapse_watermark(Some(HLC::new(5, 0)), 10, Duration::ZERO, None, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("watermark"), "{err}");
    }

    #[test]
    fn the_lowest_bound_wins() {
        let head = HLC::new(1_000_000, 3);
        let at = |cap, age_ms| {
            collapse_watermark(
                Some(head),
                1_000_500,
                Duration::from_millis(age_ms),
                cap,
                false,
            )
            .unwrap()
            .unwrap()
        };
        assert_eq!(at(None, 0), head);
        assert_eq!(at(None, 10_000), HLC::new(990_500, u64::MAX));
        assert_eq!(at(Some(HLC::new(7, 0)), 0), HLC::new(7, 0));
        assert!(collapse_watermark(None, 1, Duration::ZERO, None, false)
            .unwrap()
            .is_none());
    }
}
