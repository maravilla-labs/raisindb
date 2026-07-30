//! Resolving the **configured** spatial policy — replicated intent, as opposed to
//! the local index state that [`super::store`] owns.
//!
//! # Why this exists
//!
//! `WorkspaceConfig.spatial` and `PropertyValueSchema.spatial` were added as the
//! declared-intent side of the design, and
//! [`raisin_models::nodes::properties::resolve_spatial_policy`] was written to
//! collapse the precedence chain — but nothing read either of them, so the whole
//! configuration surface was inert: `ALTER SPATIAL INDEX` would have written a
//! record no writer or rebuild ever consulted. This module is the reader.
//!
//! # Why the read is direct rather than through the repository
//!
//! `WorkspaceRepository::get` is `async` but its body is a single synchronous
//! `get_cf`. The spatial write path and the reindex job are both synchronous by
//! construction (the writer must stay callable from the replication apply path,
//! which cannot await), so going through the async repository would force either a
//! block-on or an async writer. Reading the same key directly costs one point
//! lookup and keeps the writer sync.
//!
//! # Scope note
//!
//! Workspace records are keyed `{tenant}\0{repo}\0workspaces\0{name}` — **not**
//! branch-scoped — so a configured policy is per (tenant, repo, workspace,
//! property). The local *state* record is branch-scoped, because the index entries
//! are.

use raisin_models::nodes::properties::SpatialWorkspaceSchema;
use raisin_models::nodes::properties::{resolve_spatial_policy, SpatialPolicy};
use raisin_models::workspace::Workspace;
use rocksdb::DB;

use crate::{cf, cf_handle, keys};

/// Read a workspace's declared spatial schema.
///
/// Returns `None` when the workspace does not exist, carries no spatial block, or
/// cannot be deserialized. Every one of those means "no declared intent", which
/// resolves to the server defaults — never to an error, because a missing
/// configuration must not fail a write.
pub fn workspace_spatial_schema(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
) -> Option<SpatialWorkspaceSchema> {
    let cf = cf_handle(db, cf::WORKSPACES).ok()?;
    let key = keys::workspace_key(tenant_id, repo_id, workspace);
    let bytes = db.get_cf(cf, key).ok()??;
    let ws: Workspace = rmp_serde::from_slice(&bytes).ok()?;
    ws.config.spatial
}

/// The configured policy for one (workspace, property).
///
/// Precedence is applied by the single canonical resolver in `raisin-models`, so
/// this and any future call site cannot disagree. The NodeType layer is passed as
/// `None` here: the only writer of declared intent today is
/// `ALTER SPATIAL INDEX`, which targets the workspace record. When the
/// `SPATIAL_INDEX(...)` NodeType property modifier lands, it plugs in as the
/// highest-precedence argument without changing any caller.
pub fn configured_policy(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
    property: &str,
) -> SpatialPolicy {
    let schema = workspace_spatial_schema(db, tenant_id, repo_id, workspace);
    resolve_spatial_policy(None, schema.as_ref(), property)
}
