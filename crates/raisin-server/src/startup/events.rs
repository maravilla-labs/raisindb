//! Event handler setup.
//!
//! This module handles the registration of event handlers
//! for the event-driven architecture.

use std::sync::Arc;

use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;

use crate::{nodetype_init_handler, workspace_init_handler};

/// Register core event handlers.
pub fn register_event_handlers<S: Storage + TransactionalStorage + 'static>(storage: Arc<S>) {
    let event_bus = storage.event_bus();

    event_bus.subscribe(Arc::new(nodetype_init_handler::NodeTypeInitHandler::new(
        storage.clone(),
    )));

    event_bus.subscribe(Arc::new(workspace_init_handler::WorkspaceInitHandler::new(
        storage,
    )));

    tracing::info!("Event-driven architecture initialized");
}

/// Register graph projection event handler for event-driven invalidation.
///
/// When relations change, the handler marks affected graph projections
/// as stale in the GRAPH_PROJECTION column family. Projections are lazily
/// rebuilt on next algorithm access. No in-memory cache needed — RocksDB's
/// block cache handles repeated reads.
#[cfg(feature = "storage-rocksdb")]
pub fn register_graph_projection_handler(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    let handler = Arc::new(raisin_rocksdb::graph::GraphProjectionEventHandler::new(
        Arc::clone(storage),
    ));

    let event_bus = raisin_storage::Storage::event_bus(storage.as_ref());
    event_bus.subscribe(handler);

    tracing::info!("Graph projection event handler registered (event-driven invalidation)");
}

/// Register admin user handler.
#[cfg(feature = "storage-rocksdb")]
pub fn register_admin_handler(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    auth_service: Arc<raisin_rocksdb::AuthService>,
    initial_password: Option<&str>,
) {
    use crate::admin_user_init_handler;

    let event_bus = storage.event_bus();

    let admin_handler = if let Some(password) = initial_password {
        tracing::info!("Using configured initial admin password");
        admin_user_init_handler::AdminUserInitHandler::with_initial_password(
            auth_service,
            password.to_string(),
        )
    } else {
        admin_user_init_handler::AdminUserInitHandler::new(auth_service)
    };
    event_bus.subscribe(Arc::new(admin_handler));
}

/// Register default tenant.
#[cfg(feature = "storage-rocksdb")]
pub async fn register_default_tenant(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    use raisin_storage::RegistryRepository;

    tracing::info!("Ensuring default tenant exists...");
    let registry = Storage::registry(storage.as_ref());
    registry
        .register_tenant("default", std::collections::HashMap::new())
        .await
        .expect("Failed to register default tenant");

    registry
        .register_deployment("default", "production")
        .await
        .expect("Failed to register default deployment");

    tracing::info!("Default tenant registered (TenantCreated event fired if new tenant)");
}
