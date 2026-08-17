//! Issuing ONE command: the at-most-once sequence, step by step.
//!
//! Read this as a sequence with one hinge. Everything before the claim is free
//! to fail as often as it likes — nothing has happened yet. Everything after it
//! is committed to a single attempt, whatever the answer.

use chrono::Utc;
use serde_json::{json, Map, Value};

use super::super::item::map_item;
use super::super::materializer::{command_seq, CasOutcome, CommandCas};
use super::super::ExternalItem;
use super::super::{AdapterError, SyncCtx};
use super::plan::SubmitPlan;
use super::submit_outcome::{disposition, status, Disposition};
use super::submit_record::{map_command, sent_stamp};

/// What issuing one command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Issued {
    /// The provider accepted it. Terminal.
    Sent,
    /// Explicitly refused before the provider looked at it (`rate_limited`),
    /// so it is back at `queued` and will be tried again.
    ///
    /// Carries the provider's stated wait, because a throttle is a statement
    /// about the TENANT, not about this one command: every remaining command
    /// would be refused identically. The drain used to carry on regardless —
    /// 499 more calls into a provider that had just asked us to stop, repeated
    /// on every tick, with nothing on the mount to show for it.
    Requeued { retry_after_secs: Option<u64> },
    /// Terminal `failed`: definitively not sent.
    Failed { reason: String, stop_drain: bool },
    /// Terminal `unknown`: it may have been sent. Never auto-retried.
    Unknown(String),
    /// Someone else changed the command between the scan and the claim — a user
    /// cancelling it, most likely. Not an error and not an outcome; the drain
    /// simply does not own this node any more.
    Abandoned,
    /// Nothing was attempted and nothing changed. The command is still `queued`
    /// and the next drain will try again, which is safe precisely because no
    /// provider call was made.
    NotAttempted(String),
}

