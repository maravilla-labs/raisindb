//! The OUTBOX drain: at-most-once issuing of commands.
//!
//! A `submit` mount's members are not mirrors of remote objects, they are
//! REQUESTS with a status lifecycle and a terminal state. Sending an email is
//! not idempotent and never will be, so this path is built around one property
//! rather than around retries: **a command is claimed durably before the network
//! call, and a claim is never un-made by the engine.**
//!
//! # What that buys, precisely
//!
//! Without the claim, a crash between "user queued it" and "provider answered"
//! is an unbounded question: the command still says `queued`, so every
//! subsequent drain re-sends it, forever, once per crash. With it, the crash
//! leaves a `sending` the next drain converts to `unknown` — one attempt, one
//! recorded `attempt_id`, and a question a person can actually answer by looking
//! in the Sent folder. The protocol converts an unbounded ambiguity into a
//! bounded one; it does not remove ambiguity, because no protocol over a
//! provider without an idempotency key can.
//!
//! # What this deliberately does NOT do
//!
//! * It never retries. Not on a timeout, not on a 5xx, not on an adapter that
//!   died. See [`submit_outcome::disposition`] for the inversion and why.
//! * It never collapses or reorders. Two identical commands are two emails, and
//!   the ordering the user queued them in is the ordering they go out in — the
//!   opposite of `mirror`/`state_only`, where the current node IS the desired
//!   state and only the last intent matters.
//! * It never touches a `draft`. Composing is not authorizing.
//! * It garbage-collects nothing. A completed command is stamped as an ordinary
//!   mount-owned virtual node, so the mount's EXISTING `ephemeral` +
//!   `ttl_seconds` machinery collects it. A second reaper here would be a second
//!   implementation of one policy — this codebase's most expensive bug class.

use chrono::Utc;
use serde_json::{json, Map, Value};

use super::super::materializer::{command_seq, command_status, CasOutcome, CommandCas};
use super::super::{map_to_external, MountState, SyncCtx, ToExternalOutcome};
use super::plan::SubmitPlan;
use super::submit_outcome::{status, STALE_SENDING_REASON};
use super::submit_step::{self, Issued};
use super::DrainStats;

/// How many commands the drain issues between `stop_requested` reads.
const STOP_CHECK_EVERY: usize = 5;

