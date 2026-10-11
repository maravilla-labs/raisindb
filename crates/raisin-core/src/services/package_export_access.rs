// SPDX-License-Identifier: BSL-1.1

//! Who may build or download a package export, and what that build may read.
//!
//! `POST /api/packages/{repo}/{branch}/head/raisin:create-from-selection` and
//! `GET …/raisin:download/{job_id}` used to ignore the caller entirely: the
//! create handler took its `AuthContext` as `_auth` and never looked at it, the
//! job then read every selected node straight from storage with no row-level
//! security, and the download served any completed job's blob to anyone who
//! could name the job id — across tenants. An anonymous request could therefore
//! export any workspace of any repository.
//!
//! Three rules close it, and every entry point asks this module rather than
//! re-deriving them:
//!
//! 1. **Building or downloading an export is an operator operation**
//!    ([`authorize_package_export`]): the system context (admin console, CLI,
//!    API keys and the superadmin bearer all resolve to it), an identity user
//!    holding `system_admin`, or an identity user holding the EXPORT GRANT
//!    ([`EXPORT_GRANT_WORKSPACE`] / [`EXPORT_GRANT_PATH`], operation
//!    `create`). The grant must name the `packages` workspace explicitly: a
//!    wildcard workspace pattern (`**`, `*`) never implies it, so read/write
//!    access to everything is still not "may export". The engine stays
//!    app-agnostic — an application gives the grant to whichever of its roles
//!    may export. Anonymous and deny-all callers are `Unauthenticated`; any
//!    other signed-in principal is `Forbidden`.
//! 2. **The build reads as the requester.** The requester's context is
//!    recorded on the job at submit time ([`requester_metadata`]) and rebuilt
//!    by the job ([`requester_from_metadata`]); every node the job collects
//!    then goes through [`rls_filter`](super::rls_filter), so a requester who
//!    cannot read a workspace gets an archive without it. A job that carries no
//!    requester is REFUSED, never run as the system: a lost identity fails
//!    closed.
//! 3. **A download is answered only for a package job of the caller's own
//!    tenant and repository** ([`is_downloadable_job`]); anything else is
//!    indistinguishable from a job that does not exist.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::{PermissionScope, ResolvedPermissions};

use super::rls_filter;

/// `JobContext.metadata` key: the requester's serialized [`AuthContext`].
pub const REQUESTER_AUTH_KEY: &str = "requester_auth";

/// `JobContext.metadata` key: the requester's [`ResolvedPermissions`].
///
/// Stored separately because `AuthContext::resolved_permissions` is
/// `#[serde(skip)]`: a context round-tripped through JSON alone comes back with
/// no permissions, which RLS reads as "deny everything".
pub const REQUESTER_PERMISSIONS_KEY: &str = "requester_permissions";

/// Why an export request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportRefusal {
    /// No usable credential: anonymous, deny-all, or no context at all (401).
    Unauthenticated,
    /// Signed in, but not allowed to export packages (403).
    Forbidden,
}

impl std::fmt::Display for ExportRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unauthenticated => "package exports require signing in",
            Self::Forbidden => "package exports require an administrator",
        })
    }
}

/// May this caller build, or download, a package export?
///
/// Anonymous is checked FIRST, before any admin test, so that no grant attached
/// to the anonymous role can ever make an unauthenticated request an exporter.
pub fn authorize_package_export(auth: Option<&AuthContext>) -> Result<(), ExportRefusal> {
    let Some(auth) = auth else {
        return Err(ExportRefusal::Unauthenticated);
    };
    if auth.is_anonymous_principal() {
        return Err(ExportRefusal::Unauthenticated);
    }
    if auth.is_system || auth.permissions().is_some_and(|p| p.is_system_admin) {
        return Ok(());
    }
    if auth.permissions().is_some_and(holds_export_grant) {
        return Ok(());
    }
    Err(ExportRefusal::Forbidden)
}

/// The workspace the export grant must name, verbatim (no glob).
pub const EXPORT_GRANT_WORKSPACE: &str = "packages";
/// The path the export grant covers; a grant on `/**` covers it too.
pub const EXPORT_GRANT_PATH: &str = "/exports/**";

