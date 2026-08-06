//! What this mount may push: the mode resolution, and the refusal when a mount
//! asks for writes it cannot have.
//!
//! WHICH fields travel is [`super::fields`]; WHICH nodes still owe a push is
//! [`super::candidates`].

use super::super::{Capabilities, MapperWriteback, WriteConfig};
use super::fields::FieldPlan;
use super::guard::{DeleteLimits, DEFAULT_MAX_DELETES_PER_RUN, DEFAULT_MAX_DELETE_RATIO};
use super::policy::{resolve_delete_policy, resolve_move_policy, DeletePolicy, MovePolicy};

/// The effective write mode for one run.
///
/// Four outcomes, not a boolean, because "this mount does not write" and "this
/// mount asked to write and cannot" must reach the operator differently: the
/// first is a configuration choice and silent, the second is a problem and
/// carries its reason into `state.writeback_last_error`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WriteMode {
    /// The mount does not ask for writes. Nothing is probed, nothing is said.
    Off,
    /// The mount asks for writes it cannot have, for this stated reason.
    Refused(String),
    /// `state_only`, with the effective (non-empty) field allow-list.
    StateOnly(FieldPlan),
    /// `mirror`: the node **is** the remote object, so field updates AND
    /// deletes propagate.
    Mirror(MirrorPlan),
    /// `submit`: the node **is a command**. Its `draft -> queued` transition
    /// issues an irreversible externally-visible effect exactly once.
    Submit(SubmitPlan),
}

/// Everything a `submit` drain needs, resolved once per run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubmitPlan {
    /// Whether the adapter can forward the engine's attempt id to the provider.
    ///
    /// Carried rather than assumed, and it changes nothing about the protocol:
    /// the key is sent either way (it costs nothing and an adapter that grows
    /// the ability starts honouring it without an engine change), and the
    /// engine's own at-most-once guarantee never depends on it. What it is for
    /// is HONESTY — the difference between "this send cannot be duplicated" and
    /// "this send will not be RETRIED by us" is the difference an operator has
    /// to be able to see.
    pub supports_idempotency_key: bool,
}

/// Everything a `mirror` drain needs, resolved once per run.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MirrorPlan {
    /// The effective update allow-list — same split a `state_only` mount gets.
    pub fields: FieldPlan,
    pub delete: DeletePolicy,
    pub limits: DeleteLimits,
}

/// Resolve what this mount may actually push.
///
/// The allow-list is the INTERSECTION of what the mount declares and what the
/// adapter says the provider accepts. Both halves are needed and neither is
/// redundant: the mount's list is an operator's intent (`push read flags, not
/// folders`), the adapter's is a fact about the provider (`isRead is writable,
/// receivedDateTime is not`). Taking the union — or trusting either side alone
/// — would send a field the provider rejects on every drain, forever, for a
/// change that can never converge.
///
/// An adapter that declares no `mutable_fields` therefore writes nothing, which
/// is exactly the state every adapter shipped today is in. That is deliberate:
/// enabling the write path cannot make an existing deployment start writing
/// until its adapter opts in field by field.
pub(crate) fn resolve(
    write_config: &WriteConfig,
    capabilities: &Capabilities,
    mapper: &MapperWriteback,
) -> WriteMode {
    if write_config.wants_mirror() {
        return resolve_mirror(write_config, capabilities, mapper);
    }
    if write_config.wants_submit() {
        return resolve_submit(capabilities, mapper);
    }
    if !write_config.wants_state_only() {
        // `off` is Off. A mode the engine does not recognize at all is REFUSED,
        // with its reason — demoting it to Off silently was a mount that
        // followed the documented configuration and then sat there writing
        // nothing, saying nothing.
        return match write_config.unsupported_mode_reason() {
            Some(reason) => WriteMode::Refused(reason),
            None => WriteMode::Off,
        };
    }
    let mut reasons = Vec::new();
    if !capabilities.missing_state_only_ops().is_empty() {
        reasons.push(format!(
            "adapter does not declare {}",
            capabilities.missing_state_only_ops().join(", ")
        ));
    }
    if let Some(reason) = mapper.reason() {
        reasons.push(reason);
    }

    let declared = write_config.declared_mutable_fields();
    if declared.is_empty() {
        reasons.push("mount declares no write_config.mutable_fields".to_string());
    }
    let effective = effective_fields(write_config, capabilities, &mut reasons);
    // Resolved for `state_only` too, not only for a mirror. A mail mount whose
    // adapter declares `folder` as a move field pushes folder moves through the
    // very same update, so an unrecognized `move_policy` has to be refused here
    // as loudly as it is there — a typo that resolves to a policy the operator
    // did not choose is the one outcome `write::policy` exists to prevent.
    let move_policy = match resolve_move_policy(write_config, capabilities) {
        Ok(p) => Some(p),
        Err(e) => {
            reasons.push(e);
            None
        }
    };

    if reasons.is_empty() {
        WriteMode::StateOnly(FieldPlan::new(
            effective,
            capabilities,
            move_policy.expect("no reason recorded, so resolution succeeded"),
        ))
    } else {
        WriteMode::Refused(reasons.join("; "))
    }
}

