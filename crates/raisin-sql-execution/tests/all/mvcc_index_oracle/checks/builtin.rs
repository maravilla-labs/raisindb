//! Plan Phase 13f: the BUILT-IN `@__children_by_created_at` keyspace, read
//! straight from storage (the oracle's workspace also declares `@folder_time`
//! with the same columns, which wins the planner's tie, so SQL alone would
//! never reach the built-in). Wherever its record says `Ready`, a listing at
//! or above the build floor must be every child of the parent, newest first.

use super::order::grouped_match;
use super::tree::{path_of, sample_parents};
use super::{name, Checker};
use crate::mvcc_index_oracle::env::{REPO, TENANT, WS};
use crate::mvcc_index_oracle::model::{by_stamp, Snapshot};
use raisin_models::workspace::builtin_indexes::children_by_created_at_stored_name;
use raisin_storage::compound::CompoundBuildPhase;
use raisin_storage::{CompoundColumnValue, CompoundIndexRepository, Storage, StorageScope};

impl Checker<'_> {
    pub async fn builtin_folder_index(&mut self, s: &Snapshot) {
        let index = children_by_created_at_stored_name();
        let storage = self.env.storage.clone();
        let Ok(Some(state)) = raisin_rocksdb::compound_state::read_state(
            storage.db(),
            TENANT,
            REPO,
            &s.branch,
            WS,
            &index,
        ) else {
            return; // never built on this branch: nothing vouches for it
        };
        if state.phase != CompoundBuildPhase::Ready || s.head < state.built_through {
            return;
        }
        for p in sample_parents(s, 2) {
            let path = path_of(s, &p);
            let listed = storage
                .compound_index()
                .scan_compound_index(
                    StorageScope::new(TENANT, REPO, &s.branch, WS),
                    &index,
                    &[CompoundColumnValue::String(path.clone())],
                    false,
                    true,
                    None,
                    Some(&s.head),
                )
                .await;
            let got: Vec<String> = match listed {
                Ok(entries) => entries.into_iter().map(|e| e.node_id).collect(),
                Err(e) => {
                    self.report(name::BUILTIN_FOLDER, s, format!("{path}: {e}"));
                    continue;
                }
            };
            let mut groups = by_stamp(
                s.tree
                    .kids(&p)
                    .iter()
                    .map(|c| (c.as_str(), s.tree.nodes[c].created)),
            );
            groups.reverse();
            if !grouped_match(&got, &groups, usize::MAX) {
                self.report(
                    name::BUILTIN_FOLDER,
                    s,
                    format!("{index} under {path}: got {got:?}, model tie-groups {groups:?}"),
                );
            }
        }
    }
}