/// Does a role grant `create` on `packages:/exports/…`, naming the workspace
/// explicitly?
fn holds_export_grant(perms: &ResolvedPermissions) -> bool {
    use raisin_models::permissions::{Operation, PathMatcher};
    perms.permissions.iter().any(|p| {
        p.workspace.as_deref() == Some(EXPORT_GRANT_WORKSPACE)
            && p.allows_operation(Operation::Create)
            && PathMatcher::new(&p.path).matches("/exports/build")
    })
}

/// The metadata entries that record `auth` as a job's requester.
pub fn requester_metadata(auth: &AuthContext) -> Vec<(String, serde_json::Value)> {
    let mut entries = Vec::with_capacity(2);
    if let Ok(ctx) = serde_json::to_value(auth) {
        entries.push((REQUESTER_AUTH_KEY.to_string(), ctx));
    }
    if let Some(perms) = auth.permissions() {
        if let Ok(perms) = serde_json::to_value(perms) {
            entries.push((REQUESTER_PERMISSIONS_KEY.to_string(), perms));
        }
    }
    entries
}

/// Rebuild the requester recorded by [`requester_metadata`].
///
/// `None` when the job carries no (readable) requester. Callers must refuse to
/// run in that case rather than substitute any other identity.
pub fn requester_from_metadata(
    metadata: &HashMap<String, serde_json::Value>,
) -> Option<AuthContext> {
    let mut auth: AuthContext =
        serde_json::from_value(metadata.get(REQUESTER_AUTH_KEY)?.clone()).ok()?;
    let perms = metadata
        .get(REQUESTER_PERMISSIONS_KEY)
        .and_then(|v| serde_json::from_value::<ResolvedPermissions>(v.clone()).ok());
    auth.resolved_permissions = match perms {
        Some(perms) => Some(Arc::new(perms)),
        // The system context carries its permissions implicitly; anyone else
        // without recorded permissions reads nothing (RLS denies on `None`).
        None if auth.is_system => Some(Arc::new(ResolvedPermissions::system_admin())),
        None => None,
    };
    Some(auth)
}

/// `node` as the requester may see it (field filters applied), or `None`.
///
/// Graph (`RELATES … VIA`) conditions are evaluated without a resolver and
/// therefore fail closed: an export never contains a node only a graph
/// condition would have granted.
pub fn readable_by_requester(
    node: Node,
    auth: &AuthContext,
    workspace: &str,
    branch: &str,
) -> Option<Node> {
    rls_filter::filter_node(node, auth, &PermissionScope::new(workspace, branch))
}

/// Key, in a package job's RESULT, naming the repository it exported from.
///
/// The download needs the job's repository AFTER completion, and the job
/// context (which carries `repo_id`) is deleted by the worker when the job
/// finishes; so the job stamps it onto its result with [`stamp_result_repo`].
pub const RESULT_REPO_KEY: &str = "repo_id";

/// Record `repo` on a package job's result object (no-op on a non-object).
pub fn stamp_result_repo(result: &mut serde_json::Value, repo: &str) {
    if let Some(obj) = result.as_object_mut() {
        obj.insert(RESULT_REPO_KEY.to_string(), serde_json::json!(repo));
    }
}

/// The repository a package job's result was stamped with, if any.
pub fn result_repo(result: &serde_json::Value) -> Option<&str> {
    result.get(RESULT_REPO_KEY).and_then(|v| v.as_str())
}

