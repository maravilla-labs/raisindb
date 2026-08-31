//! `run_sync`: one mount sync run, from guards through phases to finalize.

use super::*;

impl VirtualMountSyncHandler {
    /// How long a payload-carrying run waits for the mount lease before giving up
    /// and letting the job system redeliver it.
    ///
    /// Sized against what it actually waits for: the run in front is normally a
    /// sub-second delta, and a burst of provider events arrives inside a few
    /// hundred milliseconds. Long enough to absorb that, short enough that a
    /// genuinely stuck lease (TTL is ten minutes) does not pin a worker.
    const PUSH_LEASE_WAIT: Duration = Duration::from_secs(8);
    /// Gap between attempts. Fine-grained because the point is to apply the event
    /// the instant the mount frees up, not merely eventually.
    const PUSH_LEASE_POLL: Duration = Duration::from_millis(150);

    /// Wait for the per-mount lease, for a run that must not lose its payload.
    ///
    /// `Ok(None)` means the wait expired and the caller should arrange a retry —
    /// it must NOT be read as "nothing to do".
    async fn await_mount_lease(
        lm: &std::sync::Arc<dyn raisin_locks::LockManager>,
        lock_key: &str,
        owner: &str,
        mount_id: &str,
    ) -> Result<Option<u64>> {
        let deadline = std::time::Instant::now() + Self::PUSH_LEASE_WAIT;
        loop {
            tokio::time::sleep(Self::PUSH_LEASE_POLL).await;
            if let Some(g) = lm.try_acquire(lock_key, owner, SYNC_LEASE_TTL).await? {
                tracing::debug!(
                    mount_id = %mount_id,
                    "acquired the mount lease after waiting; applying the pushed payload"
                );
                return Ok(Some(g.token));
            }
            if std::time::Instant::now() >= deadline {
                tracing::warn!(
                    mount_id = %mount_id,
                    "mount stayed locked while a pushed payload waited; deferring to a retry"
                );
                return Ok(None);
            }
        }
    }

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

        // Forwarded by the control plane when the webhook that woke this run carried the
        // resource itself. An empty list is treated as absent, so a delivery carrying
        // nothing this mount cares about still gets a normal re-read rather than a run
        // that asks the adapter to apply zero events and calls it done.
        //
        // READ BEFORE THE LEASE, deliberately: whether this run carries a payload decides
        // what to do when the mount is already busy. See the contention arm below.
        let pushed_events = context
            .metadata
            .get("pushed_events")
            .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
            .cloned();

