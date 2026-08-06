//! Whether a mount's requested writeback can be honoured, and why not.
//!
//! Lives beside the drain rather than in `run.rs` on purpose: the verdict the
//! console reads and the mode the drain executes are ONE decision, and two
//! copies of it would let the UI promise a write the engine never makes.

use super::super::{Capabilities, MapperWriteback, WriteConfig};
use super::{resolve_mode, WriteMode};

/// Decide, honestly, whether a mount's requested writeback can be honoured.
///
/// Returns `(writeback_supported, reason)` for [`MountState`]. A mount that did
/// not ask for writeback yields `(None, None)` — the console distinguishes an
/// absent flag ("not applicable") from a present `false` ("asked for, refused"),
/// and writing `Some(false)` for every read-only mount would both lie about
/// intent and churn the state blob on every run.
///
/// `write_through` is evaluated as a `mirror`: the node **is** the remote
/// object, so every op a mirror performs has to be declared. The specific
/// missing ops go into the reason, because "writeback unsupported" with no
/// "why" is what sends an operator reading engine source.
///
/// Writability belongs to the mount — **adapter and mapper together**. A
/// write-capable adapter behind a mapper that cannot answer `to_external` is
/// not a writable mount, so `mapper` vetoes independently of `capabilities`,
/// and BOTH shortfalls are reported when both apply: fixing the adapter only to
/// be refused again for the mapper is the kind of round trip an honest message
/// avoids.
pub(crate) fn writeback_verdict(
    write_config: &WriteConfig,
    capabilities: &Capabilities,
    mapper: &MapperWriteback,
) -> (Option<bool>, Option<String>) {
    // `state_only` is the mode the engine actually implements, so it answers
    // first and on its own terms: it needs `update` and a field allow-list, not
    // the create/delete a mirror needs. Judging it by `missing_mirror_ops`
    // would refuse a working mail mount for lacking a delete it never calls.
    if write_config.wants_state_only() {
        return match resolve_mode(write_config, capabilities, mapper) {
            WriteMode::StateOnly(_) => (Some(true), None),
            WriteMode::Refused(reason) => (Some(false), Some(reason)),
            WriteMode::Off => (None, None),
        };
    }
    // A `mode` the engine cannot honour is refused HERE too, through the very
    // same `resolve_mode` the drain obeys — not a second copy of the rule. The
    // drain already declines to run for `Refused`; without this the console
    // would still show `mode: "mirror"` as `(None, None)`, i.e. "writeback not
    // applicable", which is the silence this refusal exists to break.
    if let WriteMode::Refused(reason) = resolve_mode(write_config, capabilities, mapper) {
        return (Some(false), Some(reason));
    }
    if !write_config.wants_write_through() {
        return (None, None);
    }
    let mut reasons = Vec::new();
    let missing = capabilities.missing_mirror_ops();
    if !missing.is_empty() {
        reasons.push(format!("adapter does not declare {}", missing.join(", ")));
    }
    if let Some(r) = mapper.reason() {
        reasons.push(r);
    }
    if reasons.is_empty() {
        (Some(true), None)
    } else {
        (Some(false), Some(reasons.join("; ")))
    }
}
