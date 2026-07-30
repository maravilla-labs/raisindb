//! The reporting half of the spatial admin surface: `SHOW … CONFIG`,
//! `SHOW … HEALTH` and `VERIFY`.
//!
//! # The distinction these three exist to make visible
//!
//! `CONFIG` reports **intent** — the replicated `WorkspaceConfig.spatial`, which is
//! what every node agrees the policy *should* be. `HEALTH` and `VERIFY` report
//! **reality on this node** — the local state record and a physical count of the
//! index keys. Collapsing the two would recreate the exact bug this subsystem was
//! repaired for: a planner trusting an index nobody had checked.

use super::QueryEngine;
use crate::physical_plan::executor::{Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::{
    policy_key_for_path, resolve_spatial_policy, PropertyValue, SpatialPolicy,
    SPATIAL_NORMALIZER_VERSION,
};
use raisin_storage::scope::RepoScope;
use raisin_storage::spatial::{SpatialBuildPhase, SpatialIndexState};
use raisin_storage::spatial_admin::SpatialEntryCensus;
use raisin_storage::{Storage, WorkspaceRepository};

/// Render a precision list the way the DDL accepts it back.
pub(super) fn format_precisions(precisions: &[usize]) -> String {
    let list: Vec<String> = precisions.iter().map(|p| p.to_string()).collect();
    format!("({})", list.join(", "))
}

fn text(row: &mut Row, key: &str, value: impl Into<String>) {
    row.insert(key.to_string(), PropertyValue::String(value.into()));
}

/// Counts go out as `Integer`, not `Float`: an f64 cannot hold every u64 exactly and
/// a rounded count is a wrong count.
fn number(row: &mut Row, key: &str, value: u64) {
    row.insert(key.to_string(), PropertyValue::Integer(value as i64));
}

/// A `policy_hash` goes out as TEXT.
///
/// It is a full 64-bit fingerprint and `PropertyValue::Integer` is `i64`, so half the
/// hash space would come back negative — the same value would then read differently
/// from the HTTP surface (which serializes `u64`) and from SQL, and an operator
/// comparing hashes across nodes is exactly who this field exists for.
fn hash(row: &mut Row, key: &str, value: u64) {
    row.insert(key.to_string(), PropertyValue::String(value.to_string()));
}

fn rows(rows: Vec<Row>) -> Result<RowStream, Error> {
    let results: Vec<Result<Row, Error>> = rows.into_iter().map(Ok).collect();
    Ok(Box::pin(stream::iter(results)))
}

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Workspaces to report on: the one named, or every workspace in the repo.
    async fn target_workspaces(&self, workspace: Option<&str>) -> Result<Vec<String>, Error> {
        match workspace {
            Some(ws) => Ok(vec![ws.to_string()]),
            None => {
                let scope = RepoScope::new(&self.tenant_id, &self.repo_id);
                Ok(self
                    .storage
                    .workspaces()
                    .list(scope)
                    .await?
                    .into_iter()
                    .map(|w| w.name)
                    .collect())
            }
        }
    }

    /// `SHOW SPATIAL INDEX CONFIG` — declared, replicated intent.
    ///
    /// Reports the *resolved* policy alongside the scope that produced it, because
    /// the precedence chain is the part operators get wrong: a per-property override
    /// silently beats a workspace default, and both beat the server constants.
    pub(super) async fn spatial_show_config(
        &self,
        workspace: Option<&str>,
        property: Option<&str>,
    ) -> Result<RowStream, Error> {
        let scope = RepoScope::new(&self.tenant_id, &self.repo_id);
        let mut out = Vec::new();

        for ws_name in self.target_workspaces(workspace).await? {
            let ws = match self.storage.workspaces().get(scope, &ws_name).await? {
                Some(ws) => ws,
                None => continue,
            };
            let schema = ws.config.spatial.clone();

            // Which property scopes to report: the one asked for, else the workspace
            // default plus every declared override.
            let mut scopes: Vec<Option<String>> = Vec::new();
            match property {
                Some(p) => scopes.push(Some(p.to_string())),
                None => {
                    scopes.push(None);
                    if let Some(s) = &schema {
                        let mut names: Vec<String> = s.properties.keys().cloned().collect();
                        names.sort();
                        scopes.extend(names.into_iter().map(Some));
                    }
                }
            }

            for prop in scopes {
                let resolved =
                    resolve_spatial_policy(None, schema.as_ref(), prop.as_deref().unwrap_or(""));
                let declared_at = declaration_source(schema.as_ref(), prop.as_deref());

                let mut row = Row::new();
                text(&mut row, "workspace", ws_name.clone());
                text(
                    &mut row,
                    "property",
                    prop.clone().unwrap_or_else(|| "*".into()),
                );
                text(
                    &mut row,
                    "precisions",
                    format_precisions(&resolved.precisions),
                );
                text(
                    &mut row,
                    "srid",
                    resolved
                        .srid
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "inherit (4326)".into()),
                );
                text(
                    &mut row,
                    "bucket_property",
                    resolved.bucket_property.clone().unwrap_or_default(),
                );
                text(&mut row, "cover", format!("{:?}", resolved.cover));
                text(&mut row, "declared_at", declared_at);
                hash(&mut row, "policy_hash", resolved.policy_hash());
                out.push(row);
            }
        }

        rows(out)
    }

    /// `SHOW SPATIAL INDEX HEALTH` — local reality, plus the drift against intent.
    pub(super) async fn spatial_show_health(
        &self,
        workspace: Option<&str>,
        property: Option<&str>,
    ) -> Result<RowStream, Error> {
        let admin = self.spatial_admin_handle()?;
        let branch = self.effective_branch().await;
        let scope = RepoScope::new(&self.tenant_id, &self.repo_id);
        let mut out = Vec::new();

        for ws_name in self.target_workspaces(workspace).await? {
            let schema = self
                .storage
                .workspaces()
                .get(scope, &ws_name)
                .await?
                .and_then(|w| w.config.spatial);

            let states = admin.states(&self.tenant_id, &self.repo_id, &branch, &ws_name)?;
            let census =
                admin.census(&self.tenant_id, &self.repo_id, &branch, &ws_name, property)?;

            // Union of "has a state record" and "has keys on disk". A property in
            // only one of the two is the interesting case: state without keys means a
            // rebuild is outstanding, keys without state means the planner is
            // (correctly) refusing to use an index that is physically there.
            let mut names: Vec<String> = states.iter().map(|(n, _)| n.clone()).collect();
            for c in &census {
                if !names.contains(&c.property) {
                    names.push(c.property.clone());
                }
            }
            names.retain(|n| property.is_none_or(|want| want == n));
            names.sort();

            for name in names {
                let state = state_for(&states, &name);
                let counts = census.iter().find(|c| c.property == name);
                let configured = resolve_spatial_policy(None, schema.as_ref(), &name);
                out.push(health_row(&ws_name, &name, state, counts, &configured));
            }
        }

        rows(out)
    }

    /// `VERIFY SPATIAL INDEX` — the same facts as HEALTH, reduced to a verdict.
    ///
    /// Deliberately a separate statement rather than a column on HEALTH: an operator
    /// wants one line per property that says OK or does not, and a verdict is a
    /// judgement the server should make once rather than leave every caller to
    /// re-derive from six columns.
    pub(super) async fn spatial_verify(
        &self,
        workspace: &str,
        property: Option<&str>,
    ) -> Result<RowStream, Error> {
        let admin = self.spatial_admin_handle()?;
        let branch = self.effective_branch().await;
        let scope = RepoScope::new(&self.tenant_id, &self.repo_id);

        let schema = self
            .storage
            .workspaces()
            .get(scope, workspace)
            .await?
            .and_then(|w| w.config.spatial);

        let states = admin.states(&self.tenant_id, &self.repo_id, &branch, workspace)?;
        let census = admin.census(&self.tenant_id, &self.repo_id, &branch, workspace, property)?;

        let mut out = Vec::new();
        for counts in &census {
            if property.is_some_and(|want| want != counts.property) {
                continue;
            }
            let state = state_for(&states, &counts.property);
            let configured = resolve_spatial_policy(None, schema.as_ref(), &counts.property);

            let mut problems: Vec<String> = Vec::new();
            match state {
                None => problems
                    .push("no local index state record (planner will not use the index)".into()),
                Some(s) => {
                    if s.policy_hash != configured.policy_hash() {
                        problems.push(format!(
                            "policy drift: built under hash {} but configured hash is {}",
                            s.policy_hash,
                            configured.policy_hash()
                        ));
                    }
                    if s.phase == SpatialBuildPhase::NotBuilt {
                        problems.push("phase is not_built".into());
                    }
                }
            }
            if counts.legacy_entries > 0 {
                problems.push(format!(
                    "{} entries still carry the legacy value format",
                    counts.legacy_entries
                ));
            }
            if counts.policy_hashes.len() > 1 {
                problems.push(format!(
                    "entries exist under {} different policy hashes",
                    counts.policy_hashes.len()
                ));
            }
            let indexed: Vec<usize> = counts.live_per_precision.iter().map(|(p, _)| *p).collect();
            let mut expected = configured.precisions.clone();
            expected.sort_unstable();
            if counts.live_entries > 0 && indexed != expected {
                problems.push(format!(
                    "entries exist at precisions {:?} but the configured set is {:?}",
                    indexed, expected
                ));
            }

            let mut row = Row::new();
            text(&mut row, "workspace", workspace.to_string());
            text(&mut row, "property", counts.property.clone());
            text(
                &mut row,
                "status",
                if problems.is_empty() {
                    "OK"
                } else {
                    "NEEDS_REBUILD"
                },
            );
            text(&mut row, "detail", problems.join("; "));
            number(&mut row, "live_entries", counts.live_entries);
            number(&mut row, "distinct_nodes", counts.distinct_nodes);
            number(
                &mut row,
                "normalizer_version",
                SPATIAL_NORMALIZER_VERSION as u64,
            );
            out.push(row);
        }

        if out.is_empty() {
            let mut row = Row::new();
            text(&mut row, "workspace", workspace.to_string());
            text(&mut row, "property", property.unwrap_or("*").to_string());
            text(&mut row, "status", "EMPTY");
            text(
                &mut row,
                "detail",
                "no spatial index entries and no index state in this workspace",
            );
            out.push(row);
        }

        rows(out)
    }
}

