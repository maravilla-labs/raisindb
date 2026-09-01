//! Job system helper functions.
//!
//! This module provides helper functions for job system initialization.
//! The main job system initialization is done in main.rs due to complex
//! lifetime requirements with async closures.

use std::sync::Arc;

use raisin_binary::BinaryStorage;

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;

/// Create binary storage callback for storing files.
#[cfg(feature = "storage-rocksdb")]
pub fn create_binary_storage_callback<B: BinaryStorage + 'static>(
    bin: Arc<B>,
) -> raisin_rocksdb::BinaryStorageCallback {
    Arc::new(
        move |data: Vec<u8>,
              content_type: Option<String>,
              ext: Option<String>,
              filename: Option<String>,
              tenant_context: Option<String>| {
            let bin = bin.clone();
            Box::pin(async move {
                bin.put_bytes(
                    &data,
                    content_type.as_deref(),
                    ext.as_deref(),
                    filename.as_deref(),
                    tenant_context.as_deref(),
                )
                .await
                .map_err(|e| raisin_error::Error::storage(e.to_string()))
            })
        },
    )
}

/// Create the binary DELETE callback.
///
/// The counterpart of [`create_binary_storage_callback`], for the one thing that
/// gives bytes back: a virtual mount's cached file, which is a copy of something
/// the provider still holds. Without it the content TTL would clear the node's
/// `file` and leave the object on disk forever.
#[cfg(feature = "storage-rocksdb")]
pub fn create_binary_delete_callback<B: BinaryStorage + 'static>(
    bin: Arc<B>,
) -> raisin_rocksdb::BinaryDeleteCallback {
    Arc::new(move |storage_key: String| {
        let bin = bin.clone();
        Box::pin(async move {
            bin.delete(&storage_key)
                .await
                .map_err(|e| raisin_error::Error::storage(e.to_string()))
        })
    })
}

/// Create node creator callback for AI tool calls.
#[cfg(feature = "storage-rocksdb")]
pub fn create_node_creator_callback(
    storage: Arc<RocksDBStorage>,
) -> raisin_rocksdb::NodeCreatorCallback {
    Arc::new(
        move |node: raisin_models::nodes::Node,
              tenant_id: String,
              repo_id: String,
              branch: String,
              workspace: String| {
            let storage = storage.clone();
            Box::pin(async move {
                use raisin_core::services::node_service::NodeService;
                use raisin_models::auth::AuthContext;

                let svc =
                    NodeService::new_with_context(storage, tenant_id, repo_id, branch, workspace)
                        .with_auth(AuthContext::system());

                svc.create(node.clone()).await?;

                Ok(node)
            })
        },
    )
}
