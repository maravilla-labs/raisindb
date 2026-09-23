// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Where a run lives: ordinary nodes in the `raisin:system` workspace of the
//! repository's default branch — the workspace flow instances live in.
//!
//! ```text
//! /agent-runs/runs/{b}/{run_id}                 the record          {record}
//! /agent-runs/runs/{b}/{run_id}/ev/{seq:012}    one event           {event}
//! /agent-runs/runs/{b}/{run_id}/ctl/{h(id)}     a control ack       {id, ack}
//! /agent-runs/runs/{b}/{run_id}/idem/{h(key)}   an idempotency mark {key, seq}
//! /agent-runs/runs/{b}/{run_id}/ckpt/{n:010}    a checkpoint        {checkpoint}
//! /agent-runs/runs/{b}/{run_id}/dom/{rev:012}   domain state        {state}   (write-once)
//! /agent-runs/runs/{b}/{run_id}/res/{h(key)}    a large result      {key, bytes_b64}
//! /agent-runs/status/{status}/{run_id}          status index
//! /agent-runs/subjects/{h(subject)}             {subject_key, live_run_id}
//! /agent-runs/subjects/{h(subject)}/{run_id}    every run of the subject
//! /agent-runs/create-keys/{h(key)}              {key, run_id}
//! /agent-runs/owed/finalize/{run_id}            finalize-owed worklist
//! /agent-runs/owed/handback/{run_id}            hand-back / waiter-owed worklist
//! ```
//!
//! Every path is derived, so every lookup is a direct path read — never an
//! (eventually consistent) query. Free-form keys (control ids, idempotency
//! keys, subjects) enter a path only hashed; the original is kept in the node.
//! Bodies are JSON strings, so nothing inside a record (a reference envelope in
//! a tool result, say) is ever indexed as a reference FROM the run.

use std::collections::HashMap;

use raisin_agent_runtime::ids::{RunId, RunScope};
use raisin_agent_runtime::state::RunStatus;
use raisin_agent_runtime::store::StoreError;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use sha2::{Digest, Sha256};

/// The workspace runs live in (the flow instances' workspace).
pub const RUN_WORKSPACE: &str = "raisin:system";
/// Node type of every run node.
pub const RUN_NODE_TYPE: &str = "raisin:Node";
/// Type of auto-created folders ABOVE a run record (the workspace root admits
/// only folders).
pub const RUN_FOLDER_TYPE: &str = "raisin:Folder";

/// The type the missing ancestors of `path` are created with. A run record is
/// a `raisin:Node`, which admits only `raisin:Node` children, so a table
/// folder inside a record (`ev`, `ctl`, …) must be one too; everything above a
/// record is a `raisin:Folder`.
pub fn ancestor_type(path: &str) -> &'static str {
    match path.strip_prefix("/agent-runs/runs/") {
        // {bucket}/{run_id}/{table}/{entry}: the table is inside the record.
        Some(rest) if rest.split('/').count() > 3 => RUN_NODE_TYPE,
        _ => RUN_FOLDER_TYPE,
    }
}

/// Path depth, for ordering the puts of one commit parents-first.
pub fn depth(path: &str) -> usize {
    path.split('/').filter(|s| !s.is_empty()).count()
}

const ROOT: &str = "/agent-runs";

/// Hex SHA-256 prefix of a free-form key: a stable, name-safe path segment.
pub fn hashed(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    hex::encode(&digest[..16])
}

/// A run id as a path segment (ids are generated, but a client may name one).
fn segment(run: &RunId) -> Result<String, StoreError> {
    let id = run.as_str();
    let safe = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if safe {
        Ok(id.to_string())
    } else {
        Err(StoreError::Malformed(format!(
            "run id '{id}' is not a valid path segment"
        )))
    }
}

/// The record node.
pub fn record(run: &RunId) -> Result<String, StoreError> {
    let id = segment(run)?;
    let bucket: String = id.chars().take(2).collect();
    Ok(format!("{ROOT}/runs/{bucket}/{id}"))
}

/// A run's sub-table folder (`ev`, `ctl`, `idem`, `ckpt`, `dom`, `res`).
pub fn table(run: &RunId, name: &str) -> Result<String, StoreError> {
    Ok(format!("{}/{name}", record(run)?))
}

/// An event.
pub fn event(run: &RunId, seq: u64) -> Result<String, StoreError> {
    Ok(format!("{}/{seq:012}", table(run, "ev")?))
}

/// A keyed entry (`ctl`, `idem`, `res`).
pub fn keyed(run: &RunId, name: &str, key: &str) -> Result<String, StoreError> {
    Ok(format!("{}/{}", table(run, name)?, hashed(key)))
}

