// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Durable agent runs: build the node's runtime and start its sweeper.
//!
//! Everything here is an existing mechanism, composed:
//!
//! - the store is ordinary nodes in `raisin:system`, each commit one node
//!   transaction serialized by the locks subsystem and replicated like any
//!   node write — exactly as flow instances are;
//! - a wake is an `AgentRunStep` job on the ordinary queue, picked up by any
//!   worker of any node;
//! - reducers and operations are ordinary functions, run by the generic
//!   executor (any language), operations as the run's principal;
//! - a flow step waiting for a run is resumed by a `FlowInstanceExecution`
//!   job when the run ends;
//! - the sweeper is a periodic task like the flow wait sweeper: it takes over
//!   expired leases, wakes `Queued` runs, closes expired requests and
//!   finalizes terminal domain runs.

#![cfg(feature = "storage-rocksdb")]

use std::sync::Arc;

use raisin_agent_runtime::clock::SystemClock;
use raisin_agent_runtime::host::AgentRunHost;
use raisin_agent_runtime::ids::RunScope;
use raisin_agent_runtime::service::{AgentRunService, ServiceConfig};
use raisin_binary::BinaryStorage;
use raisin_functions::execution::{
    create_function_executor, ExecutionDependencies, FunctionExecutionConfig,
    FunctionReducerResolver,
};
use raisin_rocksdb::agent_runs::{
    install_agent_run_host, FlowResumeSink, FunctionCaller, FunctionModelTurnExecutor,
    FunctionOperationExecutor, JobQueueWaker, NodeAgentRunStore, DEFAULT_MODEL_TURN_FUNCTION,
};
use raisin_rocksdb::RocksDBStorage;

/// Build the runtime, install it for the job handler and the transports, and
/// start the sweeper. Returns the host.
pub fn start<B>(
    storage: Arc<RocksDBStorage>,
    deps: Arc<ExecutionDependencies<RocksDBStorage, B>>,
    lock_manager: Option<raisin_locks::LockManagerHandle>,
    node_id: String,
    sweep_interval_secs: u64,
) -> Arc<AgentRunHost>
where
    B: BinaryStorage + 'static,
{
    let store = Arc::new(NodeAgentRunStore::open(
        storage.clone(),
        lock_manager,
        node_id.clone(),
    ));
    let waker = Arc::new(JobQueueWaker::new(
        storage.job_registry().clone(),
        storage.job_data_store().clone(),
    ));
    let service = Arc::new(AgentRunService::new(
        store,
        waker,
        Arc::new(SystemClock),
        ServiceConfig::default(),
    ));
    service.set_waiter_sink(Arc::new(FlowResumeSink::new(
        storage.job_registry().clone(),
        storage.job_data_store().clone(),
    )));
    let config = FunctionExecutionConfig::default();
    let caller = FunctionCaller::new(create_function_executor(deps.clone(), config.clone()));
    let model_turns = Arc::new(FunctionModelTurnExecutor::new(
        caller.clone(),
        DEFAULT_MODEL_TURN_FUNCTION,
    ));
    let executor = Arc::new(FunctionOperationExecutor::new(
        storage.clone(),
        caller,
        model_turns,
    ));
    let reducers = Arc::new(FunctionReducerResolver::new(deps, config));
    let host = Arc::new(AgentRunHost::new(service, reducers, executor, node_id));
    install_agent_run_host(host.clone());
    spawn_sweeper(storage, host.clone(), sweep_interval_secs);
    tracing::info!("Durable agent run runtime started");
    host
}

fn spawn_sweeper(storage: Arc<RocksDBStorage>, host: Arc<AgentRunHost>, interval_secs: u64) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            sweep_once(&storage, &host).await;
        }
    });
    tracing::info!(interval_secs, "Agent run sweeper started");
}

async fn sweep_once(storage: &Arc<RocksDBStorage>, host: &AgentRunHost) {
    let tenants = match raisin_rocksdb::management::list_tenants(storage).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "Agent run sweeper: failed to list tenants");
            return;
        }
    };
    for tenant in tenants {
        let repos = match raisin_rocksdb::management::list_repositories(storage, &tenant).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(tenant = %tenant, error = %e, "Agent run sweeper: failed to list repositories");
                continue;
            }
        };
        for repo in repos {
            // Runs are keyed by (tenant, repo); a run's branch lives in its
            // record, and the host drives each run in its own scope. One
            // sweep per repository therefore covers every branch.
            {
                let scope = RunScope::new(&tenant, &repo, "main");
                match host.sweep(&scope).await {
                    Ok(report) => {
                        for (run, error) in &report.errors {
                            tracing::warn!(run = %run, error = %error, "Agent run sweeper: repair failed");
                        }
                        if report.recovered + report.finalized > 0 {
                            tracing::info!(
                                tenant = %tenant, repo = %repo,
                                recovered = report.recovered,
                                redispatched = report.redispatched,
                                finalized = report.finalized,
                                "Agent run sweeper: pass complete"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(tenant = %tenant, repo = %repo, error = %e, "Agent run sweeper failed")
                    }
                }
            }
        }
    }
}
