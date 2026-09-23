// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The server's reducer plumbing: functions loaded from the `functions`
//! workspace and executed through the generic executor under
//! [`ExecutionPolicy::Deterministic`].
//!
//! Same loading path as every other invocation (function node → entry file →
//! module set → the language's runtime); the only differences are the policy
//! and the API, which is a mock — a deterministic call reaches no host API.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use raisin_agent_runtime::domain::{DomainReducer, ReducerCallError, ReducerRef};
use raisin_agent_runtime::host::ReducerResolver;
use raisin_agent_runtime::ids::RunScope;
use raisin_binary::BinaryStorage;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::Value;

use super::agent_reducer::{
    function_artifact_hash, DeterministicInvoker, DeterministicRun, FunctionDomainReducer,
};
use super::code_loader;
use super::types::{ExecutionDependencies, FunctionExecutionConfig};
use crate::api::{FunctionApi, MockFunctionApi};
use crate::executor::FunctionExecutor;
use crate::types::{
    ExecutionContext, ExecutionPolicy, FunctionCode, FunctionLanguage, FunctionMetadata,
    LoadedFunction,
};

/// Loads functions of one `(tenant, repo, branch)` from storage and runs them
/// deterministically.
pub struct StorageDeterministicInvoker<S, B>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    deps: Arc<ExecutionDependencies<S, B>>,
    config: FunctionExecutionConfig,
    scope: RunScope,
}

impl<S, B> StorageDeterministicInvoker<S, B>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    /// An invoker for `scope`.
    pub fn new(
        deps: Arc<ExecutionDependencies<S, B>>,
        config: FunctionExecutionConfig,
        scope: RunScope,
    ) -> Self {
        Self {
            deps,
            config,
            scope,
        }
    }

    async fn load(
        &self,
        function_path: &str,
    ) -> Result<
        (
            FunctionMetadata,
            FunctionCode,
            HashMap<String, String>,
            String,
        ),
        String,
    > {
        let (t, r, b) = (
            self.scope.tenant_id.as_str(),
            self.scope.repo_id.as_str(),
            self.scope.branch.as_str(),
        );
        let ws = self.config.functions_workspace.as_str();
        let node =
            code_loader::load_function_node(self.deps.storage.as_ref(), t, r, b, ws, function_path)
                .await
                .map_err(|e| format!("reducer function {function_path}: {e}"))?;
        if super::remote_tool::proxy_block(&node).is_some() {
            return Err(format!(
                "reducer function {function_path} is a remote proxy; a reducer must run locally"
            ));
        }
        let (code, metadata) = code_loader::load_function_code(
            self.deps.storage.as_ref(),
            self.deps.binary_storage.as_ref(),
            t,
            r,
            b,
            ws,
            &node,
            function_path,
        )
        .await
        .map_err(|e| format!("reducer function {function_path}: {e}"))?;
        let mut files = HashMap::new();
        if metadata.language != FunctionLanguage::Wasm {
            let entry = metadata.entry_file_path().to_string();
            files = code_loader::load_sibling_files(
                self.deps.storage.as_ref(),
                self.deps.binary_storage.as_ref(),
                t,
                r,
                b,
                ws,
                function_path,
                &entry,
            )
            .await
            .unwrap_or_default();
            let external = code_loader::load_external_modules(
                self.deps.storage.as_ref(),
                self.deps.binary_storage.as_ref(),
                t,
                r,
                b,
                ws,
                function_path,
                code.as_text().unwrap_or(""),
                &files,
            )
            .await
            .unwrap_or_default();
            files.extend(external);
        }
        Ok((metadata, code, files, node.id))
    }
}

#[async_trait]
impl<S, B> DeterministicInvoker for StorageDeterministicInvoker<S, B>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    async fn artifact_hash(&self, function_path: &str) -> Result<String, String> {
        let (metadata, code, files, _) = self.load(function_path).await?;
        Ok(function_artifact_hash(&metadata, &code, &files))
    }

    async fn invoke(&self, function_path: &str, input: Value) -> Result<DeterministicRun, String> {
        let (metadata, code, files, node_id) = self.load(function_path).await?;
        let hash = function_artifact_hash(&metadata, &code, &files);
        let context = ExecutionContext::new(
            &self.scope.tenant_id,
            &self.scope.repo_id,
            &self.scope.branch,
            "system",
        )
        .with_workspace(&self.config.functions_workspace)
        .with_input(input)
        .with_policy(ExecutionPolicy::Deterministic);
        let api: Arc<dyn FunctionApi> = Arc::new(MockFunctionApi::new(serde_json::json!({})));
        let loaded = LoadedFunction::with_files(
            metadata,
            code,
            files,
            function_path.to_string(),
            node_id,
            self.config.functions_workspace.clone(),
        );
        let result = FunctionExecutor::new()
            .execute(&loaded, context, api)
            .await
            .map_err(|e| e.to_string())?;
        Ok(DeterministicRun {
            artifact_hash: hash,
            result,
        })
    }
}

/// Resolves run reducers to functions in the run's own repository.
pub struct FunctionReducerResolver<S, B>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    deps: Arc<ExecutionDependencies<S, B>>,
    config: FunctionExecutionConfig,
}

impl<S, B> FunctionReducerResolver<S, B>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    /// A resolver over the server's execution dependencies.
    pub fn new(deps: Arc<ExecutionDependencies<S, B>>, config: FunctionExecutionConfig) -> Self {
        Self { deps, config }
    }

    fn invoker(&self, scope: &RunScope) -> Arc<dyn DeterministicInvoker> {
        Arc::new(StorageDeterministicInvoker::new(
            self.deps.clone(),
            self.config.clone(),
            scope.clone(),
        ))
    }
}

#[async_trait]
impl<S, B> ReducerResolver for FunctionReducerResolver<S, B>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    async fn bind(
        &self,
        scope: &RunScope,
        function_path: &str,
        handler: &str,
    ) -> Result<ReducerRef, ReducerCallError> {
        let r = FunctionDomainReducer::bind(function_path, handler, self.invoker(scope)).await?;
        Ok(r.reducer_ref().clone())
    }

    fn reducer(&self, scope: &RunScope, reducer: &ReducerRef) -> Arc<dyn DomainReducer> {
        Arc::new(FunctionDomainReducer::new(
            reducer.clone(),
            self.invoker(scope),
        ))
    }
}
