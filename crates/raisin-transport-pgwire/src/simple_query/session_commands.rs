// SPDX-License-Identifier: BSL-1.1

//! Session management commands (SET app.user, SET app.branch, USE BRANCH).

use crate::auth::{ApiKeyValidator, ConnectionContext};
use crate::result_encoder::{infer_schema_from_rows, ResultEncoder};
use pgwire::api::results::{Response, Tag};
use pgwire::api::ClientInfo;
use pgwire::error::ErrorInfo;
use raisin_core::PermissionService;
use raisin_models::auth::AuthContext;
use raisin_sql_execution::Row;
use raisin_storage::{RepositoryManagementRepository, Storage};
use tracing::{debug, info, warn};

use super::handler::RaisinSimpleQueryHandler;

impl<S, V, P> RaisinSimpleQueryHandler<S, V, P>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    /// Handle SET app.user = '<jwt>' command
    ///
    /// Validates the JWT and sets the identity auth context for the connection.
    /// Also resolves the user's permissions from the raisin:access_control workspace.
    pub(super) async fn handle_set_identity_user<'a, C>(
        &self,
        query: &str,
        client: &C,
        context: &ConnectionContext,
    ) -> Response<'a>
    where
        C: ClientInfo,
    {
        let jwt = match Self::extract_jwt_from_set_query(query) {
            Some(jwt) => jwt,
            None => {
                warn!("Failed to parse JWT from SET app.user command: {}", query);
                return Response::Error(Box::new(ErrorInfo::new(
                    "ERROR".to_string(),
                    "22023".to_string(),
                    "Invalid SET app.user syntax. Expected: SET app.user = '<jwt_token>'"
                        .to_string(),
                )));
            }
        };

        let (user_id, email, home) = match Self::decode_jwt_claims(&jwt) {
            Ok(claims) => claims,
            Err(e) => {
                warn!("Failed to decode JWT: {}", e);
                return Response::Error(Box::new(ErrorInfo::new(
                    "ERROR".to_string(),
                    "28000".to_string(),
                    format!("Invalid JWT token: {}", e),
                )));
            }
        };

        debug!(
            "SET app.user: user_id='{}', email={:?}, home={:?}, repo='{}'",
            user_id, email, home, context.repository
        );

        // Resolve permissions from raisin:access_control workspace
        let permission_service = PermissionService::new(self.storage.clone());
        let resolved_permissions = permission_service
            .resolve_for_identity_id(&context.tenant_id, &context.repository, "main", &user_id)
            .await;

        let resolved_permissions = resolved_permissions.ok().flatten();

        // Create AuthContext with resolved permissions and home path for REL conditions
        let mut auth_context = AuthContext::for_user(user_id.clone());
        if let Some(email) = email {
            auth_context = auth_context.with_email(email);
        }
        if let Some(home) = home {
            auth_context = auth_context.with_home(home);
        }
        if let Some(perms) = resolved_permissions {
            debug!(
                "Resolved {} permissions for user {} (roles: {:?})",
                perms.permissions.len(),
                user_id,
                perms.effective_roles
            );
            auth_context = auth_context.with_permissions(perms);
        } else {
            debug!(
                "No permissions found for user {} in raisin:access_control",
                user_id
            );
        }

        self.auth_handler.set_identity_auth(client, auth_context);
        info!("Identity auth context set via SET app.user with resolved permissions");

        Response::Execution(Tag::new("SET"))
    }

    /// Handle SET app.branch = 'x' / SET LOCAL app.branch = 'x'
    pub(super) fn handle_set_branch<'a, C>(
        &self,
        query: &str,
        client: &C,
        query_lower: &str,
    ) -> Response<'a>
    where
        C: ClientInfo,
    {
        let branch_name = match Self::extract_value_from_set_query(query) {
            Some(branch) => branch,
            None => {
                warn!(
                    "Failed to parse branch name from SET app.branch command: {}",
                    query
                );
                return Response::Error(Box::new(ErrorInfo::new(
                    "ERROR".to_string(),
                    "42601".to_string(),
                    "Invalid SET app.branch syntax. Expected: SET app.branch = 'branch_name'"
                        .to_string(),
                )));
            }
        };

        let is_local = query_lower.starts_with("set local");
        if is_local {
            debug!(
                "SET LOCAL app.branch detected, but treating as session (pgwire connection-level)"
            );
        }

        info!("Setting session branch to: {}", branch_name);
        self.auth_handler.set_session_branch(client, branch_name);

        Response::Execution(Tag::new("SET"))
    }

    /// Handle USE BRANCH 'x' / USE LOCAL BRANCH 'x'
    pub(super) fn handle_use_branch<'a, C>(
        &self,
        query: &str,
        client: &C,
        query_lower: &str,
    ) -> Response<'a>
    where
        C: ClientInfo,
    {
        let branch_part = if query_lower.starts_with("use local branch") {
            query.trim_start_matches(|c: char| !c.is_alphanumeric())["use local branch".len()..]
                .trim()
        } else {
            query.trim_start_matches(|c: char| !c.is_alphanumeric())["use branch".len()..].trim()
        };

        let branch_name = branch_part
            .trim_matches(|c| c == '\'' || c == '"')
            .to_string();

        if branch_name.is_empty() {
            return Response::Error(Box::new(ErrorInfo::new(
                "ERROR".to_string(),
                "42601".to_string(),
                "Invalid USE BRANCH syntax. Expected: USE BRANCH 'branch_name'".to_string(),
            )));
        }

        let is_local = query_lower.starts_with("use local");
        if is_local {
            debug!("USE LOCAL BRANCH detected, but treating as session (pgwire connection-level)");
        }

        info!("Setting session branch to: {}", branch_name);
        self.auth_handler.set_session_branch(client, branch_name);

        Response::Execution(Tag::new("SET"))
    }

    /// Handle SHOW app.branch / SHOW CURRENT BRANCH
    pub(super) async fn handle_show_branch<'a, C>(
        &self,
        _client: &C,
        context: &ConnectionContext,
    ) -> Response<'a>
    where
        C: ClientInfo,
    {
        let branch = match context.session_branch() {
            Some(b) => b.to_string(),
            None => {
                match self
                    .storage
                    .repository_management()
                    .get_repository(&context.tenant_id, &context.repository)
                    .await
                {
                    Ok(Some(repo)) => repo.config.default_branch.clone(),
                    _ => "main".to_string(),
                }
            }
        };

        debug!("Current effective branch: {}", branch);

        let mut row = Row::new();
        row.insert(
            "branch".to_string(),
            raisin_models::nodes::properties::PropertyValue::String(branch),
        );

        let rows = vec![row];
        let columns = infer_schema_from_rows(&rows);
        let schema = ResultEncoder::new().encode_schema(&columns);

        Response::Query(ResultEncoder::build_query_response(rows, schema))
    }

    /// Extract JWT token from SET app.user query
    pub(super) fn extract_jwt_from_set_query(query: &str) -> Option<String> {
        let query_lower = query.to_lowercase();

        let value_start = if let Some(pos) = query_lower.find(" = '") {
            pos + 4
        } else if let Some(pos) = query_lower.find(" to '") {
            pos + 5
        } else {
            return None;
        };

        let value_portion = &query[value_start..];
        let value_end = value_portion.find('\'')?;

        Some(value_portion[..value_end].to_string())
    }

    /// Decode JWT claims without full validation
    ///
    /// Returns (subject, optional email, optional home) on success
    pub(super) fn decode_jwt_claims(
        jwt: &str,
    ) -> Result<(String, Option<String>, Option<String>), String> {
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid JWT format - expected 3 parts".to_string());
        }

        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let payload_bytes = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|e| format!("Failed to decode JWT payload: {}", e))?;

        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes)
            .map_err(|e| format!("Failed to parse JWT payload: {}", e))?;

        let sub = payload
            .get("sub")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'sub' claim in JWT")?
            .to_string();

        let email = payload
            .get("email")
            .and_then(|v| v.as_str())
            .map(String::from);
        let home = payload
            .get("home")
            .and_then(|v| v.as_str())
            .map(String::from);

        Ok((sub, email, home))
    }

    /// Extract value from SET command: SET x = 'value' or SET x TO 'value'
    pub(super) fn extract_value_from_set_query(query: &str) -> Option<String> {
        let query_lower = query.to_lowercase();

        let value_start = if let Some(pos) = query_lower.find(" = '") {
            pos + 4
        } else if let Some(pos) = query_lower.find(" to '") {
            pos + 5
        } else if let Some(pos) = query_lower.find(" = ") {
            let start = pos + 3;
            let value_portion = &query[start..];
            let end = value_portion.find(|c: char| c.is_whitespace() || c == ';');
            return Some(match end {
                Some(e) => value_portion[..e].trim().to_string(),
                None => value_portion.trim().to_string(),
            });
        } else if let Some(pos) = query_lower.find(" to ") {
            let start = pos + 4;
            let value_portion = &query[start..];
            let end = value_portion.find(|c: char| c.is_whitespace() || c == ';');
            return Some(match end {
                Some(e) => value_portion[..e].trim().to_string(),
                None => value_portion.trim().to_string(),
            });
        } else {
            return None;
        };

        let value_portion = &query[value_start..];
        let value_end = value_portion.find('\'')?;

        Some(value_portion[..value_end].to_string())
    }
}
