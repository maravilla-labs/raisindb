//! Trigger circuit breaker.
//!
//! A single tenant's buggy or malicious trigger/function code must never be
//! able to grow the job registry without bound — that's what happened in
//! production when a trigger's failure handler rewrote the same node it was
//! triggered by, which re-fired the same trigger on every retry, forever.
//! The dispatcher's priority queues don't help here: `register_job_with_id`
//! (which persists into RocksDB) always runs before dispatch is attempted,
//! and a full dispatch queue only logs a warning — it never un-registers the
//! job. So the breaker has to sit at the point a trigger is *about* to be
//! turned into a job, before registration.
//!
//! Two independent, complementary checks, both keyed off information already
//! available at that point (`enqueue_trigger_match` in `handler.rs`):
//!
//! - **Fire-rate**: how many times has this exact (tenant, repo, trigger,
//!   node) fired in the last window? Catches a single trigger looping on
//!   itself — the actual incident shape.
//! - **Per-node fire budget**: how many trigger fires (any trigger) has this
//!   node accumulated in the last window? Catches multi-trigger cascades
//!   (A fires B fires A fires B...) without needing to thread a causal chain
//!   through arbitrary function writes across crate boundaries — a much
//!   larger, riskier change to the write path that a same-node fire budget
//!   makes unnecessary for this class of bug.
//!
//! In-memory only, deliberately — mirrors [`super::super::fulltext::error_counter::FulltextErrorCounter`]'s
//! reasoning: the question this answers is "is this tenant runaway *right
//! now*", and a process restart is itself an operator intervention that
//! should give a tenant a clean window rather than replaying stale state.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Server-wide (not per-tenant, not yet configurable via `TenantLimits` —
/// that's a Phase-2+ follow-up) trigger safety configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TriggerSafetyConfig {
    /// Master switch. When false, both checks always allow.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Max fires of one (tenant, repo, trigger, node) within the window.
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_window: usize,
    /// Absolute ceiling on `rate_limit_per_window` — reserved for a future
    /// per-tenant override so no tenant-supplied config can raise the limit
    /// past this value.
    #[serde(default = "default_hard_ceiling")]
    pub rate_limit_hard_ceiling: usize,
    /// Max total trigger fires (any trigger) for one (tenant, repo, node)
    /// within the window.
    #[serde(default = "default_node_fire_budget")]
    pub node_fire_budget: usize,
    /// Sliding window, in seconds, for both checks.
    #[serde(default = "default_window_secs")]
    pub window_secs: u64,
}

fn default_enabled() -> bool {
    true
}
fn default_rate_limit() -> usize {
    500
}
fn default_hard_ceiling() -> usize {
    1000
}
fn default_node_fire_budget() -> usize {
    1000
}
fn default_window_secs() -> u64 {
    1
}

impl TriggerSafetyConfig {
    fn window(&self) -> Duration {
        Duration::from_secs(self.window_secs.max(1))
    }
}

// Defaults are scoped per (tenant, repo, trigger, node) — NOT a tenant-wide
// cap. A legitimate high-volume tenant can have thousands of different nodes
// each firing triggers concurrently; this only bounds how many times *the
// same trigger* fires on *the same node* per second, which even at 500/s is
// already many orders of magnitude above any real authoring/ingest pattern.
// A true runaway loop (the production incident) fires as fast as the worker
// pool can retry — far beyond this — so raising the ceiling here costs
// nothing in protection while eliminating any risk of throttling legitimate
// bulk writers. Tenant-wide throughput shaping (if ever needed) is a
// separate, later fair-share concern, not this breaker's job.
impl Default for TriggerSafetyConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            rate_limit_per_window: default_rate_limit(),
            rate_limit_hard_ceiling: default_hard_ceiling(),
            node_fire_budget: default_node_fire_budget(),
            window_secs: default_window_secs(),
        }
    }
}