/// Issue whatever commands this outbox has been given, at most once each.
///
/// Never fails the run, exactly like the other modes: an outbox whose provider
/// is refusing must not stop the mount reading. What went wrong reaches the
/// operator through `writeback_last_error` and through each command's own
/// `last_error`, which is where it is actually actionable.
pub(super) async fn drain_submit(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
    batcher: &mut super::super::batch::SyncBatcher<'_>,
    plan: &SubmitPlan,
) -> DrainStats {
    let mut stats = DrainStats::default();

    // Honour a stand-off before scanning anything. A provider that stated a wait
    // gets that wait: without this the throttle above would stop one drain and
    // the next tick would walk straight back into it, which is the loop the
    // stand-off exists to break.
    if let Some(retry_after) = super::standing_off(state, chrono::Utc::now().timestamp()) {
        tracing::debug!(
            mount_id = %ctx.scope.mount_id,
            retry_after,
            "outbox drain skipped: the provider asked us to wait"
        );
        return stats;
    }

    // The outbox cannot be read from `SyncIndex`: a command is authored by a
    // user and carries no `__mount_id`/`__external_id` until this drain stamps
    // one on it, and the index holds only nodes that already have both.
    let nodes = match ctx.materializer.scan_mount_nodes(&ctx.scope).await {
        Ok(nodes) => nodes,
        Err(e) => {
            tracing::warn!(
                mount_id = %ctx.scope.mount_id,
                error = %e,
                "outbox scan failed; no commands were issued this run"
            );
            stats.failed += 1;
            state.writeback_last_error = Some(format!("outbox scan failed: {e}"));
            return stats;
        }
    };

    // Partitioned from ONE snapshot, before anything is claimed. That ordering
    // is load-bearing: this drain sets `sending` itself, so deciding what is
    // stale from a later read would sweep away its own in-flight claims.
    let mut stale = Vec::new();
    let mut queued = Vec::new();
    let mut foreign = 0usize;
    for node in nodes {
        // PROVENANCE, and this is the only layer that can supply it. The delete
        // path checks `__mount_id` on the node; a command cannot be checked that
        // way because it is authored by a USER and carries no `__mount_id` until
        // this drain stamps one on it. So the node's TYPE is the ownership
        // signal, and it is checked before `status` is even read.
        //
        // Without it the partition below is "any node under the outbox path
        // whose `status` property happens to read `queued`" — a raisin:Task, a
        // draft order, an agent's scratch record. The drain would stamp
        // `sending` / `attempt_id` / `__write_seq` onto that node, and with a
        // permissive `to_external` would SEND it. The `sending` arm is worse
        // still: it rewrites a stranger's node to `unknown` with an engine
        // `last_error` without consulting the mapper at all.
        if !ctx.mount.write_config.is_command_type(&node.node_type) {
            if command_status(&node).is_some() {
                foreign += 1;
            }
            continue;
        }
        match command_status(&node).as_deref() {
            Some(status::SENDING) => stale.push(node),
            Some(status::QUEUED) => queued.push(node),
            // Everything else is inert, `draft` and every terminal state
            // included. A node with no `status` at all is not a command — it is
            // ordinary content that happens to live under this path — and is
            // skipped without comment.
            _ => {}
        }
    }
    if foreign > 0 {
        // Said out loud, because failing closed silently is how an outbox that
        // holds a custom command type appears to work and sends nothing. The fix
        // is one line of `write_config.command_node_types`.
        tracing::info!(
            mount_id = %ctx.scope.mount_id,
            skipped = foreign,
            "outbox: skipped node(s) with a lifecycle status whose node_type this mount \
             does not declare as a command; set write_config.command_node_types if they \
             were meant to be sent"
        );
    }

    for node in stale {
        stats.unresolved += 1;
        let seq = command_seq(&node);
        tracing::warn!(
            mount_id = %ctx.scope.mount_id,
            node_id = %node.id,
            path = %node.path,
            attempt_id = ?node.properties.get("attempt_id"),
            "a command was left claimed by a run that did not finish; parking it at \
             'unknown' rather than resending it"
        );
        let mut set = Map::new();
        set.insert("last_error".into(), json!(STALE_SENDING_REASON));
        let cas = CommandCas {
            node_id: node.id.clone(),
            expect_status: status::SENDING.to_string(),
            expect_seq: seq,
            set_status: status::UNKNOWN.to_string(),
            set,
        };
        if let Err(e) = ctx.materializer.cas_command(&ctx.scope, &cas).await {
            tracing::warn!(node_id = %node.id, error = %e, "could not park a stale command");
            stats.failed += 1;
        }
        if stats.first_park.is_none() {
            stats.first_park = Some(STALE_SENDING_REASON.to_string());
        }
    }

    // Commit order, deterministically. `created_at` is what the user's queueing
    // order actually is; the path breaks ties so two commands created in the
    // same transaction still drain in a stable order rather than in whatever
    // order the path scan happened to return them.
    queued.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.path.cmp(&b.path))
    });
    let limit = ctx.mount.sync_config.max_items_per_sync.max(1) as usize;
    let over_limit = queued.len().saturating_sub(limit);
    queued.truncate(limit);
    stats.pending += over_limit;

    for (i, node) in queued.iter().enumerate() {
        // Both checks BEFORE the claim, never between the claim and the call: a
        // command claimed and then abandoned for want of time would be parked at
        // `unknown` by the next drain having never been attempted at all. What
        // is not reached here is untouched, still `queued`, and picked up next
        // run in the same order.
        if ctx.out_of_time(Utc::now().timestamp()) {
            stats.truncated = true;
            stats.pending += queued.len() - i;
            break;
        }
        if i % STOP_CHECK_EVERY == 0 && super::super::stop_requested(ctx).await {
            stats.stopped = true;
            stats.pending += queued.len() - i;
            break;
        }

        let mut stop_drain = false;
        match submit_step::issue_one(ctx, node, plan).await {
            Issued::Sent { external_id, etag } => {
                stats.submitted += 1;
                // REGISTER IN THIS RUN'S INDEX. The walk runs after this drain
                // and shares the index; without this it misses the command node
                // on `by_external`, falls back to a path match, and creates a
                // second node under the provider-derived path — leaving the
                // original renamed and its path unusable. See `SyncIndex::adopt`.
                batcher.adopt(&node.id, &node.path, &external_id, etag, true);
            }
            // A THROTTLE STOPS THE DRAIN. It is a statement about the TENANT, so
            // every remaining command would be refused the same way — the drain
            // used to carry on and put 499 more calls into a provider that had
            // just asked us to stop, on every tick, with nothing on the mount to
            // show for it. A stated wait is an instruction, not a hint.
            //
            // What is left stays `queued` and goes out once the stand-off
            // expires, which is the same shape the credential-refused branch
            // below already has.
            Issued::Requeued { retry_after_secs } => {
                stats.requeued += 1;
                stop_drain = true;
                if let Some(secs) = retry_after_secs {
                    state.writeback_retry_after =
                        Some(chrono::Utc::now().timestamp() + secs as i64);
                }
                tracing::info!(
                    mount_id = %ctx.scope.mount_id,
                    retry_after_secs,
                    pending = queued.len() - i - 1,
                    "the provider throttled the outbox; leaving the rest queued"
                );
            }
            Issued::Failed {
                reason,
                stop_drain: s,
            } => {
                stats.abandoned += 1;
                stop_drain = s;
                if stats.first_park.is_none() {
                    stats.first_park = Some(reason);
                }
            }
            Issued::Unknown(reason) => {
                stats.unresolved += 1;
                if stats.first_park.is_none() {
                    stats.first_park = Some(reason);
                }
            }
            Issued::Abandoned => stats.skipped += 1,
            Issued::NotAttempted(reason) => {
                stats.failed += 1;
                if stats.first_error.is_none() {
                    stats.first_error = Some(reason);
                }
            }
        }
        // Renewed per command, like the update drain and for the same reason: a
        // provider call can take tens of seconds, and a lease that lapses
        // mid-drain lets a second run start beside this one. For an outbox that
        // is worse than for a mirror — the second run would find this run's
        // claims and park them as stale while they were still in flight.
        ctx.renew_lease().await;

        // An expired credential stops the drain. Every remaining command would
        // be failed for the identical reason, and failing a queue of mail on one
        // refused token is not a decision to make thirty times. What is left
        // stays `queued`, so reconnecting the account is all it takes.
        if stop_drain {
            let left = queued.len() - i - 1;
            stats.pending += left;
            tracing::warn!(
                mount_id = %ctx.scope.mount_id,
                pending = left,
                "the outbox drain stopped early on a mount-level refusal; the rest \
                 stays queued"
            );
            break;
        }
    }

    record(state, &stats);
    stats
}

