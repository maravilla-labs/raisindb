// SPDX-License-Identifier: BSL-1.1

//! The tenant embedding config as a function's SQL engine sees it: readable
//! through the process-wide reader, never writable.
//!
//! One `#[test]` on purpose: the reader is a process-wide `OnceLock`.

use super::QueryEngine;
use futures::StreamExt;
use raisin_embeddings::{StorageError, TenantEmbeddingConfig, TenantEmbeddingConfigStore};
use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::ResolvedPermissions;
use raisin_storage_memory::InMemoryStorage;
use std::sync::{Arc, Mutex};

const TENANT: &str = "t_embedding_config_reader";

fn configured() -> TenantEmbeddingConfig {
    let mut config = TenantEmbeddingConfig::new(TENANT.to_string());
    config.default_max_distance = Some(0.78);
    config.query_prefix = Some("task: search result | query: ".to_string());
    config
}

/// Stands in for the RocksDB config repository.
struct Store(Mutex<Option<TenantEmbeddingConfig>>);

impl TenantEmbeddingConfigStore for Store {
    fn get_config(&self, tenant_id: &str) -> Result<Option<TenantEmbeddingConfig>, StorageError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .clone()
            .filter(|c| c.tenant_id == tenant_id))
    }
    fn set_config(&self, config: &TenantEmbeddingConfig) -> Result<(), StorageError> {
        *self.0.lock().unwrap() = Some(config.clone());
        Ok(())
    }
    fn delete_config(&self, _tenant_id: &str) -> Result<(), StorageError> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

/// A function's engine: no store wired, runs as the system by default.
fn function_engine() -> QueryEngine<InMemoryStorage> {
    QueryEngine::new(Arc::new(InMemoryStorage::default()), TENANT, "repo", "main")
        .with_auth(AuthContext::system())
}

async fn rows(
    engine: &QueryEngine<InMemoryStorage>,
    sql: &str,
) -> Result<Vec<(String, String)>, Error> {
    let mut stream = engine.execute(sql).await?;
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row?;
        if let (Some(PropertyValue::String(k)), Some(PropertyValue::String(v))) =
            (row.get("key"), row.get("value"))
        {
            out.push((k.clone(), v.clone()));
        }
    }
    Ok(out)
}

/// `SELECT EMBEDDING_MAX_DISTANCE() AS d` as the caller of `engine` sees it.
async fn max_distance(engine: &QueryEngine<InMemoryStorage>) -> f64 {
    let mut stream = engine
        .execute("SELECT EMBEDDING_MAX_DISTANCE() AS d")
        .await
        .unwrap();
    let row = stream.next().await.unwrap().unwrap();
    match row.get("d") {
        Some(PropertyValue::Float(d)) => *d,
        other => panic!("EMBEDDING_MAX_DISTANCE() gave {other:?}"),
    }
}

fn value<'a>(rows: &'a [(String, String)], key: &str) -> Option<&'a str> {
    rows.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[tokio::test]
async fn functions_read_the_tenant_embedding_defaults_and_cannot_alter_them() {
    let store = Arc::new(Store(Mutex::new(Some(configured()))));
    assert!(raisin_embeddings::configure_embedding_config_reader(
        store.clone()
    ));

    // KNN / HYBRID_SEARCH without `max_distance` take this per statement.
    let engine = function_engine();
    assert_eq!(engine.tenant_default_max_distance(), Some(0.78));

    // SHOW works inside a function, and shows the query prefix too.
    let shown = rows(&engine, "SHOW EMBEDDING CONFIG").await.unwrap();
    assert_eq!(value(&shown, "default_max_distance"), Some("0.78"));
    assert_eq!(
        value(&shown, "query_prefix"),
        Some("task: search result | query: ")
    );

    // ALTER from a function is refused, although it runs as the system...
    let err = rows(
        &engine,
        "ALTER EMBEDDING CONFIG SET DEFAULT_MAX_DISTANCE = '0.9'",
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("read-only"), "{err}");
    // ...and nothing was written.
    assert_eq!(
        store
            .0
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .default_max_distance,
        Some(0.78)
    );
    assert_eq!(function_engine().tenant_default_max_distance(), Some(0.78));

    // A non-admin caller cannot ALTER even where a store IS wired (/api/sql),
    // nor read the config through the process-wide reader.
    let user = AuthContext::for_user("bob").with_permissions(ResolvedPermissions::empty("bob"));
    let user_engine =
        QueryEngine::new(Arc::new(InMemoryStorage::default()), TENANT, "repo", "main")
            .with_auth(user.clone())
            .with_embedding_config_store(store.clone());
    let err = rows(
        &user_engine,
        "ALTER EMBEDDING CONFIG SET BASE_URL = 'https://attacker.example'",
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)), "{err}");
    assert_eq!(store.0.lock().unwrap().as_ref().unwrap().base_url, None);
    let user_fn_engine =
        QueryEngine::new(Arc::new(InMemoryStorage::default()), TENANT, "repo", "main")
            .with_auth(user);
    assert!(rows(&user_fn_engine, "SHOW EMBEDDING CONFIG")
        .await
        .is_err());
    // The distance default still applies to that user's searches, and the
    // user can read it: code scaling its own cuts (a site search running
    // under a visitor's tool grant) needs the number SHOW refuses it.
    assert_eq!(user_fn_engine.tenant_default_max_distance(), Some(0.78));
    assert!((max_distance(&user_fn_engine).await - 0.78).abs() < 1e-6);
    assert!((max_distance(&engine).await - 0.78).abs() < 1e-6);

    // The admin SQL surface (store wired, system caller) can still ALTER.
    let admin_engine = function_engine().with_embedding_config_store(store.clone());
    rows(
        &admin_engine,
        "ALTER EMBEDDING CONFIG SET DEFAULT_MAX_DISTANCE = '0.9'",
    )
    .await
    .unwrap();
    assert_eq!(function_engine().tenant_default_max_distance(), Some(0.9));
    // ...and the function follows the change on the next statement.
    assert!((max_distance(&function_engine()).await - 0.9).abs() < 1e-6);

    // Another tenant without a configured default: the engine's own cutoff.
    let other = QueryEngine::new(
        Arc::new(InMemoryStorage::default()),
        "t_unconfigured",
        "repo",
        "main",
    )
    .with_auth(AuthContext::system());
    assert!(
        (max_distance(&other).await - f64::from(raisin_hnsw::DEFAULT_MAX_DISTANCE)).abs() < 1e-6
    );
}
