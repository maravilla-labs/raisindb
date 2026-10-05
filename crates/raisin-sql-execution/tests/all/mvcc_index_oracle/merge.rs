//! Fork, edit both sides, merge back with KeepOurs / KeepTheirs resolutions.
//!
//! The model merges three-way by NODE: a node changed on one side only takes
//! that side's version; a node changed on both is a conflict and takes the
//! resolved side's version (deletion included). Sibling order is decided per
//! parent: structural edits (create / delete) under one parent are confined to
//! one side at drive time, so the merged order is that side's list — plus any
//! node a resolution resurrected, at its base position. That confinement is
//! what lets the model stay label-free.
//!
//! `versionable=false` nodes are edited inside a fork on the MAIN side only
//! (`Edit::Volatile`): merge conflict detection does not see in-place writes
//! (no revision is minted), so a node rewritten in place on both sides would
//! be documented-unpredictable — but a main-only rewrite must survive the
//! merge, which re-copies the feature's view of that very key.
//!
//! The window's `main` snapshots are asserted strictly just BEFORE the merge
//! (`check_window_before_merge`); after it they are `retro`.

use super::driver::Run;
use super::env::MAIN;
use super::merge_model::{merged, Side};
use super::model::Snapshot;
use super::ops::ForkSpec;
use raisin_context::{ConflictResolution, MergeStrategy, ResolutionType};
use raisin_storage::{BranchRepository, Storage};
use std::collections::{BTreeSet, HashSet};

impl Run {
    pub async fn fork_merge(&mut self, spec: &ForkSpec) -> Option<String> {
        // Names under a parent stay unique ACROSS the merge: neither side may
        // reuse a name the base or the other side holds (a delete-then-create
        // of the same name on one side would otherwise put two nodes at one
        // path after a resolution kept the deleted one).
        self.forks += 1;
        let feature = format!("f{}", self.forks);
        if let Err(e) = self
            .env
            .storage
            .branches()
            .create_branch(
                super::env::TENANT,
                super::env::REPO,
                &feature,
                "oracle",
                None,
                Some(MAIN.to_string()),
                false,
                false,
            )
            .await
        {
            self.anomalies.push(format!("fork {feature} failed: {e}"));
            return Some(format!("fork {feature} (FAILED)"));
        }
        self.env.unlock_skip_unchanged(&feature).await;
        let base = self.tree.clone();
        let mut ft = base.clone();
        let (mut fs, mut ms) = (Side::default(), Side::default());
        for e in &spec.feature {
            self.fork_edit(&feature, &mut ft, &mut fs, &ms, &base, e)
                .await;
        }
        let fhead = self.env.head(&feature).await;
        self.snaps.push(Snapshot {
            branch: feature.clone(),
            head: fhead,
            op: self.op_index,
            tree: ft.clone(),
            tainted: Default::default(),
            retro: false,
            overlay_gap: Default::default(),
            replica: false,
        });
        let mut mt = self.tree.clone();
        let window = self.snaps.len();
        for e in &spec.main {
            self.fork_edit(MAIN, &mut mt, &mut ms, &fs, &base, e).await;
        }
        self.tree = mt;
        self.check_window_before_merge(window).await;

        // Only main-side in-place rewrites (or nothing): the merge finds the
        // branches in sync and commits nothing, so HEAD stays put.
        self.last_in_place = fs.changed.is_empty() && ms.changed.is_empty();
        let predicted: BTreeSet<String> = fs.changed.intersection(&ms.changed).cloned().collect();
        let branches = self.env.storage.branches_impl();
        let (t, r) = (super::env::TENANT, super::env::REPO);
        let attempt = branches
            .merge_branches(
                t,
                r,
                MAIN,
                &feature,
                MergeStrategy::ThreeWay,
                "merge",
                "oracle",
            )
            .await;
        let attempt = match attempt {
            Ok(a) => a,
            Err(e) => {
                self.anomalies.push(format!("merge {feature} failed: {e}"));
                return Some(format!("fork-merge {feature} (FAILED)"));
            }
        };
        let reported: BTreeSet<String> = attempt
            .conflicts
            .iter()
            .map(|c| c.node_id.clone())
            .collect();
        if reported != predicted {
            self.anomalies.push(format!(
                "merge {feature}: conflicts {reported:?}, model predicted {predicted:?}"
            ));
        }
        let mut keep_ours = HashSet::new();
        if !attempt.success {
            let resolutions: Vec<ConflictResolution> = reported
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    let ours = spec.keep_ours[i % spec.keep_ours.len()];
                    if ours {
                        keep_ours.insert(id.clone());
                    }
                    ConflictResolution {
                        node_id: id.clone(),
                        resolution_type: if ours {
                            ResolutionType::KeepOurs
                        } else {
                            ResolutionType::KeepTheirs
                        },
                        resolved_properties: serde_json::Value::Null,
                        translation_locale: None,
                    }
                })
                .collect();
            if let Err(e) = branches
                .resolve_merge_with_resolutions(
                    t,
                    r,
                    MAIN,
                    &feature,
                    resolutions,
                    "resolved",
                    "oracle",
                )
                .await
            {
                self.anomalies
                    .push(format!("resolve {feature} failed: {e}"));
            }
        }
        let main_before = self.tree.clone();
        self.tree = merged(&base, &self.tree, &ft, &fs, &predicted, &keep_ours);
        // A conflict resolved by KEEPING a node one side deleted: merge apply
        // writes no translation overlay, so the deleting side's overlay
        // tombstones win (known gap, see `Snapshot::overlay_gap`).
        for id in &predicted {
            let deleted_somewhere =
                !ft.nodes.contains_key(id) || !main_before.nodes.contains_key(id);
            if deleted_somewhere && self.tree.nodes.contains_key(id) {
                self.overlay_events.gap.push((self.op_index, id.clone()));
            }
        }
        // Only now, after they were asserted strictly above, do these
        // revisions become `retro`: the merge rewrites what they read.
        for snap in &mut self.snaps[window..] {
            if snap.branch == MAIN {
                snap.retro = true;
            }
        }
        Some(format!(
            "fork-merge {feature}: feature {:?} main {:?} conflicts {predicted:?} keep_ours {keep_ours:?}",
            spec.feature.len(),
            spec.main.len()
        ))
    }

    /// Assert the fork window's `main` snapshots NOW, while they are still
    /// the truth. After the merge they show the feature's changes
    /// retroactively (`MERGE_RETRO`, tolerated); checking them only then left
    /// every main-side edit inside a fork unchecked at its own revision.
    async fn check_window_before_merge(&mut self, window: usize) {
        let keep: HashSet<(raisin_hlc::HLC, usize)> = self.snaps[window..]
            .iter()
            .filter(|s| s.branch == MAIN)
            .map(|s| (s.head, s.op))
            .collect();
        if keep.is_empty() {
            return;
        }
        let mut snaps = self.snaps.clone();
        super::model::apply_taint(&mut snaps, &self.writes);
        super::model::apply_overlay_gaps(&mut snaps, &self.overlay_events);
        let found = super::check_snaps(&self.env, &snaps, &self.instants, |s| {
            s.branch == MAIN && keep.contains(&(s.head, s.op))
        })
        .await;
        self.pre_merge.extend(found.into_iter().map(|mut m| {
            m.detail = format!("[before merge] {}", m.detail);
            m
        }));
    }
}
