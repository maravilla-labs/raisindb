// SPDX-License-Identifier: BSL-1.1

//! Who may follow, inspect or steer a flow instance.
//!
//! A flow instance is addressed by a bare id on every transport: its live
//! event stream (WebSocket `flow_subscribe_events`, HTTP SSE), its status
//! (which carries the instance variables, i.e. step outputs), cancel, resume
//! and delete. Those entry points used to act for anyone who knew the id.
//!
//! Allowed:
//! * the system, and `system_admin`;
//! * a caller who can READ the `raisin:FlowInstance` node under their own
//!   row-level security (a role may grant that on `raisin:system`);
//! * the signed-in user who started the instance (`__triggering_user`), since
//!   ordinary users hold no grant on `raisin:system` and would otherwise lose
//!   sight of their own run. Before the instance node is written (starting a
//!   flow only queues a job), the start record in
//!   [`raisin_storage::jobs::flow_starters`] stands in for it.
//!
//! Anonymous callers are always refused, and a caller who may not access an
//! instance gets the same answer whether it exists or not, so the check is no
//! existence oracle.

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Workspace that holds flow instances.
pub const FLOW_INSTANCE_WORKSPACE: &str = "raisin:system";
/// Branch that holds flow instances.
pub const FLOW_INSTANCE_BRANCH: &str = "main";
const FLOW_INSTANCE_NODE_TYPE: &str = "raisin:FlowInstance";
/// Instance variable naming the user who started it (see
/// `raisin_flow_runtime::integration::triggers::TRIGGERING_USER_VAR`).
const TRIGGERING_USER_VAR: &str = "__triggering_user";

/// The instance node's path, or `None` for an id that is not a single path
/// segment (which no instance has).
pub fn flow_instance_path(instance_id: &str) -> Option<String> {
    let ok = !instance_id.is_empty()
        && instance_id != "."
        && instance_id != ".."
        && !instance_id.contains('/');
    ok.then(|| format!("/flows/instances/{instance_id}"))
}

fn is_admin(auth: &AuthContext) -> bool {
    auth.is_system || auth.permissions().is_some_and(|p| p.is_system_admin)
}

