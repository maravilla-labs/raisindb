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
        WriteMode::StateOnly(_) | WriteMode::Mirror(_) | WriteMode::Submit(_) => (Some(true), None),
    }
}
