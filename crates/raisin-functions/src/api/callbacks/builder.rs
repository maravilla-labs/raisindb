// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Builder for assembling all RaisinFunctionApi callbacks

use std::collections::HashMap;

use super::identity_ops::*;
use super::lock_ops::*;
use super::node_ops::*;
use super::secret_ops::*;
use super::service_ops::*;
use super::sql_ops::*;
use super::transaction_ops::*;

/// Builder for RaisinFunctionApi callbacks
#[derive(Default)]
pub struct RaisinFunctionApiCallbacks {
    /// Function-binding plugin handlers, keyed by logical method name
    /// (`"<ns>.<method>"`). Dispatched by `RaisinFunctionApi::plugin_call`.
    /// Empty in a plain public build (no plugins registered).
    pub plugin_callbacks: HashMap<String, PluginCallback>,
    pub node_get: Option<NodeGetCallback>,
    pub node_get_by_id: Option<NodeGetByIdCallback>,
    pub node_history: Option<NodeHistoryCallback>,
    pub node_create: Option<NodeCreateCallback>,
    pub node_update: Option<NodeUpdateCallback>,
    pub node_delete: Option<NodeDeleteCallback>,
    pub node_update_property: Option<NodeUpdatePropertyCallback>,
    pub node_move: Option<NodeMoveCallback>,
    pub node_query: Option<NodeQueryCallback>,
    pub node_get_children: Option<NodeGetChildrenCallback>,
    pub node_apply_child_order: Option<NodeApplyChildOrderCallback>,
    pub node_reorder_child: Option<NodeReorderChildCallback>,
    pub node_move_child_relative: Option<NodeMoveChildRelativeCallback>,
    pub node_add_resource: Option<NodeAddResourceCallback>,
    pub sql_query: Option<SqlQueryCallback>,
    pub sql_execute: Option<SqlExecuteCallback>,
    pub http_request: Option<HttpRequestCallback>,
    pub emit_event: Option<EmitEventCallback>,
    pub ai_completion: Option<AICompletionCallback>,
    pub ai_embed: Option<AIEmbedCallback>,
    pub ai_list_models: Option<AIListModelsCallback>,
    pub ai_list_providers: Option<AIListProvidersCallback>,
    pub ai_get_default_model: Option<AIGetDefaultModelCallback>,
    pub resource_get_binary: Option<ResourceGetBinaryCallback>,
    pub pdf_process_from_storage: Option<PdfProcessFromStorageCallback>,
    /// Store an extraction artifact produced by a plugin task above this layer.
    pub asset_set_extraction: Option<AssetSetExtractionCallback>,
    pub asset_reextract: Option<AssetReextractCallback>,
    pub asset_ensure_content: Option<AssetEnsureContentCallback>,
    /// Mint a signed, absolute URL for an asset's bytes.
    pub asset_signed_url: Option<AssetSignedUrlCallback>,
    pub task_create: Option<TaskCreateCallback>,
    pub task_update: Option<TaskUpdateCallback>,
    pub task_complete: Option<TaskCompleteCallback>,
    pub task_query: Option<TaskQueryCallback>,
    pub function_execute: Option<FunctionExecuteCallback>,
    pub function_call: Option<FunctionCallCallback>,
    pub flow_run: Option<FlowRunCallback>,
    pub branch_diff: Option<BranchDiffCallback>,
    pub branch_compare: Option<BranchCompareCallback>,
    pub branch_copy_nodes: Option<BranchCopyNodesCallback>,
    pub integrations_sync_now: Option<IntegrationsSyncNowCallback>,
    pub scheduler_schedule: Option<SchedulerScheduleCallback>,
    pub scheduler_cancel: Option<SchedulerCancelCallback>,
    pub scheduler_list: Option<SchedulerListCallback>,
    pub scheduler_get: Option<SchedulerGetCallback>,
    pub platform_hook: Option<PlatformHookCallback>,
    // Transaction callbacks
    pub tx_begin: Option<TxBeginCallback>,
    pub tx_commit: Option<TxCommitCallback>,
    pub tx_rollback: Option<TxRollbackCallback>,
    pub tx_set_actor: Option<TxSetActorCallback>,
    pub tx_set_message: Option<TxSetMessageCallback>,
    pub tx_create: Option<TxCreateCallback>,
    pub tx_add: Option<TxAddCallback>,
    pub tx_put: Option<TxPutCallback>,
    pub tx_upsert: Option<TxUpsertCallback>,
    pub tx_create_deep: Option<TxCreateDeepCallback>,
    pub tx_upsert_deep: Option<TxUpsertDeepCallback>,
    pub tx_update: Option<TxUpdateCallback>,
    pub tx_delete: Option<TxDeleteCallback>,
    pub tx_delete_by_id: Option<TxDeleteByIdCallback>,
    pub tx_get: Option<TxGetCallback>,
    pub tx_get_by_path: Option<TxGetByPathCallback>,
    pub tx_list_children: Option<TxListChildrenCallback>,
    pub tx_move: Option<TxMoveCallback>,
    pub tx_update_property: Option<TxUpdatePropertyCallback>,
    // Lock / inventory callbacks
    pub lock_acquire: Option<LockAcquireCallback>,
    pub lock_release: Option<LockReleaseCallback>,
    pub lock_renew: Option<LockRenewCallback>,
    pub inventory_claim: Option<InventoryClaimCallback>,
    pub inventory_release: Option<InventoryReleaseCallback>,
    // Secret store callbacks
    pub secret_get: Option<SecretGetCallback>,
    pub secret_put: Option<SecretPutCallback>,
    pub secret_list: Option<SecretListCallback>,
    pub secret_rotate: Option<SecretRotateCallback>,
    pub secret_delete: Option<SecretDeleteCallback>,
    // Tenant-identity callbacks
    pub identity_find_by_email: Option<IdentityFindByEmailCallback>,
    pub identity_update: Option<IdentityUpdateCallback>,
}

