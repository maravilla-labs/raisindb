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

        // Write-through honesty: v1 has no writeback implementation, so a mount
        // asking for `write_through` (or whose adapter can't write) is recorded
        // as unsupported and warned once — never fatal, never status-degraded.
        if mount.write_config.wants_write_through() {
            state.writeback_supported = Some(false);
            tracing::warn!(
                mount_id = %mount_id,
                can_write = capabilities.can_write,
                "mount requests write_through writeback but it is not supported \
                 (v1 has no writeback implementation); syncing read-only"
            );
        }

        // Ensure a push subscription exists for webhook/hybrid mounts whose
        // adapter supports push (idempotent: a live subscription is left alone).
        // Runs every sync so a bootstrap sync — enqueued by the check pass for a
        // webhook mount that lacks a subscription — registers push here, and the
        // periodic renew job keeps it alive thereafter. Fully provider-agnostic.
        subscription::ensure(&ctx, &mut state, &capabilities).await;

        // Choose the sync path. Full reconcile is forced on the first sync, an
        // explicit `mode: "full"`, or when the adapter cannot serve a changes
        // feed (`supports_changes: false`) even if a cursor happens to exist.
        // A remap (flagged above) is a full walk by definition: the delta feed
        // only reports what changed upstream, which for a remap is nothing.
        let use_full = remap
            || mode == "full"
            || state.last_sync_token.is_none()
            || !capabilities.supports_changes;

        // An unfinished backfill resumes on subsequent runs even when the mount
        // is otherwise on the delta path.
        let backfill_pending = !state.backfill_stack.is_empty() || state.backfill_cursor.is_some();

        // Record the phase that is ACTUALLY about to run, replacing the mode the
        // caller asked for.
        //
        // These diverge routinely and the difference is not cosmetic: a run
        // requested as `delta` executes a full reconcile whenever the provider
        // rejects a stale cursor, and the history then showed "Delta" against a
        // run writing five-year-old mail. An operator reading that has no way to
        // reach the right conclusion — it cost a long investigation.
        let phase = if remap {
            "remap"
        } else if use_full {
            "full"
        } else if backfill_pending {
            "delta+backfill"
        } else {
            "delta"
        };
        if let Some(run) = state.last_run.as_mut() {
            run.mode = phase.to_string();
        }
        raisin_storage::jobs::global_mount_broadcaster().emit(
            &raisin_storage::jobs::mount_channel_key(
                &ctx.scope.tenant,
                &ctx.scope.repo,
                &ctx.scope.mount_id,
            ),
            raisin_storage::jobs::MountSyncEvent::Started {
                phase: phase.to_string(),
                trigger: trigger.clone(),
            },
        );

        let result = if use_full {
            full::run_with(&ctx, &mut state, &mut batcher).await
        } else if backfill_pending {
            // DELTA FIRST, then one backfill chunk — in that order, deliberately.
            //
            // Importing a large mailbox takes hours of chunked walking. Running
            // the backfill first (or on its own job) would mean new mail waited
            // for the whole import to finish. Doing the incremental pass first
            // on every run keeps new items arriving within one interval while
            // history fills in behind them, and because both share this run's
            // single lease there is no second writer to race the mount state.
            let delta_outcome = delta::run_with(&ctx, &mut state, &mut batcher).await;
            match delta_outcome {
                Ok(()) => full::run_with(&ctx, &mut state, &mut batcher).await,
                // A failing delta must not be masked by a successful backfill
                // chunk: surface it and leave the resume point untouched.
                Err(e) => Err(e),
            }
        } else {
            delta::run_with(&ctx, &mut state, &mut batcher).await
        };

        // A cursor the provider no longer accepts is recoverable HERE, in the
        // same run: drop it and fall back to a full reconcile.
        //
        // Every incremental API expires its cursors, and the recovery they all
        // document is "resync from scratch". Leaving it to the job retry cannot
        // work — the retry re-sends the same rejected cursor — and the failure
        // counter it accumulates then gates the backfill fast path in
        // `check::is_due`, so a mount with an expired delta token could not even
        // drain a pending import. Recovering in-run keeps `consecutive_failures`
        // at zero, which is what keeps the chunked backfill running back-to-back.
        let result = match result {
            Err(AdapterError::CursorInvalid(msg)) => {
                tracing::warn!(
                    mount_id = %mount_id,
                    error = %msg,
                    "provider rejected the stored delta cursor; discarding it and \
                     falling back to a full reconcile"
                );
                state.last_sync_token = None;
                full::run_with(&ctx, &mut state, &mut batcher).await
            }
            other => other,
        };

        let counts = batcher.stats();
        tracing::info!(
            mount_id = %mount_id,
            written = counts.written,
            skipped = counts.skipped,
            deleted = counts.deleted,
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
