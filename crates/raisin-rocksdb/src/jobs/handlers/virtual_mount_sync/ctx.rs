//! The per-run context handed to every phase.

use super::*;

/// Lease TTL for a per-mount sync run.
pub(super) const SYNC_LEASE_TTL: Duration = Duration::from_secs(600);

/// How long a run may walk before it must stop itself at a page boundary.
///
/// Comfortably under the job's own `timeout_seconds` (600s for
/// `VirtualMountSync`), because being reaped by the watchdog is not a graceful
/// end: it aborts the task at its current await point, so `finalize` never runs,
/// the status stays `"syncing"` forever and the lease leaks for its full TTL.
/// Stopping ourselves first turns that into an ordinary truncated chunk — resume
/// point persisted, status terminal, lease released — and the next tick carries
/// on. A large mailbox therefore imports across many clean runs instead of being
/// killed at the same point every time.
pub(super) const SYNC_WALL_CLOCK_BUDGET: Duration = Duration::from_secs(480);

/// The cluster-wide lease key for one mount.
///
/// ONE definition, because every writer of this mount's state has to name the
/// same key or the lease protects nothing. It is keyed on the CONFIG branch —
/// the identity the periodic scan enqueues against, and where the state blob it
/// guards actually lives — not on the materialization target branch.
///
/// Both the sync run and the write-reconcile walk take it, and that is
/// deliberate rather than incidental: they both write [`MountState`], whose
/// `state_seq` guard makes the loser of a race skip *every remaining write of
/// its run*. A reconcile pass that ran beside a sync would not merely lose its
/// own watermark, it would poison the sync's cursor writes for the rest of the
/// run. Sharing the lease makes that race unrepresentable; the cost is that a
/// mount in a long backfill reconciles on a later tick instead.
pub(super) fn mount_lease_key(
    tenant: &str,
    repo: &str,
    config_branch: &str,
    mount_id: &str,
) -> String {
    format!("{tenant}\0{repo}\0{config_branch}\0vmount:{mount_id}")
}

/// Everything a delta/full run needs. Borrows the invoker + materializer.
pub struct SyncCtx<'a> {
    pub storage: Arc<RocksDBStorage>,
    pub scope: MountScope,
    /// The repo's config (default) branch — where the mount/integration config
    /// and the persisted mount state live. Distinct from `scope.branch`, which
    /// is the materialization target branch.
    pub config_branch: String,
    pub mount: MountConfig,
    pub adapter_path: String,
    pub invoker: &'a dyn AdapterInvoker,
    pub materializer: &'a dyn NodeMaterializer,
    pub lock_manager: Option<LockManagerHandle>,
    pub lock_key: String,
    /// Lease handle for THIS run, used only to renew and release the lock.
    ///
    /// Run-local on purpose: it is a volatile capability, not durable state, and
    /// must never be persisted or compared against stored data. The write guard
    /// is [`MountState::state_seq`].
    pub lease_token: Option<u64>,
    pub credential: Option<Value>,
    pub mount_snapshot: Value,
    /// Public origin for this connector's callback URLs, taken from the
    /// operator's stored OAuth `redirect_uri`. See
    /// [`IntegrationConfig::public_origin`].
    pub public_origin: Option<String>,
    /// Unix epoch seconds after which the walk must stop itself at the next page
    /// boundary. See [`SYNC_WALL_CLOCK_BUDGET`].
    pub deadline: i64,
    /// The write mode this run resolved, if it has resolved one.
    ///
    /// Set by `run_sync` after the capabilities/mapper probes; never set for
    /// contexts that do not drain (subscription teardown/renew, tests). Read
    /// by `write::drain_pending` so page boundaries can push local edits noted
    /// while this run holds the lease — without threading the mode through
    /// every phase signature. A `OnceLock` because the batcher already borrows
    /// the context by the time the mode is resolved.
    pub write_mode: std::sync::OnceLock<write::WriteMode>,
}

impl SyncCtx<'_> {
    /// Whether this run has used up its wall-clock budget.
    ///
    /// Asked only where [`stop_requested`] already is — after a flush, with the
    /// cursor about to be persisted — so stopping here loses nothing.
    pub fn out_of_time(&self, now: i64) -> bool {
        now >= self.deadline
    }
}

impl SyncCtx<'_> {
    /// Invoke the adapter for one operation.
    pub async fn call(
        &self,
        operation: &str,
        params: Value,
    ) -> std::result::Result<Value, AdapterError> {
        let input = build_input(operation, params, &self.credential, &self.mount_snapshot);
        self.invoker
            .invoke(&self.scope, &self.adapter_path, input)
            .await
    }

    /// Renew the lease between pages during long runs (no-op without a lock).
    pub async fn renew_lease(&self) {
        if let (Some(lm), Some(t)) = (&self.lock_manager, self.lease_token) {
            let _ = lm.renew(&self.lock_key, t, SYNC_LEASE_TTL).await;
        }
    }

    /// Resolve the adapter's capabilities, falling back conservatively (read
    /// only) when the adapter has no `capabilities` operation, returns a
    /// non-object, or throws. Never fails the sync.
    pub async fn resolve_capabilities(&self) -> Capabilities {
        match self.call("capabilities", json!({})).await {
            Ok(v) => Capabilities::from_adapter_value(&v).unwrap_or_else(|| {
                tracing::warn!(
                    mount_id = %self.scope.mount_id,
                    "adapter returned no usable capabilities; assuming read-only"
                );
                Capabilities::fallback()
            }),
            Err(e) => {
                tracing::warn!(
                    mount_id = %self.scope.mount_id,
                    error = %e,
                    "capabilities probe failed; assuming read-only"
                );
                Capabilities::fallback()
            }
        }
    }

    /// Ask this mount's mapper, once per run, whether it can translate nodes
    /// outward. Never fails the sync: a mapper that throws is reported as
    /// unable to write, not as a broken mount.
    ///
    /// The probe is its own operation (`mapper_capabilities`) rather than a
    /// `to_external` call with a null node. Probing with a null node would work
    /// against every mapper shipped today — they all open with
    /// `if (!item …) return null` — but it would then oblige every future
    /// `to_external` to tolerate a null node forever, and a strict one that
    /// threw on the probe would be misreported as read-only.
    pub async fn probe_mapper_writeback(&self) -> MapperWriteback {
        let Some(mapper) = &self.mount.mapping_function else {
            // Read-only by construction, stated rather than incidental: the
            // built-in Rust `default_mapping` is lossy, so inverting it would be
            // guessing. No mapper, no writes — and no invocation to find out.
            return MapperWriteback::NoMapper;
        };
        let input = json!({
            "operation": "mapper_capabilities",
            "mount": self.mount_snapshot,
        });
        match self.invoker.invoke(&self.scope, mapper, input).await {
            Ok(v) => {
                if v.get("to_external").and_then(|v| v.as_bool()) == Some(true) {
                    MapperWriteback::Supported
                } else {
                    MapperWriteback::NotImplemented
                }
            }
            Err(e) => {
                tracing::warn!(
                    mount_id = %self.scope.mount_id,
                    error = %e,
                    "mapper writeback probe failed; treating the mount as read-only"
                );
                MapperWriteback::ProbeFailed(e.to_string())
            }
        }
    }
}
