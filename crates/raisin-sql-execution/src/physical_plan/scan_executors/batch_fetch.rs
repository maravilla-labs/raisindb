//! How index scans read the nodes their index named: in chunks, through
//! `NodeRepository::get_many_for_read`.
//!
//! `PropertyIndexScan`, `CompoundIndexScan` and `ReferenceIndexScan` each get a
//! list of candidate ids from an index and used to await one `get` per id.
//! They now hand a chunk of ids to ONE batched read at the statement's
//! revision, through the statement's storage snapshot — so a 256-row chunk
//! costs one blocking task and one iterator per column family, and every chunk
//! of the statement sees the same view of the database.
//!
//! Under a `LIMIT` the first chunk is only as large as the rows still owed (1
//! under `LIMIT 1`), so a point query never reads ahead. When a chunk comes up
//! short (candidates dropped by RLS, a recheck, an orphan entry), the next one
//! at least doubles, so a run of rejected candidates costs O(log n) batched
//! reads, not one blocking hop per candidate. `sql.batched_fetch = false`
//! (`ExecutionContext::batched_fetch`) takes the per-row path instead.

use crate::physical_plan::executor::ExecutionContext;
use raisin_error::Error;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::{
    get_many_by_loop, BatchReadItem, BranchScope, NodeRepository, ReadOpts, Storage,
};

/// Candidate ids a scan reads per batch when nothing bounds it.
pub(crate) const FETCH_CHUNK: usize = 256;

/// How many candidates to read next: under a limit, the rows still owed but at
/// least twice the `previous` chunk (0 before the first) — a chunk is only
/// followed by another when it came up short — clamped to 1..=[`FETCH_CHUNK`];
/// without one, a full chunk.
pub(crate) fn chunk_len(limit: Option<usize>, emitted: usize, previous: usize) -> usize {
    match limit {
        Some(limit) => limit
            .saturating_sub(emitted)
            .max(previous.saturating_mul(2))
            .clamp(1, FETCH_CHUNK),
        None => FETCH_CHUNK,
    }
}

/// The nodes `ids` name in `workspace` at `at`, in order — `None` for one that
/// does not exist then. Exactly what one `get` per id would return (rows never
/// carry `has_children`, so it is not computed).
pub(crate) async fn fetch_nodes<S: Storage>(
    ctx: &ExecutionContext<S>,
    scope: BranchScope<'_>,
    workspace: &str,
    ids: &[String],
    at: &HLC,
) -> Result<Vec<Option<Node>>, Error> {
    let items: Vec<BatchReadItem> = ids
        .iter()
        .map(|id| BatchReadItem::id(workspace, id))
        .collect();
    let opts = ReadOpts::default();
    if ctx.batched_fetch {
        ctx.storage
            .nodes()
            .get_many_for_read(
                scope,
                &items,
                at,
                ctx.statement_read_snapshot().as_ref(),
                opts,
            )
            .await
    } else {
        get_many_by_loop(ctx.storage.nodes(), scope, &items, at, &opts).await
    }
}

/// The node once per locale a scan emits it in, cloned for every locale but
/// the last — which takes the node itself. A scan with one locale (the common
/// case) clones nothing.
pub(crate) fn per_locale<'a>(
    node: Node,
    locales: &'a [String],
) -> impl Iterator<Item = (&'a str, Node)> + 'a {
    let mut node = Some(node);
    let last = locales.len().saturating_sub(1);
    locales.iter().enumerate().filter_map(move |(i, locale)| {
        let copy = if i == last {
            node.take()?
        } else {
            node.clone()?
        };
        Some((locale.as_str(), copy))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_follows_the_rows_still_owed() {
        assert_eq!(chunk_len(Some(1), 0, 0), 1);
        assert_eq!(chunk_len(Some(10), 4, 0), 6);
        assert_eq!(chunk_len(Some(10), 10, 0), 1);
        assert_eq!(chunk_len(Some(10_000), 0, 0), FETCH_CHUNK);
        assert_eq!(chunk_len(None, 0, 0), FETCH_CHUNK);
    }

    /// `LIMIT 1` behind 10,000 rejected candidates: the chunks double instead
    /// of staying at one, so the scan pays ~log2 batched reads, not 10,000.
    #[test]
    fn limit_one_with_rejected_candidates_grows_chunks() {
        let (mut previous, mut reads, mut read) = (0, 0, 0);
        while read < 10_000 {
            let n = chunk_len(Some(1), 0, previous);
            read += n;
            reads += 1;
            previous = n;
        }
        assert_eq!(chunk_len(Some(1), 0, 0), 1, "a point query reads one");
        assert!(reads < 60, "{reads} batched reads for 10,000 candidates");
        assert_eq!(chunk_len(Some(1), 0, FETCH_CHUNK), FETCH_CHUNK);
    }

    #[test]
    fn one_node_per_locale_in_order() {
        let node = Node {
            id: "n".into(),
            ..Default::default()
        };
        let locales = vec!["en".to_string(), "de".to_string()];
        let got: Vec<(&str, String)> = per_locale(node, &locales).map(|(l, n)| (l, n.id)).collect();
        assert_eq!(got, vec![("en", "n".to_string()), ("de", "n".to_string())]);
        assert_eq!(per_locale(Node::default(), &[]).count(), 0);
    }
}
