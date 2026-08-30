//! Schema carry-over for cross-branch copy (branch promotion).
//!
//! # The two legs disagreed about a missing NodeType
//!
//! `copy_nodes_across_branches` promoted CONTENT and nothing else, so a node
//! could land on the target branch whose `node_type` does not exist there.
//! Two subsystems then met that same condition and made opposite decisions:
//!
//! * the indexing planner (`jobs/handlers/fulltext/plan.rs`) **failed open** —
//!   `NodeType unresolved; falling back to legacy index-all-strings`, so the
//!   node was indexed, but with per-field `VECTOR` / `FULLTEXT` selection
//!   silently discarded; and
//! * a `__branch`-targeted SQL write **failed closed** —
//!   `Not found: NodeType not found: proof:Doc`, refusing the write outright.
//!
//! Same gap, two policies, and which one a user meets depends only on which
//! publish leg they happen to be on. That is the mirrored-path drift CLAUDE.md
//! names as this codebase's dominant bug class.
//!
//! # Why this is fixed by CARRYING the schema, not by aligning the policies
//!
//! Neither policy is wrong where it sits, which is the tell that the gap itself
//! is the defect:
//!
//! * Making the planner fail closed would leave promoted content on the branch,
//!   visible to `SELECT` and invisible to both semantic and lexical search —
//!   with no error anywhere. That is precisely the failure the cross-branch
//!   event emission was added to fix; re-creating it via the plan resolver
//!   would be a regression wearing a safety argument. Failing open there
//!   OVER-indexes, which a `REBUILD` fixes; failing closed UNDER-indexes, which
//!   nothing reports and therefore nobody fixes.
//! * Making the SQL write fail open would let an arbitrary `node_type` string
//!   onto a branch, and that validation is the only thing standing between a
//!   typo and a permanently unqueryable row.
//!
//! So the fix is to stop producing the condition: a promotion carries the
//! schema its nodes reference. Both legs then agree, because the precondition
//! they disagreed about holds — and the planner's legacy fallback stops being
//! reachable through publish at all. The suppression rules below keep that free
//! in the steady state.
//!
//! # What is carried
//!
//! The transitive closure over inheritance of everything the promoted node set
//! references: NodeTypes (through `extends` **and** `mixins`), Archetypes
//! (through `extends`, plus their `base_node_type`) and ElementTypes (through
//! `extends`). A partial closure is worse than none — resolving `proof:Doc`
//! fails just as hard when `proof:Doc` is present and its supertype is not, and
//! it fails with a message naming the supertype, which reads like an unrelated
//! bug.
//!
//! ElementType names come from `fulltext::collect_element_types`, the same
//! function the planner resolves against, so the carried set cannot drift from
//! the resolved set.
//!
//! # Two rules that keep a steady-state publish silent
//!
//! 1. **Carry before the copy revision is allocated.** These upserts are their
//!    own transactions with their own revisions; running them first keeps the
//!    schema strictly older than the content that depends on it, and keeps the
//!    target HEAD monotonic.
//! 2. **Upsert only on a SEMANTIC difference.** `upsert` stamps `updated_at`
//!    and bumps `version`, so a definition read back from the target NEVER
//!    equals the source it was copied from. Comparing whole structs would
//!    therefore rewrite every schema on every publish forever — the same churn
//!    defect the translation copy carried. `same_definition` strips identity
//!    and versioning metadata before comparing, so re-publishing an unchanged
//!    site writes nothing here.
//!
//! # Carrying is best-effort, never a gate
//!
//! A name that does not resolve on the SOURCE branch either is skipped with a
//! debug line. A publish must not start failing because one legacy node carries
//! a `node_type` string nobody ever defined; that node keeps the behaviour it
//! has today (the planner's legacy fallback), and everything else still gets
//! its schema.

use super::super::super::super::NodeRepositoryImpl;
use super::CopyEntry;
use crate::jobs::handlers::fulltext::collect_element_types;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::NodeType;
use raisin_storage::{
    ArchetypeRepository, BranchScope, CommitMetadata, ElementTypeRepository, NodeTypeRepository,
};
use std::collections::{HashSet, VecDeque};

/// What a promotion actually wrote to the target branch's schema.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct SchemaCarrySummary {
    pub node_types: usize,
    pub archetypes: usize,
    pub element_types: usize,
}

impl SchemaCarrySummary {
    /// True when the carry advanced the target branch HEAD.
    pub fn wrote_anything(&self) -> bool {
        self.node_types + self.archetypes + self.element_types > 0
    }
}

/// A schema definition whose identity and versioning metadata can be stripped
/// for comparison.
///
/// One trait rather than three ad-hoc comparisons, because the three types
/// carry the SAME metadata set and a field missed on one of them is invisible:
/// nothing fails, that kind of schema just churns on every publish while the
/// other two stay quiet.
trait Versioned: Clone + PartialEq {
    /// Blank every field the STORE owns rather than the author.
    fn strip_versioning(&mut self);
}

/// Do these two definitions MEAN the same thing?
///
/// Deliberately not `a == b`: `upsert` rewrites `id`, `version`, `updated_at`
/// and `previous_version` on the way in, so a definition read back from the
/// target never equals the source it came from.
fn same_definition<T: Versioned>(a: &T, b: &T) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.strip_versioning();
    b.strip_versioning();
    a == b
}

impl Versioned for NodeType {
    fn strip_versioning(&mut self) {
        self.id = None;
        self.version = None;
        self.created_at = None;
        self.updated_at = None;
        self.published_at = None;
        self.published_by = None;
        self.previous_version = None;
    }
}

