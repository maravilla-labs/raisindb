//! Dropping a node tombstone must not revive the node's translations.
//!
//! `translation_read` ends a node-overlay version at `R` when the node has a
//! delete tombstone in `[R, bound]` — a READ rule, so it is the same on every
//! node whatever order the delete and the translation reached it in. Its
//! evidence is the `NODES` tombstone, and retention drops tombstones (one
//! below the cutoff that is not the newest there, or an orphan). Before one
//! goes, the run writes what it meant for the stored versions: `T` at the
//! delete's revision for every locale live there
//! (`translation_write::materialize_node_deletion`), in the SAME batch as the
//! delete of the tombstone, so no reader ever sees one without the other.

use crate::cf;
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};

/// `(tenant, repo, branch, workspace, node_id)` of a `NODES` chunk
/// (`{t}\0{r}\0{b}\0{ws}\0nodes\0{id}`); `None` for anything else.
pub(super) struct NodeChunk {
    tenant_id: String,
    repo_id: String,
    branch: String,
    workspace: String,
    node_id: String,
}

impl NodeChunk {
    pub(super) fn parse(cf_name: &str, chunk_prefix: &[u8]) -> Option<Self> {
        if cf_name != cf::NODES {
            return None;
        }
        let parts: Vec<&str> = chunk_prefix
            .split(|b| *b == 0)
            .map(std::str::from_utf8)
            .collect::<std::result::Result<_, _>>()
            .ok()?;
        match parts.as_slice() {
            [tenant, repo, branch, workspace, "nodes", node_id] => Some(Self {
                tenant_id: (*tenant).to_string(),
                repo_id: (*repo).to_string(),
                branch: (*branch).to_string(),
                workspace: (*workspace).to_string(),
                node_id: (*node_id).to_string(),
            }),
            _ => None,
        }
    }

    /// Stage the translation tombstones the node delete at `deleted_at`
    /// stands for — including the dead generation up to `next_record`, the
    /// node's next stored record above it; returns how many puts were staged.
    pub(super) fn materialize(
        &self,
        db: &DB,
        batch: &mut WriteBatch,
        deleted_at: &HLC,
        next_record: Option<&HLC>,
    ) -> Result<usize> {
        crate::translation_write::materialize_node_deletion(
            db,
            batch,
            (
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &self.workspace,
            ),
            &self.node_id,
            deleted_at,
            next_record,
        )
    }
}

/// A `NODES` chunk's survivors, newest first (`tomb[i]`: version `i` is a
/// delete tombstone): keep the live record right ABOVE every kept tombstone.
///
/// The translation read rule ends a version at `R` when the node's newest
/// record at or before `R` is a tombstone (a dead generation). Dropping the
/// live record that started the next generation would make the tombstone
/// below it that newest record for every version of that generation — and
/// end translations that were live. One extra record per delete-recreate
/// cycle keeps the answer.
pub(super) fn keep_generation_starts(tomb: &[bool], keep: &mut [bool]) {
    for i in 1..tomb.len() {
        if tomb[i] && keep[i] && !tomb[i - 1] {
            keep[i - 1] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::keep_generation_starts;

    #[test]
    fn a_dropped_recreate_above_a_kept_delete_is_kept() {
        // newest first: update, recreate, delete (kept by a pin), create
        let tomb = [false, false, true, false];
        let mut keep = [true, false, true, false];
        keep_generation_starts(&tomb, &mut keep);
        assert_eq!(keep, [true, true, true, false]);
        // A dropped delete needs nothing (GC materializes it first).
        let mut keep = [true, false, false, false];
        keep_generation_starts(&tomb, &mut keep);
        assert_eq!(keep, [true, false, false, false]);
    }
}
