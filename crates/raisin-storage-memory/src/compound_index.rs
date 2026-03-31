use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::scope::StorageScope;
use raisin_storage::{CompoundColumnValue, CompoundIndexRepository, CompoundIndexScanEntry};

#[derive(Clone, Default)]
pub struct InMemoryCompoundIndexRepo;

impl CompoundIndexRepository for InMemoryCompoundIndexRepo {
    fn index_compound(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        column_values: &[CompoundColumnValue],
        revision: &HLC,
        node_id: &str,
        is_published: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let _ = (
            scope,
            index_name,
            column_values,
            revision,
            node_id,
            is_published,
        );
        async move { Ok(()) }
    }

    fn unindex_compound(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        column_values: &[CompoundColumnValue],
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let _ = (scope, index_name, column_values, node_id);
        async move { Ok(()) }
    }

    fn scan_compound_index(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        equality_values: &[CompoundColumnValue],
        published_only: bool,
        ascending: bool,
        limit: Option<usize>,
    ) -> impl std::future::Future<Output = Result<Vec<CompoundIndexScanEntry>>> + Send {
        let _ = (
            scope,
            index_name,
            equality_values,
            published_only,
            ascending,
            limit,
        );
        async move { Ok(Vec::new()) }
    }

    fn remove_all_compound_indexes_for_node(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let _ = (scope, node_id);
        async move { Ok(()) }
    }
}