impl Versioned for Archetype {
    fn strip_versioning(&mut self) {
        self.id = String::new();
        self.version = None;
        self.created_at = None;
        self.updated_at = None;
        self.published_at = None;
        self.published_by = None;
        self.previous_version = None;
    }
}

impl Versioned for ElementType {
    fn strip_versioning(&mut self) {
        self.id = String::new();
        self.version = None;
        self.created_at = None;
        self.updated_at = None;
        self.published_at = None;
        self.published_by = None;
        self.previous_version = None;
    }
}

impl NodeRepositoryImpl {
    /// Carry onto the target branch every schema definition the promoted node
    /// set references, transitively through inheritance.
    ///
    /// Must run BEFORE the copy revision is allocated — see the module docs.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn carry_schema_to_target(
        &self,
        entries: &[CopyEntry],
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        source_revision: Option<&HLC>,
        actor: &str,
        is_system: bool,
    ) -> Result<SchemaCarrySummary> {
        // Promoting onto the branch you read from would compare every
        // definition against itself and write nothing, but the reads are real
        // work on a large set — short-circuit.
        if source_branch == target_branch || entries.is_empty() {
            return Ok(SchemaCarrySummary::default());
        }

        let src = BranchScope {
            tenant_id,
            repo_id,
            branch: source_branch,
        };
        let dst = BranchScope {
            tenant_id,
            repo_id,
            branch: target_branch,
        };
        let commit = CommitMetadata {
            message: format!(
                "Carry schema referenced by promotion from '{}' to '{}'",
                source_branch, target_branch
            ),
            actor: actor.to_string(),
            is_system,
        };

        let mut node_types: VecDeque<String> = VecDeque::new();
        let mut archetypes: VecDeque<String> = VecDeque::new();
        let mut element_types: VecDeque<String> = VecDeque::new();

        for entry in entries {
            node_types.push_back(entry.node.node_type.clone());
            if let Some(archetype) = &entry.node.archetype {
                archetypes.push_back(archetype.clone());
            }
            for element_type in collect_element_types(&entry.node.properties) {
                element_types.push_back(element_type);
            }
        }

        let mut summary = SchemaCarrySummary::default();

        // ARCHETYPES FIRST, because an archetype names the NodeType it is based
        // on and that type has to travel too — draining archetypes before node
        // types is what lets it just push onto the node-type queue.
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(name) = archetypes.pop_front() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(source_def) = self.archetype_repo.get(src, &name, source_revision).await?
            else {
                tracing::debug!(
                    archetype = %name,
                    branch = %source_branch,
                    "cross-branch schema carry: Archetype not on the source branch, skipping"
                );
                continue;
            };

            if let Some(parent) = &source_def.extends {
                archetypes.push_back(parent.clone());
            }
            if let Some(base) = &source_def.base_node_type {
                node_types.push_back(base.clone());
            }

            if self
                .archetype_repo
                .get(dst, &name, None)
                .await?
                .as_ref()
                .is_some_and(|existing| same_definition(existing, &source_def))
            {
                continue;
            }

            self.archetype_repo
                .upsert(dst, source_def, commit.clone())
                .await?;
            summary.archetypes += 1;
            tracing::debug!(
                archetype = %name,
                target_branch = %target_branch,
                "cross-branch schema carry: Archetype written to the target branch"
            );
        }

        // ELEMENT TYPES: closure over `extends`.
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(name) = element_types.pop_front() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(source_def) = self
                .element_type_repo
                .get(src, &name, source_revision)
                .await?
            else {
                tracing::debug!(
                    element_type = %name,
                    branch = %source_branch,
                    "cross-branch schema carry: ElementType not on the source branch, skipping"
                );
                continue;
            };

            if let Some(parent) = &source_def.extends {
                element_types.push_back(parent.clone());
            }

            if self
                .element_type_repo
                .get(dst, &name, None)
                .await?
                .as_ref()
                .is_some_and(|existing| same_definition(existing, &source_def))
            {
                continue;
            }

            self.element_type_repo
                .upsert(dst, source_def, commit.clone())
                .await?;
            summary.element_types += 1;
            tracing::debug!(
                element_type = %name,
                target_branch = %target_branch,
                "cross-branch schema carry: ElementType written to the target branch"
            );
        }

        // NODE TYPES LAST: closure over `extends` AND `mixins`, over a queue the
        // archetype pass may have added to.
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(name) = node_types.pop_front() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(source_def) = self.node_type_repo.get(src, &name, source_revision).await?
            else {
                tracing::debug!(
                    node_type = %name,
                    branch = %source_branch,
                    "cross-branch schema carry: NodeType not on the source branch, skipping"
                );
                continue;
            };

            if let Some(parent) = &source_def.extends {
                node_types.push_back(parent.clone());
            }
            for mixin in &source_def.mixins {
                node_types.push_back(mixin.clone());
            }

            if self
                .node_type_repo
                .get(dst, &name, None)
                .await?
                .as_ref()
                .is_some_and(|existing| same_definition(existing, &source_def))
            {
                continue;
            }

            self.node_type_repo
                .upsert(dst, source_def, commit.clone())
                .await?;
            summary.node_types += 1;
            tracing::debug!(
                node_type = %name,
                target_branch = %target_branch,
                "cross-branch schema carry: NodeType written to the target branch"
            );
        }

        if summary.wrote_anything() {
            tracing::info!(
                source_branch = %source_branch,
                target_branch = %target_branch,
                node_types = summary.node_types,
                archetypes = summary.archetypes,
                element_types = summary.element_types,
                "cross-branch schema carry: wrote referenced definitions to the target branch"
            );
        }

        Ok(summary)
    }
}