/// Which scope declared the policy that won.
fn declaration_source(
    schema: Option<&raisin_models::nodes::properties::SpatialWorkspaceSchema>,
    property: Option<&str>,
) -> String {
    let Some(schema) = schema else {
        return "server default".to_string();
    };
    if let Some(name) = property {
        if schema.properties.contains_key(name) {
            return format!("workspace property override '{}'", name);
        }
    }
    if schema.default != Default::default() {
        return "workspace default".to_string();
    }
    "server default".to_string()
}

/// The index-state record for one property PATH.
///
/// State records are keyed by the POLICY key, in which array indices collapse to
/// `[]` (`stops.3.geo` → `stops[].geo`, see `policy_key_for_path`) — one
/// declaration for a modelled field rather than one per element. The physical
/// census, by contrast, is keyed by the CONCRETE path, because that is what the
/// index key embeds.
///
/// So a straight name comparison finds nothing for any array element: HEALTH and
/// VERIFY reported `not_built` / "no local index state record" for `stops.0.geo`
/// while the planner — which normalises — was happily using the index. An admin
/// surface that contradicts the planner is worse than no admin surface, hence the
/// normalised lookup here.
fn state_for<'a>(
    states: &'a [(String, SpatialIndexState)],
    property: &str,
) -> Option<&'a SpatialIndexState> {
    let policy_key = policy_key_for_path(property);
    states
        .iter()
        .find(|(name, _)| name == property || *name == policy_key)
        .map(|(_, state)| state)
}

