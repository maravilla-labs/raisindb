//! Localized URL lookup (plan Phase 12): `NodeService::resolve_localized_path`,
//! the ONE core function every surface (SQL, HTTP, WS, functions) calls.

use std::collections::BTreeMap;
use std::sync::Arc;

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleCode;
use raisin_storage::localized::LocalizedServedBy;
use raisin_storage::{
    transactional::TransactionalStorage, BranchRepository, RepositoryManagementRepository, Storage,
};
use serde::Serialize;

use super::NodeService;
use crate::TranslationResolver;

/// A node found by its localized path.
#[derive(Debug, Clone, Serialize)]
pub struct LocalizedNode {
    /// The node, translated into the requested locale.
    pub node: Node,
    pub canonical_path: String,
    /// The node's own path in the requested locale.
    pub canonical_localized_path: String,
    /// The request did not use `canonical_localized_path`: answer 301.
    pub redirect: bool,
    /// `locale -> localized path` for hreflang, one per supported language
    /// in which the node is visible AND readable by the caller. A hidden
    /// locale, or one RLS denies, is omitted, so alternates never leak a path.
    pub alternates: BTreeMap<String, String>,
    pub served_by: LocalizedServedBy,
}

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Resolve `path` (its segments in `locale`) to a node, at the service's
    /// revision (HEAD when unset), under the service's auth context.
    ///
    /// Missing, forbidden and hidden-in-locale all answer `Ok(None)` — the
    /// same 404 — so a lookup never tells a caller that a node it cannot read
    /// exists.
    pub async fn resolve_localized_path(
        &self,
        locale: &str,
        path: &str,
    ) -> Result<Option<LocalizedNode>> {
        let source = self.storage.localized_names().ok_or_else(|| {
            Error::Validation("localized path lookup is not supported by this backend".into())
        })?;
        let locale_code = LocaleCode::parse(locale)?;
        let Some(found) = source.resolve(self.scope(), locale, path, self.revision.as_ref())?
        else {
            return Ok(None);
        };
        // RLS through the permission-checked read.
        let Some(base) = self.get(&found.node_id).await? else {
            return Ok(None);
        };
        let config = self
            .storage
            .repository_management()
            .get_repository(&self.tenant_id, &self.repo_id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("repository {}", self.repo_id)))?
            .config;
        let revision = self.read_revision().await?;
        let resolver = TranslationResolver::new(
            Arc::new(self.storage.translations().clone()),
            config.clone(),
        );
        let Some(node) = self
            .translated_readable(&resolver, base.clone(), &locale_code, &revision)
            .await?
        else {
            return Ok(None);
        };

        let mut alternates = BTreeMap::new();
        for language in &config.supported_languages {
            let Ok(code) = LocaleCode::parse(language) else {
                continue;
            };
            let Some(localized) = source.localized_path(
                self.scope(),
                &found.node_id,
                language,
                self.revision.as_ref(),
            )?
            else {
                continue;
            };
            if self
                .translated_readable(&resolver, base.clone(), &code, &revision)
                .await?
                .is_some()
            {
                alternates.insert(language.clone(), localized);
            }
        }

        Ok(Some(LocalizedNode {
            node,
            canonical_path: found.canonical_path,
            canonical_localized_path: found.canonical_localized_path,
            redirect: found.redirect,
            alternates,
            served_by: found.served_by,
        }))
    }

    /// `node` translated into `locale`, if visible there and readable by the
    /// caller as translated.
    async fn translated_readable(
        &self,
        resolver: &TranslationResolver<S::Translations>,
        node: Node,
        locale: &LocaleCode,
        revision: &HLC,
    ) -> Result<Option<Node>> {
        let translated = resolver
            .resolve_node(
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &self.workspace_id,
                node,
                locale,
                revision,
            )
            .await?;
        Ok(match translated {
            Some(node) => self.apply_rls_filter(node).await,
            None => None,
        })
    }

    /// The revision translation reads use: the pinned one, else branch HEAD.
    async fn read_revision(&self) -> Result<HLC> {
        if let Some(revision) = self.revision {
            return Ok(revision);
        }
        Ok(self
            .storage
            .branches()
            .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
            .await?
            .map(|b| b.head)
            .unwrap_or_else(HLC::now))
    }
}
