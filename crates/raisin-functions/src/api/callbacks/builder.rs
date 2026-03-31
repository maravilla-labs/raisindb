// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Builder for assembling all RaisinFunctionApi callbacks

use super::node_ops::*;
use super::service_ops::*;
use super::sql_ops::*;
use super::transaction_ops::*;

/// Builder for RaisinFunctionApi callbacks
#[derive(Default)]
pub struct RaisinFunctionApiCallbacks {
    pub node_get: Option<NodeGetCallback>,
    pub node_get_by_id: Option<NodeGetByIdCallback>,
    pub node_create: Option<NodeCreateCallback>,
    pub node_update: Option<NodeUpdateCallback>,
    pub node_delete: Option<NodeDeleteCallback>,
    pub node_update_property: Option<NodeUpdatePropertyCallback>,
    pub node_move: Option<NodeMoveCallback>,
    pub node_query: Option<NodeQueryCallback>,
    pub node_get_children: Option<NodeGetChildrenCallback>,
    pub node_add_resource: Option<NodeAddResourceCallback>,
    pub sql_query: Option<SqlQueryCallback>,
    pub sql_execute: Option<SqlExecuteCallback>,
    pub http_request: Option<HttpRequestCallback>,
    pub emit_event: Option<EmitEventCallback>,
    pub ai_completion: Option<AICompletionCallback>,
    pub ai_embed: Option<AIEmbedCallback>,
    pub ai_list_models: Option<AIListModelsCallback>,
    pub ai_get_default_model: Option<AIGetDefaultModelCallback>,
    pub resource_get_binary: Option<ResourceGetBinaryCallback>,
    pub pdf_process_from_storage: Option<PdfProcessFromStorageCallback>,
    pub task_create: Option<TaskCreateCallback>,
    pub task_update: Option<TaskUpdateCallback>,
    pub task_complete: Option<TaskCompleteCallback>,
    pub task_query: Option<TaskQueryCallback>,
    pub function_execute: Option<FunctionExecuteCallback>,
    pub function_call: Option<FunctionCallCallback>,
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
}
