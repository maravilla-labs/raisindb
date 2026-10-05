//! What an `update_impl` write is: an edit, or the timestamp backfill (plan
//! Phase 13g) — one write funnel for both, so the backfill reaches every
//! derived index, the revision index and replication exactly like an edit.
//!
//! **Where a backfill writes (Phase 13g review).** IN PLACE, at the revision
//! of the version it read ([`UpdateMode::backfill_in_place_at`]) — not at a
//! fresh revision. Any edit the backfill races (a transaction that allocated
//! its revision earlier and commits later, a peer's edit this node has not
//! applied yet, a replica lagging behind) carries a HIGHER revision than the
//! version read, so it wins at HEAD whatever the commit or arrival order; a
//! fresh revision would put the read content back over it, on every node.
//! Two nodes that backfill the same version write the same bytes at the same
//! key. And the HEAD does not move, so no revision without a record enters
//! the branch's ancestry. The commit is also CONDITIONAL
//! (`NodeCommit::only_if_unchanged`): a version written between the read and
//! the commit (an in-place `versionable: false` rewrite at the same revision
//! included) makes it write nothing, and the next run retries the node.
//!
//! Only when an index-only write sits above the version read (an ancestor
//! move re-keyed the node's path after its last record write) or the version
//! is above the branch HEAD does a backfill take a fresh revision — with its
//! revision record (empty `changed_nodes`, so merges see no change), like
//! every revision the funnel moves the HEAD to.

use super::super::NodeRepositoryImpl;
use chrono::{DateTime, Utc};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{NodeChangeInfo, RevisionMeta};

/// How `update_impl_as` treats the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdateMode {
    /// An edit: the caller's node, validated (placement, unique claims),
    /// `updated_at` stamped now and the edit counter bumped.
    Edit,
    /// The timestamp backfill: the STORED version, read inside the funnel,
    /// with a missing `created_at` / `updated_at` set to these values and
    /// nothing else changed — no placement, unique-claim or localized-name
    /// validation and no UNIQUE claim written (nothing they judge or hold
    /// changes, and legacy duplicates must neither refuse it nor be handed a
    /// claim another node holds), no `updated_at = now`, no edit-counter
    /// bump; written in place, conditionally (see the module doc).
    TimestampBackfill {
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    },
}

impl UpdateMode {
    pub(crate) fn is_edit(&self) -> bool {
        matches!(self, Self::Edit)
    }

    /// The version a backfill writes over `stored`, or `None` when `stored`
    /// already has both timestamps (idempotent: such a node is never touched).
    pub(crate) fn backfilled(&self, stored: &Node) -> Option<Node> {
        let Self::TimestampBackfill {
            created_at,
            updated_at,
        } = self
        else {
            return None;
        };
        if stored.created_at.is_some() && stored.updated_at.is_some() {
            return None;
        }
        let mut node = stored.clone();
        node.created_at.get_or_insert(*created_at);
        node.updated_at.get_or_insert(*updated_at);
        node.parent = Node::extract_parent_name_from_path(&node.path);
        node.has_children = None;
        Some(node)
    }

    /// The revision a backfill rewrites in place: the version it read,
    /// whose revision `recorded` holds (newest NODES version, newest
    /// NODE_PATH entry), when no NODE_PATH entry sits above it (an index-only
    /// re-key: the path materialized at HEAD is then not the version's own)
    /// and it is not above the branch `head`. `None` for an edit, or when a
    /// backfill must take a fresh revision.
    pub(crate) fn backfill_in_place_at(
        &self,
        recorded: (Option<HLC>, Option<HLC>),
        head: HLC,
    ) -> Option<HLC> {
        match (self, recorded) {
            (Self::TimestampBackfill { .. }, (Some(version), Some(path)))
                if path <= version && version <= head =>
            {
                Some(version)
            }
            _ => None,
        }
    }

