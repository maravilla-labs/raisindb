//! A transactional move's view of the subtree it moves, and what it leaves
//! behind in the read cache.
//!
//! `move_node_tree` lists the subtree from COMMITTED state, but a node in it
//! may already have been written (or moved, or deleted) earlier in the same
//! transaction, at the same revision R the move writes at. Moving the
//! committed copy of such a node is wrong both ways round:
//!
//! - **write, then move**: `put_node(c)` stages c's record at R with the
//!   pre-move path. (A pre-Phase-10 binary embedded that path in the blob
//!   while the move wrote only `NODE_PATH` at R, so a blob and an entry at ONE
//!   revision named different paths; the read rule's tie-break still reads
//!   such old data.) The move re-stages the record from the STAGED node, with
//!   the new path, so the write's own changes survive and the record names one
//!   path.
//! - **move, then write**: the move left the moved descendants' records
//!   untouched in the cache, so a later `get_node(c)` read committed state —
//!   the pre-move path — and `put_node` stored it back at R, over the move's
//!   `NODE_PATH`. [`record_moves`] leaves every moved node, with its new path,
//!   where `get_node` finds it.

use raisin_error::Result;
use raisin_models::nodes::Node;

use crate::transaction::RocksDBTransaction;

/// One node of the subtree as THIS transaction sees it.
pub(super) struct MoveSubject {
    /// The node as the transaction last left it: what it staged, what an
    /// earlier move in it produced, or the committed record.
    pub(super) node: Node,
    pub(super) depth: usize,
    /// The transaction already wrote or moved it, so its record at R names
    /// the pre-move path and must be re-staged.
    pub(super) touched: bool,
    /// The parent's id (`None` for the moved root, whose parent changes).
    pub(super) parent_id: Option<String>,
}

/// Map the committed pre-order listing `descendants` onto the transaction's
/// view. A node this transaction deleted is dropped: moving it would write a
/// live path entry over its tombstone.
pub(super) fn transaction_view(
    tx: &RocksDBTransaction,
    workspace: &str,
    descendants: Vec<(Node, usize)>,
) -> Result<Vec<MoveSubject>> {
    let cache = tx
        .read_cache
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    // The listing is pre-order depth-first, so a node's parent is the latest
    // node one level up. Ids never change, so committed ids serve.
    let mut ids_by_depth: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(descendants.len());
    for (committed, depth) in descendants {
        ids_by_depth.truncate(depth);
        let parent_id = depth
            .checked_sub(1)
            .and_then(|up| ids_by_depth.get(up).cloned());
        ids_by_depth.push(committed.id.clone());

        let key = (workspace.to_string(), committed.id.clone());
        let (node, touched) = match cache.nodes.get(&key) {
            Some(Some(staged)) => (staged.clone(), true),
            Some(None) => continue,
            None => match cache.moved_nodes.get(&key) {
                Some(moved) => (moved.clone(), true),
                None => (committed, false),
            },
        };
        out.push(MoveSubject {
            node,
            depth,
            touched,
            parent_id,
        });
    }
    Ok(out)
}

/// Leave each moved node — `(old path, node as moved)` — in the read cache:
/// vacate the old paths, claim the new ones, and serve the moved record to
/// later reads (in `nodes` when the transaction wrote it, else in
/// `moved_nodes`, which still passes RLS).
pub(super) fn record_moves(
    tx: &RocksDBTransaction,
    workspace: &str,
    moves: &[(String, Node)],
) -> Result<()> {
    let mut guard = tx
        .read_cache
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
    let cache = &mut *guard;

    // Vacate the old paths first, THEN claim the new ones. Doing it in one
    // pass would let a node moved onto a sibling's vacated path be erased
    // again by that sibling's removal.
    for (old_path, _) in moves {
        cache
            .paths
            .insert((workspace.to_string(), old_path.clone()), None);
    }

    // Register the new paths. Without this the moved node was unreachable
    // by path for the REST OF THE TRANSACTION — the batch had already
    // written the new PATH_INDEX entry, but an in-transaction read still
    // resolved through the cache and saw nothing. A caller that moved a
    // node and then upserted it at its new path therefore took the CREATE
    // branch and minted a duplicate, which for a node type with a
    // `unique: true` property failed the whole write.
    for (_, node) in moves {
        cache.paths.insert(
            (workspace.to_string(), node.path.clone()),
            Some(node.id.clone()),
        );
        let key = (workspace.to_string(), node.id.clone());
        match cache.nodes.get_mut(&key) {
            Some(Some(staged)) => *staged = node.clone(),
            _ => {
                cache.moved_nodes.insert(key, node.clone());
            }
        }
    }
    Ok(())
}
