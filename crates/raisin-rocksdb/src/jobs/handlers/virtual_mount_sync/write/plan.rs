//! What this mount may push, and which of its nodes still need pushing.

use super::super::materializer::VirtualNodeRef;
use super::super::{Capabilities, MapperWriteback, WriteConfig};

/// The effective write mode for one run.
///
/// Three outcomes, not a boolean, because "this mount does not write" and "this
/// mount asked to write and cannot" must reach the operator differently: the
/// first is a configuration choice and silent, the second is a problem and
/// carries its reason into `state.writeback_last_error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WriteMode {
    /// The mount does not ask for writes. Nothing is probed, nothing is said.
    Off,
    /// The mount asks for writes it cannot have, for this stated reason.
    Refused(String),
    /// `state_only`, with the effective (non-empty) field allow-list.
    StateOnly(Vec<String>),
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
    if !write_config.wants_state_only() {
        // `off` is Off. A mode the engine does not implement (`mirror`,
        // `submit`) or does not recognize at all is REFUSED, with its reason —
        // demoting it to Off silently was a mount that followed the documented
        // configuration and then sat there writing nothing, saying nothing.
        return match write_config.unsupported_mode_reason() {
            Some(reason) => WriteMode::Refused(reason),
            None => WriteMode::Off,
        };
    }
    let mut reasons = Vec::new();

    let missing = capabilities.missing_state_only_ops();
    if !missing.is_empty() {
        reasons.push(format!("adapter does not declare {}", missing.join(", ")));
    }
    if let Some(reason) = mapper.reason() {
        reasons.push(reason);
    }

    let declared = write_config.declared_mutable_fields();
    if declared.is_empty() {
        reasons.push("mount declares no write_config.mutable_fields".to_string());
    }
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

    if reasons.is_empty() {
        WriteMode::StateOnly(effective)
    } else {
        WriteMode::Refused(reasons.join("; "))
    }
}

/// One node the drain will act on.
#[derive(Debug, Clone)]
pub(super) struct Candidate {
    pub node_id: String,
    pub external_id: String,
}

/// Nodes whose watched fields no longer match what was last pushed.
///
/// Derived from the index the run already loaded, so detection costs no reads
/// at all. It is allowed to be imprecise in the noisy direction: every
/// candidate is re-read and re-checked before anything is sent (see
/// [`super::push`]), which is the same division of labour the read path uses
/// between a sloppy delta feed and the exact etag skip-write.
pub(super) fn candidates(
    nodes: Vec<VirtualNodeRef>,
    fields: &[String],
    limit: usize,
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = nodes
        .into_iter()
        .filter_map(|node| {
            // An unseeded node (no `__pushed_state`) needs no special arm: with
            // nothing recorded as pushed, any watched field it carries already
            // counts as diverged, and it is PUSHED rather than baselined from
            // its own local values — see [`super::push::push_one`] for why
            // baselining is the one outcome that can silently lose an edit.
            node.write_view
                .as_ref()?
                .diverges(fields)
                .then_some(Candidate {
                    node_id: node.id,
                    external_id: node.external_id,
                })
        })
        .collect();
    // The index is a hash map, so without this the truncation below would drop
    // a different arbitrary subset on every run and a mount with more pending
    // pushes than the cap could starve one of them indefinitely.
    out.sort_by(|a, b| a.external_id.cmp(&b.external_id));
    out.truncate(limit);
    out
}
