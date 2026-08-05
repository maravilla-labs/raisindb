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
}
