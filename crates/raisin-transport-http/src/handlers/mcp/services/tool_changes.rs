// SPDX-License-Identifier: BSL-1.1

//! Announcing tool-list changes to subscribed MCP clients.
//!
//! A client that opted into `toolsListChanged` on its `subscriptions/listen`
//! stream is told whenever this server's tool set changes. Tools come from
//! `raisin:Function` nodes in the `functions` workspace, and the event bus
//! already delivers every one of those — the same events
//! `McpPlanCacheInvalidator` keys on.
//!
//! **One bus handler per process, not one per subscription.** Its sibling
//! [`BusEventSource`](super::events::BusEventSource) calls `subscribe_fn` for
//! every subscription and never unsubscribes; `clear_subscribers` is the bus's
//! only removal API, so its handler list grows for the process lifetime. Here a
//! single handler fans out over a `broadcast` channel instead, and a subscriber
//! going away drops its receiver with nothing left behind.

use std::sync::{Arc, OnceLock};

use raisin_events::{Event, EventBus, EventBusExt, EventFilter};
use tokio::sync::broadcast;

/// Workspace whose functions become tools.
const FUNCTIONS_WORKSPACE: &str = "functions";
/// NodeType a tool is generated from.
const FUNCTION_NODE_TYPE: &str = "raisin:Function";

/// How many pending changes a slow subscriber may fall behind before lagging.
///
/// Small on purpose: the notification carries no payload, so a subscriber that
/// misses ten of them and one of them is told exactly the same thing — "re-list".
const CHANNEL_CAPACITY: usize = 16;

/// A tool-relevant change, scoped so a subscriber can ignore other tenants'.
#[derive(Debug, Clone)]
pub(in crate::handlers::mcp) struct ToolChange {
    pub tenant_id: String,
    pub repo: String,
}

/// Subscribe to tool changes, registering the single bus handler on first use.
pub(in crate::handlers::mcp) fn subscribe(
    bus: &Arc<dyn EventBus>,
) -> broadcast::Receiver<ToolChange> {
    static SENDER: OnceLock<broadcast::Sender<ToolChange>> = OnceLock::new();

    SENDER
        .get_or_init(|| {
            let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
            let forward = tx.clone();

            bus.subscribe_fn(
                "mcp-tool-list-changed".to_string(),
                EventFilter::AllNode,
                move |event| {
                    let change = match event {
                        Event::Node(node) if is_tool_change(node) => Some(ToolChange {
                            tenant_id: node.tenant_id.clone(),
                            repo: node.repository_id.clone(),
                        }),
                        _ => None,
                    };
                    let forward = forward.clone();
                    Box::pin(async move {
                        if let Some(change) = change {
                            // No receivers is the normal case — nobody is
                            // subscribed. Not an error.
                            let _ = forward.send(change);
                        }
                        Ok(())
                    })
                },
            );
            tx
        })
        .subscribe()
}

/// Whether a node event changes what this server would list as tools.
///
/// Deliberately broad: any create, update or delete of a function counts,
/// because the notification says only "re-list" and it is cheaper to have the
/// client re-list once too often than to miss a change. Narrowing this to
/// "functions with `mcp: true`" would need the node's properties, which a
/// delete event does not carry.
fn is_tool_change(event: &raisin_events::NodeEvent) -> bool {
    if event.workspace_id != FUNCTIONS_WORKSPACE {
        return false;
    }
    match event.node_type.as_deref() {
        Some(node_type) => node_type == FUNCTION_NODE_TYPE,
        // A change in the functions workspace whose type we cannot read counts.
        // Missing a real tool change leaves a client's list stale with nothing
        // to correct it; an extra "re-list" costs one `tools/list`.
        None => true,
    }
}

/// Whether a received change belongs to this subscription's scope.
pub(in crate::handlers::mcp) fn matches(change: &ToolChange, tenant_id: &str, repo: &str) -> bool {
    change.tenant_id == tenant_id && change.repo == repo
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_events::{NodeEvent, NodeEventKind};

    fn event(workspace: &str, node_type: &str) -> NodeEvent {
        NodeEvent {
            tenant_id: "t".into(),
            repository_id: "r".into(),
            branch: "main".into(),
            workspace_id: workspace.into(),
            node_id: "n".into(),
            node_type: Some(node_type.into()),
            revision: raisin_hlc::HLC::now(),
            kind: NodeEventKind::Updated,
            path: Some("/mcp/linear/search".into()),
            metadata: None,
        }
    }

    #[test]
    fn a_function_change_counts() {
        assert!(is_tool_change(&event("functions", "raisin:Function")));
    }

    /// A function-shaped node in another workspace is not a tool, and a
    /// different node type in `functions` is not one either.
    #[test]
    fn other_workspaces_and_types_do_not_count() {
        assert!(!is_tool_change(&event("content", "raisin:Function")));
        assert!(!is_tool_change(&event("functions", "raisin:AIAgent")));
    }

    /// A delete may arrive without a readable node type. Treat it as a change:
    /// an extra re-list costs one request, a missed one leaves the client
    /// permanently stale.
    #[test]
    fn an_untyped_change_in_the_functions_workspace_counts() {
        let mut e = event("functions", "raisin:Function");
        e.node_type = None;
        assert!(is_tool_change(&e));

        let mut elsewhere = event("content", "raisin:Function");
        elsewhere.node_type = None;
        assert!(!is_tool_change(&elsewhere));
    }

    #[test]
    fn scope_matching_is_tenant_and_repo() {
        let change = ToolChange {
            tenant_id: "t".into(),
            repo: "r".into(),
        };
        assert!(matches(&change, "t", "r"));
        assert!(!matches(&change, "other", "r"));
        assert!(!matches(&change, "t", "other"));
    }
}
