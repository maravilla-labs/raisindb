//! Durable status transitions for `submit` command nodes.
//!
//! A `submit` mount's members are COMMANDS, not mirrors: creating one and moving
//! it to `queued` issues an irreversible externally-visible effect (an email
//! leaves the building, an organizer is told you declined). The engine therefore
//! cannot use the batcher for their lifecycle. Everything in this file is a
//! separate, immediately-committed transaction, because the whole at-most-once
//! protocol rests on one property: the `queued -> sending` transition is
//! **durable before the network call is made**. A staged write that flushed
//! afterwards would leave a crash indistinguishable from a send that never
//! happened — which is precisely the ambiguity this exists to bound.
//!
//! # Why a node-resident counter and not the lease
//!
//! [`WRITE_SEQ_PROP`] guards every transition. The lock manager's fencing token
//! is volatile — it resets when the process restarts — so it cannot answer "has
//! anyone touched this command since I read it?" across the very crash the
//! protocol is designed around. The counter lives beside the data and survives
//! with it, the same durable-counter-beside-the-data pattern as
//! `MountState::state_seq`.

use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use serde_json::{Map, Value};

use super::index::MountScope;
use super::RocksDbMaterializer;

/// The node-resident, monotonically increasing guard on a command's lifecycle.
///
/// Engine-owned and `__`-prefixed, so `build_properties` drops any mapper's
/// attempt to supply one, exactly as it does for `__etag`.
pub const WRITE_SEQ_PROP: &str = "__write_seq";

/// The property whose value IS the command's lifecycle.
pub const STATUS_PROP: &str = "status";

/// One compare-and-swap against a command node.
///
/// The comparison is on BOTH the status and the sequence. Status alone is not
/// enough: `queued -> sending -> (crash) -> unknown -> (human requeues) ->
/// queued` returns to a status a stale in-flight attempt would still match, and
/// that attempt must not then be allowed to write `sent` over the new one.
#[derive(Debug, Clone)]
pub struct CommandCas {
    pub node_id: String,
    /// The status this transition is only valid FROM.
    pub expect_status: String,
    /// The `__write_seq` this transition is only valid AT.
    pub expect_seq: i64,
    /// The status to move to.
    pub set_status: String,
    /// Additional properties to land in the SAME transaction.
    ///
    /// Engine-constructed only — never mapper or adapter output — so reserved
    /// `__` keys are written through rather than filtered. A `Value::Null` here
    /// REMOVES the property, which is how `last_error` is cleared on a requeue.
    pub set: Map<String, Value>,
}

/// What a [`CommandCas`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CasOutcome {
    /// Applied. Carries the node's NEW `__write_seq`, which the caller must use
    /// as the `expect_seq` of the transition that follows it.
    Applied(i64),
    /// The node was not in the expected state. Nothing was written.
    ///
    /// Not an error: a user cancelling a queued mail between the scan and the
    /// claim, or a second drain that lost the race, both land here and both are
    /// correct outcomes. Carries what was actually found, for the log.
    Refused { status: Option<String>, seq: i64 },
    /// The node was deleted between the scan and the transition.
    Missing,
}

/// A command node's lifecycle status, or `None` when it carries none.
///
/// A node with no `status` is not a command — an ordinary node that happens to
/// live under a `submit` mount's path — and must never be submitted.
pub fn command_status(node: &Node) -> Option<String> {
    match node.properties.get(STATUS_PROP)? {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// A command node's `__write_seq`, defaulting to 0.
///
/// Zero is the right default and not a guess: a command a user has just created
/// has never been transitioned, so it is at sequence zero by definition. A
/// non-integer value is also read as zero — a corrupt counter must not make the
/// node permanently un-claimable, because the CAS still compares the status and
/// the transition still writes a real integer back.
pub fn command_seq(node: &Node) -> i64 {
    match node.properties.get(WRITE_SEQ_PROP) {
        Some(PropertyValue::Integer(i)) => *i,
        Some(PropertyValue::Float(f)) => *f as i64,
        _ => 0,
    }
}

impl RocksDbMaterializer {
    /// Apply one command transition, or refuse it.
    ///
    /// Read and write share ONE transaction. Under the mount lease that is the
    /// whole story — the lease is the cluster-wide gate and no second drain
    /// exists to race. What the sequence check adds is protection against the
    /// writer the lease does not cover: a USER (or an agent, or a flow) editing
    /// the same command node concurrently. It also makes a resumed attempt after
    /// a crash provably stale rather than plausibly current.
    pub(super) async fn cas_command_impl(
        &self,
        scope: &MountScope,
        cas: &CommandCas,
    ) -> Result<CasOutcome> {
        let tx = self
            .begin(scope, "virtual mount sync: submit command transition")
            .await?;
        let Some(mut node) = tx.get_node(&scope.workspace, &cas.node_id).await? else {
            return Ok(CasOutcome::Missing);
        };
        let status = command_status(&node);
        let seq = command_seq(&node);
        if status.as_deref() != Some(cas.expect_status.as_str()) || seq != cas.expect_seq {
            return Ok(CasOutcome::Refused { status, seq });
        }

        node.properties.insert(
            STATUS_PROP.to_string(),
            PropertyValue::String(cas.set_status.clone()),
        );
        for (key, value) in &cas.set {
            if value.is_null() {
                node.properties.remove(key);
                continue;
            }
            match serde_json::from_value::<PropertyValue>(value.clone()) {
                Ok(pv) => {
                    node.properties.insert(key.clone(), pv);
                }
                // Dropped rather than aborting the transition. The status change
                // is what the protocol depends on; a diagnostic string that will
                // not convert must not be able to strand a command in `sending`.
                Err(e) => tracing::warn!(
                    node_id = %cas.node_id,
                    key = %key,
                    error = %e,
                    "submit: dropping an unconvertible command property"
                ),
            }
        }
        let next = seq + 1;
        node.properties
            .insert(WRITE_SEQ_PROP.to_string(), PropertyValue::Integer(next));
        tx.upsert_node(&scope.workspace, &node).await?;
        tx.commit().await?;
        Ok(CasOutcome::Applied(next))
    }
}