        // Acquire the per-mount lease (cluster safety). Keyed on the config
        // branch — that is the identity the periodic scan enqueues against.
        let lock_key = ctx::mount_lease_key(&tenant, &repo, &config_branch, &mount_id);
        let owner = format!("{}:{}", self.instance_id, job.id);
        let lease_token = match &self.lock_manager {
            Some(lm) => match lm.try_acquire(&lock_key, &owner, SYNC_LEASE_TTL).await? {
                Some(g) => Some(g.token),
                None if pushed_events.is_none() => {
                    // No payload: another run is already re-reading this mount from the
                    // provider, and it will see whatever this one would have. Skipping is
                    // correct and is what keeps a burst of periodic kicks cheap.
                    tracing::warn!(mount_id = %mount_id, "mount is being synced elsewhere; no-op");
                    return Ok(Some(skip_result("locked_elsewhere")));
                }
                None => {
                    // A PAYLOAD MUST NEVER BE DROPPED HERE.
                    //
                    // This run carries the changed resource itself, delivered under a
                    // verified webhook signature. The run holding the lease carries a
                    // DIFFERENT event and will not apply ours, so skipping loses it
                    // outright — the change then waits for the next scheduled poll.
                    //
                    // That is not hypothetical. One Stripe payment emits three events
                    // within ~150ms (`payment_intent.created`, `payment_intent.succeeded`,
                    // `checkout.session.completed`), the control plane fans each to this
                    // mount, and they race. The one that wins applies its own event; the
                    // `checkout.session.completed` that actually settles the order lost
                    // the race and was discarded, so a paid order sat `pending` until a
                    // poll caught up minutes later. Single-event resources never bursted,
                    // which is why only payments looked broken.
                    //
                    // Waiting briefly is the right answer rather than failing: the run in
                    // front is normally a sub-second delta, so the events end up applied
                    // in sequence and the node write — and the live subscription that
                    // updates somebody's order page — still happens in real time.
                    match Self::await_mount_lease(lm, &lock_key, &owner, &mount_id).await? {
                        Some(token) => Some(token),
                        None => {
                            // Still held after the wait. Return an error rather than a
                            // skip so the job system redelivers this run WITH its payload;
                            // reporting success here would drop the event silently, which
                            // is the whole failure being fixed.
                            return Err(Error::Backend(format!(
                                "mount {mount_id} stayed locked while holding a pushed \
                                 payload; retrying so the event is not lost"
                            )));
                        }
                    }
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
            pushed_events,
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
            binary_retrieval: self.binary_retrieval.clone(),
            write_mode: std::sync::OnceLock::new(),
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
        // ...with ONE exception: a latency drain does not touch `status`.
        //
        // `status` describes the READ side of a mount — the console greys out
        // `Sync now` on `"syncing"` and `check::is_due` refuses to schedule an
        // `"auth_required"` mount. A drain fires on every local edit, so writing
        // `"syncing"` here would stomp a `degraded`/`error` verdict the read
        // phases had reached, and `finalize` would then write `"ok"` over it —
        // a mount reporting healthy because someone marked a mail read.
        let write_only = mode == write::WRITE_ONLY_MODE;
        let run_started_at = Utc::now().timestamp();

        // Close out a run that was KILLED before it could finalize. See
        // [`close_interrupted_run`] for why holding the lease makes this safe.
        if let Some(ran_for) = close_interrupted_run(&mut state, run_started_at) {
            tracing::warn!(
                mount_id = %mount_id,
                ran_for_secs = ran_for,
                "previous run never finalized; recording it as interrupted"
            );
        }

        if !write_only {
            state.status = Some("syncing".to_string());
        }
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
        // Any declared write mode earns the probe, `mirror` included — gating it
        // on the two modes that existed when this was written is how a new mode
        // silently arrives with `MapperWriteback::NoMapper` and is refused for a
        // mapper it never asked about.
        let wants_writeback = mount.write_config.wants_any_write();
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
        // Let page boundaries push pending local edits under this run's lease.
        let _ = ctx.write_mode.set(write_mode.clone());
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
        // Skipped for a latency drain: registering or renewing a push
        // subscription is read-path work, it costs a provider round trip, and a
        // drain runs as often as a user edits.
        if !write_only {
            subscription::ensure(&ctx, &mut state, &capabilities).await;
        }

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
            retained_excluded = counts.retained_excluded,
            "virtual mount sync finished"
        );

        // A stop is a REQUEST TO END THIS RUN, not a mode the mount stays in.
        //
        // Nothing else clears the flag. Every phase reads it and quits — the
        // drain at its first candidate, the delta and full walks at their first
        // page boundary — so a flag left set makes every subsequent run exit
        // immediately and report `ok` with nothing done. Scheduling is
        // unaffected, which is what makes it so hard to see: runs keep
        // appearing every minute, each one "completes without errors", and the
        // mount quietly never syncs or pushes again. One production mount sat
        // like that with a single edit pending, indefinitely, after an operator
        // pressed Stop during an incident.
        //
        // Cleared HERE rather than where the stop is observed, because reaching
        // this point is what proves the request was served: the run has ended.
        // Read fresh rather than trusted from `state`, so a stop pressed while
        // this run was working is honoured and then cleared, instead of being
        // carried into the next run.
        if super::stop_requested(&ctx).await {
            state.stop_requested = false;
            tracing::info!(
                mount_id = %mount_id,
                "stop request served by this run; clearing it so the next run proceeds"
            );
        }

        let outcome = self.finalize(&ctx, &mut state, result, counts).await;

        // Release the lease.
        if let (Some(lm), Some(token)) = (&self.lock_manager, lease_token) {
            let _ = lm.release(&lock_key, token).await;
        }
        outcome
    }
}

/// Close out a run that was KILLED before it could finalize, returning how long
/// it had been open.
///
/// Only ever called with the per-mount lease held, and that is what makes it
/// safe: no other run can be live, so an open record belongs to one that died —
/// the watchdog dropping the task at an await point, or a restart mid-sync.
/// `finalize` never ran for it, so nothing else in the system will ever close
/// it.
///
/// Left open it is not merely untidy. `status` stays `"syncing"`, so the console
/// reports a sync in progress indefinitely and greys out `Sync now`, while the
/// newest run in the history sits at `running` forever — a mount that looks busy
/// and is doing nothing. It also buries the real signal: a mount whose runs keep
/// being killed (a throttled provider is the usual cause) becomes
/// indistinguishable from one that is merely slow.
pub(super) fn close_interrupted_run(state: &mut MountState, now: i64) -> Option<i64> {
    let open = state.last_run.as_mut()?;
    if open.outcome != "running" {
        return None;
    }
    open.finish(
        now,
        "interrupted",
        Some(
            "the run was killed before it could finish - most often the job watchdog's \
             timeout, or a restart mid-sync. Nothing was lost; the resume point is intact."
                .to_string(),
        ),
    );
    let closed = open.clone();
    let ran_for = now - closed.started_at;
    state.recent_runs.insert(0, closed);
    state.recent_runs.truncate(config::MAX_RUN_HISTORY);
    Some(ran_for)
}

#[cfg(test)]
mod push_contention_tests {
    use super::*;
    use raisin_locks::{InProcessLockManager, LockManager};

