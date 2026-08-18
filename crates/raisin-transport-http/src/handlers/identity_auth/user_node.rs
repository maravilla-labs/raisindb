// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! User node management for identity authentication.
//!
//! This module handles creating and looking up `raisin:User` nodes in the
//! repository's `raisin:access_control` workspace.

#[cfg(feature = "storage-rocksdb")]
use std::sync::Arc;

/// Convert email to a safe node name.
#[cfg(feature = "storage-rocksdb")]
pub fn email_to_node_name(email: &str) -> String {
    email
        .to_lowercase()
        .replace('@', "-at-")
        .replace('.', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}

/// The branch on which the `raisin:access_control` workspace lives.
///
/// This is deliberately a constant and NOT the repository's configured
/// `default_branch`.
///
/// Global workspaces pin their own branch at the WORKSPACE level:
/// `raisin-core/global_workspaces/access_control.yaml` ends with
/// `config: { default_branch: main }`, and `workspace_structure_init`
/// (raisin-core/src/workspace_structure_init/mod.rs:92-93) seeds the
/// workspace's `initial_structure` on `workspace.config.default_branch`.
/// Nothing propagates `RepositoryConfig.default_branch` into `WorkspaceConfig`.
/// So `/roles/*`, `/users/system/anonymous` and the `/users/internal` parent
/// all live on `main` no matter what branch the repository defaults to.
///
/// Every reader agrees: all seven `resolve_for_identity_id` call sites pass the
/// literal "main" (middleware/auth.rs, mcp/auth.rs, oauth_as/authorize.rs, the
/// two pgwire session paths, and raisin-transport-ws/src/handlers/auth.rs).
///
/// This module used to resolve the REPOSITORY's default branch instead, which
/// meant that on a repository configured with any other default branch the
/// `raisin:User` node was written to a branch nobody reads - reproducing the
/// aiopenhouse blackout exactly (silent, `200 []`, no error) with an otherwise
/// perfectly valid node. Writing where the roles already are is the correct
/// direction: moving the READERS to the repo branch would only find the user
/// node and then fail to resolve its roles, which live on `main`.
///
/// If workspace-level `default_branch` ever becomes per-repository
/// configurable, this constant and those call sites must move together.
#[cfg(feature = "storage-rocksdb")]
const ACCESS_CONTROL_BRANCH: &str = "main";

/// Disambiguating suffix for an email whose slug is already taken by a
/// *different* address.
///
/// `email_to_node_name` maps both '.' and '@' onto '-', so `a.b@x.com` and
/// `a-b@x.com` collide on `/users/internal/a-b-at-x-com`. When that happens the
/// second user must still get a node - refusing would leave them with no ACL
/// node, hence no roles, hence a silent `200 []` on every read, which is the
/// exact outage this module exists to prevent.
///
/// FNV-1a over the lowercased address: deterministic (so the same user lands on
/// the same node at every login), dependency-free, and only ever used as a
/// disambiguator - never as a security boundary.
#[cfg_attr(not(feature = "storage-rocksdb"), allow(dead_code))]
fn email_slug_suffix(email: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in email.to_lowercase().as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (hash & 0xffff_ffff) as u32)
}

// ---------------------------------------------------------------------------
// Reconciliation of a pre-existing raisin:User node
// ---------------------------------------------------------------------------
//
// Background (incident, tenant "aiopenhouse"): deleting an identity calls
// `cascade_user_status(.., "deleted")`
// (handlers/identity_users.rs:408) which only flips the ACL node's `status`
// property - the node itself stays at its email-derived path
// /users/internal/{email-slug}. Re-creating an identity with the same email
// mints a NEW identity_id, but `ensure_user_node` used to early-return the
// node it found without rebinding anything. Role resolution goes by the
// `user_id` property (`find_user_by_identity_id`,
// raisin-core/src/services/permission_service/mod.rs:340-386), so it stopped
// matching -> `resolve_for_identity_id` returns None -> AuthContext carries
// `resolved_permissions: None` -> the RLS filter
// (raisin-core/src/services/rls_filter/mod.rs:22-82) drops EVERY row. Reads
// silently returned "200 []"; only writes errored. Hence: found nodes must be
// reconciled, never blindly returned.
//
// Note that `status` alone was NOT what broke reads - permission_service never
// inspects `status`. The stale `user_id` was the whole defect. We still restore
// `status` to "active" because it is the flag admin tooling and the cascade
// itself reason about, and leaving a live user marked "deleted" would make the
// next cascade/report lie.

