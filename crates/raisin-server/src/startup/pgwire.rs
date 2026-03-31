//! PostgreSQL wire protocol server setup.
//!
//! This module handles initialization of the pgwire server for SQL access.

#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
use raisin_rocksdb::{AuthService, RocksDBStorage};
#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
use raisin_transport_pgwire::{
    ApiKeyValidator, PgWireConfig, PgWireServer, RaisinAuthHandler, RaisinExtendedQueryHandler,
    RaisinSimpleQueryHandler,
};
#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
use std::sync::Arc;
#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
use tokio::task::JoinHandle;

/// API key validator that integrates with AuthService
#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
pub struct AuthServiceValidator {
    auth_service: Arc<AuthService>,
}

#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
impl AuthServiceValidator {
    pub fn new(auth_service: Arc<AuthService>) -> Self {
        Self { auth_service }
    }
}

#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
#[async_trait::async_trait]
impl ApiKeyValidator for AuthServiceValidator {
    async fn validate_api_key(&self, api_key: &str) -> Result<Option<(String, String)>, String> {
        match self.auth_service.validate_api_key(api_key) {
            Ok(Some(key)) => {
                if !key.is_active {
                    return Ok(None);
                }
                Ok(Some((key.user_id, key.tenant_id)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn has_pgwire_access(&self, tenant_id: &str, user_id: &str) -> Result<bool, String> {
        use raisin_models::admin_user::AdminInterface;
        // Use get_user_by_id since API keys store user_id, not username
        match self.auth_service.get_user_by_id(tenant_id, user_id) {
            Ok(Some(user)) => Ok(user.can_access(AdminInterface::Pgwire)),
            Ok(None) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Server parameter provider for pgwire
#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
pub struct RaisinServerParameterProvider;

#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
impl pgwire::api::auth::ServerParameterProvider for RaisinServerParameterProvider {
    fn server_parameters<C>(&self, _client: &C) -> Option<std::collections::HashMap<String, String>>
    where
        C: pgwire::api::ClientInfo,
    {
        let mut params = std::collections::HashMap::new();
        params.insert("server_version".to_string(), "16.0 (RaisinDB)".to_string());
        params.insert("server_encoding".to_string(), "UTF8".to_string());
        params.insert("client_encoding".to_string(), "UTF8".to_string());
        params.insert("DateStyle".to_string(), "ISO, MDY".to_string());
        params.insert("TimeZone".to_string(), "UTC".to_string());
        params.insert("integer_datetimes".to_string(), "on".to_string());
        Some(params)
    }
}

/// Handler factory for pgwire
#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
pub struct RaisinHandlerFactory<S, V, P>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    pub storage: Arc<S>,
    pub auth_handler: Arc<RaisinAuthHandler<V, P>>,
    pub indexing_engine: Option<Arc<raisin_indexer::TantivyIndexingEngine>>,
    pub hnsw_engine: Option<Arc<raisin_hnsw::HnswIndexingEngine>>,
}

#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
impl<S, V, P> pgwire::api::PgWireHandlerFactory for RaisinHandlerFactory<S, V, P>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    V: ApiKeyValidator + 'static,
    P: pgwire::api::auth::ServerParameterProvider + 'static,
{
    type StartupHandler = RaisinAuthHandler<V, P>;
    type SimpleQueryHandler = RaisinSimpleQueryHandler<S, V, P>;
    type ExtendedQueryHandler = RaisinExtendedQueryHandler<S, V, P>;
    type CopyHandler = pgwire::api::copy::NoopCopyHandler;

    fn simple_query_handler(&self) -> Arc<RaisinSimpleQueryHandler<S, V, P>> {
        let mut handler =
            RaisinSimpleQueryHandler::new(self.storage.clone(), self.auth_handler.clone());

        if let Some(ref indexing) = self.indexing_engine {
            handler = handler.with_indexing_engine(indexing.clone());
        }

        if let Some(ref hnsw) = self.hnsw_engine {
            handler = handler.with_hnsw_engine(hnsw.clone());
        }

        Arc::new(handler)
    }

    fn extended_query_handler(&self) -> Arc<RaisinExtendedQueryHandler<S, V, P>> {
        let handler =
            RaisinExtendedQueryHandler::new(self.storage.clone(), self.auth_handler.clone());

        // Note: indexing and hnsw engines can be added here when needed
        // for extended query support

        Arc::new(handler)
    }

    fn startup_handler(&self) -> Arc<RaisinAuthHandler<V, P>> {
        self.auth_handler.clone()
    }

    fn copy_handler(&self) -> Arc<pgwire::api::copy::NoopCopyHandler> {
        Arc::new(pgwire::api::copy::NoopCopyHandler)
    }
}

/// Start the pgwire server if enabled
#[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
pub fn start_pgwire_server(
    storage: Arc<RocksDBStorage>,
    auth_service: Arc<AuthService>,
    indexing_engine: Option<Arc<raisin_indexer::TantivyIndexingEngine>>,
    hnsw_engine: Option<Arc<raisin_hnsw::HnswIndexingEngine>>,
    bind_address: &str,
    port: u16,
    max_connections: usize,
) -> JoinHandle<()> {
    // Create auth handler
    let validator = AuthServiceValidator::new(auth_service);
    let param_provider = RaisinServerParameterProvider;
    let auth_handler = Arc::new(RaisinAuthHandler::new(validator, param_provider));

    // Create handler factory
    let handler_factory = RaisinHandlerFactory {
        storage,
        auth_handler,
        indexing_engine,
        hnsw_engine,
    };

    // Create pgwire config
    let pgwire_config = PgWireConfig::builder()
        .bind_addr(format!("{}:{}", bind_address, port))
        .max_connections(max_connections)
        .build();

    // Create and spawn pgwire server
    let pgwire_server = PgWireServer::new(pgwire_config).with_handler(handler_factory);

    tokio::spawn(async move {
        if let Err(e) = pgwire_server.run().await {
            tracing::error!("PostgreSQL wire protocol server error: {}", e);
        }
    })
}
