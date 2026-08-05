//! The remap pre-pass: relocating already-synced nodes whose mapped path moved.

use raisin_error::Result;

use super::index::{MountScope, SyncIndex};
use super::node_paths::{ensure_folder_chain, is_item_level, join_path};
use super::ops::BatchOp;
use super::RocksDbMaterializer;

impl RocksDbMaterializer {
    /// Relocate already-synced nodes whose remapped path changed.
    ///
    /// Runs FIRST, in its own committed transaction, because a move and a write
    /// cannot be combined in one: the in-transaction read cache does not reflect
    /// the move, so writing after it collides on the id, and moving after a write
    /// relocates the pre-write version and discards the new mapping. Moving first
    /// leaves the ordinary upsert to find the node at its destination and update
    /// it in place.
    ///
    /// The node id is preserved throughout, so revision history and anything
    /// added locally survive the migration — delete-and-recreate loses all of it.
    pub(super) async fn remap_moves(
        &self,
        scope: &MountScope,
        index: &mut SyncIndex,
        ops: &[BatchOp],
    ) -> Result<()> {
        let mut moves: Vec<(String, String, String)> = Vec::new(); // (ext, node_id, new_path)
        for op in ops {
            let BatchOp::Upsert { rel_path, virt, .. } = op else {
                continue;
            };
            let new_path = join_path(&scope.mount_path, rel_path);
            if let Some(existing) = index.by_external(&virt.external_id) {
                if existing.path != new_path {
                    moves.push((virt.external_id.clone(), existing.id.clone(), new_path));
                }
            }
        }
        if moves.is_empty() {
            return Ok(());
        }

        let tx = self.begin(scope, "virtual mount sync: remap move").await?;
        let mut applied: Vec<(String, String)> = Vec::new();
        for (external_id, node_id, new_path) in moves {
            ensure_folder_chain(tx.as_ref(), &scope.workspace, &new_path).await?;
            match tx
                .move_node_tree(&scope.workspace, &node_id, &new_path)
                .await
            {
                Ok(()) => applied.push((external_id, new_path)),
                Err(e) if is_item_level(&e) => {
                    tracing::warn!(
                        mount_id = %scope.mount_id,
                        external_id = %external_id,
                        new_path = %new_path,
                        error = %e,
                        "remap move rejected; leaving the node at its current path"
                    );
                }
                Err(e) => {
                    let _ = tx.rollback().await;
                    return Err(e);
                }
            }
        }
        tx.commit().await?;
        for (external_id, new_path) in applied {
            index.record_move(&external_id, &new_path);
        }
        Ok(())
    }
}