/// The operator-facing receipt for one outbox drain.
fn record(state: &mut MountState, stats: &DrainStats) {
    if stats.submitted == 0
        && stats.requeued == 0
        && stats.abandoned == 0
        && stats.unresolved == 0
        && stats.failed == 0
    {
        // A quiet outbox writes nothing, so a mount with no mail to send does
        // not churn its state blob on every tick.
        return;
    }
    tracing::info!(
        submitted = stats.submitted,
        requeued = stats.requeued,
        abandoned = stats.abandoned,
        unresolved = stats.unresolved,
        failed = stats.failed,
        pending = stats.pending,
        "virtual mount outbox drain finished"
    );
    // `unknown` is the status that needs a human, so it — not a failure — is
    // what badges the mount. A `failed` command is definitively unsent and the
    // user can see that on the command itself; an `unknown` one is a question
    // nobody has asked yet.
    state.writeback_status = if stats.unresolved > 0 {
        Some("unresolved".to_string())
    } else if stats.abandoned > 0 {
        Some("failed".to_string())
    } else {
        None
    };
    if stats.first_error.is_some() {
        state.writeback_last_error = stats.first_error.clone();
    } else if let Some(park) = &stats.first_park {
        state.writeback_last_error = Some(park.clone());
    }
    state.last_drain = Some(stats.summary());
}
