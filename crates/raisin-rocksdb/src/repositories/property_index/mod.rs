//! Property index repository implementation

mod helpers;
mod query;
mod scan;
mod write_ops;

use raisin_error::Result;
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
    ) -> Result<Vec<String>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        query::find_by_property(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            property_value,
            published_only,
        )
        .await
    }

    async fn find_by_property_with_limit(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        property_value: &PropertyValue,
        published_only: bool,
        limit: Option<usize>,
    ) -> Result<Vec<String>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        query::find_by_property_with_limit(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            property_value,
            published_only,
            limit,
        )
        .await
    }

    async fn count_by_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        property_value: &PropertyValue,
        published_only: bool,
    ) -> Result<usize> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        query::count_by_property(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            property_value,
            published_only,
        )
        .await
    }

    async fn find_nodes_with_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        published_only: bool,
    ) -> Result<Vec<String>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        query::find_nodes_with_property(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            published_only,
        )
        .await
    }

    async fn scan_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        published_only: bool,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<PropertyScanEntry>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        scan::scan_property(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            published_only,
            ascending,
            limit,
        )
        .await
    }

    async fn scan_property_range(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        lower_bound: Option<(&PropertyValue, bool)>,
        upper_bound: Option<(&PropertyValue, bool)>,
        published_only: bool,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<PropertyScanEntry>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        scan::scan_property_range(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            lower_bound,
            upper_bound,
            published_only,
            ascending,
            limit,
        )
        .await
    }
}