/// Why a fire was blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakerTripReason {
    /// One (tenant, repo, trigger, node) fired too often.
    TriggerRateLimit,
    /// One node accumulated too many fires across all its triggers.
    NodeFireBudget,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TriggerBreakerStats {
    pub rate_limit_trips: u64,
    pub node_budget_trips: u64,
    pub last_trip_trigger: Option<String>,
    pub last_trip_reason: Option<BreakerTripReason>,
    pub last_trip_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TriggerKey {
    tenant_id: String,
    repo_id: String,
    trigger_name: String,
    node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct NodeKey {
    tenant_id: String,
    repo_id: String,
    node_id: String,
}

#[derive(Default)]
struct FireWindow {
    timestamps: Vec<Instant>,
}

impl FireWindow {
    /// Drop timestamps outside `window`, then return the remaining count.
    fn prune(&mut self, now: Instant, window: Duration) -> usize {
        self.timestamps.retain(|t| now.duration_since(*t) < window);
        self.timestamps.len()
    }
}

/// Process-wide trigger circuit breaker. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct TriggerBreaker {
    config: TriggerSafetyConfig,
    trigger_windows: Arc<RwLock<HashMap<TriggerKey, FireWindow>>>,
    node_windows: Arc<RwLock<HashMap<NodeKey, FireWindow>>>,
    stats: Arc<RwLock<HashMap<String, TriggerBreakerStats>>>, // keyed by tenant_id
}

impl TriggerBreaker {
    pub fn new(config: TriggerSafetyConfig) -> Self {
        Self {
            config,
            trigger_windows: Arc::new(RwLock::new(HashMap::new())),
            node_windows: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check both the fire-rate and node-fire-budget limits for a trigger
    /// about to be turned into a job, recording the fire if allowed.
    ///
    /// Returns `Some(reason)` if the fire must be blocked (caller should
    /// skip enqueueing and return the existing "trigger skipped" contract).
    pub async fn check_and_record(
        &self,
        tenant_id: &str,
        repo_id: &str,
        trigger_name: &str,
        node_id: &str,
    ) -> Option<BreakerTripReason> {
        if !self.config.enabled {
            return None;
        }

        let now = Instant::now();
        let effective_rate_limit = self
            .config
            .rate_limit_per_window
            .min(self.config.rate_limit_hard_ceiling);

        // Check the per-trigger rate first — it's the more specific signal.
        {
            let key = TriggerKey {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                trigger_name: trigger_name.to_string(),
                node_id: node_id.to_string(),
            };
            let mut windows = self.trigger_windows.write().await;
            let window = windows.entry(key).or_default();
            let count = window.prune(now, self.config.window());
            if count >= effective_rate_limit {
                self.record_trip(tenant_id, trigger_name, BreakerTripReason::TriggerRateLimit)
                    .await;
                return Some(BreakerTripReason::TriggerRateLimit);
            }
        }

        // Then the per-node cross-trigger budget.
        {
            let key = NodeKey {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                node_id: node_id.to_string(),
            };
            let mut windows = self.node_windows.write().await;
            let window = windows.entry(key).or_default();
            let count = window.prune(now, self.config.window());
            if count >= self.config.node_fire_budget {
                self.record_trip(tenant_id, trigger_name, BreakerTripReason::NodeFireBudget)
                    .await;
                return Some(BreakerTripReason::NodeFireBudget);
            }
        }

        // Both checks passed — record this fire in both windows.
        {
            let key = TriggerKey {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                trigger_name: trigger_name.to_string(),
                node_id: node_id.to_string(),
            };
            self.trigger_windows
                .write()
                .await
                .entry(key)
                .or_default()
                .timestamps
                .push(now);
        }
        {
            let key = NodeKey {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                node_id: node_id.to_string(),
            };
            self.node_windows
                .write()
                .await
                .entry(key)
                .or_default()
                .timestamps
                .push(now);
        }

        None
    }

    async fn record_trip(&self, tenant_id: &str, trigger_name: &str, reason: BreakerTripReason) {
        let mut stats = self.stats.write().await;
        let entry = stats.entry(tenant_id.to_string()).or_default();
        match reason {
            BreakerTripReason::TriggerRateLimit => entry.rate_limit_trips += 1,
            BreakerTripReason::NodeFireBudget => entry.node_budget_trips += 1,
        }
        entry.last_trip_trigger = Some(trigger_name.to_string());
        entry.last_trip_reason = Some(reason);
        entry.last_trip_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        );

        tracing::warn!(
            tenant_id,
            trigger_name,
            reason = ?reason,
            "Trigger circuit breaker tripped, skipping job enqueue"
        );
    }

    /// Snapshot the trip stats for one tenant. All-zero default if the
    /// breaker has never tripped for that tenant.
    pub async fn snapshot(&self, tenant_id: &str) -> TriggerBreakerStats {
        self.stats
            .read()
            .await
            .get(tenant_id)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_config(rate_limit: usize, node_budget: usize) -> TriggerSafetyConfig {
        TriggerSafetyConfig {
            enabled: true,
            rate_limit_per_window: rate_limit,
            rate_limit_hard_ceiling: rate_limit.max(1000),
            node_fire_budget: node_budget,
            window_secs: 60,
        }
    }

    #[tokio::test]
    async fn allows_fires_under_the_limit() {
        let breaker = TriggerBreaker::new(fast_config(5, 100));
        for _ in 0..5 {
            assert_eq!(
                breaker.check_and_record("t", "r", "trig", "node-1").await,
                None
            );
        }
    }

    #[tokio::test]
    async fn trips_rate_limit_when_one_trigger_refires_on_the_same_node() {
        let breaker = TriggerBreaker::new(fast_config(3, 1000));
        for _ in 0..3 {
            assert_eq!(
                breaker.check_and_record("t", "r", "trig", "node-1").await,
                None
            );
        }
        assert_eq!(
            breaker.check_and_record("t", "r", "trig", "node-1").await,
            Some(BreakerTripReason::TriggerRateLimit)
        );

        let stats = breaker.snapshot("t").await;
        assert_eq!(stats.rate_limit_trips, 1);
        assert_eq!(stats.last_trip_trigger.as_deref(), Some("trig"));
    }

    #[tokio::test]
    async fn trips_node_budget_across_multiple_distinct_triggers() {
        // Rate limit per-trigger is generous; the node-wide budget is not,
        // so a cascade of DIFFERENT triggers on the same node still trips.
        let breaker = TriggerBreaker::new(fast_config(1000, 4));
        assert_eq!(
            breaker.check_and_record("t", "r", "trig-a", "node-1").await,
            None
        );
        assert_eq!(
            breaker.check_and_record("t", "r", "trig-b", "node-1").await,
            None
        );
        assert_eq!(
            breaker.check_and_record("t", "r", "trig-a", "node-1").await,
            None
        );
        assert_eq!(
            breaker.check_and_record("t", "r", "trig-b", "node-1").await,
            None
        );
        assert_eq!(
            breaker.check_and_record("t", "r", "trig-a", "node-1").await,
            Some(BreakerTripReason::NodeFireBudget)
        );

        let stats = breaker.snapshot("t").await;
        assert_eq!(stats.node_budget_trips, 1);
    }

    #[tokio::test]
    async fn tenants_and_nodes_are_isolated() {
        let breaker = TriggerBreaker::new(fast_config(1, 1000));
        assert_eq!(
            breaker
                .check_and_record("tenant-a", "r", "trig", "node-1")
                .await,
            None
        );
        // Different tenant, same trigger/node name: independent window.
        assert_eq!(
            breaker
                .check_and_record("tenant-b", "r", "trig", "node-1")
                .await,
            None
        );
        // Same tenant, different node: independent window.
        assert_eq!(
            breaker
                .check_and_record("tenant-a", "r", "trig", "node-2")
                .await,
            None
        );
        // Back to the first (tenant, trigger, node): now over the limit.
        assert_eq!(
            breaker
                .check_and_record("tenant-a", "r", "trig", "node-1")
                .await,
            Some(BreakerTripReason::TriggerRateLimit)
        );
    }

    #[tokio::test]
    async fn disabled_breaker_always_allows() {
        let mut config = fast_config(1, 1);
        config.enabled = false;
        let breaker = TriggerBreaker::new(config);
        for _ in 0..10 {
            assert_eq!(
                breaker.check_and_record("t", "r", "trig", "node-1").await,
                None
            );
        }
    }

    #[tokio::test]
    async fn hard_ceiling_wins_even_if_configured_limit_is_higher() {
        // rate_limit_per_window (100) exceeds rate_limit_hard_ceiling (2) —
        // the effective limit must be the ceiling.
        let breaker = TriggerBreaker::new(TriggerSafetyConfig {
            enabled: true,
            rate_limit_per_window: 100,
            rate_limit_hard_ceiling: 2,
            node_fire_budget: 1000,
            window_secs: 60,
        });
        assert_eq!(
            breaker.check_and_record("t", "r", "trig", "node-1").await,
            None
        );
        assert_eq!(
            breaker.check_and_record("t", "r", "trig", "node-1").await,
            None
        );
        assert_eq!(
            breaker.check_and_record("t", "r", "trig", "node-1").await,
            Some(BreakerTripReason::TriggerRateLimit)
        );
    }
}
