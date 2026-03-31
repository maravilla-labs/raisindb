//! Translation metadata operations.

use raisin_error::Result;
use raisin_models::translations::{LocaleCode, TranslationMeta};
use rocksdb::DB;
use std::sync::Arc;

use super::{keys, serialization};

/// Get translation metadata for a node
pub(super) async fn get_translation_meta(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &LocaleCode,
) -> Result<Option<TranslationMeta>> {
    let cf = crate::cf_handle(db, crate::cf::REVISIONS)?;

    let prefix = keys::translation_meta_prefix(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale.as_str(),
    );

    let mut iter = db.prefix_iterator_cf(&cf, &prefix);

    // First entry is the most recent (descending revision)
    if let Some(Ok((_key, value))) = iter.next() {
        let meta = serialization::deserialize_translation_meta(&value)?;
        return Ok(Some(meta));
    }

    Ok(None)
}
