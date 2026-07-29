//! Read and write operations for the persistent audit log.

use std::sync::atomic::Ordering;

use raisin_audit::{AuditRepository, AuditScope};
use raisin_error::Result;
use raisin_models::nodes::audit_log::AuditLog;
use tracing::{debug, error, warn};

use super::{RocksDBAuditRepo, DEFAULT_AUDIT_READ_LIMIT};
use crate::keys;

impl RocksDBAuditRepo {
    /// Persist one entry under `scope`.
    fn put_scoped(&self, scope: AuditScope<'_>, log: &AuditLog) -> Result<()> {
        let cf = self.cf_audit()?;

        let key = keys::audit_entry_key(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
            &log.node_id,
            log.timestamp.timestamp_millis(),
            self.seq.fetch_add(1, Ordering::Relaxed),
        );

        // `to_vec_named` (map-encoded) rather than the positional `to_vec` used
        // by some records: adding a field to `AuditLog` later must not shift the
        // meaning of already-written entries.
        let bytes = rmp_serde::to_vec_named(log)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        self.db.put_cf(cf, &key, &bytes).map_err(|e| {
            error!("Failed to write audit log for node {}: {}", log.node_id, e);
            raisin_error::Error::storage(format!("Failed to write audit log: {}", e))
        })?;

        debug!(
            node_id = %log.node_id,
            tenant = %scope.tenant_id,
            repo = %scope.repo_id,
            branch = %scope.branch,
            "Wrote audit log entry"
        );
        Ok(())
    }

    /// Read a node's entries under `scope`, newest first.
    fn scan_scoped(
        &self,
        scope: AuditScope<'_>,
        node_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<AuditLog>> {
        let cf = self.cf_audit()?;
        // Clamp, don't just default: a caller-supplied limit (WS audit_query
        // forwards one verbatim) would otherwise scan and serialize a node's
        // entire history into one frame — exactly what the cap exists to stop.
        let limit = limit
            .unwrap_or(DEFAULT_AUDIT_READ_LIMIT)
            .min(DEFAULT_AUDIT_READ_LIMIT);
        if limit == 0 {
            return Ok(Vec::new());
        }

        let prefix = keys::audit_node_prefix(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
            node_id,
        );

        let mut logs = Vec::new();
        // `prefix_iterator_cf` seeks to the prefix but keeps reading past it, so
        // the `starts_with` re-check below is what actually bounds the scan.
        for item in self.db.prefix_iterator_cf(cf, &prefix) {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix) {
                break;
            }

            // A single unreadable record must not blank out the whole history —
            // audit reads are a forensic surface, so surface what we can and
            // leave a trace of what we could not.
            match rmp_serde::from_slice::<AuditLog>(&value) {
                Ok(log) => logs.push(log),
                Err(e) => {
                    warn!(
                        node_id = %node_id,
                        error = %e,
                        "Skipping undecodable audit log entry"
                    );
                }
            }

            if logs.len() >= limit {
                break;
            }
        }

        // Already newest-first: the inverted timestamp/seq suffix puts the most
        // recent entry lowest in key order, so iteration order IS the result
        // order. No sort here on purpose.
        Ok(logs)
    }
}

impl AuditRepository for RocksDBAuditRepo {
    /// Write without a scope.
    ///
    /// Only reachable from callers that predate the scoped API. Such an entry
    /// lands in a reserved `_unscoped` bucket that no scoped read can see, so
    /// this is effectively a no-op for retrieval — the production writer
    /// (`AuditEventHandler`) always has the full scope and uses
    /// [`AuditRepository::write_log_scoped`].
    async fn write_log(&self, log: AuditLog) -> Result<()> {
        warn!(
            node_id = %log.node_id,
            "Unscoped audit write; entry will not be visible to scoped reads"
        );
        let workspace = log.workspace.clone();
        self.put_scoped(
            AuditScope::new("_unscoped", "_unscoped", "_unscoped", &workspace),
            &log,
        )
    }

    /// Read without a scope.
    ///
    /// A persistent store cannot answer this: entries are keyed by tenant / repo
    /// / branch / workspace and there is no global index over node id. Every
    /// real reader (both HTTP audit routes, the `audit_log` node command, and
    /// the WS `audit_query`) holds the scope and uses
    /// [`AuditRepository::get_logs_scoped`].
    async fn get_logs_by_node_id(&self, node_id: &str) -> Result<Vec<AuditLog>> {
        warn!(
            node_id = %node_id,
            "Unscoped audit read on the persistent store; returning empty"
        );
        Ok(Vec::new())
    }

    async fn write_log_scoped(&self, scope: AuditScope<'_>, log: AuditLog) -> Result<()> {
        self.put_scoped(scope, &log)
    }

    async fn get_logs_scoped(
        &self,
        scope: AuditScope<'_>,
        node_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<AuditLog>> {
        self.scan_scoped(scope, node_id, limit)
    }
}