/// Claim → call → record, exactly once.
pub(super) async fn issue_one(
    ctx: &SyncCtx<'_>,
    node: &raisin_models::nodes::Node,
    plan: &SubmitPlan,
) -> Issued {
    // 0. READ THE NODE AS IT STANDS NOW, not as the drain first saw it.
    //
    //    `drain_submit` takes ONE snapshot and issues up to
    //    `max_items_per_sync` commands from it, each a provider call that can
    //    take tens of seconds. Command #200's turn comes minutes after its node
    //    was read — and a person fixing a typo or dropping a recipient on a
    //    still-`queued` mail writes ordinary properties, which move neither
    //    `status` nor `__write_seq`. So the CAS claim below applied happily and
    //    the PRE-EDIT payload went out. Mail cannot be recalled.
    //
    //    `push_one` already re-reads for exactly this reason (its step 2); the
    //    submit path never got the same treatment. `command_seq` comes from the
    //    fresh read too, so the claim asserts against what was actually mapped.
    let fresh = match ctx.materializer.read_node(&ctx.scope, &node.id).await {
        Ok(Some(fresh)) => fresh,
        // Gone between the scan and now: nothing was sent, and there is nothing
        // left to send it for.
        Ok(None) => return Issued::Abandoned,
        Err(e) => return Issued::NotAttempted(format!("re-read before send failed: {e}")),
    };
    let node = &fresh;

    // 1. PREPARE, before claiming anything. The mapper is pure, so asking it
    //    first costs nothing and keeps a mapper problem out of the claim's
    //    ambiguity window — a command stranded at `unknown` because its mapper
    //    was misconfigured would be a question about the provider that the
    //    provider was never asked.
    let mapped = match map_command(ctx, node).await {
        Ok(Some(mapped)) => mapped,
        // A mapper declining a command is an answer, and a definitive one: this
        // node is not something this mount knows how to send. Terminal, so it
        // stops appearing in every drain, and stated on the node so the author
        // can see why.
        Ok(None) => {
            return terminal(
                ctx,
                node,
                command_seq(node),
                status::QUEUED,
                status::FAILED,
                "the mount's mapping function does not know how to send this node \
                 (to_external returned null)",
            )
            .await
        }
        Err(reason) => return Issued::NotAttempted(reason),
    };

    // 2. THE HINGE. The claim is a single transaction that commits before any
    //    network call: `queued -> sending`, stamped with the attempt id the
    //    idempotency key is built from. A crash from here on leaves `sending`,
    //    which the next drain parks at `unknown` — bounded, attributable, and
    //    never resent.
    let attempt_id = nanoid::nanoid!();
    let seq = command_seq(node);
    let mut claim = Map::new();
    claim.insert("attempt_id".into(), json!(attempt_id));
    claim.insert("attempted_at".into(), json!(Utc::now().to_rfc3339()));
    claim.insert("last_error".into(), Value::Null);
    let claimed = ctx
        .materializer
        .cas_command(
            &ctx.scope,
            &CommandCas {
                node_id: node.id.clone(),
                expect_status: status::QUEUED.to_string(),
                expect_seq: seq,
                set_status: status::SENDING.to_string(),
                set: claim,
            },
        )
        .await;
    let claim_seq = match claimed {
        Ok(CasOutcome::Applied(seq)) => seq,
        Ok(CasOutcome::Refused { status: s, seq }) => {
            tracing::info!(
                node_id = %node.id,
                found_status = ?s,
                found_seq = seq,
                "outbox command changed between the scan and the claim; leaving it alone"
            );
            return Issued::Abandoned;
        }
        Ok(CasOutcome::Missing) => return Issued::Abandoned,
        // The claim did not commit, so NOTHING was sent and the command is
        // still exactly as it was. Retrying it next run is safe — this is the
        // one failure in this file that is.
        Err(e) => return Issued::NotAttempted(format!("could not claim the command: {e}")),
    };

    // 3. The call. Sent whether or not the adapter declares it can forward the
    //    key: it costs nothing, and an adapter that later gains the ability
    //    starts honouring it with no engine change.
    let idempotency_key = format!("{}:{}:{}", ctx.scope.mount_id, node.id, attempt_id);
    let result = ctx
        .call(
            "submit",
            json!({
                "payload": mapped.payload,
                "external_id": mapped.external_id,
                "idempotency_key": idempotency_key,
            }),
        )
        .await;
    if !plan.supports_idempotency_key {
        tracing::debug!(
            mount_id = %ctx.scope.mount_id,
            node_id = %node.id,
            attempt_id = %attempt_id,
            "this adapter cannot forward an idempotency key; at-most-once rests \
             entirely on the durable claim"
        );
    }

    // 4. Record the answer. Every arm is a CAS from `sending` at the sequence
    //    the claim produced, so an attempt that has been superseded — by a
    //    human requeueing a command this run had already given up on — cannot
    //    write over the newer state.
    match result {
        Ok(value) if value.is_object() => {
            let external_id = value
                .get("external_id")
                .or_else(|| value.get("provider_id"))
                .and_then(|v| v.as_str());
            let etag = value.get("etag").and_then(|v| v.as_str());
            let mut set = sent_stamp(&node.id, &ctx.scope.mount_id, external_id, etag);
            merge_created_item(ctx, node, &value, &mut set).await;
            apply(ctx, node, claim_seq, status::SENT, set).await;
            Issued::Sent
        }
        // A non-object answer is NOT the `gone` an `update` gets. There is no
        // object here that could be gone: the call was made, the adapter
        // answered something the contract does not define, and whether the
        // message left is unknowable from it. Parked, not settled.
        Ok(_) => {
            let reason = "the adapter answered the submit with no result object, so \
                          whether the command was issued is unknown"
                .to_string();
            terminal(
                ctx,
                node,
                claim_seq,
                status::SENDING,
                status::UNKNOWN,
                &reason,
            )
            .await;
            Issued::Unknown(reason)
        }
        Err(e) => match disposition(&e) {
            Disposition::Requeue => {
                let mut set = Map::new();
                set.insert("last_error".into(), json!(e.to_string()));
                apply(ctx, node, claim_seq, status::QUEUED, set).await;
                Issued::Requeued {
                    retry_after_secs: e.retry_after_secs(),
                }
            }
            Disposition::Failed => {
                let reason = e.to_string();
                terminal(
                    ctx,
                    node,
                    claim_seq,
                    status::SENDING,
                    status::FAILED,
                    &reason,
                )
                .await;
                Issued::Failed {
                    reason,
                    // One expired credential fails every remaining command for
                    // the identical reason. That is not thirty decisions.
                    stop_drain: matches!(e, AdapterError::AuthExpired),
                }
            }
            Disposition::Unknown => {
                let reason = format!(
                    "{e}. The command was claimed and sent to the adapter, so it may have \
                     reached the provider; it is NOT retried automatically."
                );
                terminal(
                    ctx,
                    node,
                    claim_seq,
                    status::SENDING,
                    status::UNKNOWN,
                    &reason,
                )
                .await;
                Issued::Unknown(reason)
            }
        },
    }
}

/// A terminal transition carrying a reason, returned as the matching outcome.
async fn terminal(
    ctx: &SyncCtx<'_>,
    node: &raisin_models::nodes::Node,
    seq: i64,
    from: &str,
    to: &str,
    reason: &str,
) -> Issued {
    let mut set = Map::new();
    set.insert("last_error".into(), json!(reason));
    let _ = ctx
        .materializer
        .cas_command(
            &ctx.scope,
            &CommandCas {
                node_id: node.id.clone(),
                expect_status: from.to_string(),
                expect_seq: seq,
                set_status: to.to_string(),
                set,
            },
        )
        .await
        .map_err(|e| {
            // The command stays where it was. For a `sending` that is the SAFE
            // failure: the next drain finds it stale and parks it at `unknown`,
            // which is where this transition was trying to put it anyway.
            tracing::warn!(
                node_id = %node.id,
                to = %to,
                error = %e,
                "could not record a command's terminal state; the next drain will park it"
            )
        });
    if to == status::FAILED {
        Issued::Failed {
            reason: reason.to_string(),
            stop_drain: false,
        }
    } else {
        Issued::Unknown(reason.to_string())
    }
}

