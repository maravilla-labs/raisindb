//! Secret tombstones for node deletion.
//!
//! # Why a prefix scan and not a property walk
//!
//! Every other tombstoner in this module derives its targets from the node's
//! own properties, because that is where the indexed values live. Secrets are
//! different: a vaulted field's storage NAME embeds the node id
//! (`node/{node_id}/{field.path}`), so `node/{node_id}/` is an exact key prefix
//! and the scan finds every secret the node ever owned — including one whose
//! field was cleared on an earlier revision, which a walk of the *current*
//! properties would miss and leave live forever.
//!
//! # Why prior versions survive
//!
//! Only a tombstone is appended; nothing is removed. Older node revisions still
//! carry `secret://name@N` references, and a time-travel read of one must still
//! resolve. That is the same rule the store's own `delete` follows.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};

use super::{TombstoneColumnFamilies, TombstoneContext};
use crate::secret_store::lifecycle;
use crate::secret_store::SecretScope;

/// Append a tombstone for every live secret the node owns.
///
/// Needs no master keyring: a tombstone carries no ciphertext (see
/// [`lifecycle::tombstone_of`]). A delete must never fail because the
/// deployment has no keys configured.
pub(super) fn tombstone_secrets(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    let scope = SecretScope::new(ctx.tenant_id, ctx.repo_id, ctx.branch);
    let name_prefix = raisin_models::secret_ref::node_field_secret_name(&node.id, "");

    for stored in lifecycle::newest_with_name_prefix(db, &scope, &name_prefix)? {
        if stored.record.deleted {
            continue; // already retired; a second tombstone buys nothing
        }
        let record = lifecycle::tombstone_of(&stored, node.updated_by.as_deref());
        // The delete revision is normally later than any secret version, but the
        // two HLCs are not guaranteed to come from one ticking state outside the
        // server; a tombstone that does not sort ahead never takes effect.
        let at = lifecycle::strictly_after(*revision, stored.revision);
        lifecycle::write_record_to_batch(batch, cfs.secrets, &scope, &stored.name, &at, &record)?;
    }

    Ok(())
}