impl RaisinFunctionApiCallbacks {
    pub fn new() -> Self {
        Self::default()
    }

    // Node builder methods

    pub fn with_node_get(mut self, callback: NodeGetCallback) -> Self {
        self.node_get = Some(callback);
        self
    }

    pub fn with_node_get_by_id(mut self, callback: NodeGetByIdCallback) -> Self {
        self.node_get_by_id = Some(callback);
        self
    }

    pub fn with_node_history(mut self, callback: NodeHistoryCallback) -> Self {
        self.node_history = Some(callback);
        self
    }

    pub fn with_node_create(mut self, callback: NodeCreateCallback) -> Self {
        self.node_create = Some(callback);
        self
    }

    pub fn with_node_update(mut self, callback: NodeUpdateCallback) -> Self {
        self.node_update = Some(callback);
        self
    }

    pub fn with_node_delete(mut self, callback: NodeDeleteCallback) -> Self {
        self.node_delete = Some(callback);
        self
    }

    pub fn with_node_update_property(mut self, callback: NodeUpdatePropertyCallback) -> Self {
        self.node_update_property = Some(callback);
        self
    }

    pub fn with_node_move(mut self, callback: NodeMoveCallback) -> Self {
        self.node_move = Some(callback);
        self
    }

    pub fn with_node_query(mut self, callback: NodeQueryCallback) -> Self {
        self.node_query = Some(callback);
        self
    }

    pub fn with_node_get_children(mut self, callback: NodeGetChildrenCallback) -> Self {
        self.node_get_children = Some(callback);
        self
    }

    pub fn with_node_reorder_child(mut self, callback: NodeReorderChildCallback) -> Self {
        self.node_reorder_child = Some(callback);
        self
    }

    pub fn with_node_move_child_relative(
        mut self,
        callback: NodeMoveChildRelativeCallback,
    ) -> Self {
        self.node_move_child_relative = Some(callback);
        self
    }

    pub fn with_node_apply_child_order(mut self, callback: NodeApplyChildOrderCallback) -> Self {
        self.node_apply_child_order = Some(callback);
        self
    }

    pub fn with_node_add_resource(mut self, callback: NodeAddResourceCallback) -> Self {
        self.node_add_resource = Some(callback);
        self
    }

    // SQL builder methods

    pub fn with_sql_query(mut self, callback: SqlQueryCallback) -> Self {
        self.sql_query = Some(callback);
        self
    }

    pub fn with_sql_execute(mut self, callback: SqlExecuteCallback) -> Self {
        self.sql_execute = Some(callback);
        self
    }

    // Service builder methods

    pub fn with_http_request(mut self, callback: HttpRequestCallback) -> Self {
        self.http_request = Some(callback);
        self
    }

    pub fn with_emit_event(mut self, callback: EmitEventCallback) -> Self {
        self.emit_event = Some(callback);
        self
    }

