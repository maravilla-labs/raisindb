//! Path-ordered paging over the PATH_INDEX column family.
//!
//! PATH_INDEX keys are `{tenant}\0{repo}\0{branch}\0{workspace}\0path\0{path}\0{~revision}`.
//! A path never contains `\0`, so the byte that ends one path (`\0`) sorts
//! below every byte that could continue a longer one: key order IS byte-wise
//! path order, which is exactly what `ORDER BY path` means. Paging in that
//! order is therefore a seek plus a bounded walk, never a load-and-sort.

use super::super::super::helpers::is_tombstone;
use super::super::super::storage_node::PropertiesMode;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::PathOrderPage;

/// The live entry chosen for one path: path, node id, and the entry's revision.
type PathHit = (String, String, HLC);

/// The path being read and the entry chosen for it so far:
/// `(is_tombstone, node_id, revision)`.
type PathGroup = (String, Option<(bool, String, HLC)>);

/// Finish the current path: keep it if its visible entry is live.
fn close_group(group: &mut Option<PathGroup>, hits: &mut Vec<PathHit>) {
    if let Some((path, Some((false, node_id, revision)))) = group.take() {
        hits.push((path, node_id, revision));
    }
}

impl NodeRepositoryImpl {
    /// One page of a workspace in byte-wise path order; see
    /// [`raisin_storage::NodeRepository::scan_by_path_order_page`].
    ///
    /// Per path, the visible entry is the newest one at or below the read
    /// bound. A tombstone there means the path is free at that revision (the
    /// node was deleted or moved away) and the path is skipped. `limit` counts
    /// live paths, so a node that then fails to load still uses its slot —
    /// which is why the caller pages on `next_after`, not on the row count.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) fn scan_by_path_order_page_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        after_path: Option<&str>,
        limit: usize,
        descending: bool,
        max_revision: Option<&HLC>,
        properties: PropertiesMode,
    ) -> Result<PathOrderPage> {
        let limit = limit.max(1);

        // `{…}\0path\0` — every path entry of this workspace, and nothing else.
        let base = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("path")
            .build_prefix();
        let mut upper = base.clone();
        *upper.last_mut().expect("prefix ends with a separator") = 1;

        // Ascending resumes at `{after}\x01`: above every entry of `after`
        // itself (their next byte is the `\0` separator) and at or below every
        // longer or greater path. Descending seeks for the last key at or below
        // `{after}\0`, a strict prefix of `after`'s own entries — so it lands on
        // the greatest entry of the greatest path below `after`.
        let seek = match (after_path, descending) {
            (Some(after), false) => [base.as_slice(), after.as_bytes(), &[1]].concat(),
            (Some(after), true) => [base.as_slice(), after.as_bytes(), &[0]].concat(),
            (None, false) => base.clone(),
            (None, true) => upper.clone(),
        };
        let direction = if descending {
            rocksdb::Direction::Reverse
        } else {
            rocksdb::Direction::Forward
        };

        let mut read_opts = rocksdb::ReadOptions::default();
        read_opts.set_iterate_lower_bound(base.clone());
        read_opts.set_iterate_upper_bound(upper);
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let iter = self.db.iterator_cf_opt(
            cf_path,
            read_opts,
            rocksdb::IteratorMode::From(&seek, direction),
        );

        // Entries of one path are adjacent. Forward they come newest-first, so
        // the first one inside the bound decides; reversed they come
        // oldest-first, so the last one inside the bound does. Either way the
        // decision is final only once the path changes.
        let mut hits: Vec<PathHit> = Vec::new();
        let mut exhausted = true;
        let mut group: Option<PathGroup> = None;

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            let (Some(path), Some(revision)) = (
                keys::decode_path_from_path_index_key(&key),
                keys::decode_revision_from_path_index_key(&key),
            ) else {
                tracing::warn!("scan_by_path_order_page: undecodable path_index key, skipping");
                continue;
            };

            if group.as_ref().is_some_and(|(p, _)| *p != path) {
                close_group(&mut group, &mut hits);
                if hits.len() >= limit {
                    exhausted = false;
                    break;
                }
            }
            let (_, chosen) = group.get_or_insert_with(|| (path, None));

            if max_revision.is_some_and(|bound| revision > *bound) {
                continue;
            }
            if descending || chosen.is_none() {
                let node_id = String::from_utf8_lossy(&value).into_owned();
                *chosen = Some((is_tombstone(&value), node_id, revision));
            }
        }
        if exhausted {
            close_group(&mut group, &mut hits);
        }

        let next_after = if exhausted {
            None
        } else {
            hits.last().map(|(path, _, _)| path.clone())
        };

        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let mut nodes = Vec::with_capacity(hits.len());
        for (path, node_id, path_revision) in hits {
            let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, &node_id);
            let Some((blob_revision, bytes)) =
                crate::mvcc_read::newest_at_or_before(&self.db, cf_nodes, &prefix, max_revision)?
            else {
                continue;
            };
            if is_tombstone(&bytes) {
                continue;
            }
            let read_at = match max_revision {
                Some(bound) => *bound,
                None => blob_revision.max(path_revision),
            };
            let node = self.deserialize_node_with_path_as(
                &bytes,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node_id,
                &read_at,
                &blob_revision,
                properties,
            )?;
            // A live entry whose node now lives elsewhere is a stale index
            // row; the node is emitted at its real path instead.
            if node.path == path {
                nodes.push(node);
            } else {
                tracing::debug!(
                    "scan_by_path_order_page: index path '{}' resolves to node '{}' at '{}', skipping",
                    path,
                    node_id,
                    node.path
                );
            }
        }

        Ok(PathOrderPage { nodes, next_after })
    }
}
