// SPDX-License-Identifier: BSL-1.1

//! Payload types for operational commands.
//!
//! Covers branches, tags, workspaces, repositories, translations,
//! transactions, event subscriptions, and authentication.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Branch operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCreatePayload {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protected: Option<bool>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub include_revision_history: bool,
}

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchGetPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchListPayload {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDeletePayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchGetHeadPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchUpdateHeadPayload {
    pub name: String,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchMergePayload {
    pub source_branch: String,
    pub target_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchComparePayload {
    pub branch: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDiffPayload {
    pub branch: String,
    pub base_branch: String,
}

/// Cross-branch node-set copy (branch promotion): copy `roots` (node paths on
/// `source_branch`) onto `target_branch`, preserving node ids, in one atomic
/// commit. `delete_missing` prunes target nodes absent from the source set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCopyNodesPayload {
    pub source_branch: String,
    pub target_branch: String,
    pub workspace: String,
    pub roots: Vec<String>,
    #[serde(default = "default_true")]
    pub recursive: bool,
    #[serde(default)]
    pub delete_missing: bool,
}

// ---------------------------------------------------------------------------
// Scheduled invocation payloads
// ---------------------------------------------------------------------------

/// Create a one-shot scheduled invocation: run a function or flow once at
/// `run_at`. A `run_at` in the past dispatches immediately. The optional
/// `external_key` is a caller-supplied idempotency/lookup key that can later
/// be used to find or cancel the invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledInvocationCreatePayload {
    /// Invocation target kind: "function" or "flow"
    pub target_kind: String,
    /// Path of the target function or flow node
    pub target_path: String,
    /// Input passed to the target when it fires
    #[serde(default)]
    pub input: serde_json::Value,
    /// When to fire (RFC3339 timestamp)
    pub run_at: String,
    /// Optional caller-supplied idempotency/lookup key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_key: Option<String>,
    /// Branch the invocation executes against (defaults to "main")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Workspace the invocation executes against (defaults to "functions")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Retry attempts if the invocation fails (defaults to 0 — one-shot)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
}

/// Reference a scheduled invocation by job id or external key (cancel / get).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledInvocationRefPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_key: Option<String>,
}

/// List scheduled invocations in the current repository.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScheduledInvocationListPayload {
    /// Only invocations with this external key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_key: Option<String>,
    /// Only invocations in this status ("scheduled", "running", "completed",
    /// "cancelled", "failed")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

// ---------------------------------------------------------------------------
// Tag operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagCreatePayload {
    pub name: String,
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagGetPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagListPayload {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagDeletePayload {
    pub name: String,
}

// ---------------------------------------------------------------------------
// Workspace operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCreatePayload {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceGetPayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDeletePayload {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceUpdatePayload {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_node_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_root_node_types: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Repository operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryCreatePayload {
    pub repository_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryGetPayload {
    pub repository_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryListPayload {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryUpdatePayload {
    pub repository_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryDeletePayload {
    pub repository_id: String,
}

// ---------------------------------------------------------------------------
// Translation operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationUpdatePayload {
    pub node_path: String,
    pub locale: String,
    pub properties: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationListPayload {
    pub node_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationDeletePayload {
    pub node_path: String,
    pub locale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationHidePayload {
    pub node_path: String,
    pub locale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationUnhidePayload {
    pub node_path: String,
    pub locale: String,
}

// ---------------------------------------------------------------------------
// Transaction operation payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionBeginPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionCommitPayload {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRollbackPayload {}

// ---------------------------------------------------------------------------
// Event subscription payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribePayload {
    /// Filters for this subscription
    pub filters: SubscriptionFilters,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubscriptionFilters {
    /// Filter by workspace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,

    /// Filter by path pattern (supports wildcards like "/folder/*")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Filter by event types
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_types: Option<Vec<String>>,

    /// Filter by node type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,

    /// Include full node data in event payload (default: false)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_node: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribePayload {
    /// Subscription ID to cancel
    pub subscription_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionResponse {
    /// Unique subscription ID
    pub subscription_id: String,
}

// ---------------------------------------------------------------------------
// Authentication payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatePayload {
    pub username: String,
    pub password: String,
}

/// Request to authenticate with JWT token (identity user)
///
/// Used by SPAs and clients that have already obtained a JWT token
/// via HTTP API (e.g., /auth/{repo}/login)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticateJwtPayload {
    /// JWT access token from identity provider
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticateResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

/// Response for JWT authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticateJwtResponse {
    /// User ID from the JWT
    pub user_id: String,
    /// Effective roles assigned to the user
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshTokenPayload {
    pub refresh_token: String,
}
