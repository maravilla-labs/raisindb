//! HTTP handlers for AI processing rules management
//!
//! Provides REST API endpoints for:
//! - Getting and setting processing rules per repository
//! - Creating, updating, and deleting individual rules
//! - Reordering rules (first-match-wins)
//! - Testing rule matching against node metadata

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use raisin_ai::{ProcessingRule, ProcessingSettings, RuleMatchContext, RuleMatcher};
use raisin_storage::scope::RepoScope;
use raisin_storage::ProcessingRulesRepository;

use crate::{error::ApiError, state::AppState};

// ============================================================================
// Request/Response Types
// ============================================================================

/// Response containing all rules for a repository
#[derive(Debug, Serialize)]
pub struct RulesListResponse {
    pub repo_id: String,
    pub rules: Vec<ProcessingRuleResponse>,
}

/// A processing rule in API response format
#[derive(Debug, Serialize)]
pub struct ProcessingRuleResponse {
    pub id: String,
    pub name: String,
    pub order: i32,
    pub enabled: bool,
    pub matcher: RuleMatcherResponse,
    pub settings: ProcessingSettings,
}

/// Rule matcher in API response format (recursive for Combined)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleMatcherResponse {
    All,
    NodeType { node_type: String },
    Path { pattern: String },
    MimeType { mime_type: String },
    Workspace { workspace: String },
    Property { name: String, value: String },
    Combined { matchers: Vec<RuleMatcherResponse> },
}

impl From<RuleMatcher> for RuleMatcherResponse {
    fn from(matcher: RuleMatcher) -> Self {
        match matcher {
            RuleMatcher::All => RuleMatcherResponse::All,
            RuleMatcher::NodeType(node_type) => RuleMatcherResponse::NodeType { node_type },
            RuleMatcher::Path { pattern } => RuleMatcherResponse::Path { pattern },
            RuleMatcher::MimeType { mime_type } => RuleMatcherResponse::MimeType { mime_type },
            RuleMatcher::Workspace { workspace } => RuleMatcherResponse::Workspace { workspace },
            RuleMatcher::Property { name, value } => RuleMatcherResponse::Property { name, value },
            RuleMatcher::Combined { matchers } => RuleMatcherResponse::Combined {
                matchers: matchers
                    .into_iter()
                    .map(RuleMatcherResponse::from)
                    .collect(),
            },
        }
    }
}

impl From<RuleMatcherResponse> for RuleMatcher {
    fn from(matcher: RuleMatcherResponse) -> Self {
        match matcher {
            RuleMatcherResponse::All => RuleMatcher::All,
            RuleMatcherResponse::NodeType { node_type } => RuleMatcher::NodeType(node_type),
            RuleMatcherResponse::Path { pattern } => RuleMatcher::Path { pattern },
            RuleMatcherResponse::MimeType { mime_type } => RuleMatcher::MimeType { mime_type },
            RuleMatcherResponse::Workspace { workspace } => RuleMatcher::Workspace { workspace },
            RuleMatcherResponse::Property { name, value } => RuleMatcher::Property { name, value },
            RuleMatcherResponse::Combined { matchers } => RuleMatcher::Combined {
                matchers: matchers.into_iter().map(RuleMatcher::from).collect(),
            },
        }
    }
}

impl From<ProcessingRule> for ProcessingRuleResponse {
    fn from(rule: ProcessingRule) -> Self {
        Self {
            id: rule.id,
            name: rule.name,
            order: rule.order,
            enabled: rule.enabled,
            matcher: RuleMatcherResponse::from(rule.matcher),
            settings: rule.settings,
        }
    }
}

/// Request body for creating a new rule
#[derive(Debug, Deserialize)]
pub struct CreateRuleRequest {
    /// Optional ID (auto-generated if not provided)
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    /// Optional order (appended to end if not provided)
    #[serde(default)]
    pub order: Option<i32>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub matcher: RuleMatcherResponse,
    #[serde(default)]
    pub settings: ProcessingSettings,
}

fn default_enabled() -> bool {
    true
}

/// Request body for updating an existing rule
#[derive(Debug, Deserialize)]
pub struct UpdateRuleRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub order: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub matcher: Option<RuleMatcherResponse>,
    #[serde(default)]
    pub settings: Option<ProcessingSettings>,
}

/// Request body for reordering rules
#[derive(Debug, Deserialize)]
pub struct ReorderRulesRequest {
    /// Rule IDs in the desired order
    pub rule_ids: Vec<String>,
}

