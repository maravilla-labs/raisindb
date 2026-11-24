//! Request isolation: heavy scans run in a bounded pool, point reads never
//! wait behind them (scale item 6).
//!
//! A query whose plan reads an unbounded or large part of a workspace (a
//! table scan, an unbounded prefix / type / range / order scan, a key count)
//! takes a permit from a process-wide pool before it runs and holds it until
//! its row stream ends or is dropped. Point lookups and bounded index reads
//! never take one. Under load a handful of heavy scans therefore occupy at most
//! `RAISIN_SQL_HEAVY_CONCURRENCY` (default: half the cores, at least 2) of the
//! runtime, instead of every worker, and a 7 ms point read is not queued
//! behind four 3 s scans.
//!
//! A heavy query that cannot get a permit within `RAISIN_SQL_HEAVY_WAIT_MS`
//! (default 2,000 ms) is refused at once with a retryable error — cheaply,
//! instead of joining the queue and spending its own scan budget later.

use crate::physical_plan::executor::RowStream;
use crate::physical_plan::operators::PhysicalPlan;
use futures::StreamExt;
use raisin_error::Error;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// A scan bounded at or below this many rows is not heavy.
const LIGHT_LIMIT: usize = 1_000;

fn pool() -> &'static Arc<Semaphore> {
    static POOL: OnceLock<Arc<Semaphore>> = OnceLock::new();
    POOL.get_or_init(|| {
        let default = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).max(2))
            .unwrap_or(2);
        let size = std::env::var("RAISIN_SQL_HEAVY_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(default);
        Arc::new(Semaphore::new(size))
    })
}

fn wait() -> Duration {
    let ms = std::env::var("RAISIN_SQL_HEAVY_WAIT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2_000);
    Duration::from_millis(ms)
}

fn large(limit: Option<usize>) -> bool {
    limit.is_none_or(|l| l > LIGHT_LIMIT)
}

/// Whether running `plan` may read an unbounded or large part of a workspace.
pub(crate) fn is_heavy(plan: &PhysicalPlan) -> bool {
    let leaf_heavy = match plan {
        PhysicalPlan::TableScan { .. } | PhysicalPlan::CountScan { .. } => true,
        PhysicalPlan::PrefixScan { limit, .. }
        | PhysicalPlan::PropertyIndexScan { limit, .. }
        | PhysicalPlan::PropertyRangeScan { limit, .. } => large(*limit),
        PhysicalPlan::PropertyOrderScan { limit, .. } => *limit > LIGHT_LIMIT,
        _ => false,
    };
    leaf_heavy || plan.inputs().into_iter().any(is_heavy)
}

/// A permit for a heavy plan (`None` for a light one), or a retryable
/// refusal when the pool stays full.
pub(crate) async fn admit(plan: &PhysicalPlan) -> Result<Option<OwnedSemaphorePermit>, Error> {
    admit_within(plan, wait()).await
}

async fn admit_within(
    plan: &PhysicalPlan,
    wait: Duration,
) -> Result<Option<OwnedSemaphorePermit>, Error> {
    if !is_heavy(plan) {
        return Ok(None);
    }
    match tokio::time::timeout(wait, pool().clone().acquire_owned()).await {
        Ok(Ok(permit)) => Ok(Some(permit)),
        Ok(Err(_)) => Ok(None), // closed: never happens, run unpooled
        // The same refusal a scan that ran out of budget gives (clients
        // already treat it as "narrow or retry"), without spending one.
        Err(_) => Err(Error::ScanBudgetExceeded {
            scanned: 0,
            elapsed_ms: wait.as_millis() as u64,
            limit: "the pool for heavy queries stayed full; retry shortly, or narrow the \
                    query with an indexed filter or a LIMIT"
                .to_string(),
        }),
    }
}

/// `stream`, holding `permit` until it ends or is dropped.
pub(crate) fn hold(stream: RowStream, permit: Option<OwnedSemaphorePermit>) -> RowStream {
    match permit {
        None => stream,
        Some(permit) => Box::pin(async_stream::stream! {
            let _permit = permit;
            let mut stream = stream;
            while let Some(item) = stream.next().await {
                yield item;
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_scan() -> PhysicalPlan {
        PhysicalPlan::CountScan {
            tenant_id: "t".into(),
            repo_id: "r".into(),
            branch: "main".into(),
            workspace: "w".into(),
            max_revision: None,
        }
    }

    #[test]
    fn heavy_leaves_are_found_anywhere_in_the_plan() {
        assert!(!is_heavy(&PhysicalPlan::Empty));
        assert!(is_heavy(&count_scan()));
        // Wrapped in a Limit, a heavy scan is still heavy.
        assert!(is_heavy(&PhysicalPlan::Limit {
            input: Box::new(count_scan()),
            limit: 5,
            offset: 0,
        }));
        assert!(!is_heavy(&PhysicalPlan::Limit {
            input: Box::new(PhysicalPlan::Empty),
            limit: 5,
            offset: 0,
        }));
        assert!(large(None) && large(Some(5_001)) && !large(Some(101)));
    }

    /// A full pool refuses a heavy query after the wait, and a light one is
    /// admitted without a permit meanwhile.
    #[tokio::test]
    async fn a_full_pool_refuses_heavy_and_admits_light() {
        let all = pool().available_permits();
        let held = pool().clone().acquire_many_owned(all as u32).await.unwrap();
        let refused = admit_within(&count_scan(), Duration::from_millis(50)).await;
        assert!(
            matches!(refused, Err(Error::ScanBudgetExceeded { .. })),
            "{refused:?}"
        );
        assert!(admit(&PhysicalPlan::Empty).await.unwrap().is_none());
        drop(held);
        let permit = admit(&count_scan()).await.unwrap();
        assert!(permit.is_some());
    }
}
