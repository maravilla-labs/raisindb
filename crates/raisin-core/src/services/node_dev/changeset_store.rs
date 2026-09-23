// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Where a changeset record lives: an ordinary `raisin:Node` in the
//! `raisin:system` workspace of the changeset's own branch, at
//! `/changesets/<first two id chars>/<id>`, holding the record as a JSON
//! string (a string, so reference envelopes inside proposed properties are
//! never indexed as references FROM the record).

use std::collections::HashMap;

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{NodeRepository, Storage};
use sha2::{Digest, Sha256};

use super::changeset_types::{ChangesetRecord, ChangesetStatus};
use super::types::{hex, DevResult, NodeDevError};
use super::{DevScope, NodeDevService};

/// The workspace holding changeset records.
pub const RECORD_WORKSPACE: &str = "raisin:system";
/// The node type of a record.
pub const RECORD_NODE_TYPE: &str = "raisin:Node";
/// Folder type for the record buckets.
pub const RECORD_FOLDER_TYPE: &str = "raisin:Folder";

/// A changeset id: derived from the idempotency key (scoped to repository,
/// branch and owner) when one is given, random otherwise.
pub fn changeset_id(scope: &DevScope, owner: &str, key: Option<&str>) -> String {
    match key {
        Some(k) => {
            let mut h = Sha256::new();
            for part in [&scope.tenant, &scope.repo, &scope.branch, owner, k] {
                h.update(part.as_bytes());
                h.update([0x1f]);
            }
            hex(&h.finalize()[..16])
        }
        None => {
            let mut h = Sha256::new();
            h.update(nanoid::nanoid!(32).as_bytes());
            hex(&h.finalize()[..16])
        }
    }
}

/// Validate an id before it becomes a path.
pub fn check_id(id: &str) -> DevResult<()> {
    if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(NodeDevError::invalid(format!(
            "'{id}' is not a changeset id"
        )))
    }
}

/// Record path.
pub fn record_path(id: &str) -> String {
    format!("/changesets/{}/{id}", &id[..2])
}

/// Build the record node.
pub fn record_node(rec: &ChangesetRecord) -> DevResult<Node> {
    let json = serde_json::to_string(rec)
        .map_err(|e| NodeDevError::new(500, "internal", e.to_string()))?;
    let status = serde_json::to_value(rec.status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String(format!("changeset {}", rec.changeset_id)),
    );
    properties.insert("status".to_string(), PropertyValue::String(status));
    properties.insert("record_json".to_string(), PropertyValue::String(json));
    let path = record_path(&rec.changeset_id);
    Ok(Node {
        id: nanoid::nanoid!(),
        name: rec.changeset_id.clone(),
        path,
        node_type: RECORD_NODE_TYPE.to_string(),
        archetype: None,
        properties,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: None,
        version: 1,
        created_at: Some(chrono::Utc::now()),
        created_by: Some(rec.owner.clone()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: None,
        translations: None,
        tenant_id: None,
        workspace: Some(RECORD_WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    })
}

/// Stage a record write in a transaction.
pub async fn stage_record(ctx: &dyn TransactionalContext, rec: &ChangesetRecord) -> DevResult<()> {
    ctx.upsert_deep_node(RECORD_WORKSPACE, &record_node(rec)?, RECORD_FOLDER_TYPE)
        .await?;
    Ok(())
}

/// Open a transaction on `scope` acting as `auth`'s principal with system
/// rights: every op has already been authorized against the caller's own
/// rights by the planner, and the record lives in a system workspace.
pub async fn begin<S: TransactionalStorage>(
    storage: &S,
    scope: &DevScope,
    auth: &AuthContext,
    message: &str,
) -> DevResult<Box<dyn TransactionalContext>> {
    let ctx = storage.begin_context().await?;
    ctx.set_tenant_repo(&scope.tenant, &scope.repo)?;
    ctx.set_branch(&scope.branch)?;
    ctx.set_message(message)?;
    let actor = auth.principal_id().unwrap_or_else(|| auth.actor_id());
    ctx.set_actor(&actor)?;
    let mut sys = AuthContext::system_as(actor);
    sys.agent = auth.agent.clone();
    ctx.set_auth_context(sys)?;
    Ok(ctx)
}

impl<S: Storage + TransactionalStorage> NodeDevService<S> {
    /// Load a record.
    pub(crate) async fn load_record(
        &self,
        scope: &DevScope,
        id: &str,
    ) -> DevResult<Option<ChangesetRecord>> {
        check_id(id)?;
        let node = self
            .storage
            .nodes()
            .get_by_path(scope.storage(RECORD_WORKSPACE), &record_path(id), None)
            .await?;
        let Some(node) = node else { return Ok(None) };
        match node.properties.get("record_json") {
            Some(PropertyValue::String(s)) => serde_json::from_str(s)
                .map(Some)
                .map_err(|e| NodeDevError::new(500, "corrupt_record", e.to_string())),
            _ => Err(NodeDevError::new(
                500,
                "corrupt_record",
                "record has no body",
            )),
        }
    }

    /// Write a record in its own transaction.
    pub(crate) async fn save_record(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        rec: &ChangesetRecord,
    ) -> DevResult<()> {
        let status = match rec.status {
            ChangesetStatus::Proposed => "propose",
            ChangesetStatus::Committed => "receipt",
            ChangesetStatus::Discarded => "discard",
        };
        let ctx = begin(
            self.storage.as_ref(),
            scope,
            auth,
            &format!("changeset {} {status}", rec.changeset_id),
        )
        .await?;
        stage_record(ctx.as_ref(), rec).await?;
        ctx.commit().await?;
        Ok(())
    }
}