fn triggering_user(node: &Node) -> Option<&str> {
    match node.properties.get("variables") {
        Some(PropertyValue::Object(vars)) => match vars.get(TRIGGERING_USER_VAR) {
            Some(PropertyValue::String(user)) if !user.is_empty() => Some(user.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// The whole decision, free of I/O.
///
/// `instance` is the node as STORED (`None` when there is none);
/// `recorded_starter` is the start record for the id, consulted only while the
/// node is missing.
pub fn may_access_flow_instance(
    auth: Option<&AuthContext>,
    instance: Option<Node>,
    recorded_starter: Option<&str>,
) -> bool {
    let Some(auth) = auth else {
        return false;
    };
    if is_admin(auth) {
        return true;
    }
    if auth.is_anonymous_principal() {
        return false;
    }
    let Some(user_id) = auth.user_id.as_deref().filter(|id| !id.is_empty()) else {
        return false;
    };
    match instance {
        Some(node) => {
            if node.node_type != FLOW_INSTANCE_NODE_TYPE {
                return false;
            }
            if triggering_user(&node) == Some(user_id) {
                return true;
            }
            let scope = PermissionScope::new(FLOW_INSTANCE_WORKSPACE, FLOW_INSTANCE_BRANCH);
            super::rls_filter::filter_node(node, auth, &scope).is_some()
        }
        None => recorded_starter == Some(user_id),
    }
}

/// Load the instance and decide. Storage errors count as "no instance".
pub async fn authorize_flow_instance<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    instance_id: &str,
    auth: Option<&AuthContext>,
) -> bool {
    if auth.is_some_and(is_admin) {
        return true;
    }
    let Some(path) = flow_instance_path(instance_id) else {
        return false;
    };
    let scope = StorageScope::new(
        tenant_id,
        repo,
        FLOW_INSTANCE_BRANCH,
        FLOW_INSTANCE_WORKSPACE,
    );
    let node = storage
        .nodes()
        .get_by_path(scope, &path, None)
        .await
        .ok()
        .flatten();
    let starter = raisin_storage::jobs::flow_instance_starter(tenant_id, repo, instance_id);
    may_access_flow_instance(auth, node, starter.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
    use std::collections::HashMap;

    fn user(id: &str) -> AuthContext {
        AuthContext::for_user(id).with_permissions(ResolvedPermissions::empty(id))
    }

    /// A user whose role grants read on every flow instance.
    fn operator(id: &str) -> AuthContext {
        let read_instances = Permission::new("flows/instances/**", vec![Operation::Read])
            .with_workspace(FLOW_INSTANCE_WORKSPACE);
        let mut resolved = ResolvedPermissions::empty(id);
        resolved.permissions = vec![read_instances];
        AuthContext::for_user(id).with_permissions(resolved)
    }

    fn anonymous() -> AuthContext {
        let public = Permission::new("**", vec![Operation::Read]);
        AuthContext::anonymous_user("anon-node")
            .with_permissions(ResolvedPermissions::anonymous(vec![public]))
    }

    fn admin() -> AuthContext {
        AuthContext::for_user("root").with_permissions(ResolvedPermissions::system_admin())
    }

    fn instance(id: &str, started_by: Option<&str>) -> Node {
        let mut vars = HashMap::new();
        if let Some(u) = started_by {
            vars.insert(
                TRIGGERING_USER_VAR.to_string(),
                PropertyValue::String(u.to_string()),
            );
        }
        let mut node = Node {
            id: format!("node-{id}"),
            name: id.to_string(),
            path: flow_instance_path(id).unwrap(),
            node_type: FLOW_INSTANCE_NODE_TYPE.to_string(),
            ..Default::default()
        };
        node.properties
            .insert("variables".into(), PropertyValue::Object(vars));
        node
    }

    #[test]
    fn the_user_who_started_it_may_access_it() {
        let alice = user("alice");
        assert!(may_access_flow_instance(
            Some(&alice),
            Some(instance("i1", Some("alice"))),
            None
        ));
    }

    #[test]
    fn the_starter_may_access_it_before_the_node_is_written() {
        let alice = user("alice");
        assert!(may_access_flow_instance(Some(&alice), None, Some("alice")));
    }

    #[test]
    fn another_user_may_not() {
        let bob = user("bob");
        assert!(!may_access_flow_instance(
            Some(&bob),
            Some(instance("i1", Some("alice"))),
            None
        ));
        // Nor before the node exists.
        assert!(!may_access_flow_instance(Some(&bob), None, Some("alice")));
    }

    #[test]
    fn a_role_that_can_read_the_instance_may() {
        let ops = operator("ops");
        assert!(may_access_flow_instance(
            Some(&ops),
            Some(instance("i1", Some("alice"))),
            None
        ));
    }

    #[test]
    fn anonymous_callers_may_not() {
        let anon = anonymous();
        // Even with a public read grant everywhere, and even as the recorded
        // "starter" (every anonymous caller shares one principal).
        let node_user = anon.user_id.clone();
        assert!(!may_access_flow_instance(
            Some(&anon),
            Some(instance("i1", node_user.as_deref())),
            node_user.as_deref()
        ));
        assert!(!may_access_flow_instance(
            None,
            Some(instance("i1", Some("alice"))),
            None
        ));
    }

    /// Unknown and guessed ids answer exactly like an existing instance the
    /// caller may not see: refused, no oracle.
    #[test]
    fn unknown_and_guessed_ids_are_refused_like_foreign_ones() {
        let bob = user("bob");
        assert!(!may_access_flow_instance(Some(&bob), None, None));
        assert!(!may_access_flow_instance(
            Some(&operator("ops")),
            None,
            None
        ));
        assert!(flow_instance_path("../../users/alice").is_none());
        assert!(flow_instance_path("").is_none());
        assert!(flow_instance_path("..").is_none());
    }

    #[test]
    fn a_node_that_is_not_a_flow_instance_is_refused() {
        let alice = user("alice");
        let mut node = instance("i1", Some("alice"));
        node.node_type = "raisin:Folder".into();
        assert!(!may_access_flow_instance(Some(&alice), Some(node), None));
    }

    #[test]
    fn the_system_and_admins_may_access_anything() {
        assert!(may_access_flow_instance(
            Some(&AuthContext::system()),
            None,
            None
        ));
        assert!(may_access_flow_instance(
            Some(&admin()),
            Some(instance("i1", Some("alice"))),
            None
        ));
    }

    /// End to end over storage: the WS and HTTP handlers call exactly this.
    #[tokio::test]
    async fn authorize_reads_the_stored_instance_and_the_start_record() {
        use raisin_storage::{CreateNodeOptions, NodeRepository, StorageScope};
        use raisin_storage_memory::InMemoryStorage;

        let (tenant, repo) = ("t_flow_access", "r_flow_access");
        let storage = InMemoryStorage::default();
        let scope = StorageScope::new(tenant, repo, FLOW_INSTANCE_BRANCH, FLOW_INSTANCE_WORKSPACE);
        storage
            .nodes()
            .create(
                scope,
                instance("inst-alice", Some("alice")),
                CreateNodeOptions {
                    validate_schema: false,
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let check = |id: &'static str, auth: Option<AuthContext>| {
            let storage = &storage;
            async move { authorize_flow_instance(storage, tenant, repo, id, auth.as_ref()).await }
        };

        // The owner.
        assert!(check("inst-alice", Some(user("alice"))).await);
        // Another user, anonymous, no identity.
        assert!(!check("inst-alice", Some(user("bob"))).await);
        assert!(!check("inst-alice", Some(anonymous())).await);
        assert!(!check("inst-alice", None).await);
        // Unknown and guessed ids: the same refusal.
        assert!(!check("no-such-instance", Some(user("bob"))).await);
        assert!(!check("../../users/alice", Some(user("bob"))).await);
        assert!(!check("inst-alice/../inst-alice", Some(user("alice"))).await);
        // The system.
        assert!(check("inst-alice", Some(AuthContext::system())).await);
        assert!(check("no-such-instance", Some(AuthContext::system())).await);

        // Started but not yet written: only the recorded starter, in this
        // tenant and repo.
        raisin_storage::jobs::record_flow_instance_starter(tenant, repo, "inst-queued", "alice");
        assert!(check("inst-queued", Some(user("alice"))).await);
        assert!(!check("inst-queued", Some(user("bob"))).await);
        assert!(
            !authorize_flow_instance(
                &storage,
                "other-tenant",
                repo,
                "inst-queued",
                Some(&user("alice"))
            )
            .await
        );
    }
}