/// Is a job with this tenant, repository and kind one a download on
/// `(tenant, repo)` may serve? `is_package_job` is whether the job is a
/// package export / create-from-selection job.
pub fn is_downloadable_job(
    request_tenant: &str,
    request_repo: &str,
    job_tenant: &str,
    job_repo: Option<&str>,
    is_package_job: bool,
) -> bool {
    is_package_job && job_tenant == request_tenant && job_repo == Some(request_repo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::permissions::{Operation, Permission};

    fn user_with(perms: Vec<Permission>, system_admin: bool) -> AuthContext {
        let mut resolved = ResolvedPermissions::empty("alice");
        resolved.permissions = perms;
        resolved.is_system_admin = system_admin;
        AuthContext::for_user("alice").with_permissions(resolved)
    }

    fn node(path: &str) -> Node {
        Node {
            id: format!("id{}", path.replace('/', "-")),
            name: path.rsplit('/').next().unwrap_or("").to_string(),
            path: path.to_string(),
            node_type: "raisin:Folder".to_string(),
            ..Default::default()
        }
    }

    // ---- the gate ---------------------------------------------------------

    #[test]
    fn no_context_is_unauthenticated() {
        assert_eq!(
            authorize_package_export(None),
            Err(ExportRefusal::Unauthenticated)
        );
    }

    #[test]
    fn anonymous_and_deny_all_are_unauthenticated() {
        assert_eq!(
            authorize_package_export(Some(&AuthContext::anonymous())),
            Err(ExportRefusal::Unauthenticated)
        );
        assert_eq!(
            authorize_package_export(Some(&AuthContext::deny_all())),
            Err(ExportRefusal::Unauthenticated)
        );
        // The physical anonymous user, as the HTTP middleware builds it.
        let anon = AuthContext::anonymous_user("anonymous")
            .with_permissions(ResolvedPermissions::anonymous(vec![]));
        assert_eq!(
            authorize_package_export(Some(&anon)),
            Err(ExportRefusal::Unauthenticated)
        );
    }

    #[test]
    fn anonymous_stays_refused_even_if_its_role_were_system_admin() {
        let mut perms = ResolvedPermissions::anonymous(vec![Permission::full_access("**")]);
        perms.is_system_admin = true;
        let anon = AuthContext::anonymous_user("anonymous").with_permissions(perms);
        assert_eq!(
            authorize_package_export(Some(&anon)),
            Err(ExportRefusal::Unauthenticated)
        );
    }

    #[test]
    fn a_signed_in_non_admin_is_forbidden_whatever_it_may_read() {
        // Full read+write on everything is still not "may export": exporting is
        // an operator action, not a consequence of read access.
        let editor = user_with(vec![Permission::full_access("**")], false);
        assert_eq!(
            authorize_package_export(Some(&editor)),
            Err(ExportRefusal::Forbidden)
        );
        // A user whose permissions were never resolved.
        assert_eq!(
            authorize_package_export(Some(&AuthContext::for_user("bob"))),
            Err(ExportRefusal::Forbidden)
        );
    }

    fn grant(workspace: Option<&str>, path: &str, ops: Vec<Operation>) -> Permission {
        let mut p = Permission::full_access(path);
        p.workspace = workspace.map(str::to_string);
        p.operations = ops;
        p
    }

    #[test]
    fn the_explicit_export_grant_lets_a_person_export() {
        for path in [EXPORT_GRANT_PATH, "/**", "/exports/*"] {
            let exporter = user_with(
                vec![grant(Some("packages"), path, vec![Operation::Create])],
                false,
            );
            assert_eq!(authorize_package_export(Some(&exporter)), Ok(()), "{path}");
        }
        // Read on packages, create elsewhere, a wildcard workspace, another
        // path: none of them is the grant.
        for p in [
            grant(Some("packages"), "/**", vec![Operation::Read]),
            grant(Some("stories"), "/**", vec![Operation::Create]),
            grant(None, "/**", vec![Operation::Create]),
            grant(Some("*"), "/**", vec![Operation::Create]),
            grant(Some("packages"), "/installed/**", vec![Operation::Create]),
        ] {
            let not_exporter = user_with(vec![p.clone()], false);
            assert_eq!(
                authorize_package_export(Some(&not_exporter)),
                Err(ExportRefusal::Forbidden),
                "{p:?}"
            );
        }
        // Anonymous with the grant is still refused, and first.
        let mut resolved = ResolvedPermissions::empty("anonymous");
        resolved.permissions = vec![grant(Some("packages"), "/**", vec![Operation::Create])];
        let anon = AuthContext::anonymous_user("anonymous").with_permissions(resolved);
        assert_eq!(
            authorize_package_export(Some(&anon)),
            Err(ExportRefusal::Unauthenticated)
        );
    }

    #[test]
    fn system_and_system_admin_may_export() {
        assert_eq!(
            authorize_package_export(Some(&AuthContext::system())),
            Ok(())
        );
        assert_eq!(
            authorize_package_export(Some(&user_with(vec![], true))),
            Ok(())
        );
    }

    // ---- requester round trip -------------------------------------------

    fn metadata_for(auth: &AuthContext) -> HashMap<String, serde_json::Value> {
        requester_metadata(auth).into_iter().collect()
    }

    #[test]
    fn a_job_without_a_requester_has_none() {
        assert!(requester_from_metadata(&HashMap::new()).is_none());
        let mut garbage = HashMap::new();
        garbage.insert(REQUESTER_AUTH_KEY.to_string(), serde_json::json!(42));
        assert!(requester_from_metadata(&garbage).is_none());
    }

    #[test]
    fn the_requesters_permissions_survive_the_job_metadata() {
        let alice = user_with(
            vec![Permission::read_only("/**").with_workspace("content")],
            false,
        );
        let back = requester_from_metadata(&metadata_for(&alice)).unwrap();
        assert_eq!(back.user_id.as_deref(), Some("alice"));
        assert!(!back.is_system);
        let perms = back.permissions().expect("permissions rehydrated");
        assert_eq!(perms.permissions.len(), 1);
        assert!(!perms.is_system_admin);
    }

    #[test]
    fn the_system_requester_round_trips_as_the_system() {
        let back = requester_from_metadata(&metadata_for(&AuthContext::system())).unwrap();
        assert!(back.is_system);
        assert!(readable_by_requester(node("/a"), &back, "anything", "main").is_some());
    }

    #[test]
    fn a_requester_whose_permissions_were_lost_reads_nothing() {
        let alice = user_with(vec![Permission::full_access("/**")], false);
        let mut meta = metadata_for(&alice);
        meta.remove(REQUESTER_PERMISSIONS_KEY);
        let back = requester_from_metadata(&meta).unwrap();
        assert!(readable_by_requester(node("/a"), &back, "content", "main").is_none());
    }

    // ---- the read runs under the requester ----------------------------------

    #[test]
    fn a_workspace_the_requester_cannot_read_is_left_out() {
        let alice = user_with(
            vec![Permission::read_only("/**").with_workspace("content")],
            false,
        );
        let alice = requester_from_metadata(&metadata_for(&alice)).unwrap();

        assert!(readable_by_requester(node("/home"), &alice, "content", "main").is_some());
        assert!(readable_by_requester(node("/secret"), &alice, "hr", "main").is_none());
    }

    #[test]
    fn a_subtree_the_requester_cannot_read_is_left_out() {
        let alice = user_with(vec![Permission::read_only("/public/**")], false);
        let alice = requester_from_metadata(&metadata_for(&alice)).unwrap();

        assert!(readable_by_requester(node("/public/a"), &alice, "content", "main").is_some());
        assert!(readable_by_requester(node("/private/a"), &alice, "content", "main").is_none());
    }

    #[test]
    fn write_only_grants_do_not_let_the_requester_read() {
        let alice = user_with(
            vec![Permission::new(
                "/**",
                vec![Operation::Create, Operation::Update],
            )],
            false,
        );
        assert!(readable_by_requester(node("/a"), &alice, "content", "main").is_none());
    }

    // ---- download scoping -------------------------------------------------

    #[test]
    fn downloads_are_scoped_to_tenant_repo_and_package_jobs() {
        assert!(is_downloadable_job("t1", "r1", "t1", Some("r1"), true));
        assert!(!is_downloadable_job("t1", "r1", "t2", Some("r1"), true));
        assert!(!is_downloadable_job("t1", "r1", "t1", Some("r2"), true));
        assert!(!is_downloadable_job("t1", "r1", "t1", None, true));
        assert!(!is_downloadable_job("t1", "r1", "t1", Some("r1"), false));
    }

    #[test]
    fn the_result_carries_its_repository() {
        let mut result = serde_json::json!({ "blob_key": "k" });
        assert_eq!(result_repo(&result), None);
        stamp_result_repo(&mut result, "r1");
        assert_eq!(result_repo(&result), Some("r1"));
        assert_eq!(result["blob_key"], "k");
    }
}
