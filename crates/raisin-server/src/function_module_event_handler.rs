//! Event handler for invalidating the function module cache.
//!
//! The module cache holds the set of files a function may import. It is off by
//! default (see `raisin_functions::execution::module_cache`); this handler is
//! what makes it safe to turn on, by dropping cached entries the moment function
//! code is written through the normal node path.
//!
//! Invalidation is per WORKSPACE, not per function, and deliberately so: editing
//! a shared module has to invalidate every function that imports it, and the
//! import graph is precisely what the cache is holding — so the changed node's
//! path cannot identify the affected entries. Re-resolving a workspace's
//! functions is cheap; serving stale code is not.

use raisin_events::{Event, EventHandler};
use std::future::Future;
use std::pin::Pin;

/// Invalidates cached function module sets when nodes in a functions workspace change.
pub struct FunctionModuleEventHandler {
    /// Workspace that holds function code, e.g. `functions`.
    functions_workspace: String,
}

impl FunctionModuleEventHandler {
    pub fn new(functions_workspace: impl Into<String>) -> Self {
        Self {
            functions_workspace: functions_workspace.into(),
        }
    }
}

impl EventHandler for FunctionModuleEventHandler {
    fn name(&self) -> &str {
        "function_module_invalidator"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // Skip the work entirely when the cache is disabled, which is the default.
            if !raisin_functions::execution::module_cache::is_enabled() {
                return Ok(());
            }

            if let Event::Node(node_event) = event {
                if node_event.workspace_id == self.functions_workspace {
                    raisin_functions::execution::module_cache::invalidate_workspace_functions(
                        &node_event.tenant_id,
                        &node_event.repository_id,
                        &node_event.branch,
                        &node_event.workspace_id,
                    );
                }
            }
            Ok(())
        })
    }
}
