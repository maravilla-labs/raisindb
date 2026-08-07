//! Reading and writing the mount's `state` blob, and the webhook delivery
//! counters that live alongside it.

use super::*;

/// Persist the mount's `state` back to its `raisin:system` node, honoring the
/// fencing-token rule: a write whose fencing token is older than the one
/// already stored is skipped (a GC-stalled sync must not clobber a newer
/// cursor). Returns `true` if the state was written.
/// Record the outcome of one inbound push notification on the mount.
///
/// Webhook DELIVERY health is deliberately separate from subscription health:
/// `push_status: "active"` only means the provider accepted our URL at
/// subscribe time. A mount can hold a live subscription while every actual
/// notification is turned away, and that combination — green badge, zero
/// deliveries — is invisible without this.
///
/// Reads the current state, stamps the counters, writes it back through the
/// normal seq-guarded path so it cannot clobber a concurrent sync's write.
pub async fn record_push_delivery(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    mount_id: &str,
    accepted: bool,
) -> Result<bool> {
    // Read through the module's system-auth reader: an unauthenticated
    // NodeService read is silently denied by RLS and reports as "mount node
    // not found on this branch", which is a lie — the node is there, the read
    // was refused. That mislabel cost a full branch-resolution investigation.
    let Some(mut state) = read_mount_state(storage, tenant, repo, branch, mount_id).await? else {
        tracing::warn!(
            mount_id = %mount_id,
            branch = %branch,
            accepted,
            "skipping push-delivery counter write: mount node not found on this branch"
        );
        return Ok(false);
    };

    let now = Utc::now().timestamp();
    if accepted {
        state.push_last_delivery_at = Some(now);
        state.push_deliveries_ok = state.push_deliveries_ok.saturating_add(1);
    } else {
        state.push_last_rejected_at = Some(now);
        state.push_deliveries_rejected = state.push_deliveries_rejected.saturating_add(1);
        state.push_last_rejected_reason =
            Some("notification secret not present in any carrier".to_string());
    }

    persist_mount_state(storage, tenant, repo, branch, mount_id, &mut state).await
}

/// Why a state write did not land.
///
/// `persist_mount_state` answers both with the same `Ok(false)`, and the two
/// could not be more different: a missing node is a mount that does not exist
/// on this branch (routine — the config lives elsewhere, and a caller may
/// legitimately carry on), while a superseded write means ANOTHER WRITER owns
/// this mount right now. Only the second makes continuing dangerous, and
/// collapsing them is why the full walk could not react to either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StateWrite {
    Written,
    /// No mount node on this branch. Nothing was written and nothing is wrong.
    MountMissing,
    /// The stored `state_seq` has moved past ours: someone else is writing.
    ///
    /// `persist_mount_state` does NOT advance our seq on this path, so every
    /// later write in this run is refused too.
    Superseded,
}

impl StateWrite {
    pub(super) fn is_written(self) -> bool {
        self == StateWrite::Written
    }
}

/// Persist mount state, reporting whether — and why not — it landed.
///
/// Callers must not discard this. Silently dropping it is what let a sync
/// materialize 500 nodes and then throw away its own resume cursor, with the
/// console still reporting `status: "ok"`.
pub(super) async fn persist_state(ctx: &SyncCtx<'_>, state: &mut MountState) -> Result<StateWrite> {
    // Mount state lives with the mount config on the CONFIG branch, never on the
    // materialization target branch.
    persist_mount_state_detailed(
        &ctx.storage,
        &ctx.scope.tenant,
        &ctx.scope.repo,
        &ctx.config_branch,
        &ctx.scope.mount_id,
        state,
    )
    .await
}

/// Whether an operator has asked the running sync to stop.
///
/// Re-read from the store rather than trusted from the in-memory copy: a stop
/// arrives *while* this run is in flight, so the value the run started with is
/// always stale.
///
/// Deliberately cooperative. Aborting the task instead would skip the lease
/// release on both exit paths, leaving the mount locked for the full
/// `SYNC_LEASE_TTL` with `status` stuck at `"syncing"` — a phantom run the
/// console has no way to clear. Callers ask this only where they have just
/// flushed and persisted, so a stop always lands on a clean boundary: no
/// in-flight batch, cursor durable.
pub async fn stop_requested(ctx: &SyncCtx<'_>) -> bool {
    let read = read_mount_state(
        &ctx.storage,
        &ctx.scope.tenant,
        &ctx.scope.repo,
        &ctx.config_branch,
        &ctx.scope.mount_id,
    )
    .await;
    match read {
        Ok(Some(state)) => state.stop_requested,
        // An unreadable state is NOT a stop. Guessing "yes" would abandon a
        // healthy multi-hour import on one transient read failure.
        _ => false,
    }
}