    pub fn with_ai_completion(mut self, callback: AICompletionCallback) -> Self {
        self.ai_completion = Some(callback);
        self
    }

    pub fn with_ai_embed(mut self, callback: AIEmbedCallback) -> Self {
        self.ai_embed = Some(callback);
        self
    }

    pub fn with_ai_list_providers(mut self, callback: AIListProvidersCallback) -> Self {
        self.ai_list_providers = Some(callback);
        self
    }

    pub fn with_ai_list_models(mut self, callback: AIListModelsCallback) -> Self {
        self.ai_list_models = Some(callback);
        self
    }

    pub fn with_ai_get_default_model(mut self, callback: AIGetDefaultModelCallback) -> Self {
        self.ai_get_default_model = Some(callback);
        self
    }

    pub fn with_resource_get_binary(mut self, callback: ResourceGetBinaryCallback) -> Self {
        self.resource_get_binary = Some(callback);
        self
    }

    /// Set the extraction-writeback callback.
    pub fn with_asset_set_extraction(mut self, callback: AssetSetExtractionCallback) -> Self {
        self.asset_set_extraction = Some(callback);
        self
    }

    /// Set the re-extraction callback.
    pub fn with_asset_reextract(mut self, callback: AssetReextractCallback) -> Self {
        self.asset_reextract = Some(callback);
        self
    }

    /// Set the mount-content fetch callback.
    pub fn with_asset_ensure_content(mut self, callback: AssetEnsureContentCallback) -> Self {
        self.asset_ensure_content = Some(callback);
        self
    }

    /// Set the signed-asset-URL callback.
    pub fn with_asset_signed_url(mut self, callback: AssetSignedUrlCallback) -> Self {
        self.asset_signed_url = Some(callback);
        self
    }

    pub fn with_pdf_process_from_storage(
        mut self,
        callback: PdfProcessFromStorageCallback,
    ) -> Self {
        self.pdf_process_from_storage = Some(callback);
        self
    }

    pub fn with_task_create(mut self, callback: TaskCreateCallback) -> Self {
        self.task_create = Some(callback);
        self
    }

    pub fn with_task_update(mut self, callback: TaskUpdateCallback) -> Self {
        self.task_update = Some(callback);
        self
    }

    pub fn with_task_complete(mut self, callback: TaskCompleteCallback) -> Self {
        self.task_complete = Some(callback);
        self
    }

    pub fn with_task_query(mut self, callback: TaskQueryCallback) -> Self {
        self.task_query = Some(callback);
        self
    }

    pub fn with_function_execute(mut self, callback: FunctionExecuteCallback) -> Self {
        self.function_execute = Some(callback);
        self
    }

    pub fn with_function_call(mut self, callback: FunctionCallCallback) -> Self {
        self.function_call = Some(callback);
        self
    }

    pub fn with_flow_run(mut self, callback: FlowRunCallback) -> Self {
        self.flow_run = Some(callback);
        self
    }

    pub fn with_branch_diff(mut self, callback: BranchDiffCallback) -> Self {
        self.branch_diff = Some(callback);
        self
    }

    pub fn with_branch_compare(mut self, callback: BranchCompareCallback) -> Self {
        self.branch_compare = Some(callback);
        self
    }

    pub fn with_scheduler_schedule(mut self, callback: SchedulerScheduleCallback) -> Self {
        self.scheduler_schedule = Some(callback);
        self
    }

    pub fn with_scheduler_cancel(mut self, callback: SchedulerCancelCallback) -> Self {
        self.scheduler_cancel = Some(callback);
        self
    }

    pub fn with_scheduler_list(mut self, callback: SchedulerListCallback) -> Self {
        self.scheduler_list = Some(callback);
        self
    }

    pub fn with_platform_hook(mut self, callback: PlatformHookCallback) -> Self {
        self.platform_hook = Some(callback);
        self
    }

    pub fn with_scheduler_get(mut self, callback: SchedulerGetCallback) -> Self {
        self.scheduler_get = Some(callback);
        self
    }

    pub fn with_branch_copy_nodes(mut self, callback: BranchCopyNodesCallback) -> Self {
        self.branch_copy_nodes = Some(callback);
        self
    }

    pub fn with_integrations_sync_now(mut self, callback: IntegrationsSyncNowCallback) -> Self {
        self.integrations_sync_now = Some(callback);
        self
    }