    /// The decision this file makes on lease contention, isolated from timing.
    ///
    /// Reproduces the shape that lost a payment: a mount is already leased, and a
    /// second run arrives carrying a webhook payload. Before the fix both cases
    /// took the same branch and answered `locked_elsewhere`, discarding the
    /// event.
    fn manager() -> std::sync::Arc<dyn LockManager> {
        std::sync::Arc::new(InProcessLockManager::new())
    }

    /// No payload: skipping is correct and must stay cheap. The run in front is
    /// re-reading from the provider and will see whatever this one would have.
    #[tokio::test]
    async fn a_kick_with_no_payload_still_skips_when_the_mount_is_busy() {
        let lm = manager();
        let key = "tenant\0repo\0main\0mount-a";
        let held = lm
            .try_acquire(key, "holder", SYNC_LEASE_TTL)
            .await
            .expect("lock call")
            .expect("first acquire wins");
        assert!(held.token > 0);

        // A second acquire is refused — the condition the skip branch keys on.
        let second = lm
            .try_acquire(key, "other", SYNC_LEASE_TTL)
            .await
            .expect("lock call");
        assert!(second.is_none(), "a held lease must refuse a second owner");
    }

    /// With a payload the run must WAIT rather than skip, and must pick the lease
    /// up as soon as the holder releases — that is what keeps the node write, and
    /// the live subscription behind it, real time.
    #[tokio::test]
    async fn a_pushed_payload_waits_for_the_lease_and_then_applies() {
        let lm = manager();
        let key = "tenant\0repo\0main\0mount-b";
        let held = lm
            .try_acquire(key, "holder", SYNC_LEASE_TTL)
            .await
            .expect("lock call")
            .expect("first acquire wins");

        // Release shortly, as a sub-second delta run would.
        let releaser = {
            let lm = lm.clone();
            let key = key.to_string();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let _ = lm.release(&key, held.token).await;
            })
        };

        let got = VirtualMountSyncHandler::await_mount_lease(&lm, key, "pushed", "mount-b")
            .await
            .expect("wait must not error");
        assert!(
            got.is_some(),
            "a payload-carrying run must acquire the lease once the holder releases, \
             not drop its event"
        );
        releaser.await.expect("releaser");
    }

    /// A lease that never frees must NOT be reported as success. Returning
    /// `Ok(None)` is what tells the caller to fail the run so the job system
    /// redelivers it with its payload intact.
    #[tokio::test]
    async fn a_lease_that_never_frees_defers_rather_than_dropping() {
        let lm = manager();
        let key = "tenant\0repo\0main\0mount-c";
        let _held = lm
            .try_acquire(key, "holder", SYNC_LEASE_TTL)
            .await
            .expect("lock call")
            .expect("first acquire wins");

        tokio::time::pause();
        let wait = tokio::spawn({
            let lm = lm.clone();
            let key = key.to_string();
            async move {
                VirtualMountSyncHandler::await_mount_lease(&lm, &key, "pushed", "mount-c").await
            }
        });
        tokio::time::advance(VirtualMountSyncHandler::PUSH_LEASE_WAIT + Duration::from_secs(1))
            .await;
        let got = wait.await.expect("join").expect("wait must not error");
        assert!(
            got.is_none(),
            "an expired wait must report None so the caller retries; reporting success \
             would silently drop the event"
        );
    }
}
