// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function API definitions
//!
//! This module defines the API surface exposed to user-defined functions.
//! The API mirrors the structure of raisin-client-js for familiarity.
//!
//! ## Module Structure
//!
//! - [`traits`] - `FunctionApi` trait with all operations available to functions
//! - [`mock`] - `MockFunctionApi` for testing
//! - [`callbacks`] - Callback type definitions connecting to backend services
//! - [`raisindb`] - Real `RaisinFunctionApi` implementation using callbacks
//! - [`ai`] - AI/LLM provider trait and types

mod ai;
mod callbacks;
mod mock;
mod raisindb;
mod traits;

pub use ai::{
    AIApi, AIApiError, CompletionRequest, CompletionResponse, Message, MockAIApi,
    ModelCapabilities, ModelInfo, UsageStats,
};
pub use callbacks::{
    AICompletionCallback,
    AIEmbedCallback,
    AIGetDefaultModelCallback,
    AIListModelsCallback,
    EmitEventCallback,
    FunctionCallCallback,
    FunctionExecuteCallback,
    FunctionExecuteContext,
    HttpRequestCallback,
    NodeAddResourceCallback,
    NodeCreateCallback,
    NodeDeleteCallback,
    NodeGetByIdCallback,
    NodeGetCallback,
    NodeGetChildrenCallback,
    NodeMoveCallback,
    NodeQueryCallback,
    NodeUpdateCallback,
    NodeUpdatePropertyCallback,
    PdfProcessFromStorageCallback,
    RaisinFunctionApiCallbacks,
    ResourceGetBinaryCallback,
    SqlExecuteCallback,
    SqlQueryCallback,
    TaskCompleteCallback,
    TaskCreateCallback,
    TaskQueryCallback,
    TaskUpdateCallback,
    // Transaction callbacks
    TxAddCallback,
    TxBeginCallback,
    TxCommitCallback,
    TxCreateCallback,
    TxCreateDeepCallback,
    TxDeleteByIdCallback,
    TxDeleteCallback,
    TxGetByPathCallback,
    TxGetCallback,
    TxListChildrenCallback,
    TxMoveCallback,
    TxPutCallback,
    TxRollbackCallback,
    TxSetActorCallback,
    TxSetMessageCallback,
    TxUpdateCallback,
    TxUpdatePropertyCallback,
    TxUpsertCallback,
    TxUpsertDeepCallback,
};
pub use mock::MockFunctionApi;
pub use raisindb::RaisinFunctionApi;
pub use traits::FunctionApi;
