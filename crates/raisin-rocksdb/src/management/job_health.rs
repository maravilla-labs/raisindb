//! The wire projections behind the two job-health routes, and nothing else.
//!
//! Its own module because these three strings — `closed`, `open`, `half_open` —
//! are a contract with the admin console, not an implementation detail of the
//! breaker. `BreakerState` may gain a variant or be renamed; the console must
//! keep rendering the same words until someone changes both sides on purpose.
//! Kept apart from `background_jobs_impl` so that contract can be tested
//! without standing up a storage.

use crate::jobs::fair::TenantQueueStats;
use crate::jobs::{BreakerState, BreakerStatus};
use raisin_storage::jobs::JobCategory;
use raisin_storage::{BreakerHealth, TenantQueueDepth, TenantQueueHealth};

/// Project one breaker's live state onto the operator-facing shape.
pub(crate) fn breaker_health(status: BreakerStatus) -> BreakerHealth {
    BreakerHealth {
        key: status.key,
        state: state_name(status.state).to_string(),
        consecutive_failures: status.consecutive_failures,
        last_error: status.last_error,
        next_probe_in_secs: status.next_probe_in.map(|remaining| {
            // Round UP. A breaker with 400ms left would otherwise report
            // "next probe in 0s" while still OPEN, which reads as a stall —
            // the opposite of what it means.
            remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
        }),
    }
}

/// Project the fair scheduler's per-tenant depths onto the operator shape.
///
/// Rows with nothing queued are DROPPED. The scheduler removes a tenant's queue
/// the moment it empties, so a zero row can only come from a race; emitting it
/// would make the list grow with every tenant this process has ever served
/// rather than with what is actually waiting.
pub(crate) fn tenant_queue_health(
    per_category: Vec<(JobCategory, Vec<TenantQueueStats>)>,
) -> Vec<TenantQueueHealth> {
    let mut out: Vec<TenantQueueHealth> = per_category
        .into_iter()
        .flat_map(|(category, tenants)| {
            let category = category.to_string();
            tenants.into_iter().map(move |t| TenantQueueHealth {
                tenant: t.tenant,
                category: category.clone(),
                weight: t.weight,
                depth_high: t.depth_high,
                depth_normal: t.depth_normal,
                depth_low: t.depth_low,
                depth_total: t.depth_high + t.depth_normal + t.depth_low,
            })
        })
        .filter(|row| row.depth_total > 0)
        .collect();
    // Deterministic ordering, so a dashboard row does not jump between polls.
    out.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.tenant.cmp(&b.tenant))
    });
    out
}

/// One tenant's own queued work, summed across every category pool.
///
/// The categories are summed rather than reported separately because the
/// distinction (realtime vs background vs system) is a scheduling concept of
/// OURS — a tenant cannot choose a pool, so breaking the number down by pool
/// would describe our machine rather than their backlog.
pub(crate) fn tenant_queue_depth(
    per_category: &[(JobCategory, Vec<TenantQueueStats>)],
    tenant: &str,
) -> TenantQueueDepth {
    let mut depth = TenantQueueDepth::default();
    for (_category, tenants) in per_category {
        for t in tenants.iter().filter(|t| t.tenant == tenant) {
            depth.high += t.depth_high;
            depth.normal += t.depth_normal;
            depth.low += t.depth_low;
        }
    }
    depth.total = depth.high + depth.normal + depth.low;
    depth
}

fn state_name(state: BreakerState) -> &'static str {
    match state {
        BreakerState::Closed => "closed",
        BreakerState::Open => "open",
        BreakerState::HalfOpen => "half_open",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn status(state: BreakerState, next_probe_in: Option<Duration>) -> BreakerStatus {
        BreakerStatus {
            key: "embeddings:openai".to_string(),
            state,
            consecutive_failures: 5,
            last_error: Some("503".to_string()),
            next_probe_in,
        }
    }

    #[test]
    fn state_names_are_the_console_contract() {
        assert_eq!(state_name(BreakerState::Closed), "closed");
        assert_eq!(state_name(BreakerState::Open), "open");
        assert_eq!(state_name(BreakerState::HalfOpen), "half_open");
    }

    #[test]
    fn a_sub_second_cooldown_never_reports_zero() {
        // An open breaker reporting "0s" is indistinguishable from a stuck one.
        let health = breaker_health(status(BreakerState::Open, Some(Duration::from_millis(400))));
        assert_eq!(health.next_probe_in_secs, Some(1));

        let health = breaker_health(status(
            BreakerState::Open,
            Some(Duration::from_millis(30_001)),
        ));
        assert_eq!(health.next_probe_in_secs, Some(31));
    }

    fn tenant_stats(tenant: &str, h: usize, n: usize, l: usize) -> TenantQueueStats {
        TenantQueueStats {
            tenant: tenant.to_string(),
            weight: 1,
            depth_high: h,
            depth_normal: n,
            depth_low: l,
        }
    }

    #[test]
    fn a_tenants_depth_sums_its_pools_and_ignores_everyone_else() {
        // The leak this asserts against: summing every row would report the
        // whole host's backlog to whoever asked.
        let per_category = vec![
            (
                JobCategory::Realtime,
                vec![
                    tenant_stats("acme", 1, 2, 0),
                    tenant_stats("other", 9, 9, 9),
                ],
            ),
            (JobCategory::Background, vec![tenant_stats("acme", 0, 4, 3)]),
        ];
        let depth = tenant_queue_depth(&per_category, "acme");
        assert_eq!((depth.high, depth.normal, depth.low), (1, 6, 3));
        assert_eq!(depth.total, 10);

        // A tenant the scheduler has never seen is empty, not an error.
        let depth = tenant_queue_depth(&per_category, "nobody");
        assert_eq!(depth.total, 0);
    }

    #[test]
    fn empty_tenant_queues_produce_no_operator_rows() {
        let rows = tenant_queue_health(vec![
            (
                JobCategory::Background,
                vec![tenant_stats("b", 0, 0, 0), tenant_stats("a", 0, 5, 0)],
            ),
            (JobCategory::Realtime, vec![tenant_stats("a", 1, 0, 0)]),
        ]);
        let seen: Vec<(&str, &str, usize)> = rows
            .iter()
            .map(|r| (r.category.as_str(), r.tenant.as_str(), r.depth_total))
            .collect();
        assert_eq!(
            seen,
            vec![("background", "a", 5), ("realtime", "a", 1)],
            "zero-depth rows must be dropped and the order must be stable"
        );
    }

    #[test]
    fn a_closed_breaker_has_no_probe_time() {
        let health = breaker_health(status(BreakerState::Closed, None));
        assert_eq!(health.state, "closed");
        assert_eq!(health.next_probe_in_secs, None);
    }
}
