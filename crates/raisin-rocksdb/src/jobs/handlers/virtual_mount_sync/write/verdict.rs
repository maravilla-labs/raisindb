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
/// This is a projection of [`resolve_mode`], never a second computation of it.
/// It used to be one: `state_only` was resolved properly while `write_through`
/// was judged by a separate `missing_mirror_ops` block right here, which is the
/// mirrored-path drift this codebase pays for most often. Every mode now
/// answers through the same function the drain obeys, so the console cannot
/// promise a write the engine will not make, or refuse one it would.
///
/// Writability belongs to the mount — **adapter, mapper and policy together**. A
/// write-capable adapter behind a mapper that cannot answer `to_external` is not
/// a writable mount, and neither is a mirror whose `delete_policy` names
/// something its adapter cannot do. `resolve_mode` reports every shortfall it
/// finds at once: being refused a second time for the next one is the round trip
/// an honest message avoids.
pub(crate) fn writeback_verdict(
    write_config: &WriteConfig,
    capabilities: &Capabilities,
    mapper: &MapperWriteback,
) -> (Option<bool>, Option<String>) {
    match resolve_mode(write_config, capabilities, mapper) {
        WriteMode::Off => (None, None),
        WriteMode::Refused(reason) => (Some(false), Some(reason)),
        // Supported, but with a stated caveat when only PART of the mount's
        // allow-list is actually writable.
        //
        // `(Some(true), None)` used to mean two different things: "everything
        // you asked for is pushable" and "some of it is". A mount declaring two
        // fields against an adapter that accepts one looked identical to a fully
        // working mount, while every edit to the dropped field was silently
        // never nominated — `candidates` tests divergence against the EFFECTIVE
        // list, so the node was filtered out with no log line and no counter.
        //
        // Deliberately not a refusal: the fields the adapter does accept still
        // push, and refusing the mount over a partial shortfall would stop those
        // too. The reason is the whole fix — it names which field is stuck.
        WriteMode::StateOnly(_) | WriteMode::Mirror(_) | WriteMode::Submit(_) => {
            (Some(true), unpushable_note(write_config, capabilities))
        }
    }
}

/// The declared mutable fields the adapter will not accept, as an operator-facing
/// note — `None` when the whole allow-list is honoured.
///
/// Named, never counted: WHICH field is unpushable is the whole of what someone
/// needs in order to fix it.
fn unpushable_note(write_config: &WriteConfig, capabilities: &Capabilities) -> Option<String> {
    let dropped: Vec<&str> = write_config
        .declared_mutable_fields()
        .iter()
        .filter(|f| !capabilities.mutable_fields.contains(f))
        .map(String::as_str)
        .collect();
    if dropped.is_empty() {
        return None;
    }
    Some(format!(
        "the adapter does not accept {} of this mount's mutable_fields ({}); \
         edits to those fields are never pushed",
        dropped.len(),
        dropped.join(", ")
    ))
}
