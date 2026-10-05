//! Compound-index fixtures for the Phase 8 tests: a `test:Item` type with a
//! `by_cat (cat String)` index and a `unique: true` `code`.

use super::env::{node, Env, REPO, TENANT, WS};
use super::node_types::register_type_with;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_models::nodes::Node;
use raisin_storage::{CompoundColumnValue, CompoundIndexRepository, IndexType, Storage};

pub(super) const ITEM: &str = "test:Item";
pub(super) const BY_CAT: &str = "by_cat";

pub(super) fn by_cat() -> CompoundIndexDefinition {
    CompoundIndexDefinition {
        name: BY_CAT.to_string(),
        columns: vec![CompoundIndexColumn {
            property: "cat".to_string(),
            ascending: None,
            column_type: CompoundColumnType::String,
        }],
        has_order_column: false,
        owner: None,
    }
}

/// An item at `/{id}` with `cat` and any further string properties.
pub(super) fn item(id: &str, cat: &str, extra: &[(&str, &str)]) -> Node {
    let mut props = vec![("cat", cat)];
    props.extend_from_slice(extra);
    let mut n = node(id, &format!("/{id}"), &props);
    n.node_type = ITEM.to_string();
    n
}

impl Env {
    /// Register `test:Item` on `branch` and build its compound index there
    /// (the workspace is empty, so the build only earns `Ready`).
    pub(super) async fn with_items(&self, branch: &str) -> Result<()> {
        register_type_with(
            &self.storage,
            branch,
            ITEM,
            Some("code"),
            None,
            Some(vec![by_cat()]),
        )
        .await?;
        self.build_compound(branch).await
    }

    pub(super) async fn build_compound(&self, branch: &str) -> Result<()> {
        raisin_rocksdb::management::async_indexing::rebuild_indexes(
            &self.storage,
            TENANT,
            REPO,
            branch,
            WS,
            IndexType::Compound,
        )
        .await?;
        Ok(())
    }

    /// The node ids `by_cat = cat` lists, as of `at` (every entry if `None`).
    pub(super) async fn compound(
        &self,
        branch: &str,
        cat: &str,
        at: Option<&HLC>,
    ) -> Result<Vec<String>> {
        let mut ids: Vec<String> = self
            .storage
            .compound_index()
            .scan_compound_index(
                self.scope(branch),
                BY_CAT,
                &[CompoundColumnValue::String(cat.to_string())],
                false,
                true,
                None,
                at,
            )
            .await?
            .into_iter()
            .map(|entry| entry.node_id)
            .collect();
        ids.sort();
        Ok(ids)
    }

    /// Whether the planner may serve `by_cat` from the index on `branch`.
    pub(super) fn compound_ready(&self, branch: &str) -> bool {
        self.storage
            .compound_state()
            .expect("compound state source")
            .compound_availability(TENANT, REPO, branch, WS, &by_cat())
            .is_ready()
    }
}
