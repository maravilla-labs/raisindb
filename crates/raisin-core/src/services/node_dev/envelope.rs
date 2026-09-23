// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The node-development surface speaking `raisin.tool-result/1`, so every
//! operation is directly usable as an agent tool: reads carry `reads[]` with
//! the revision seen, commits carry the exact `writes[]` (moves with their
//! `from`), one primary artifact, and read-back evidence.

use raisin_agent_contract::tool_result::*;
use raisin_agent_contract::TOOL_RESULT_V1;
use serde_json::{json, Value};

use super::changeset_types::*;
use super::types::{NodeDevError, NodeLocator};

fn base(operation_id: &str, status: ToolStatus) -> ToolResultEnvelope {
    ToolResultEnvelope {
        envelope: TOOL_RESULT_V1.to_string(),
        operation_id: operation_id.to_string(),
        status,
        resume_key: None,
        reads: vec![],
        writes: vec![],
        artifact_refs: vec![],
        evidence: vec![],
        diagnostics: vec![],
        suggested_next_actions: vec![],
        retry_policy: None,
        payload: None,
        legacy: false,
    }
}

fn write(l: &NodeLocator, action: WriteAction, from: Option<&NodeLocator>) -> Write {
    Write {
        locator: l.contract(),
        action,
        from: from.map(NodeLocator::contract),
        revision: l.revision.clone(),
    }
}

/// A successful read of `locators`, with `payload` as the domain result.
pub fn read_envelope(
    operation_id: &str,
    locators: &[NodeLocator],
    payload: Value,
) -> ToolResultEnvelope {
    let mut env = base(operation_id, ToolStatus::Succeeded);
    env.reads = locators
        .iter()
        .filter_map(|l| {
            Some(Read {
                read_id: l.node_id.clone(),
                locator: l.contract(),
                revision: l.revision.clone()?,
            })
        })
        .collect();
    env.payload = Some(payload);
    env
}

/// A committed receipt. `primary` names the op whose node is the artifact
/// the call was for; `kind` is the artifact kind reported.
pub fn receipt_envelope(
    operation_id: &str,
    receipt: &Receipt,
    primary: usize,
    kind: &str,
) -> ToolResultEnvelope {
    let mut env = base(operation_id, ToolStatus::Succeeded);
    for op in &receipt.ops {
        let (Some(new), old) = (op.new.as_ref(), op.old.as_ref()) else {
            if let Some(old) = &op.old {
                env.writes.push(write(old, WriteAction::Deleted, None));
            }
            for d in &op.deleted_descendants {
                env.writes.push(write(d, WriteAction::Deleted, None));
            }
            continue;
        };
        match op.action {
            OpAction::Created | OpAction::Copied => {
                env.writes.push(write(new, WriteAction::Created, None))
            }
            OpAction::Updated => env.writes.push(write(new, WriteAction::Updated, None)),
            OpAction::Moved => env.writes.push(write(new, WriteAction::Moved, old)),
            OpAction::Deleted => {}
        }
        for d in &op.created_descendants {
            env.writes.push(write(d, WriteAction::Created, None));
        }
        for m in &op.moved_descendants {
            let from = NodeLocator {
                path: m.from_path.clone(),
                revision: None,
                ..m.to.clone()
            };
            env.writes
                .push(write(&m.to, WriteAction::Moved, Some(&from)));
        }
        if let Some(rev) = &new.revision {
            env.evidence.push(Evidence {
                evidence_id: None,
                kind: "read_back".into(),
                subject: EvidenceSubject {
                    locator: new.contract(),
                    revision: rev.clone(),
                },
                level: None,
                ok: true,
                checks: vec![],
                depends_on: vec![],
            });
        }
    }
    let primary = receipt
        .ops
        .iter()
        .find(|o| o.index == primary)
        .or_else(|| receipt.ops.first());
    for op in &receipt.ops {
        let Some(l) = op.new.as_ref().or(op.old.as_ref()) else {
            continue;
        };
        let is_primary = primary.is_some_and(|p| p.index == op.index);
        env.artifact_refs.push(ArtifactRef {
            logical_key: None,
            kind: kind.to_string(),
            locator: l.contract(),
            revision: l.revision.clone(),
            role: if is_primary {
                ArtifactRole::Primary
            } else {
                ArtifactRole::Supporting
            },
        });
    }
    env.payload = serde_json::to_value(receipt).ok();
    env
}

fn diagnostic(code: &str, message: &str, class: Option<&str>) -> Diagnostic {
    serde_json::from_value(json!({
        "code": code, "severity": "error", "message": message, "class": class,
    }))
    .expect("diagnostic shape")
}

/// A changeset that could not commit: nothing was written. The caller
/// re-reads the conflicting nodes and re-plans.
pub fn conflict_envelope(
    operation_id: &str,
    changeset_id: &str,
    conflicts: &[Conflict],
    digest: &str,
) -> ToolResultEnvelope {
    let mut env = base(operation_id, ToolStatus::Failed);
    env.diagnostics = conflicts
        .iter()
        .map(|c| diagnostic(&c.code, &c.message, Some("repairable")))
        .collect();
    env.suggested_next_actions = conflicts
        .iter()
        .filter_map(|c| c.actual.as_ref())
        .map(|l| json!({ "action": "read", "args": { "target": { "workspace": l.workspace, "node_id": l.node_id } }, "reason": "re-read before re-planning" }))
        .collect();
    env.payload = Some(
        json!({ "result": "conflict", "changeset_id": changeset_id, "conflicts": conflicts, "digest": digest }),
    );
    env
}

/// A refused call.
pub fn error_envelope(operation_id: &str, err: &NodeDevError) -> ToolResultEnvelope {
    let transient = err.status == 503 || err.status >= 500;
    let status = if transient {
        ToolStatus::Retryable
    } else {
        ToolStatus::Failed
    };
    let class = if transient {
        "transient"
    } else if err.status == 403 {
        "blocking"
    } else {
        "repairable"
    };
    let mut env = base(operation_id, status);
    env.diagnostics = vec![diagnostic(&err.code, &err.message, Some(class))];
    env.payload = Some(json!({ "error": err }));
    env
}

/// A proposed (not committed) changeset: a succeeded call with no writes,
/// whose payload is the reviewable record.
pub fn proposal_envelope(operation_id: &str, record: &ChangesetRecord) -> ToolResultEnvelope {
    let mut env = base(operation_id, ToolStatus::Succeeded);
    env.reads = record
        .plan
        .ops
        .iter()
        .filter_map(|p| p.before.as_ref())
        .filter_map(|l| {
            Some(Read {
                read_id: l.node_id.clone(),
                locator: l.contract(),
                revision: l.revision.clone()?,
            })
        })
        .collect();
    env.diagnostics = record
        .plan
        .conflicts
        .iter()
        .map(|c| diagnostic(&c.code, &c.message, Some("repairable")))
        .collect();
    env.payload = serde_json::to_value(record).ok();
    env
}