    // Transaction builder methods

    pub fn with_tx_begin(mut self, callback: TxBeginCallback) -> Self {
        self.tx_begin = Some(callback);
        self
    }

    pub fn with_tx_commit(mut self, callback: TxCommitCallback) -> Self {
        self.tx_commit = Some(callback);
        self
    }

    pub fn with_tx_rollback(mut self, callback: TxRollbackCallback) -> Self {
        self.tx_rollback = Some(callback);
        self
    }

    pub fn with_tx_set_actor(mut self, callback: TxSetActorCallback) -> Self {
        self.tx_set_actor = Some(callback);
        self
    }

    pub fn with_tx_set_message(mut self, callback: TxSetMessageCallback) -> Self {
        self.tx_set_message = Some(callback);
        self
    }

    pub fn with_tx_create(mut self, callback: TxCreateCallback) -> Self {
        self.tx_create = Some(callback);
        self
    }

    pub fn with_tx_add(mut self, callback: TxAddCallback) -> Self {
        self.tx_add = Some(callback);
        self
    }

    pub fn with_tx_put(mut self, callback: TxPutCallback) -> Self {
        self.tx_put = Some(callback);
        self
    }

    pub fn with_tx_upsert(mut self, callback: TxUpsertCallback) -> Self {
        self.tx_upsert = Some(callback);
        self
    }

    pub fn with_tx_update(mut self, callback: TxUpdateCallback) -> Self {
        self.tx_update = Some(callback);
        self
    }

    pub fn with_tx_delete(mut self, callback: TxDeleteCallback) -> Self {
        self.tx_delete = Some(callback);
        self
    }

    pub fn with_tx_delete_by_id(mut self, callback: TxDeleteByIdCallback) -> Self {
        self.tx_delete_by_id = Some(callback);
        self
    }

    pub fn with_tx_get(mut self, callback: TxGetCallback) -> Self {
        self.tx_get = Some(callback);
        self
    }

    pub fn with_tx_get_by_path(mut self, callback: TxGetByPathCallback) -> Self {
        self.tx_get_by_path = Some(callback);
        self
    }

    pub fn with_tx_list_children(mut self, callback: TxListChildrenCallback) -> Self {
        self.tx_list_children = Some(callback);
        self
    }

    pub fn with_tx_move(mut self, callback: TxMoveCallback) -> Self {
        self.tx_move = Some(callback);
        self
    }

    pub fn with_tx_update_property(mut self, callback: TxUpdatePropertyCallback) -> Self {
        self.tx_update_property = Some(callback);
        self
    }

    // Lock / inventory builder methods

    pub fn with_lock_acquire(mut self, callback: LockAcquireCallback) -> Self {
        self.lock_acquire = Some(callback);
        self
    }

    pub fn with_lock_release(mut self, callback: LockReleaseCallback) -> Self {
        self.lock_release = Some(callback);
        self
    }

    pub fn with_lock_renew(mut self, callback: LockRenewCallback) -> Self {
        self.lock_renew = Some(callback);
        self
    }

    pub fn with_inventory_claim(mut self, callback: InventoryClaimCallback) -> Self {
        self.inventory_claim = Some(callback);
        self
    }

    pub fn with_inventory_release(mut self, callback: InventoryReleaseCallback) -> Self {
        self.inventory_release = Some(callback);
        self
    }

    // Secret store builder methods

    pub fn with_secret_get(mut self, callback: SecretGetCallback) -> Self {
        self.secret_get = Some(callback);
        self
    }

    pub fn with_secret_put(mut self, callback: SecretPutCallback) -> Self {
        self.secret_put = Some(callback);
        self
    }

    pub fn with_secret_list(mut self, callback: SecretListCallback) -> Self {
        self.secret_list = Some(callback);
        self
    }

    pub fn with_secret_rotate(mut self, callback: SecretRotateCallback) -> Self {
        self.secret_rotate = Some(callback);
        self
    }

    pub fn with_secret_delete(mut self, callback: SecretDeleteCallback) -> Self {
        self.secret_delete = Some(callback);
        self
    }

    pub fn with_identity_find_by_email(mut self, callback: IdentityFindByEmailCallback) -> Self {
        self.identity_find_by_email = Some(callback);
        self
    }

    pub fn with_identity_update(mut self, callback: IdentityUpdateCallback) -> Self {
        self.identity_update = Some(callback);
        self
    }
}