/// Request body for testing rule matching
#[derive(Debug, Deserialize)]
pub struct TestRuleMatchRequest {
    /// Node path to test
    #[serde(default)]
    pub path: Option<String>,
    /// Node type to test
    #[serde(default)]
    pub node_type: Option<String>,
    /// MIME type to test
    #[serde(default)]
    pub mime_type: Option<String>,
    /// Workspace to test
    #[serde(default)]
    pub workspace: Option<String>,
    /// Properties to test against (name -> value)
    #[serde(default)]
    pub properties: std::collections::HashMap<String, String>,
}

/// Response for rule matching test
#[derive(Debug, Serialize)]
pub struct TestRuleMatchResponse {
    /// Whether any rule matched
    pub matched: bool,
    /// The matching rule (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_rule: Option<ProcessingRuleResponse>,
    /// All rules that were evaluated
    pub rules_evaluated: usize,
}

/// Generic success response
#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

// ============================================================================
// Handler Functions
// ============================================================================

/// List all processing rules for a repository
///
/// GET /api/repository/{repo}/ai/rules
#[axum::debug_handler]
pub async fn list_rules(
    Path(repo): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<RulesListResponse>, ApiError> {
    // Get tenant from context (default for now)
    let tenant_id = "default";

    let repo_impl = state.storage().processing_rules_repository();

    let scope = RepoScope::new(tenant_id, &repo);

    match repo_impl.get_rules(scope).await {
        Ok(Some(rules)) => {
            let response = RulesListResponse {
                repo_id: repo,
                rules: rules
                    .rules
                    .into_iter()
                    .map(ProcessingRuleResponse::from)
                    .collect(),
            };
            Ok(Json(response))
        }
        Ok(None) => {
            // Return empty list if no rules configured
            Ok(Json(RulesListResponse {
                repo_id: repo,
                rules: Vec::new(),
            }))
        }
        Err(e) => {
            tracing::error!(
                "Failed to get processing rules for {}/{}: {}",
                tenant_id,
                repo,
                e
            );
            Err(ApiError::internal(format!("Storage error: {}", e)))
        }
    }
}

/// Get a single processing rule by ID
///
/// GET /api/repository/{repo}/ai/rules/{rule_id}
#[axum::debug_handler]
pub async fn get_rule(
    Path((repo, rule_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<ProcessingRuleResponse>, ApiError> {
    let tenant_id = "default";

    let repo_impl = state.storage().processing_rules_repository();

    let scope = RepoScope::new(tenant_id, &repo);

    match repo_impl.get_rule(scope, &rule_id).await {
        Ok(Some(rule)) => Ok(Json(ProcessingRuleResponse::from(rule))),
        Ok(None) => Err(ApiError::not_found(format!("Rule '{}' not found", rule_id))),
        Err(e) => {
            tracing::error!(
                "Failed to get rule {}/{}/{}: {}",
                tenant_id,
                repo,
                rule_id,
                e
            );
            Err(ApiError::internal(format!("Storage error: {}", e)))
        }
    }
}

/// Create a new processing rule
///
/// POST /api/repository/{repo}/ai/rules
#[axum::debug_handler]
pub async fn create_rule(
    Path(repo): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<CreateRuleRequest>,
) -> Result<(StatusCode, Json<ProcessingRuleResponse>), ApiError> {
    let tenant_id = "default";

    let repo_impl = state.storage().processing_rules_repository();

    let scope = RepoScope::new(tenant_id, &repo);

    // Get existing rules to determine order if not specified
    let existing = repo_impl
        .get_rules(scope)
        .await
        .map_err(|e| ApiError::internal(format!("Storage error: {}", e)))?
        .unwrap_or_default();

    let order = req
        .order
        .unwrap_or_else(|| existing.rules.iter().map(|r| r.order).max().unwrap_or(0) + 1);

    // Generate ID if not provided
    let id = req.id.unwrap_or_else(|| nanoid::nanoid!(12));

    // Check for duplicate ID
    if existing.get_rule(&id).is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "RULE_ID_EXISTS",
            format!("Rule with ID '{}' already exists", id),
        ));
    }

    let rule = ProcessingRule {
        id: id.clone(),
        name: req.name,
        order,
        enabled: req.enabled,
        matcher: RuleMatcher::from(req.matcher),
        settings: req.settings,
    };

    repo_impl
        .upsert_rule(scope, &rule)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to create rule: {}", e)))?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %repo,
        rule_id = %id,
        "Created processing rule"
    );

    Ok((
        StatusCode::CREATED,
        Json(ProcessingRuleResponse::from(rule)),
    ))
}