    /// The revision record of a write at a FRESH `revision` (its parent is
    /// set at the commit, under the branch lock). An edit lists the node it
    /// changed, so merges and conflict detection carry it; a backfill lists
    /// nothing — it changes no content a merge could carry or conflict on.
    pub(crate) fn revision_record(
        &self,
        branch: &str,
        workspace: &str,
        node: &Node,
        revision: HLC,
        actor: Option<&str>,
    ) -> RevisionMeta {
        let actor = actor.map(str::trim).filter(|a| !a.is_empty());
        let (message, changed_nodes) = if self.is_edit() {
            let change = NodeChangeInfo {
                node_id: node.id.clone(),
                workspace: workspace.to_string(),
                operation: ChangeOperation::Modified,
                translation_locale: None,
            };
            (format!("Update {}", node.path), vec![change])
        } else {
            (
                format!("Backfill created_at/updated_at of {}", node.id),
                vec![],
            )
        };
        RevisionMeta {
            revision,
            parent: None,
            merge_parent: None,
            branch: branch.to_string(),
            timestamp: Utc::now(),
            actor: actor.unwrap_or(crate::constants::SYSTEM_ACTOR).to_string(),
            message,
            is_system: !self.is_edit() || actor.is_none(),
            changed_nodes,
            changed_node_types: Vec::new(),
            changed_archetypes: Vec::new(),
            changed_element_types: Vec::new(),
            operation: None,
        }
    }
}

/// The actor a backfill writes as (its replication op and revision record).
pub(crate) const TIMESTAMP_BACKFILL_ACTOR: &str = crate::constants::SYSTEM_ACTOR;

impl NodeRepositoryImpl {
    /// Fill the missing `created_at` / `updated_at` of node `id`'s NEWEST
    /// version with `created_at` / `updated_at`, as the system actor, through
    /// the repository write funnel: rewritten in place at that version's
    /// revision (see the module doc), every derived index maintained,
    /// captured for replication. Publishes no node event — the repository
    /// layer never does (events come from `NodeService`), so no trigger,
    /// webhook or subscription sees a backfill. Returns whether a version was
    /// written: `false` when the node has both timestamps already, is
    /// deleted, or was written meanwhile (the next run retries it).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn backfill_timestamps(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Result<bool> {
        let stub = Node {
            id: id.to_string(),
            ..Node::default()
        };
        self.update_impl_as(
            tenant_id,
            repo_id,
            branch,
            workspace,
            stub,
            crate::repositories::nodes::WriteAttribution::actor(Some(TIMESTAMP_BACKFILL_ACTOR)),
            &UpdateMode::TimestampBackfill {
                created_at,
                updated_at,
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn a_backfill_fills_only_what_is_missing() {
        let mode = UpdateMode::TimestampBackfill {
            created_at: at(1),
            updated_at: at(2),
        };
        let mut stored = Node {
            id: "n".into(),
            path: "/a/n".into(),
            ..Node::default()
        };
        let filled = mode.backfilled(&stored).expect("both missing");
        assert_eq!(
            (filled.created_at, filled.updated_at),
            (Some(at(1)), Some(at(2)))
        );
        assert_eq!(filled.parent.as_deref(), Some("a"));

        stored.updated_at = Some(at(9));
        let filled = mode.backfilled(&stored).expect("created_at missing");
        assert_eq!(
            (filled.created_at, filled.updated_at),
            (Some(at(1)), Some(at(9)))
        );

        stored.created_at = Some(at(8));
        assert!(
            mode.backfilled(&stored).is_none(),
            "nothing missing: no write"
        );
        assert!(UpdateMode::Edit.backfilled(&Node::default()).is_none());
    }

    #[test]
    fn a_backfill_rewrites_in_place_unless_something_sits_above_the_version() {
        let mode = UpdateMode::TimestampBackfill {
            created_at: at(1),
            updated_at: at(2),
        };
        let (r1, r2, head) = (HLC::new(10, 0), HLC::new(20, 0), HLC::new(30, 0));
        assert_eq!(
            mode.backfill_in_place_at((Some(r2), Some(r2)), head),
            Some(r2)
        );
        assert_eq!(
            mode.backfill_in_place_at((Some(r2), Some(r1)), head),
            Some(r2)
        );
        assert_eq!(
            mode.backfill_in_place_at((Some(r1), Some(r2)), head),
            None,
            "an index-only re-key above the version"
        );
        assert_eq!(mode.backfill_in_place_at((Some(r2), None), head), None);
        assert_eq!(
            mode.backfill_in_place_at((Some(r2), Some(r2)), r1),
            None,
            "a version above HEAD"
        );
        assert_eq!(
            UpdateMode::Edit.backfill_in_place_at((Some(r2), Some(r2)), head),
            None
        );
    }
}