/// Read the persisted mount state, if the node is present on this branch.
pub async fn read_mount_state(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    mount_id: &str,
) -> Result<Option<MountState>> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(tenant, repo)?;
    tx.set_branch(branch)?;
    tx.set_auth_context(AuthContext::system())?;

    let Some(node) = tx.get_node(SYSTEM_WORKSPACE, mount_id).await? else {
        return Ok(None);
    };
    // Same parse the full config uses, so the two can never disagree — including
    // its refusal to invent a default for a blob that is present but corrupt.
    match config::parse_object_checked::<MountState>(&node, "state") {
        Ok(state) => Ok(Some(state.unwrap_or_default())),
        Err(e) => {
            tracing::warn!(
                mount_id = %mount_id,
                branch = %branch,
                error = %e,
                "mount state is present but unparseable; refusing to treat it as a fresh default"
            );
            Err(Error::Validation(format!("mount {mount_id}: {e}")))
        }
    }
}

/// Standalone state-persist (also directly unit-testable).
///
/// Guarded by [`MountState::state_seq`], a durable counter stored alongside the
/// data it guards — deliberately NOT the lock manager's fencing token, which is
/// volatile and reset to zero on every restart. See the field's doc comment.
///
/// On success `state.state_seq` is advanced to the value just written, so the
/// same in-memory `state` can be persisted repeatedly across one run.
pub async fn persist_mount_state(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    mount_id: &str,
    state: &mut MountState,
) -> Result<bool> {
    persist_mount_state_detailed(storage, tenant, repo, branch, mount_id, state)
        .await
        .map(StateWrite::is_written)
}

/// [`persist_mount_state`] with the REASON preserved.
///
/// The public `bool` form above is a thin wrapper, so callers outside this
/// module keep their existing contract while the sync engine gets to tell a
/// missing mount node apart from a concurrent writer — a distinction the full
/// walk needs, because only the second means continuing is unsafe.
pub(super) async fn persist_mount_state_detailed(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    mount_id: &str,
    state: &mut MountState,
) -> Result<StateWrite> {
    use raisin_models::nodes::properties::PropertyValue;

    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(tenant, repo)?;
    tx.set_branch(branch)?;
    tx.set_actor(SYNC_ACTOR)?;
    // System privileges, sync identity: the auth context (not the raw actor)
    // is what stamps `updated_by`, so an `AuthContext::system()` here would
    // attribute the mount node to `"system"`.
    tx.set_auth_context(AuthContext::system_as(SYNC_ACTOR))?;
    tx.set_message("virtual mount sync: persist state")?;

    let Some(mut node) = tx.get_node(SYSTEM_WORKSPACE, mount_id).await? else {
        // At `warn`, not silence: production runs `RUST_LOG=warn`, and a state
        // write that vanishes is exactly the class of failure that has to reach
        // the journal.
        tracing::warn!(
            mount_id = %mount_id,
            branch = %branch,
            "skipping mount-state write: mount node not found on this branch"
        );
        return Ok(StateWrite::MountMissing);
    };

    // Compare-and-set on the durable, node-resident sequence. `state.state_seq`
    // is what THIS writer last read or wrote; if the stored value has moved past
    // it, another writer got there first and ours is stale.
    //
    // Note the asymmetry with the old fencing check: a stored seq that is BEHIND
    // ours is not a conflict (it is our own earlier write, or a node restored
    // from an older snapshot), so we proceed. Only a stored seq that has moved
    // AHEAD means someone else wrote.
    let stored_seq = node
        .properties
        .get("state")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .and_then(|v| v.get("state_seq").and_then(|t| t.as_u64()))
        .unwrap_or(0);
    if stored_seq > state.state_seq {
        tracing::warn!(
            mount_id = %mount_id,
            stored_seq,
            ours = state.state_seq,
            "skipping mount-state write: another writer advanced this mount's state"
        );
        return Ok(StateWrite::Superseded);
    }
    state.state_seq = stored_seq + 1;

    let state_value = serde_json::to_value(&*state)
        .map_err(|e| Error::Validation(format!("state serialize failed: {e}")))?;
    let state_pv: PropertyValue = serde_json::from_value(state_value)
        .map_err(|e| Error::Validation(format!("state to PropertyValue failed: {e}")))?;
    node.properties.insert("state".to_string(), state_pv);
    tx.upsert_node(SYSTEM_WORKSPACE, &node).await?;
    tx.commit().await?;
    Ok(StateWrite::Written)
}
