//! A deleted translation stays deleted on the TRANSACTION read path.
//!
//! The transaction reader skipped a tombstone and fell through to the older
//! live version, so every SQL/WS read of a deleted translation brought it back
//! — while the repository reader, applying newest-wins, did not. Both now share
//! one reader (`translation_read`); this pins the transaction side.

use std::collections::HashMap;

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::translations::LocaleOverlay;
use raisin_rocksdb::{cf, RocksDBStorage};
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    BranchRepository, RegistryRepository, RepoScope, RepositoryManagementRepository, Storage,
    WorkspaceRepository,
};
use tempfile::TempDir;

const TENANT: &str = "tr-tenant";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";
const NODE: &str = "translated-node";

async fn setup() -> Result<(RocksDBStorage, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path())?;
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;
    storage
        .repository_management()
        .create_repository(
            TENANT,
            REPO,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string(), "fr".to_string()],
                locale_fallback_chains: HashMap::new(),
                default_branch: BRANCH.to_string(),
                description: None,
                tags: HashMap::new(),
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WORKSPACE.to_string()),
        )
        .await?;
    Ok((storage, temp_dir))
}

async fn begin(storage: &RocksDBStorage) -> Result<Box<dyn TransactionalContext>> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("translations")?;
    tx.set_auth_context(AuthContext::system())?;
    Ok(tx)
}

async fn sorted_locales(tx: &dyn TransactionalContext) -> Result<Vec<String>> {
    let mut locales = tx.list_translations_for_node(WORKSPACE, NODE).await?;
    locales.sort();
    Ok(locales)
}

/// Write the tombstone a node delete writes for one locale
/// (`tombstones::tombstone_translation_data`), newer than every version so far.
fn tombstone(storage: &RocksDBStorage, locale: &str, revision: &HLC) {
    let mut key =
        format!("{TENANT}\0{REPO}\0{BRANCH}\0{WORKSPACE}\0translations\0{NODE}\0{locale}\0")
            .into_bytes();
    key.extend_from_slice(&revision.encode_descending());
    let db = storage.db();
    db.put_cf(db.cf_handle(cf::TRANSLATION_DATA).unwrap(), key, b"T")
        .unwrap();
}

#[tokio::test]
async fn a_deleted_translation_is_absent_and_unlisted_in_a_transaction() -> Result<()> {
    let (storage, _tmp) = setup().await?;

    let tx = begin(&storage).await?;
    tx.store_translation(WORKSPACE, NODE, "fr", LocaleOverlay::Hidden)
        .await?;
    tx.store_translation(WORKSPACE, NODE, "en", LocaleOverlay::Hidden)
        .await?;
    tx.commit().await?;

    let tx = begin(&storage).await?;
    assert!(tx.get_translation(WORKSPACE, NODE, "fr").await?.is_some());
    assert_eq!(sorted_locales(tx.as_ref()).await?, vec!["en", "fr"]);
    drop(tx);

    // Delete fr after the commit, at a revision newer than its version.
    let head = storage.branches().get_head(TENANT, REPO, BRANCH).await?;
    tombstone(&storage, "fr", &HLC::new(head.timestamp_ms + 1_000, 0));

    let tx = begin(&storage).await?;
    assert!(
        tx.get_translation(WORKSPACE, NODE, "fr").await?.is_none(),
        "a deleted translation must not fall through to its older live version"
    );
    assert_eq!(sorted_locales(tx.as_ref()).await?, vec!["en"]);
    assert!(tx.get_translation(WORKSPACE, NODE, "en").await?.is_some());

    // Re-translating inside a transaction is visible to that transaction ...
    tx.store_translation(WORKSPACE, NODE, "fr", LocaleOverlay::Hidden)
        .await?;
    assert!(tx.get_translation(WORKSPACE, NODE, "fr").await?.is_some());
    assert_eq!(sorted_locales(tx.as_ref()).await?, vec!["en", "fr"]);
    Ok(())
}
