// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The node's `AgentRunHost`, late-bound.
//!
//! The job system is initialised before the server has built the pieces a
//! host needs (the function executor, the reducer resolver), so the step
//! handler reads the host from here at run time instead of capturing it at
//! construction — the same construction-order problem the mount-content
//! resolver solves. Until a host is installed, a step job fails and is
//! retried; the sweeper re-wakes the run anyway.

use std::sync::{Arc, OnceLock, RwLock};

use raisin_agent_runtime::host::AgentRunHost;

static HOST: OnceLock<RwLock<Option<Arc<AgentRunHost>>>> = OnceLock::new();

fn slot() -> &'static RwLock<Option<Arc<AgentRunHost>>> {
    HOST.get_or_init(|| RwLock::new(None))
}

/// Install (or replace) this process's host.
pub fn install_agent_run_host(host: Arc<AgentRunHost>) {
    *slot().write().expect("agent run host slot poisoned") = Some(host);
}

/// This process's host, once installed.
pub fn agent_run_host() -> Option<Arc<AgentRunHost>> {
    slot().read().expect("agent run host slot poisoned").clone()
}
