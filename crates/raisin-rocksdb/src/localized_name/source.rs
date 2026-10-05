//! [`raisin_storage::localized::LocalizedNameSource`] for RocksDB: the lookup
//! of [`super::lookup`], as `Storage::localized_names` hands it out.

use super::keys::NameScope;
use super::lookup::{LocalizedLookup, ServedBy};
use crate::repositories::nodes::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::localized::{LocalizedNameSource, LocalizedResolution, LocalizedServedBy};
use raisin_storage::scope::StorageScope;

/// The RocksDB localized name source (a cheap clone of the node repository).
pub struct RocksLocalizedNames {
    nodes: NodeRepositoryImpl,
}

impl RocksLocalizedNames {
    pub(crate) fn new(nodes: NodeRepositoryImpl) -> Self {
        Self { nodes }
    }
}

fn name_scope<'a>(scope: &StorageScope<'a>) -> NameScope<'a> {
    NameScope::new(
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
    )
}

impl LocalizedNameSource for RocksLocalizedNames {
    fn resolve(
        &self,
        scope: StorageScope<'_>,
        locale: &str,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<LocalizedResolution>> {
        let found = LocalizedLookup::new(&self.nodes).resolve(
            name_scope(&scope),
            locale,
            path,
            max_revision,
        )?;
        Ok(found.map(|r| LocalizedResolution {
            node_id: r.node_id,
            canonical_path: r.canonical_path,
            canonical_localized_path: r.canonical_localized_path,
            redirect: r.redirect,
            served_by: match r.served_by {
                ServedBy::DefaultLanguage => LocalizedServedBy::DefaultLanguage,
                ServedBy::Index => LocalizedServedBy::Index,
                ServedBy::Fallback => LocalizedServedBy::Fallback,
            },
        }))
    }

    fn localized_path(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        locale: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<String>> {
        LocalizedLookup::new(&self.nodes).localized_path(
            name_scope(&scope),
            node_id,
            locale,
            max_revision,
        )
    }

    fn node_name(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        locale: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<String>> {
        LocalizedLookup::new(&self.nodes).node_name(
            name_scope(&scope),
            node_id,
            locale,
            max_revision,
        )
    }
}