/// Apply a transition, logging rather than propagating a write failure.
async fn apply(
    ctx: &SyncCtx<'_>,
    node: &raisin_models::nodes::Node,
    seq: i64,
    to: &str,
    set: Map<String, Value>,
) {
    match ctx
        .materializer
        .cas_command(
            &ctx.scope,
            &CommandCas {
                node_id: node.id.clone(),
                expect_status: status::SENDING.to_string(),
                expect_seq: seq,
                set_status: to.to_string(),
                set,
            },
        )
        .await
    {
        Ok(CasOutcome::Applied(_)) => {}
        Ok(other) => tracing::warn!(
            node_id = %node.id,
            to = %to,
            outcome = ?other,
            "a command's terminal transition was refused; something else moved it"
        ),
        Err(e) => tracing::warn!(
            node_id = %node.id,
            to = %to,
            error = %e,
            "could not record a command's outcome"
        ),
    }
}

/// Fold the object the command CREATED back onto the command node.
///
/// Optional and additive: an adapter that answers a submit with only
/// `{external_id, etag}` — every adapter written before this — reaches the
/// `else` and nothing changes.
///
/// # Why a stamp alone is not enough
///
/// `sent_stamp` writes `__etag` and `__synced_at` from the adapter's answer,
/// which asserts "this node is up to date at that version". For a mail send that
/// is true and complete: the command IS the whole object, and there is nothing
/// at the provider to learn. For a command that MINTS an object it is a lie, and
/// an expensive one — the next walk compares etags, finds a match, and skips the
/// item, so the properties that only exist at the provider never arrive at all.
///
/// Stripe made that concrete. Creating a Checkout Session returns the pay `url`,
/// and the url is the entire point of the command. But Stripe emits no
/// `checkout.session.created` event, so the delta feed never surfaces a fresh
/// session, and the full walk that would list it skipped it on the etag the
/// submit had just stamped. The node reached `sent` — correctly, the session was
/// real — carrying no url, forever. Dropping the etag instead would only trade
/// that for "never, because delta never lists it".
///
/// So the data has to be taken at the one moment it is in hand: the answer.
///
/// # Through the mapper, never raw
///
/// The item is mapped with the SAME `to_node` the read path uses. Writing the
/// adapter's metadata straight onto the node would bypass the mapper's
/// translation and store provider-shaped keys the nodetype never declared —
/// `kind`, a nested `metadata` bag — and would diverge from what the next sync
/// writes for the very same object. One object must have one mapping.
///
/// # Precedence
///
/// The stamp WINS. Mapped properties are inserted only where the stamp has not
/// already spoken, so `__external_id`, `__etag` and `__synced_at` keep the
/// values the submit protocol chose. `status` is excluded outright: it is the
/// command lifecycle, owned by the CAS in `apply`, and a provider's own status
/// field arriving under that name would overwrite `sent` with something the
/// outbox cannot read.
async fn merge_created_item(
    ctx: &SyncCtx<'_>,
    node: &raisin_models::nodes::Node,
    answer: &Value,
    set: &mut Map<String, Value>,
) {
    let Some(raw) = answer.get("item") else {
        return;
    };
    let item: ExternalItem = match serde_json::from_value(raw.clone()) {
        Ok(item) => item,
        Err(e) => {
            // Not fatal: the command WAS sent, and refusing to record that
            // because the optional half of the answer was malformed would strand
            // a node the provider has already acted on.
            tracing::warn!(
                mount_id = %ctx.scope.mount_id,
                node_id = %node.id,
                error = %e,
                "submit answered with an `item` that is not an ExternalItem; \
                 recording the send without it"
            );
            return;
        }
    };

    match map_item(ctx, &item).await {
        Ok(Some(mapped)) => {
            for (key, value) in mapped.node.properties {
                if key == "status" {
                    continue;
                }
                set.entry(key).or_insert(value);
            }
        }
        // The mapper filtering the item out is a legitimate answer, not an
        // error — and it is the mapper's business, so it is not second-guessed.
        Ok(None) => {}
        Err(e) => tracing::warn!(
            mount_id = %ctx.scope.mount_id,
            node_id = %node.id,
            error = %e,
            "could not map the submitted item back onto its command node; \
             recording the send without it"
        ),
    }
}