/// A checkpoint.
pub fn checkpoint(run: &RunId, n: u32) -> Result<String, StoreError> {
    Ok(format!("{}/{n:010}", table(run, "ckpt")?))
}

/// A domain state revision.
pub fn domain_state(run: &RunId, rev: u64) -> Result<String, StoreError> {
    Ok(format!("{}/{rev:012}", table(run, "dom")?))
}

/// The status index folder of `status`.
pub fn status_folder(status: RunStatus) -> String {
    format!("{ROOT}/status/{}", status.as_str())
}

/// A status index entry.
pub fn status_entry(status: RunStatus, run: &RunId) -> Result<String, StoreError> {
    Ok(format!("{}/{}", status_folder(status), segment(run)?))
}

/// The subject node (its live run, and every run as a child).
pub fn subject(subject_key: &str) -> String {
    format!("{ROOT}/subjects/{}", hashed(subject_key))
}

/// One run of a subject.
pub fn subject_run(subject_key: &str, run: &RunId) -> Result<String, StoreError> {
    Ok(format!("{}/{}", subject(subject_key), segment(run)?))
}

/// A create key.
pub fn create_key(key: &str) -> String {
    format!("{ROOT}/create-keys/{}", hashed(key))
}

/// The finalize-owed worklist.
pub const FINALIZE: &str = "finalize";
/// The hand-back-owed worklist (a parent's mailbox or a waiting flow).
pub const HANDBACK: &str = "handback";

/// A worklist folder ([`FINALIZE`], [`HANDBACK`]).
pub fn owed_folder(name: &str) -> String {
    format!("{ROOT}/owed/{name}")
}

/// A worklist entry.
pub fn owed(name: &str, run: &RunId) -> Result<String, StoreError> {
    Ok(format!("{}/{}", owed_folder(name), segment(run)?))
}

/// A run node carrying string properties.
pub fn node(scope: &RunScope, path: &str, props: &[(&str, String)]) -> Node {
    let name = path.rsplit('/').next().unwrap_or_default().to_string();
    let mut properties = HashMap::new();
    for (k, v) in props {
        properties.insert((*k).to_string(), PropertyValue::String(v.clone()));
    }
    Node {
        id: nanoid::nanoid!(),
        name,
        path: path.to_string(),
        node_type: RUN_NODE_TYPE.to_string(),
        archetype: None,
        properties,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: Node::extract_parent_name_from_path(path),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        created_by: Some("system".to_string()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: None,
        translations: None,
        tenant_id: Some(scope.tenant_id.clone()),
        workspace: Some(RUN_WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// A string property of a node.
pub fn prop<'a>(node: &'a Node, key: &str) -> Option<&'a str> {
    match node.properties.get(key) {
        Some(PropertyValue::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// A JSON body property of a node.
pub fn body<T: serde::de::DeserializeOwned>(node: &Node, key: &str) -> Result<T, StoreError> {
    let raw =
        prop(node, key).ok_or_else(|| StoreError::Backend(format!("{}: no '{key}'", node.path)))?;
    serde_json::from_str(raw).map_err(|e| StoreError::Backend(format!("{}: {e}", node.path)))
}

/// Serialize a body.
pub fn json<T: serde::Serialize>(v: &T) -> Result<String, StoreError> {
    serde_json::to_string(v).map_err(|e| StoreError::Backend(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_derived_and_name_safe() {
        let run = RunId("0f3c-run_1".into());
        assert_eq!(record(&run).unwrap(), "/agent-runs/runs/0f/0f3c-run_1");
        assert_eq!(
            event(&run, 7).unwrap(),
            "/agent-runs/runs/0f/0f3c-run_1/ev/000000000007"
        );
        assert!(keyed(&run, "ctl", "a/b c\0")
            .unwrap()
            .ends_with(&hashed("a/b c\0")));
        assert!(record(&RunId("../x".into())).is_err());
        assert_eq!(hashed("k").len(), 32);
    }

    #[test]
    fn a_table_inside_a_record_is_a_node_everything_above_is_a_folder() {
        let run = RunId("0f3c-run_1".into());
        assert_eq!(ancestor_type(&event(&run, 1).unwrap()), RUN_NODE_TYPE);
        assert_eq!(
            ancestor_type(&keyed(&run, "ctl", "c").unwrap()),
            RUN_NODE_TYPE
        );
        assert_eq!(ancestor_type(&record(&run).unwrap()), RUN_FOLDER_TYPE);
        assert_eq!(
            ancestor_type(&status_entry(RunStatus::Queued, &run).unwrap()),
            RUN_FOLDER_TYPE
        );
        assert_eq!(
            ancestor_type(&subject_run("s", &run).unwrap()),
            RUN_FOLDER_TYPE
        );
    }
}
