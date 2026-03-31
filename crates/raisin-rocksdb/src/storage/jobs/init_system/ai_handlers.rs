//! AI-related job handler construction
//!
//! Creates handlers for AI tool call execution, tool result aggregation,
//! auth user node creation, resumable uploads, and asset processing.

use std::sync::Arc;

use crate::jobs::{
    AIToolCallExecutionHandler, AuthCreateUserNodeHandler, BinaryRetrievalCallback,
    BinaryUploadCallback, FunctionExecutorCallback, NodeCreatorCallback, RocksDBUserNodeCreator,
};
use crate::storage::RocksDBStorage;
use raisin_storage::jobs::JobRegistry;

/// Create the AI tool call execution handler
pub fn create_ai_tool_call_execution_handler(
    storage: Arc<RocksDBStorage>,
    function_executor: Option<FunctionExecutorCallback>,
    node_creator: Option<&NodeCreatorCallback>,
) -> Arc<AIToolCallExecutionHandler<RocksDBStorage>> {
    let mut builder = AIToolCallExecutionHandler::new(storage);
    if let Some(executor) = function_executor {
        builder = builder.with_executor(executor);
    }
    if let Some(creator) = node_creator {
        builder = builder.with_node_creator(creator.clone());
    }
    Arc::new(builder)
}

/// Create the AI tool result aggregation handler
pub fn create_ai_tool_result_aggregation_handler(
    storage: Arc<RocksDBStorage>,
    node_creator: Option<NodeCreatorCallback>,
) -> Arc<crate::jobs::AIToolResultAggregationHandler<RocksDBStorage>> {
    let mut builder = crate::jobs::AIToolResultAggregationHandler::new(storage);
    if let Some(creator) = node_creator {
        builder = builder.with_node_creator(creator);
    }
    Arc::new(builder)
}

/// Create the auth user node handler for creating user nodes on registration
pub fn create_auth_user_node_handler(
    storage: Arc<RocksDBStorage>,
) -> Option<Arc<AuthCreateUserNodeHandler<RocksDBUserNodeCreator>>> {
    let user_node_creator = Arc::new(RocksDBUserNodeCreator::new(storage));
    Some(Arc::new(AuthCreateUserNodeHandler::new(user_node_creator)))
}

/// Create the resumable upload handler
pub fn create_resumable_upload_handler(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    binary_upload: Option<BinaryUploadCallback>,
) -> Arc<crate::jobs::ResumableUploadHandler<RocksDBStorage>> {
    let mut builder = crate::jobs::ResumableUploadHandler::new(storage, job_registry);
    if let Some(callback) = binary_upload {
        builder = builder.with_binary_upload_callback(callback);
    }
    Arc::new(builder)
}

/// Create the upload session cleanup handler
pub fn create_upload_session_cleanup_handler() -> Arc<crate::jobs::UploadSessionCleanupHandler> {
    Arc::new(crate::jobs::UploadSessionCleanupHandler::new())
}

/// Create the HuggingFace model handler
pub fn create_huggingface_model_handler() -> Option<Arc<crate::jobs::HuggingFaceModelHandler>> {
    Some(Arc::new(crate::jobs::HuggingFaceModelHandler::new()))
}

/// Create the asset processing handler (deprecated but still active)
#[allow(deprecated)]
pub fn create_asset_processing_handler(
    storage: Arc<RocksDBStorage>,
    binary_retrieval: Option<&BinaryRetrievalCallback>,
) -> Option<Arc<crate::jobs::AssetProcessingHandler>> {
    let mut builder = crate::jobs::AssetProcessingHandler::new(storage);
    if let Some(callback) = binary_retrieval {
        builder = builder.with_binary_callback(callback.clone());
    }
    Some(Arc::new(builder))
}

/// Create the node delete cleanup handler
pub fn create_node_delete_cleanup_handler(
    storage: &RocksDBStorage,
) -> Arc<crate::jobs::NodeDeleteCleanupHandler> {
    Arc::new(crate::jobs::NodeDeleteCleanupHandler::new(
        storage.db.clone(),
    ))
}

/// Create the relation consistency handler
pub fn create_relation_consistency_handler(
    storage: &RocksDBStorage,
) -> Arc<crate::jobs::RelationConsistencyHandler> {
    Arc::new(crate::jobs::RelationConsistencyHandler::new(
        storage.db.clone(),
    ))
}
