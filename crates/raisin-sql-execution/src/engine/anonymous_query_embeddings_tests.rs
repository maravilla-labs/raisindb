// SPDX-License-Identifier: BSL-1.1

//! `ANONYMOUS_QUERY_EMBEDDINGS`: whether an anonymous caller may have query
//! text embedded. The default keeps the old behaviour; `deny` refuses the
//! anonymous principal before any provider is called, and nobody else.

use super::QueryEngine;
use crate::physical_plan::executor::ExecutionContext;
use futures::StreamExt;
use raisin_embeddings::config::AnonymousQueryEmbeddings;
use raisin_embeddings::{StorageError, TenantEmbeddingConfig, TenantEmbeddingConfigStore};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::ResolvedPermissions;
use raisin_storage_memory::InMemoryStorage;
use std::sync::{Arc, Mutex};

const TENANT: &str = "t_anonymous_query_embeddings";

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

fn store(setting: AnonymousQueryEmbeddings) -> Arc<Store> {
    let mut config = TenantEmbeddingConfig::new(TENANT.to_string());
    config.enabled = true;
    config.anonymous_query_embeddings = setting;
    Arc::new(Store(Mutex::new(Some(config))))
}

fn anonymous() -> AuthContext {
    AuthContext::anonymous_user("anonymous")
        .with_permissions(ResolvedPermissions::anonymous(vec![]))
}

/// The context a statement of `auth` gets, with the tenant's defaults applied.
fn context(store: &Arc<Store>, auth: Option<AuthContext>) -> ExecutionContext<InMemoryStorage> {
    let storage = Arc::new(InMemoryStorage::default());
    let mut engine = QueryEngine::new(storage.clone(), TENANT, "repo", "main")
        .with_embedding_config_store(store.clone());
    if let Some(auth) = auth.clone() {
        engine = engine.with_auth(auth);
    }
    let mut ctx = ExecutionContext::new(
        storage,
        TENANT.into(),
        "repo".into(),
        "main".into(),
        "ws".into(),
    );
    ctx.auth_context = auth;
    engine.apply_embedding_defaults(&mut ctx);
    ctx
}

async fn rows(
    engine: &QueryEngine<InMemoryStorage>,
    sql: &str,
) -> Result<Vec<(String, String)>, raisin_error::Error> {
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

#[tokio::test]
async fn alter_and_show_round_trip_and_reject_unknown_values() {
    let store = store(AnonymousQueryEmbeddings::Allow);
    let admin = QueryEngine::new(Arc::new(InMemoryStorage::default()), TENANT, "repo", "main")
        .with_auth(AuthContext::system())
        .with_embedding_config_store(store.clone());

    let shown = rows(&admin, "SHOW EMBEDDING CONFIG").await.unwrap();
    assert!(shown.contains(&("anonymous_query_embeddings".into(), "allow".into())));

    rows(
        &admin,
        "ALTER EMBEDDING CONFIG SET ANONYMOUS_QUERY_EMBEDDINGS = 'deny'",
    )
    .await
    .unwrap();
    assert_eq!(
        store
            .0
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .anonymous_query_embeddings,
        AnonymousQueryEmbeddings::Deny
    );
    let shown = rows(&admin, "SHOW EMBEDDING CONFIG").await.unwrap();
    assert!(shown.contains(&("anonymous_query_embeddings".into(), "deny".into())));

    let err = rows(
        &admin,
        "ALTER EMBEDDING CONFIG SET ANONYMOUS_QUERY_EMBEDDINGS = 'sometimes'",
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("expected 'allow' or 'deny'"),
        "{err}"
    );
}

#[test]
fn a_stored_config_without_the_field_allows() {
    // configs written before the setting existed
    let mut v = serde_json::to_value(TenantEmbeddingConfig::new(TENANT.into())).unwrap();
    v.as_object_mut()
        .unwrap()
        .remove("anonymous_query_embeddings");
    let config: TenantEmbeddingConfig = serde_json::from_value(v).unwrap();
    assert_eq!(
        config.anonymous_query_embeddings,
        AnonymousQueryEmbeddings::Allow
    );
}

#[test]
fn only_the_anonymous_principal_is_refused_and_only_on_deny() {
    let deny = store(AnonymousQueryEmbeddings::Deny);
    let allow = store(AnonymousQueryEmbeddings::Allow);

    let refused = context(&deny, Some(anonymous())).refuse_query_embedding();
    assert!(
        refused
            .as_deref()
            .is_some_and(|r| r.contains("ANONYMOUS_QUERY_EMBEDDINGS")),
        "{refused:?}"
    );
    // the default leaves anonymous callers as they were
    assert_eq!(
        context(&allow, Some(anonymous())).refuse_query_embedding(),
        None
    );

    // everyone else is untouched by deny
    let user = AuthContext::for_user("u1").with_permissions(ResolvedPermissions::empty("u1"));
    assert_eq!(context(&deny, Some(user)).refuse_query_embedding(), None);
    assert_eq!(
        context(&deny, Some(AuthContext::system())).refuse_query_embedding(),
        None
    );
    assert_eq!(context(&deny, None).refuse_query_embedding(), None);
    // an agent's tools for a visitor run under the anonymous TOOL GRANT, as
    // the agent: not the anonymous principal
    let mut grant = ResolvedPermissions::empty("agent:/agents/site");
    grant.effective_roles = vec!["site_reader".into()];
    let tool = AuthContext::for_user("agent:/agents/site")
        .with_permissions(grant)
        .with_agent("agent:/agents/site");
    assert_eq!(context(&deny, Some(tool)).refuse_query_embedding(), None);
}

#[tokio::test]
async fn embedding_is_refused_before_any_provider_or_cache() {
    let deny = store(AnonymousQueryEmbeddings::Deny);
    let ctx = context(&deny, Some(anonymous()));
    // no provider is configured in this context at all: a refusal that came
    // from the provider would read differently
    let err = crate::physical_plan::eval::generate_embedding_cached("wer ist der ceo", &ctx)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("ANONYMOUS_QUERY_EMBEDDINGS"),
        "{err}"
    );
}