/// An OUTBOX. Its members are commands, so almost nothing the other two modes
/// resolve applies: there is no field allow-list (a command is submitted whole,
/// not patched), no delete or move policy (deleting an unsent command cancels
/// it locally and there is nothing remote to delete), and no conflict policy (a
/// command addresses no object that could have changed underneath it).
///
/// What it does still need is the MAPPER, for the same reason every other mode
/// does: the reverse translation must live with the forward one or the two
/// drift. A `submit` mount with no `to_external` is refused rather than issuing
/// a guess — and issuing a guess here means sending someone an email.
fn resolve_submit(capabilities: &Capabilities, mapper: &MapperWriteback) -> WriteMode {
    let mut reasons = Vec::new();
    let missing = capabilities.missing_submit_ops();
    if !missing.is_empty() {
        reasons.push(format!("adapter does not declare {}", missing.join(", ")));
    }
    if let Some(reason) = mapper.reason() {
        reasons.push(reason);
    }
    if !reasons.is_empty() {
        return WriteMode::Refused(reasons.join("; "));
    }
    WriteMode::Submit(SubmitPlan {
        supports_idempotency_key: capabilities.supports_idempotency_key,
    })
}

/// A `mirror` mount, which is a `state_only` update path plus delete
/// propagation plus the rails that make the latter survivable.
///
/// Note what is NOT refused here: an empty effective field list. A mirror whose
/// only outbound effect is a delete is a real configuration (a file mount where
/// no node property maps back to a writable provider field), and demanding a
/// mutable field to unlock deletes would push operators into inventing one.
fn resolve_mirror(
    write_config: &WriteConfig,
    capabilities: &Capabilities,
    mapper: &MapperWriteback,
) -> WriteMode {
    let mut reasons = Vec::new();
    // `can_create` is demanded only from a mount that actually creates. The
    // engine calls `create` for a node the user authored locally under a mount
    // that opted in by type (`create_node_types`); a mirror mount that has not
    // opted in never reaches the create path, so requiring the capability there
    // would refuse an otherwise correct adapter for an operation it would never
    // be asked to perform. Google Drive is the only shipped adapter declaring
    // the full set, and gating every mirror mount on the rarest capability is
    // what made `mirror` look unimplementable against providers that update and
    // delete perfectly well.
    let missing = capabilities.missing_mirror_ops(write_config.creates_locally());
    if !missing.is_empty() {
        reasons.push(format!("adapter does not declare {}", missing.join(", ")));
    }
    if let Some(reason) = mapper.reason() {
        reasons.push(reason);
    }
    // Both policies resolve even when the adapter probe already failed, so one
    // pass reports every shortfall. Fixing the adapter only to be refused again
    // for a typo in `delete_policy` is the round trip an honest message avoids.
    let delete = match resolve_delete_policy(write_config, capabilities) {
        Ok(p) => Some(p),
        Err(e) => {
            reasons.push(e);
            None
        }
    };
    let move_policy = match resolve_move_policy(write_config, capabilities) {
        Ok(p) => Some(p),
        Err(e) => {
            reasons.push(e);
            None
        }
    };
    // A mirror may declare an empty allow-list; what it may NOT do is declare
    // fields the adapter rejects, which would send a doomed patch on every
    // drain forever for a change that can never converge.
    let effective = effective_fields(write_config, capabilities, &mut reasons);

    if !reasons.is_empty() {
        return WriteMode::Refused(reasons.join("; "));
    }
    WriteMode::Mirror(MirrorPlan {
        fields: FieldPlan::new(
            effective,
            capabilities,
            move_policy.expect("no reason recorded, so resolution succeeded"),
        ),
        delete: delete.expect("no reason recorded, so resolution succeeded"),
        limits: DeleteLimits {
            max_per_run: write_config
                .max_deletes_per_run
                .unwrap_or(DEFAULT_MAX_DELETES_PER_RUN),
            ratio: write_config
                .max_delete_ratio
                .unwrap_or(DEFAULT_MAX_DELETE_RATIO),
            confirm_on_bulk: write_config.require_confirmation_on_bulk.unwrap_or(true),
        },
    })
}

/// The intersection of what the mount declares and what the adapter accepts,
/// recording a reason when the mount asked for fields and got none of them.
///
/// Both halves are needed and neither is redundant: the mount's list is an
/// operator's intent (`push read flags, not folders`), the adapter's is a fact
/// about the provider (`isRead is writable, receivedDateTime is not`).
fn effective_fields(
    write_config: &WriteConfig,
    capabilities: &Capabilities,
    reasons: &mut Vec<String>,
) -> Vec<String> {
    let declared = write_config.declared_mutable_fields();
    let effective: Vec<String> = declared
        .iter()
        .filter(|f| capabilities.mutable_fields.contains(f))
        .cloned()
        .collect();
    if !declared.is_empty() && effective.is_empty() {
        reasons.push(format!(
            "adapter accepts none of the mount's mutable_fields ({})",
            declared.join(", ")
        ));
    }
    effective
}