/// The facts about a found node that the rebind decision depends on.
///
/// Kept as plain `Option<&str>` so the decision is a pure function that can be
/// unit-tested without standing up storage.
#[derive(Debug, Default, Clone, Copy)]
#[cfg_attr(not(feature = "storage-rocksdb"), allow(dead_code))]
struct FoundNodeFacts<'a> {
    /// `user_id` property - the identity_id the node is currently bound to.
    existing_user_id: Option<&'a str>,
    /// `status` property - "active", "deleted", or absent on older nodes.
    existing_status: Option<&'a str>,
    /// `email` property - absent on some hand-created nodes.
    existing_email: Option<&'a str>,
}

/// What `ensure_user_node` should do with the node it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "storage-rocksdb"), allow(dead_code))]
enum ReconcileAction {
    /// Already active and already bound to this identity - the only case where
    /// returning the node untouched is correct.
    LeaveAsIs,
    /// Bind the node to this identity (and reactivate it).
    Rebind(RebindReason),
    /// The node demonstrably belongs to a different email. Do not touch it and
    /// do not hand its path back as this user's home.
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "storage-rocksdb"), allow(dead_code))]
enum RebindReason {
    /// Same identity, node was not "active" (deleted, or some other status).
    Reactivate,
    /// Node has no `user_id` at all - an orphan from an older provisioning path.
    ClaimOrphan,
    /// Node is bound to a *different* identity_id but is marked "deleted" -
    /// the delete-then-recreate-same-email case from the incident.
    ClaimDeleted,
    /// Node is bound to a different identity_id and still claims to be
    /// "active". See the note in `reconcile_action` - this one gets a warn!.
    TakeoverActive,
}

/// Decide what to do with an existing `raisin:User` node.
///
/// Pure on purpose: `ensure_user_node` needs storage, this decision does not,
/// and this is the part worth testing exhaustively.
///
/// # Why we rebind rather than refuse
///
/// Every lookup in `ensure_user_node` is keyed by something the caller has
/// already authenticated: the email-derived path, the `email` property, or the
/// `user_id` property. A node reached by `user_id` is this identity's by
/// construction. A node reached by email is this identity's because email is
/// the unique handle an identity authenticates with in a tenant. So the
/// conservative-looking option ("leave it alone") is actually the dangerous
/// one - it reproduces the incident: a silently permission-less session.
///
/// # The one case we refuse
///
/// `email_to_node_name` is lossy - it maps '.' and '@' onto '-', so
/// `a.b@x.com` and `a-b@x.com` collide on the same path
/// `/users/internal/a-b-at-x-com`. A path hit is therefore NOT proof of
/// identity. If the node carries an `email` property that differs from the
/// authenticated email, it is somebody else's node and we bail out
/// (`Conflict`) instead of stealing it. This can only trigger on the by-path
/// lookup; the by-email lookup matches by construction, and the by-user_id
/// lookup is settled before email is ever consulted.
///
/// # "different but still live" - what we can and cannot tell
///
/// The brief asks us to warn when the node belongs to a *different, still-live*
/// identity. `ensure_user_node` cannot actually determine liveness: it is
/// handed only `RocksDBStorage` and the repo scope, not an `IdentityRepository`
/// or the tenant's identity table, so "identity X still exists" is not a
/// question it can ask. The node's own `status` is the only signal available,
/// and it is a lagging one (the cascade at identity_users.rs:428-470 logs and
/// continues on failure, so a "deleted" identity can leave an "active" node
/// behind). We therefore split the difference:
///   - different user_id + status "deleted"  -> `ClaimDeleted`, info!.
///     Unambiguous: this is exactly the incident shape.
///   - different user_id + status "active"   -> `TakeoverActive`, warn!.
///     We still rebind (email uniqueness says it is the same human, and
///     refusing means a broken session), but it is logged loudly because it
///     means either the cascade failed or two identities share an email.
fn reconcile_action(
    facts: &FoundNodeFacts<'_>,
    identity_id: &str,
    email: &str,
) -> ReconcileAction {
    // 1. Already bound to this identity: the only question left is status.
    //    Email is deliberately not consulted here - a user who changed their
    //    email address still owns this node.
    if facts.existing_user_id == Some(identity_id) {
        return match facts.existing_status {
            Some("active") => ReconcileAction::LeaveAsIs,
            // Reactivate covers "deleted" AND "inactive". That is only safe
            // because every login path rejects a deactivated identity BEFORE
            // reaching ensure_user_node (local.rs:153, session.rs:152,
            // magic_link.rs:674, oauth_as/authorize.rs:298 all check
            // `identity.is_active`). This property is load-bearing: a future
            // login path that forgets that check would let a deactivated user
            // silently undo an admin's deactivation, since
            // `cascade_user_status` records it as "inactive"
            // (identity_users.rs:352-358) and we would flip it back to
            // "active" here.
            _ => ReconcileAction::Rebind(RebindReason::Reactivate),
        };
    }

    // 2. Not bound to us. Guard the lossy-slug collision described above.
    match facts.existing_email {
        // Demonstrably somebody else's node.
        Some(existing_email) if !existing_email.eq_ignore_ascii_case(email) => {
            return ReconcileAction::Conflict;
        }
        // Bound to a different identity AND carrying no email at all: there is
        // no evidence this is the same human. `email_to_node_name` is lossy, so
        // a bare path hit proves nothing on its own - the email property is the
        // only thing that ties a node to an address, and it is missing.
        // Rebinding here would let whoever registers a colliding address take
        // over the node and inherit its roles. Be strictly LESS permissive when
        // we know less, not more.
        //
        // `raisin_user.yaml` marks `email` required and our own create path
        // always writes it, but `update_property_by_path_impl` bypasses
        // nodetype validation, so imported / replicated / hand-created nodes
        // can lack it - and those are exactly the ones carrying hand-granted
        // roles.
        None if facts.existing_user_id.is_some() => return ReconcileAction::Conflict,
        _ => {}
    }

    // 3. Same email (or an unbound node with no email), different or absent
    //    user_id.
    match (facts.existing_user_id, facts.existing_status) {
        (None, _) => ReconcileAction::Rebind(RebindReason::ClaimOrphan),
        // An ABSENT status is an ACTIVE node, not a deleted one:
        // `raisin-core/global_nodetypes/raisin_user.yaml:22-26` declares
        // `status` as `required: false, default: active`. Nodes predating the
        // property, or created through the generic node API / replication,
        // have no status and are live. Treat them as the live takeover they
        // are so the operator gets the warn!.
        (Some(_), Some("active") | None) => ReconcileAction::Rebind(RebindReason::TakeoverActive),
        (Some(_), Some(_)) => ReconcileAction::Rebind(RebindReason::ClaimDeleted),
    }
}

