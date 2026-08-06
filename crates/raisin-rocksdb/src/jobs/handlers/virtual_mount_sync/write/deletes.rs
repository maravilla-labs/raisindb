//! Pushing local deletes — the first thing this engine does that can destroy
//! remote data.
//!
//! Every delete that reaches an adapter has passed three independent checks, in
//! three different layers, and it is worth being explicit that this is not
//! belt-and-braces:
//!
//! 1. **Provenance at detection** (`write::preimage`). The MVCC pre-image has to
//!    carry this mount's `__mount_id` and a non-empty `__external_id`, or the
//!    delete is never recorded. A user's own node under the mount path is not
//!    this mount's to retract.
//! 2. **The rails** (`write::guard`). Proportional blast radius, plus the
//!    bulk-origin flag that no count can reconstruct.
//! 3. **The policy** (`write::policy`). `detach` never reaches the provider at
//!    all; `trash` is refused unless the adapter declares a trash; `purge` is
//!    never a default.
//!
//! Each guards a case the others cannot see, which is why none of them is
//! redundant: the rails cannot tell a mount-owned node from a stranger's, and
//! provenance cannot tell one deliberate delete from ten thousand accidental
//! ones.

use chrono::Utc;
use serde_json::json;

use super::super::config::{MountState, PendingDelete};
use super::super::{AdapterError, SyncCtx};
use super::guard::{self, GuardedDeletes};
use super::plan::MirrorPlan;
use super::{DrainStats, STOP_CHECK_EVERY};

/// Push (or deliberately do not push) every pending delete this mount holds.
///
/// Returns `true` when a rail blocked the batch, which the caller reads as "park
/// the rest of the drain too": §9.4 parks every pending intent, not only the
/// deletes. Reads are untouched either way — the sync phases run after this
/// regardless, because refusing to destroy remote data must never also stop new
/// data arriving.
pub(super) async fn drain_deletes(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
    virtual_len: usize,
    plan: &MirrorPlan,
    stats: &mut DrainStats,
) -> bool {
    let now = Utc::now().timestamp();
    let pending = match guard::evaluate(state, &plan.limits, virtual_len, now) {
        GuardedDeletes::Nothing => return false,
        GuardedDeletes::Allow(pending) => pending,
        GuardedDeletes::Blocked => {
            stats.blocked = true;
            // The parked deletes are still on `state.writeback_pending_deletes`
            // and are counted as pending work, or a blocked mount reports the
            // same "nothing to do" as a caught-up one.
            stats.pending += state.writeback_pending_deletes.len();
            if let Some(block) = state.writeback_blocked.clone() {
                raisin_storage::jobs::global_mount_broadcaster().emit(
                    &raisin_storage::jobs::mount_channel_key(
                        &ctx.scope.tenant,
                        &ctx.scope.repo,
                        &ctx.scope.mount_id,
                    ),
                    raisin_storage::jobs::MountSyncEvent::WritebackBlocked {
                        deletes: block.deletes,
                        reason: block.reason.clone(),
                        confirm_token: block.token.clone(),
                    },
                );
                state.writeback_last_error = Some(block.reason);
            }
            return true;
        }
    };

    // Node ids whose intent is finished — pushed, deliberately detached, or
    // permanently unpushable. Removed from the queue in one pass at the end so
    // an early break leaves everything it did not reach exactly as it was.
    let mut done: Vec<String> = Vec::new();

    for (i, item) in pending.iter().enumerate() {
        // Both checks decide whether to START another provider call; a call
        // already made cannot be taken back. Breaking rather than returning is
        // what keeps the queue pruning below on the path.
        if ctx.out_of_time(Utc::now().timestamp()) {
            stats.truncated = true;
            break;
        }
        if i % STOP_CHECK_EVERY == 0 && super::super::stop_requested(ctx).await {
            stats.stopped = true;
            break;
        }

        if !plan.delete.pushes() {
            // `detach`, stated honestly. The node is gone locally and the remote
            // is untouched — which means the item is still listed by the
            // provider, and the next full reconcile (or the next delta that
            // mentions it) RE-IMPORTS it. The node comes back. There is no
            // per-mount suppression set anywhere in the engine, so this is
            // inherent rather than a gap, and describing it as "the delete was
            // suppressed" is what would generate the bug reports.
            tracing::info!(
                mount_id = %ctx.scope.mount_id,
                external_id = %item.external_id,
                path = item.path.as_deref().unwrap_or(""),
                "delete_policy is 'detach': nothing is sent to the provider. The remote \
                 object still exists and a later reconcile will re-import this node."
            );
            stats.detached += 1;
            done.push(item.node_id.clone());
            continue;
        }

        match push_one(ctx, item, plan).await {
            Ok(()) => {
                stats.deleted += 1;
                done.push(item.node_id.clone());
            }
            Err(e) => {
                stats.failed += 1;
                if stats.first_error.is_none() {
                    stats.first_error = Some(e.to_string());
                }
                match e {
                    // Every subsequent call fails identically; the read phase
                    // reaches the same conclusion and moves the mount to
                    // `auth_required`, which is where that decision belongs.
                    AdapterError::AuthExpired => break,
                    // The request can never succeed as written. Retrying it
                    // every tick forever is how an OAuth app gets throttled, so
                    // the intent is retired — loudly, because the remote object
                    // is now permanently out of sync with a node that is gone.
                    AdapterError::Config(ref msg) => {
                        tracing::error!(
                            mount_id = %ctx.scope.mount_id,
                            external_id = %item.external_id,
                            error = %msg,
                            "the provider can never accept this delete as written; retiring \
                             the intent. The remote object still exists and this mount will \
                             not try again."
                        );
                        done.push(item.node_id.clone());
                    }
                    // Everything else stays queued and is retried next run.
                    _ => tracing::warn!(
                        mount_id = %ctx.scope.mount_id,
                        external_id = %item.external_id,
                        error = %e,
                        "delete push failed; leaving it queued"
                    ),
                }
            }
        }
        // Per item, not per page: one `delete` against a throttled provider can
        // take tens of seconds, and a lease that lapses mid-drain lets a second
        // run start beside this one.
        ctx.renew_lease().await;
    }

    state
        .writeback_pending_deletes
        .retain(|p| !done.contains(&p.node_id));
    // Whatever is still queued IS the outstanding work — unreached, failed, or
    // deferred by a stop, it makes no difference to an operator asking whether
    // this mount is caught up. Counted once, from the queue itself, rather than
    // accumulated per break path, so the four exits cannot disagree.
    stats.pending += state.writeback_pending_deletes.len();
    false
}

/// One `delete` call. The policy travels with it, because "delete" alone is
/// ambiguous at every provider that has a trash — and guessing wrong turns a
/// recoverable delete into an irreversible one.
async fn push_one(
    ctx: &SyncCtx<'_>,
    item: &PendingDelete,
    plan: &MirrorPlan,
) -> std::result::Result<(), AdapterError> {
    ctx.call(
        "delete",
        json!({
            "item_id": item.external_id,
            "policy": plan.delete.as_str(),
            // The concurrency base captured from the MVCC pre-image at detection
            // time. An adapter that honours it refuses to delete a remote object
            // that has changed since this mount last saw it.
            "etag": item.etag,
        }),
    )
    .await
    .map(|_| ())
}
