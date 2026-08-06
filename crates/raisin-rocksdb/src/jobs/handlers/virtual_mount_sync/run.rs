//! `run_sync`: one mount sync run, from guards through phases to finalize.

use super::*;

impl VirtualMountSyncHandler {
    /// Sync a single mount. Returns the run summary as the job result.
    pub(super) async fn run_sync(
        &self,
        job: &JobInfo,
        context: &JobContext,
        mount_id: String,
        mode: String,
        trigger: String,
    ) -> Result<Option<Value>> {
        let tenant = context.tenant_id.clone();
        let repo = context.repo_id.clone();
        // The job is always enqueued (by `check`) on the repo's config (default)
        // branch: that is the ONLY branch the mount/integration configuration is
        // read from. Materialized virtual nodes go to `mount.target_branch`
        // (resolved below), which may differ.
        let config_branch = if context.branch.is_empty() {
            "main".to_string()
        } else {
            context.branch.clone()
        };

        let svc = self.system_service(&tenant, &repo, &config_branch);
        let (mount, invoker) = match self
            .preflight(&svc, &tenant, &repo, &config_branch, &mount_id)
            .await?
        {
            Preflight::Skip(reason) => return Ok(Some(skip_result(reason))),
            Preflight::Proceed { mount, invoker } => (*mount, invoker),
        };

        // Resolve integration + adapter path + credential + scope + snapshot.
        let CtxParts {
            integ_node_id,
            adapter_path,
            credential,
            mount_snapshot,
            public_origin,
            scope,
        } = self
            .build_ctx_parts(&svc, &tenant, &repo, &config_branch, &mount)
            .await?;

        // Acquire the per-mount lease (cluster safety). Keyed on the config
        // branch — that is the identity the periodic scan enqueues against.
        let lock_key = format!("{tenant}\0{repo}\0{config_branch}\0vmount:{mount_id}");
        let owner = format!("{}:{}", self.instance_id, job.id);
        let lease_token = match &self.lock_manager {
            Some(lm) => match lm.try_acquire(&lock_key, &owner, SYNC_LEASE_TTL).await? {
                Some(g) => Some(g.token),
                None => {
                    tracing::warn!(mount_id = %mount_id, "mount is being synced elsewhere; no-op");
                    return Ok(Some(skip_result("locked_elsewhere")));
                }
            },
            None => {
                if self.replication_active() {
                    tracing::warn!(
                        mount_id = %mount_id,
                        "locks subsystem disabled while replication is active: virtual-mount \
                         sync is not cluster-safe (configure [locks] backend=redis)"
                    );
                }
                None
            }
        };

        // `scope` (built by `build_ctx_parts`) targets `mount.target_branch` for
        // materialization; mount/integration config and mount state stay on the
        // config branch (`ctx.config_branch`).
        let mut ctx = SyncCtx {
            public_origin,
            storage: self.storage.clone(),
            scope: scope.clone(),
            config_branch: config_branch.clone(),
            mount: mount.clone(),
            adapter_path,
            invoker: invoker.as_ref(),
            materializer: self.materializer.as_ref(),
            lock_manager: self.lock_manager.clone(),
            lock_key: lock_key.clone(),
            lease_token,
            credential,
            mount_snapshot,
            deadline: Utc::now().timestamp() + SYNC_WALL_CLOCK_BUDGET.as_secs() as i64,
        };

        // A remap re-materializes everything through the CURRENT mapper and path
        // template, ignoring etags — the migration path for a connector whose
        // mapping changed (a new node type, a new folder hierarchy) while the
        // provider's items did not.
        //
        // Set BEFORE the batcher is built: the batcher snapshots the scope, and a
        // remap that reached it as `force_rewrite: false` would silently degrade
        // into an ordinary sync where every etag matches and nothing is rewritten.
        let remap = mode == "remap";
        if remap {
            ctx.scope.force_rewrite = true;
        }

        // ONE batcher for the whole run: it owns the prefetched index that
        // replaced the per-item workspace scan, so every phase below — TTL
        // cleanup, delta, backfill — shares one read of the workspace and one
        // write buffer.
        let mut batcher = match batch::SyncBatcher::new(&ctx).await {
            Ok(b) => b,
            Err(e) => {
                let mut state = mount.state.clone();
                state.last_run = Some(SyncRun::started(Utc::now().timestamp(), &mode, &trigger));
                let outcome = self
                    .finalize(&ctx, &mut state, Err(e), BatchStats::default())
                    .await;
                if let (Some(lm), Some(token)) = (&self.lock_manager, lease_token) {
                    let _ = lm.release(&lock_key, token).await;
                }
                return outcome;
            }
        };

        // Ephemeral TTL cleanup up front.
        if mount.sync_config.ephemeral {
            if let Some(ttl) = mount.sync_config.ttl_seconds {
                let _ = ephemeral::cleanup_expired(&mut batcher, ttl, Utc::now().timestamp()).await;
            }
        }

        let mut state = mount.state.clone();

        // Mark the mount as syncing and open the run record BEFORE any adapter
        // work. `"syncing"` is in the schema and the console already renders it,
        // but no Rust code ever wrote it — so a mount mid-import reported a
        // cheerful `ok` and a running sync was indistinguishable from an idle
        // one. This write is also what lights up the console's live feed: it is
        // an ordinary node write, so it emits `node:updated` for free.
        let run_started_at = Utc::now().timestamp();
        state.status = Some("syncing".to_string());
        state.last_run = Some(SyncRun::started(run_started_at, &mode, &trigger));
        if !persist_state(&ctx, &mut state)
            .await
            .map(StateWrite::is_written)
            .unwrap_or(false)
        {
            // Refused before the run even starts: another writer holds this
            // mount's state and `state_seq` has moved past ours. Every write
            // this run would make is already doomed, so say so rather than
            // doing a mount's worth of work that can never be recorded.
            tracing::warn!(
                mount_id = %mount_id,
                "mount-state write refused at run start; another writer owns this mount"
            );
        }

        // Resolve capabilities once and cache them on the integration node (on
        // the config branch) so the admin UI can read them without invoking the
        // adapter.
        let capabilities = ctx.resolve_capabilities().await;
        if let Err(e) = self
            .cache_capabilities(
                &tenant,
                &repo,
                &config_branch,
                &integ_node_id,
                &capabilities,
            )
            .await
        {
            tracing::warn!(mount_id = %mount_id, error = %e, "failed to cache capabilities");
        }

        // Write-through honesty: record whether the mount's requested writeback
        // can be honoured, computed from what the adapter declares rather than
        // hardcoded. Never fatal, never status-degraded — a mount that cannot
        // write still syncs read-only.
        //
        // The mapper is probed only for a mount that actually asked for
        // writeback: the verdict for every other mount is `(None, None)`
        // regardless of what the mapper says, so probing them would spend a
        // QuickJS invocation per run to compute a value that is discarded —
        // the same reasoning that keeps `stage_item` from mapping an item whose
        // etag already matches.
        let wants_writeback =
            mount.write_config.wants_write_through() || mount.write_config.wants_state_only();
        let mapper_writeback = if wants_writeback {
            ctx.probe_mapper_writeback().await
        } else {
            MapperWriteback::NoMapper
        };
        let (writeback_supported, writeback_reason) =
            write::writeback_verdict(&mount.write_config, &capabilities, &mapper_writeback);
        // The same resolution the verdict was derived from, kept so the drain
        // pushes exactly the fields the operator was told are supported. Two
        // independent computations of "what may this mount write" is precisely
        // the mirrored-path drift that would let the console promise one thing
        // and the engine send another.
        let write_mode = write::resolve_mode(&mount.write_config, &capabilities, &mapper_writeback);
        if writeback_supported == Some(false) {
            tracing::warn!(
                mount_id = %mount_id,
                reason = writeback_reason.as_deref().unwrap_or(""),
                "mount requests writeback but it is not supported; syncing read-only"
            );
        }
        state.writeback_supported = writeback_supported;
        state.writeback_last_error = writeback_reason;

        // Ensure a push subscription exists for webhook/hybrid mounts whose
        // adapter supports push (idempotent: a live subscription is left alone).
        // Runs every sync so a bootstrap sync — enqueued by the check pass for a
        // webhook mount that lacks a subscription — registers push here, and the
        // periodic renew job keeps it alive thereafter. Fully provider-agnostic.
        subscription::ensure(&ctx, &mut state, &capabilities).await;

        let result = phases::run_phases(
            &ctx,
            &mut state,
            &mut batcher,
            &capabilities,
            &write_mode,
            remap,
            &mode,
            &trigger,
        )
        .await;
        let counts = batcher.stats();
        tracing::info!(
            mount_id = %mount_id,
            written = counts.written,
            skipped = counts.skipped,
            deleted = counts.deleted,
            stamped = counts.stamped,
            failed = counts.failed,
            "virtual mount sync finished"
        );

        let outcome = self.finalize(&ctx, &mut state, result, counts).await;

        // Release the lease.
        if let (Some(lm), Some(token)) = (&self.lock_manager, lease_token) {
            let _ = lm.release(&lock_key, token).await;
        }
        outcome
    }
}