/// Update an existing processing rule
///
/// PUT /api/repository/{repo}/ai/rules/{rule_id}
#[axum::debug_handler]
pub async fn update_rule(
    Path((repo, rule_id)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(req): Json<UpdateRuleRequest>,
) -> Result<Json<ProcessingRuleResponse>, ApiError> {
    let tenant_id = "default";

    let repo_impl = state.storage().processing_rules_repository();

    let scope = RepoScope::new(tenant_id, &repo);

    // Get existing rule
    let mut rule = repo_impl
        .get_rule(scope, &rule_id)
        .await
        .map_err(|e| ApiError::internal(format!("Storage error: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Rule '{}' not found", rule_id)))?;

    // Apply updates
    if let Some(name) = req.name {
        rule.name = name;
    }
    if let Some(order) = req.order {
        rule.order = order;
    }
    if let Some(enabled) = req.enabled {
        rule.enabled = enabled;
    }
    if let Some(matcher) = req.matcher {
        rule.matcher = RuleMatcher::from(matcher);
    }
    if let Some(settings) = req.settings {
        rule.settings = settings;
    }

    repo_impl
        .upsert_rule(scope, &rule)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to update rule: {}", e)))?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %repo,
        rule_id = %rule_id,
        "Updated processing rule"
    );

    Ok(Json(ProcessingRuleResponse::from(rule)))
}

/// Delete a processing rule
///
/// DELETE /api/repository/{repo}/ai/rules/{rule_id}
#[axum::debug_handler]
pub async fn delete_rule(
    Path((repo, rule_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let tenant_id = "default";

    let repo_impl = state.storage().processing_rules_repository();

    let scope = RepoScope::new(tenant_id, &repo);

    // Check if rule exists
    let exists = repo_impl
        .get_rule(scope, &rule_id)
        .await
        .map_err(|e| ApiError::internal(format!("Storage error: {}", e)))?
        .is_some();

    if !exists {
        return Err(ApiError::not_found(format!("Rule '{}' not found", rule_id)));
    }

    repo_impl
        .delete_rule(scope, &rule_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to delete rule: {}", e)))?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %repo,
        rule_id = %rule_id,
        "Deleted processing rule"
    );

    Ok(Json(SuccessResponse {
        success: true,
        message: format!("Rule '{}' deleted", rule_id),
    }))
}

/// Reorder processing rules
///
/// PUT /api/repository/{repo}/ai/rules/reorder
#[axum::debug_handler]
pub async fn reorder_rules(
    Path(repo): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<ReorderRulesRequest>,
) -> Result<Json<SuccessResponse>, ApiError> {
    let tenant_id = "default";

    if req.rule_ids.is_empty() {
        return Err(ApiError::validation_failed("rule_ids cannot be empty"));
    }

    let repo_impl = state.storage().processing_rules_repository();

    let scope = RepoScope::new(tenant_id, &repo);

    repo_impl
        .reorder_rules(scope, &req.rule_ids)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to reorder rules: {}", e)))?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %repo,
        rule_count = %req.rule_ids.len(),
        "Reordered processing rules"
    );

    Ok(Json(SuccessResponse {
        success: true,
        message: format!("Reordered {} rules", req.rule_ids.len()),
    }))
}

/// Test rule matching against provided metadata
///
/// POST /api/repository/{repo}/ai/rules/test
#[axum::debug_handler]
pub async fn test_rule_match(
    Path(repo): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<TestRuleMatchRequest>,
) -> Result<Json<TestRuleMatchResponse>, ApiError> {
    let tenant_id = "default";

    let repo_impl = state.storage().processing_rules_repository();

    let scope = RepoScope::new(tenant_id, &repo);

    let rules = repo_impl
        .get_rules(scope)
        .await
        .map_err(|e| ApiError::internal(format!("Storage error: {}", e)))?
        .unwrap_or_default();

    // Build match context
    let context = RuleMatchContext {
        path: req.path,
        node_type: req.node_type,
        mime_type: req.mime_type,
        workspace: req.workspace,
        properties: req.properties,
    };

    // Find first matching rule (first-match-wins)
    let matched_rule = rules.find_matching_rule(&context);

    Ok(Json(TestRuleMatchResponse {
        matched: matched_rule.is_some(),
        matched_rule: matched_rule.map(|r| ProcessingRuleResponse::from(r.clone())),
        rules_evaluated: rules.rules.len(),
    }))
}