/// One HEALTH row, merging local state with the physical census.
fn health_row(
    workspace: &str,
    property: &str,
    state: Option<&SpatialIndexState>,
    counts: Option<&SpatialEntryCensus>,
    configured: &SpatialPolicy,
) -> Row {
    let mut row = Row::new();
    text(&mut row, "workspace", workspace.to_string());
    text(&mut row, "property", property.to_string());

    match state {
        Some(s) => {
            text(&mut row, "phase", format!("{:?}", s.phase).to_lowercase());
            text(
                &mut row,
                "indexed_precisions",
                format_precisions(&s.precisions),
            );
            hash(&mut row, "indexed_policy_hash", s.policy_hash);
            text(&mut row, "built_through", s.built_through.to_string());
            number(&mut row, "nodes_indexed", s.nodes_indexed);
        }
        None => {
            // No record means the planner reports `NotBuilt` and keeps the spatial
            // predicate as a row-level filter: correct results, full scan.
            text(&mut row, "phase", "not_built (no state record)");
            text(&mut row, "indexed_precisions", "()");
            hash(&mut row, "indexed_policy_hash", 0);
            text(&mut row, "built_through", "");
            number(&mut row, "nodes_indexed", 0);
        }
    }

    text(
        &mut row,
        "configured_precisions",
        format_precisions(&configured.precisions),
    );
    hash(&mut row, "configured_policy_hash", configured.policy_hash());
    row.insert(
        "needs_rebuild".to_string(),
        PropertyValue::Boolean(match state {
            None => true,
            Some(s) => {
                s.policy_hash != configured.policy_hash() || s.phase == SpatialBuildPhase::NotBuilt
            }
        }),
    );

    let counts = counts.cloned().unwrap_or_default();
    number(&mut row, "live_entries", counts.live_entries);
    number(&mut row, "tombstoned_entries", counts.tombstoned_entries);
    number(&mut row, "distinct_nodes", counts.distinct_nodes);
    number(&mut row, "legacy_entries", counts.legacy_entries);
    text(
        &mut row,
        "live_per_precision",
        counts
            .live_per_precision
            .iter()
            .map(|(p, n)| format!("{}:{}", p, n))
            .collect::<Vec<_>>()
            .join(", "),
    );
    number(
        &mut row,
        "normalizer_version",
        SPATIAL_NORMALIZER_VERSION as u64,
    );
    row
}
