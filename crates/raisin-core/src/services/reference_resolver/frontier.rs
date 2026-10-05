//! The frontier walk: which targets a resolution needs, level by level.
//!
//! The old resolver rewrote the WHOLE document once per level and re-walked it
//! to find the next level's references — so a depth-3 resolution of a page
//! walked the page, then the page with level 1 inlined, then the page with
//! level 2 inlined. Here the document is walked once, and after that only the
//! targets are: level N+1 is the references of targets FIRST reached at level
//! N, which were collected when those targets were fetched. A target reached
//! again later needs no more levels than it already got, so it is never
//! descended into twice, which is also what makes a cycle cheap.

use super::memo::ReadScope;
use super::walk::{self, Resolved, TargetRef};
use super::ReferenceResolver;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;
use std::collections::HashSet;
use std::sync::Arc;

impl<S: Storage> ReferenceResolver<S> {
    /// Every target the inlining of `values` to `depth` levels can reach,
    /// decided. Targets beyond the last level are never read.
    ///
    /// Several documents share ONE walk (a chunk of rows): level 1 is the union
    /// of their references, so each level is one batched read for all of them.
    /// The union reaches exactly the targets the documents reach one by one —
    /// a BFS from many sources visits the union of each source's ball.
    pub(super) async fn gather(
        &self,
        workspace: &str,
        values: &[PropertyValue],
        depth: u32,
        read: &Arc<ReadScope>,
        fields: Option<&[String]>,
    ) -> Result<Resolved> {
        let mut resolved = Resolved::new();
        let mut queued: HashSet<TargetRef> = HashSet::new();
        let mut frontier: Vec<TargetRef> = Vec::new();
        for value in values {
            for raw in walk::distinct_refs(value) {
                let target = raw.target(workspace);
                if queued.insert(target.clone()) {
                    frontier.push(target);
                }
            }
        }

        let mut level = 1;
        while !frontier.is_empty() {
            let entries = self.fetch_all(read, &frontier, fields).await?;
            let mut next = Vec::new();
            for (target, entry) in frontier.drain(..).zip(entries) {
                if level < depth {
                    if let Some(found) = &entry {
                        for raw in &found.refs {
                            let child = raw.target(workspace);
                            if queued.insert(child.clone()) {
                                next.push(child);
                            }
                        }
                    }
                }
                resolved.insert(target, entry);
            }
            frontier = next;
            level += 1;
        }
        Ok(resolved)
    }
}
