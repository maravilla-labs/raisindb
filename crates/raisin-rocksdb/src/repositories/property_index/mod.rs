//! Property index repository implementation

mod helpers;
pub(crate) mod orphans;
pub(crate) mod reader;
mod write_ops;

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::scope::StorageScope;
use raisin_storage::{PropertyIndexRepository, PropertyScanEntry};
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct PropertyIndexRepositoryImpl {
    db: Arc<DB>,
}

impl PropertyIndexRepositoryImpl {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Every read goes through the one revision-bounded reader.
    fn reader<'a>(
        &'a self,
        scope: StorageScope<'_>,
        property_name: &'a str,
        published_only: bool,
        max_revision: Option<&HLC>,
    ) -> Result<reader::PropertyIndexReader<'a>> {
        reader::PropertyIndexReader::new(
            &self.db,
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
            property_name,
            published_only,
            max_revision,
        )
    }
}

impl PropertyIndexRepository for PropertyIndexRepositoryImpl {
    async fn index_properties(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        is_published: bool,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        write_ops::index_properties(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            properties,
            is_published,
        )
        .await
    }

    async fn unindex_properties(&self, scope: StorageScope<'_>, node_id: &str) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        write_ops::unindex_properties(&self.db, tenant_id, repo_id, branch, workspace, node_id)
            .await
    }

    async fn update_publish_status(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        is_published: bool,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        write_ops::update_publish_status(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            properties,
            is_published,
        )
        .await
    }

    async fn find_by_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        property_value: &PropertyValue,
        published_only: bool,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<String>> {
        self.reader(scope, property_name, published_only, max_revision)?
            .find(property_value, None)
    }

    async fn find_by_property_with_limit(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        property_value: &PropertyValue,
        published_only: bool,
        max_revision: Option<&HLC>,
        limit: Option<usize>,
    ) -> Result<Vec<String>> {
        self.reader(scope, property_name, published_only, max_revision)?
            .find(property_value, limit)
    }

    async fn count_by_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        property_value: &PropertyValue,
        published_only: bool,
        max_revision: Option<&HLC>,
    ) -> Result<usize> {
        self.reader(scope, property_name, published_only, max_revision)?
            .count(property_value)
    }

    async fn find_nodes_with_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        published_only: bool,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<String>> {
        self.reader(scope, property_name, published_only, max_revision)?
            .nodes_with_property()
    }

    async fn scan_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        published_only: bool,
        max_revision: Option<&HLC>,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<PropertyScanEntry>> {
        self.reader(scope, property_name, published_only, max_revision)?
            .scan(None, None, ascending, limit)
    }

    async fn scan_property_range(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        lower_bound: Option<(&PropertyValue, bool)>,
        upper_bound: Option<(&PropertyValue, bool)>,
        published_only: bool,
        max_revision: Option<&HLC>,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<PropertyScanEntry>> {
        self.reader(scope, property_name, published_only, max_revision)?
            .scan(lower_bound, upper_bound, ascending, limit)
    }
}