/// Read a string property off a node, if present and actually a string.
#[cfg_attr(not(feature = "storage-rocksdb"), allow(dead_code))]
fn prop_str<'a>(
    node: &'a raisin_models::nodes::Node,
    key: &str,
) -> Option<&'a str> {
    match node.properties.get(key) {
        Some(raisin_models::nodes::properties::PropertyValue::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// Union `default_roles` into an existing roles array.
///
/// Returns `None` when nothing would change, so the caller can skip the write.
///
/// The existing entries are carried over *verbatim* rather than re-serialized
/// from strings. The schema says they should all BE strings
/// (`raisin-core/global_nodetypes/raisin_user.yaml` declares `roles` as an
/// Array of String), but `update_property_by_path_impl` bypasses nodetype
/// validation, so nothing structurally guarantees it. Copying whatever is there
/// cannot drop a grant; rebuilding the array from the string entries alone
/// could. Dedupe correspondingly only compares string entries, so a
/// hypothetical non-string duplicate would survive - a duplicate grant, never a
/// dropped one, and role resolution treats the array as a set.
///
/// Roles are only ever ADDED here. The incident node carried
/// ["studio_admin","studio_viewer"] and those must survive a rebind; a login
/// must never downgrade a user to the default role set.
#[cfg_attr(not(feature = "storage-rocksdb"), allow(dead_code))]
fn union_roles(
    existing: &[raisin_models::nodes::properties::PropertyValue],
    default_roles: &[String],
) -> Option<Vec<raisin_models::nodes::properties::PropertyValue>> {
    use raisin_models::nodes::properties::PropertyValue;

    let held: std::collections::HashSet<&str> = existing
        .iter()
        .filter_map(|v| match v {
            PropertyValue::String(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();

    let missing: Vec<&String> = default_roles
        .iter()
        .filter(|role| !held.contains(role.as_str()))
        .collect();

    if missing.is_empty() {
        return None;
    }

    let mut merged = existing.to_vec();
    merged.extend(
        missing
            .into_iter()
            .map(|role| PropertyValue::String(role.clone())),
    );
    Some(merged)
}

/// Apply `reconcile_action` to a node we just found, returning its home path.
///
/// `Ok(None)` means the node is NOT this identity's (an `email_to_node_name`
/// slug collision) and the caller should keep looking / create its own node.
///
/// Returns the node's ACTUAL path, not the recomputed `user_path`: the by-email
/// and by-user_id lookups can land on a node that does not sit under the
/// email-derived name (renamed email, hand-placed node), and that path is what
/// goes into the JWT "home" claim.
#[cfg(feature = "storage-rocksdb")]
#[allow(clippy::too_many_arguments)]
async fn reconcile_user_node(
    node_service: &raisin_core::services::node_service::NodeService<raisin_rocksdb::RocksDBStorage>,
    existing: raisin_models::nodes::Node,
    identity_id: &str,
    email: &str,
    display_name: Option<&str>,
    default_roles: &[String],
    found_by: &str,
) -> Result<Option<String>, String> {
    use raisin_models::nodes::properties::PropertyValue;

    // Cloned up front so the immutable borrows below don't fight the move.
    let path = existing.path.clone();
    let node_id = existing.id.clone();

    let facts = FoundNodeFacts {
        existing_user_id: prop_str(&existing, "user_id"),
        existing_status: prop_str(&existing, "status"),
        existing_email: prop_str(&existing, "email"),
    };

    let action = reconcile_action(&facts, identity_id, email);

    match action {
        ReconcileAction::LeaveAsIs => {
            tracing::debug!(
                user_node_id = %node_id,
                user_path = %path,
                found_by = found_by,
                "User node already active and bound to this identity"
            );
            return Ok(Some(path));
        }
        ReconcileAction::Conflict => {
            // Only reachable from the by-path lookup, via an email-slug
            // collision (see reconcile_action). Handing this path back would
            // give one user another user's home; failing is the safe answer.
            // Callers (identity_auth/local.rs:276, :371, magic_link.rs:712)
            // log-and-continue with home = None, so login still succeeds - it
            // just has no home claim, which is loud in a way "200 []" was not.
            tracing::error!(
                user_node_id = %node_id,
                user_path = %path,
                found_by = found_by,
                node_email = facts.existing_email.unwrap_or("<none>"),
                "User node at this path belongs to a different email \
                 (email_to_node_name slug collision) - refusing to rebind"
            );
            // NOT an error: returning Err here would leave this user with no
            // ACL node at all, which is the very outage this module exists to
            // prevent (no node -> no roles -> RLS filters every row -> a
            // silent "200 []"). Ok(None) tells ensure_user_node "this path is
            // not yours, keep looking / create your own".
            return Ok(None);
        }
        ReconcileAction::Rebind(reason) => {
            if reason == RebindReason::TakeoverActive {
                tracing::warn!(
                    user_node_id = %node_id,
                    user_path = %path,
                    found_by = found_by,
                    previous_user_id = facts.existing_user_id.unwrap_or("<none>"),
                    new_user_id = identity_id,
                    "User node with the same email is bound to a DIFFERENT identity \
                     and still marked active - rebinding anyway. Either the delete \
                     cascade failed or two identities share this email; \
                     ensure_user_node cannot tell which (it has no identity repo \
                     in scope). Investigate the previous_user_id."
                );
            }
        }
    }

    // --- rebind ------------------------------------------------------------
    //
    // Property-at-a-time because that is the only update primitive available
    // here (NodeService::update_property_by_path). A failure part-way through
    // leaves a partially rebound node, which is why every failure is logged at
    // error! and the whole call returns Err rather than reporting success.
    let mut changed: Vec<&str> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    let mut writes: Vec<(&'static str, PropertyValue)> = Vec::new();

    if facts.existing_user_id != Some(identity_id) {
        writes.push((
            "user_id",
            PropertyValue::String(identity_id.to_string()),
        ));
    }
    if facts.existing_status != Some("active") {
        writes.push(("status", PropertyValue::String("active".to_string())));
    }
    // Only refresh display_name when the identity actually supplies one - a
    // None here means "no opinion", not "clear it". Never overwrite a name the
    // user set in the repo with an email-derived placeholder.
    if let Some(name) = display_name {
        if prop_str(&existing, "display_name") != Some(name) {
            writes.push((
                "display_name",
                PropertyValue::String(name.to_string()),
            ));
        }
    }
    // Email can drift, so refresh it as part of a rebind. Note this only ever
    // runs on a node we are already rebinding: an active node reached by
    // user_id short-circuits at LeaveAsIs above and is never rewritten.
    if facts.existing_email != Some(email) {
        writes.push(("email", PropertyValue::String(email.to_string())));
    }

    let existing_roles: Vec<PropertyValue> = match existing.properties.get("roles") {
        Some(PropertyValue::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    if let Some(merged) = union_roles(&existing_roles, default_roles) {
        writes.push(("roles", PropertyValue::Array(merged)));
    }

    for (field, value) in writes {
        match node_service.update_property_by_path(&path, field, value).await {
            Ok(()) => changed.push(field),
            Err(e) => {
                tracing::error!(
                    user_node_id = %node_id,
                    user_path = %path,
                    field = field,
                    error = %e,
                    "Failed to rebind property on existing user node"
                );
                failures.push(format!("{}: {}", field, e));
            }
        }
    }

    if !failures.is_empty() {
        return Err(format!(
            "failed to rebind user node {} to identity {}: {}",
            path,
            identity_id,
            failures.join("; ")
        ));
    }

    tracing::info!(
        user_node_id = %node_id,
        user_path = %path,
        found_by = found_by,
        reason = reason_label(action),
        identity_id = identity_id,
        previous_user_id = facts.existing_user_id.unwrap_or("<none>"),
        previous_status = facts.existing_status.unwrap_or("<none>"),
        changed = %changed.join(","),
        "Rebound existing user node to this identity"
    );

    Ok(Some(path))
}

/// Stable log label for a reconcile outcome.
#[cfg_attr(not(feature = "storage-rocksdb"), allow(dead_code))]
fn reason_label(action: ReconcileAction) -> &'static str {
    match action {
        ReconcileAction::LeaveAsIs => "leave_as_is",
        ReconcileAction::Conflict => "conflict",
        ReconcileAction::Rebind(RebindReason::Reactivate) => "reactivate",
        ReconcileAction::Rebind(RebindReason::ClaimOrphan) => "claim_orphan",
        ReconcileAction::Rebind(RebindReason::ClaimDeleted) => "claim_deleted",
        ReconcileAction::Rebind(RebindReason::TakeoverActive) => "takeover_active",
    }
}

/// Create or reconcile the user node inline, returning the home path.
///
/// This creates the `raisin:User` node in the `raisin:access_control` workspace
/// synchronously during registration/login, so the home path is immediately
/// available in the JWT token.
///
/// When a node already exists it is RECONCILED, not returned as-is: see
/// `reconcile_action` for why, and for the delete/recreate incident that made
/// the previous early-return behaviour a silent permission outage.
#[cfg(feature = "storage-rocksdb")]
pub async fn ensure_user_node(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    identity_id: &str,
    email: &str,
    display_name: Option<&str>,
    default_roles: &[String],
) -> Result<String, String> {
    use raisin_core::services::node_service::NodeService;
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::nodes::Node;
    use raisin_storage::{NodeRepository, RepositoryManagementRepository, Storage, StorageScope};
    use std::collections::HashMap;

    let workspace = "raisin:access_control";
    let users_path = "/users/internal";
    let node_name = email_to_node_name(email);
    let user_path = format!("{}/{}", users_path, node_name);

    // The ACL workspace is pinned to its own branch - see ACCESS_CONTROL_BRANCH.
    let default_branch = ACCESS_CONTROL_BRANCH.to_string();

    // Use NodeService with system auth context (bypasses RLS)
    let node_service: NodeService<raisin_rocksdb::RocksDBStorage> = NodeService::new_with_context(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        default_branch.clone(),
        workspace.to_string(),
    )
    .with_auth(AuthContext::system());

    // Check if user node already exists at expected path.
    //
    // A hit here is NOT proof of identity (`email_to_node_name` is lossy), so
    // reconcile may hand back None meaning "that node is someone else's".
    // In that case we keep looking, and if nothing else matches we create our
    // own node under a de-collided name - never bail out, because a user with
    // no ACL node reads zero rows from everywhere, silently.
    let mut slug_collision = false;
    if let Some(existing_node) = node_service
        .get_by_path(&user_path)
        .await
        .map_err(|e| e.to_string())?
    {
        match reconcile_user_node(
            &node_service,
            existing_node,
            identity_id,
            email,
            display_name,
            default_roles,
            "path",
        )
        .await?
        {
            Some(path) => return Ok(path),
            None => slug_collision = true,
        }
    }

    // Path check failed - search by email property (node might be at different path)
    let email_value = PropertyValue::String(email.to_string());
    let nodes_by_email = Storage::nodes(storage.as_ref())
        .find_by_property(
            StorageScope::new(tenant_id, repo_id, &default_branch, workspace),
            "email",
            &email_value,
        )
        .await
        .map_err(|e| e.to_string())?;

    if let Some(existing_node) = nodes_by_email
        .into_iter()
        .find(|n| n.node_type == "raisin:User")
    {
        // A by-email hit is unambiguous, so None cannot occur here; treat it
        // defensively rather than unwrapping.
        if let Some(path) = reconcile_user_node(
            &node_service,
            existing_node,
            identity_id,
            email,
            display_name,
            default_roles,
            "email",
        )
        .await?
        {
            return Ok(path);
        }
    }

    // Also search by user_id (identity_id) property
    let user_id_value = PropertyValue::String(identity_id.to_string());
    let nodes_by_user_id = Storage::nodes(storage.as_ref())
        .find_by_property(
            StorageScope::new(tenant_id, repo_id, &default_branch, workspace),
            "user_id",
            &user_id_value,
        )
        .await
        .map_err(|e| e.to_string())?;

    if let Some(existing_node) = nodes_by_user_id
        .into_iter()
        .find(|n| n.node_type == "raisin:User")
    {
        // A by-user_id hit is unambiguous, so None cannot occur here; treat it
        // defensively rather than unwrapping.
        if let Some(path) = reconcile_user_node(
            &node_service,
            existing_node,
            identity_id,
            email,
            display_name,
            default_roles,
            "user_id",
        )
        .await?
        {
            return Ok(path);
        }
    }

    // No existing node found - create new one.
    //
    // If the email-derived name is occupied by a different address, create
    // under a de-collided name instead of giving up (see email_slug_suffix).
    let node_name = if slug_collision {
        let disambiguated = format!("{}-{}", node_name, email_slug_suffix(email));
        tracing::warn!(
            taken_path = %user_path,
            new_name = %disambiguated,
            "Email slug collides with a different address; creating the user \
             node under a disambiguated name"
        );
        disambiguated
    } else {
        node_name
    };


    // Build role references (bare role_id strings, not paths)
    let role_refs: Vec<PropertyValue> = default_roles
        .iter()
        .map(|role| PropertyValue::String(role.clone()))
        .collect();

    // Build user properties
    let mut properties = HashMap::new();
    properties.insert(
        "user_id".to_string(),
        PropertyValue::String(identity_id.to_string()),
    );
    properties.insert(
        "email".to_string(),
        PropertyValue::String(email.to_string()),
    );
    properties.insert(
        "display_name".to_string(),
        PropertyValue::String(
            display_name
                .unwrap_or_else(|| email.split('@').next().unwrap_or("User"))
                .to_string(),
        ),
    );
    properties.insert(
        "status".to_string(),
        PropertyValue::String("active".to_string()),
    );
    properties.insert("roles".to_string(), PropertyValue::Array(role_refs));
    properties.insert(
        "created_at".to_string(),
        PropertyValue::String(chrono::Utc::now().to_rfc3339()),
    );

    // Create user node with initial_structure (profile, inbox, outbox children)
    let user_node = Node {
        id: String::new(),
        node_type: "raisin:User".to_string(),
        name: node_name,
        path: String::new(),
        workspace: Some(workspace.to_string()),
        properties,
        ..Default::default()
    };

    let created_node = node_service
        .add_deep_node(users_path, user_node)
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!(
        user_node_id = %created_node.id,
        user_path = %created_node.path,
        "User node created successfully with initial_structure"
    );

    Ok(created_node.path)
}

/// Look up existing user node by email, returning the home path.
#[cfg(feature = "storage-rocksdb")]
pub async fn lookup_user_home(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    email: &str,
) -> Result<Option<String>, String> {
    use raisin_core::services::node_service::NodeService;
    use raisin_models::auth::AuthContext;
    use raisin_storage::{RepositoryManagementRepository, Storage};

    let workspace = "raisin:access_control";
    let node_name = email_to_node_name(email);
    let user_path = format!("/users/internal/{}", node_name);

    // The ACL workspace is pinned to its own branch - see ACCESS_CONTROL_BRANCH.
    let default_branch = ACCESS_CONTROL_BRANCH.to_string();

    let node_service: NodeService<raisin_rocksdb::RocksDBStorage> = NodeService::new_with_context(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        default_branch,
        workspace.to_string(),
    )
    .with_auth(AuthContext::system());

    match node_service.get_by_path(&user_path).await {
        Ok(Some(node)) => Ok(Some(node.path)),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::PropertyValue;

    const ME: &str = "identity-new";
    const OTHER: &str = "identity-old";
    const EMAIL: &str = "ada@aiopenhouse.example";

    fn facts<'a>(
        user_id: Option<&'a str>,
        status: Option<&'a str>,
        email: Option<&'a str>,
    ) -> FoundNodeFacts<'a> {
        FoundNodeFacts {
            existing_user_id: user_id,
            existing_status: status,
            existing_email: email,
        }
    }

    // --- bound to this identity -------------------------------------------

    #[test]
    fn active_and_same_identity_is_left_alone() {
        assert_eq!(
            reconcile_action(&facts(Some(ME), Some("active"), Some(EMAIL)), ME, EMAIL),
            ReconcileAction::LeaveAsIs
        );
    }

    #[test]
    fn deleted_and_same_identity_is_reactivated() {
        assert_eq!(
            reconcile_action(&facts(Some(ME), Some("deleted"), Some(EMAIL)), ME, EMAIL),
            ReconcileAction::Rebind(RebindReason::Reactivate)
        );
    }

    #[test]
    fn missing_status_and_same_identity_is_reactivated() {
        // Older nodes predate the `status` property; treat absent as not-active
        // so the rebind writes it in rather than leaving a node the delete
        // cascade cannot reason about.
        assert_eq!(
            reconcile_action(&facts(Some(ME), None, Some(EMAIL)), ME, EMAIL),
            ReconcileAction::Rebind(RebindReason::Reactivate)
        );
    }

    #[test]
    fn unknown_status_value_and_same_identity_is_reactivated() {
        assert_eq!(
            reconcile_action(&facts(Some(ME), Some("suspended"), Some(EMAIL)), ME, EMAIL),
            ReconcileAction::Rebind(RebindReason::Reactivate)
        );
    }

    #[test]
    fn same_identity_wins_over_changed_email() {
        // Reached by the user_id lookup after the identity changed address:
        // the node is ours, the email property is simply stale.
        assert_eq!(
            reconcile_action(&facts(Some(ME), Some("active"), Some("old@example.com")), ME, EMAIL),
            ReconcileAction::LeaveAsIs
        );
    }

    // --- bound to a different identity ------------------------------------

    /// The incident: identity deleted (cascade set status=deleted, node stayed
    /// at the email path), a new identity created with the same email.
    #[test]
    fn deleted_and_different_identity_is_claimed() {
        assert_eq!(
            reconcile_action(&facts(Some(OTHER), Some("deleted"), Some(EMAIL)), ME, EMAIL),
            ReconcileAction::Rebind(RebindReason::ClaimDeleted)
        );
    }

    #[test]
    fn active_and_different_identity_is_taken_over_loudly() {
        // Rebound (email uniqueness says it is the same human) but warn!-worthy:
        // the delete cascade either failed or never ran.
        assert_eq!(
            reconcile_action(&facts(Some(OTHER), Some("active"), Some(EMAIL)), ME, EMAIL),
            ReconcileAction::Rebind(RebindReason::TakeoverActive)
        );
    }

    /// An ABSENT status is an ACTIVE node - raisin_user.yaml declares `status`
    /// as `required: false, default: active`. This is the case with the least
    /// information, so it must be the loud one, not the quiet one.
    #[test]
    fn missing_status_and_different_identity_is_a_loud_takeover() {
        assert_eq!(
            reconcile_action(&facts(Some(OTHER), None, Some(EMAIL)), ME, EMAIL),
            ReconcileAction::Rebind(RebindReason::TakeoverActive)
        );
    }

    // --- no user_id at all ------------------------------------------------

    #[test]
    fn missing_user_id_is_claimed_as_orphan() {
        assert_eq!(
            reconcile_action(&facts(None, Some("active"), Some(EMAIL)), ME, EMAIL),
            ReconcileAction::Rebind(RebindReason::ClaimOrphan)
        );
    }

    #[test]
    fn missing_user_id_and_missing_status_is_claimed_as_orphan() {
        assert_eq!(
            reconcile_action(&facts(None, None, None), ME, EMAIL),
            ReconcileAction::Rebind(RebindReason::ClaimOrphan)
        );
    }

    // --- email guard ------------------------------------------------------

    #[test]
    fn different_email_is_a_conflict_and_never_stolen() {
        assert_eq!(
            reconcile_action(
                &facts(Some(OTHER), Some("active"), Some("someone.else@example.com")),
                ME,
                EMAIL
            ),
            ReconcileAction::Conflict
        );
        assert_eq!(
            reconcile_action(
                &facts(Some(OTHER), Some("deleted"), Some("someone.else@example.com")),
                ME,
                EMAIL
            ),
            ReconcileAction::Conflict
        );
        assert_eq!(
            reconcile_action(
                &facts(None, None, Some("someone.else@example.com")),
                ME,
                EMAIL
            ),
            ReconcileAction::Conflict
        );
    }

    #[test]
    fn email_comparison_is_case_insensitive() {
        assert_eq!(
            reconcile_action(
                &facts(Some(OTHER), Some("deleted"), Some("Ada@AIOpenHouse.Example")),
                ME,
                EMAIL
            ),
            ReconcileAction::Rebind(RebindReason::ClaimDeleted)
        );
    }

    /// A node bound to a DIFFERENT identity that carries no email at all gives
    /// us nothing tying it to this address, and the path slug is lossy. Be
    /// strictly less permissive when we know less: refuse rather than hand over
    /// whatever roles it holds. ensure_user_node then creates a de-collided
    /// node for this user, so refusing costs them nothing.
    #[test]
    fn missing_email_on_a_foreign_node_is_a_conflict() {
        assert_eq!(
            reconcile_action(&facts(Some(OTHER), Some("deleted"), None), ME, EMAIL),
            ReconcileAction::Conflict
        );
    }

    /// ...but an UNBOUND node with no email is still a safe orphan to claim:
    /// there is no other identity to take it from.
    #[test]
    fn missing_email_on_an_unbound_node_is_still_an_orphan_claim() {
        assert_eq!(
            reconcile_action(&facts(None, Some("deleted"), None), ME, EMAIL),
            ReconcileAction::Rebind(RebindReason::ClaimOrphan)
        );
    }

    /// The disambiguator must be stable across logins and differ per address.
    #[test]
    fn email_slug_suffix_is_deterministic_and_distinguishing() {
        assert_eq!(email_slug_suffix(EMAIL), email_slug_suffix(EMAIL));
        assert_eq!(email_slug_suffix(EMAIL), email_slug_suffix(&EMAIL.to_uppercase()));
        assert_ne!(
            email_slug_suffix("ada.lovelace@example.com"),
            email_slug_suffix("ada-lovelace@example.com")
        );
        assert_eq!(email_slug_suffix(EMAIL).len(), 8);
    }

    /// The reason the email guard exists at all: the path slug is lossy, so a
    /// path hit is not proof of identity.
    #[cfg(feature = "storage-rocksdb")]
    #[test]
    fn email_slug_collides_for_distinct_emails() {
        assert_eq!(
            email_to_node_name("a.b@x.com"),
            email_to_node_name("a-b@x.com")
        );
    }

    // --- role union -------------------------------------------------------

    fn strings(values: &[&str]) -> Vec<PropertyValue> {
        values
            .iter()
            .map(|v| PropertyValue::String((*v).to_string()))
            .collect()
    }

    fn owned(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn union_roles_preserves_roles_the_node_already_had() {
        // The incident node carried ["studio_admin","studio_viewer"]; a login
        // adding the JIT defaults must not downgrade it.
        let existing = strings(&["studio_admin", "studio_viewer"]);
        let merged = union_roles(&existing, &owned(&["viewer", "authenticated_user"]))
            .expect("defaults are missing, so a write is required");
        assert_eq!(
            merged,
            strings(&[
                "studio_admin",
                "studio_viewer",
                "viewer",
                "authenticated_user"
            ])
        );
    }

    #[test]
    fn union_roles_returns_none_when_nothing_changes() {
        let existing = strings(&["viewer", "authenticated_user", "studio_admin"]);
        assert!(union_roles(&existing, &owned(&["viewer", "authenticated_user"])).is_none());
    }

    #[test]
    fn union_roles_on_empty_node_yields_the_defaults() {
        let merged = union_roles(&[], &owned(&["viewer"])).expect("must write");
        assert_eq!(merged, strings(&["viewer"]));
    }

    #[test]
    fn union_roles_carries_non_string_entries_through_verbatim() {
        // Non-string entries are off-schema but not structurally prevented
        // (update_property_by_path bypasses nodetype validation). Copying
        // verbatim cannot drop one; rebuilding from strings could.
        let existing = vec![
            PropertyValue::String("studio_admin".to_string()),
            PropertyValue::Integer(7),
        ];
        let merged = union_roles(&existing, &owned(&["viewer"])).expect("must write");
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0], PropertyValue::String("studio_admin".to_string()));
        assert_eq!(merged[1], PropertyValue::Integer(7));
        assert_eq!(merged[2], PropertyValue::String("viewer".to_string()));
    }

    #[test]
    fn union_roles_with_no_defaults_is_a_no_op() {
        assert!(union_roles(&strings(&["studio_admin"]), &[]).is_none());
    }
}
