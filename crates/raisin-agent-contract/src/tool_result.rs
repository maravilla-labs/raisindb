// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The tool-result envelope (`raisin.tool-result/1`).
//!
//! What a tool returns, typed enough that a reducer can attach writes, reads
//! and evidence to artifacts without parsing prose. Core never parses
//! `payload`; `legacy: true` marks a pre-envelope result wrapped by an adapter,
//! whose writes and evidence are untrusted.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultEnvelope {
    /// Always [`crate::TOOL_RESULT_V1`].
    pub envelope: String,
    /// The operation this result answers.
    pub operation_id: String,
    /// Outcome.
    pub status: ToolStatus,
    /// Required when `status == waiting`: opens an external request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_key: Option<String>,
    /// What the tool read, with the revision it saw.
    #[serde(default)]
    pub reads: Vec<Read>,
    /// What the tool wrote.
    #[serde(default)]
    pub writes: Vec<Write>,
    /// Artifacts the result is about; a succeeded, non-legacy result with
    /// writes carries exactly one `primary`.
    #[serde(default)]
    pub artifact_refs: Vec<ArtifactRef>,
    /// Evidence produced.
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// Diagnostics.
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    /// Suggested next actions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_next_actions: Vec<Value>,
    /// Retry policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<Value>,
    /// The tool's own result, unchanged. Opaque to core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Set by an adapter wrapping a pre-envelope result.
    #[serde(default)]
    pub legacy: bool,
}

/// Tool outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    /// Done.
    Succeeded,
    /// Parked on something external; `resume_key` names it.
    Waiting,
    /// Failed, may succeed if retried.
    Retryable,
    /// Cannot proceed without a human or capability.
    Blocked,
    /// Failed.
    Failed,
}

/// Where something lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locator {
    /// Repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Workspace.
    pub workspace: String,
    /// Path.
    pub path: String,
    /// Stable node id, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

/// A revision under a named scheme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    /// The value.
    pub value: String,
    /// Open string naming the scheme (`sha256`, `node_revision`, …).
    pub alg: String,
}

/// A read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Read {
    /// Id a later spec can reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_id: Option<String>,
    /// What was read.
    pub locator: Locator,
    /// At which revision.
    pub revision: Revision,
}

/// A write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Write {
    /// What was written.
    pub locator: Locator,
    /// How.
    pub action: WriteAction,
    /// Previous location, for a move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Locator>,
    /// Revision after the write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<Revision>,
}

/// Kind of write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteAction {
    /// Created.
    Created,
    /// Updated.
    Updated,
    /// Deleted.
    Deleted,
    /// Moved.
    Moved,
}

/// An artifact a result is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// The spec's logical key, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_key: Option<String>,
    /// Domain kind.
    pub kind: String,
    /// Where it lives.
    pub locator: Locator,
    /// Revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<Revision>,
    /// Role.
    pub role: ArtifactRole,
}

/// Role of an artifact in a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRole {
    /// The artifact the operation was for.
    Primary,
    /// Written in support of it.
    Supporting,
    /// Depended on.
    Dependency,
}

/// Evidence about a subject at a revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// Id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<String>,
    /// Open string: `read_back`, `validation`, `fixture`, `verification`, …
    pub kind: String,
    /// What the evidence is about.
    pub subject: EvidenceSubject,
    /// Open string; the domain defines its proof vocabulary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// Whether it holds.
    pub ok: bool,
    /// Individual checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<Value>,
    /// What it depends on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<Value>,
}

/// Subject of a piece of evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceSubject {
    /// Where.
    pub locator: Locator,
    /// At which revision.
    pub revision: Revision,
}

/// A diagnostic: code, severity, message and fix, plus an optional class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable code.
    pub code: String,
    /// Severity.
    pub severity: Severity,
    /// The tool's own classification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<DiagClass>,
    /// Message.
    pub message: String,
    /// Suggested fix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// Location inside the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Error.
    Error,
    /// Warning.
    Warning,
    /// Info.
    Info,
}

/// Classification of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagClass {
    /// The agent can fix it.
    Repairable,
    /// Needs a human or a capability.
    Blocking,
    /// Retry.
    Transient,
}
