// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Session-level command handlers for SET, SHOW, RESET, and USE BRANCH.
//!
//! These handlers manage connection state such as identity authentication
//! (via JWT in `SET app.user`), branch selection, and PostgreSQL-compatible
//! configuration variables that JDBC drivers expect during initialization.

use crate::auth::{ApiKeyValidator, ConnectionContext, RaisinAuthHandler};
use crate::result_encoder::{infer_schema_from_rows, ResultEncoder};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use pgwire::api::results::{Response, Tag};
use pgwire::api::ClientInfo;
use pgwire::error::ErrorInfo;
use raisin_core::PermissionService;
use raisin_models::auth::AuthContext;
use raisin_sql_execution::Row;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{RepositoryManagementRepository, Storage, WorkspaceRepository};
use std::sync::Arc;
use tracing::{debug, info, warn};

use super::RaisinExtendedQueryHandler;

impl<S, V, P> RaisinExtendedQueryHandler<S, V, P>
where
    S: Storage + TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    // ── Identity (SET app.user) ──────────────────────────────────────────

    /// Handle SET app.user = '<jwt>' command
    ///
    /// Validates the JWT and sets the identity auth context for the connection.
    /// Also resolves the user's permissions from the raisin:access_control workspace.
    pub(crate) async fn handle_set_identity_user<'a, C>(
        &self,
        query: &str,
        client: &C,
        context: &ConnectionContext,
    ) -> Response<'a>
    where
        C: ClientInfo,
    {
        // Extract JWT from query: SET [LOCAL] app.user = 'eyJ...' or SET [LOCAL] app.user TO 'eyJ...'
        let jwt = match Self::extract_jwt_from_set_query(query) {
            Some(jwt) => jwt,
            None => {
                warn!("Failed to parse JWT from SET app.user command: {}", query);
                return Response::Error(Box::new(ErrorInfo::new(
                    "ERROR".to_string(),
                    "22023".to_string(), // invalid_parameter_value
                    "Invalid SET app.user syntax. Expected: SET app.user = '<jwt_token>'"
                        .to_string(),
                )));
            }
        };

        // Decode the JWT to extract user identity
        let (user_id, email, home) = match Self::decode_jwt_claims(&jwt) {
            Ok(claims) => claims,
            Err(e) => {
                warn!("Failed to decode JWT: {}", e);
                return Response::Error(Box::new(ErrorInfo::new(
                    "ERROR".to_string(),
                    "28000".to_string(), // invalid_authorization_specification
                    format!("Invalid JWT token: {}", e),
                )));
            }
        };

        debug!(
            "SET app.user: user_id='{}', email={:?}, home={:?}, repo='{}'",
            user_id, email, home, context.repository
        );

        // Resolve permissions from raisin:access_control workspace
        // Use the connection context repository (from connection URL)
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

        // Store the identity auth in connection context
        self.auth_handler.set_identity_auth(client, auth_context);
        info!(
            "Identity auth context set via SET app.user (extended query) with resolved permissions"
        );

        Response::Execution(Tag::new("SET"))
    }

    /// Extract JWT token from SET app.user query
    ///
    /// Supports:
    /// - SET app.user = 'jwt...'
    /// - SET LOCAL app.user = 'jwt...'
    /// - SET app.user TO 'jwt...'
    /// - SET LOCAL app.user TO 'jwt...'
    pub(crate) fn extract_jwt_from_set_query(query: &str) -> Option<String> {
        // Simple parsing: find the value between single quotes after = or TO
        let query_lower = query.to_lowercase();

        // Find the start of the value
        let value_start = if let Some(pos) = query_lower.find(" = '") {
            pos + 4
        } else if let Some(pos) = query_lower.find(" to '") {
            pos + 5
        } else {
            return None;
        };

        // Find the end quote (use original query for correct case)
        let value_portion = &query[value_start..];
        let value_end = value_portion.find('\'')?;

        Some(value_portion[..value_end].to_string())
    }

    /// Decode JWT claims without full validation
    ///
    /// Returns (subject, optional email, optional home) on success
    pub(crate) fn decode_jwt_claims(
        jwt: &str,
    ) -> std::result::Result<(String, Option<String>, Option<String>), String> {
        // Split JWT into parts
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid JWT format - expected 3 parts".to_string());
        }

        // Decode the payload (second part)
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

    // ── Branch management ────────────────────────────────────────────────

    /// Handle SET app.branch = 'x' / SET LOCAL app.branch = 'x'
    pub(crate) fn handle_set_branch<'a, C>(
        &self,
        query: &str,
        client: &C,
        query_lower: &str,
    ) -> Response<'a>
    where
        C: ClientInfo,
    {
        // Extract branch name from query
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

        // Note: LOCAL is ignored for pgwire - session branch persists for connection
        let is_local = query_lower.starts_with("set local");
        if is_local {
            debug!(
                "SET LOCAL app.branch detected, but treating as session (pgwire connection-level)"
            );
        }

        info!("Extended query: Setting session branch to: {}", branch_name);
        self.auth_handler.set_session_branch(client, branch_name);

        Response::Execution(Tag::new("SET"))
    }

    /// Handle USE BRANCH 'x' / USE LOCAL BRANCH 'x'
    pub(crate) fn handle_use_branch<'a, C>(
        &self,
        query: &str,
        client: &C,
        query_lower: &str,
    ) -> Response<'a>
    where
        C: ClientInfo,
    {
        // Extract branch name: USE [LOCAL] BRANCH 'name' or USE [LOCAL] BRANCH name
        let branch_part = if query_lower.starts_with("use local branch") {
            query.trim_start_matches(|c: char| !c.is_alphanumeric())["use local branch".len()..]
                .trim()
        } else {
            query.trim_start_matches(|c: char| !c.is_alphanumeric())["use branch".len()..].trim()
        };

        // Remove quotes if present
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

        // Note: LOCAL is ignored for pgwire - session branch persists for connection
        let is_local = query_lower.starts_with("use local");
        if is_local {
            debug!("USE LOCAL BRANCH detected, but treating as session (pgwire connection-level)");
        }

        info!("Extended query: Setting session branch to: {}", branch_name);
        self.auth_handler.set_session_branch(client, branch_name);

        Response::Execution(Tag::new("SET"))
    }

    /// Handle SHOW app.branch / SHOW CURRENT BRANCH
    pub(crate) async fn handle_show_branch<'a, C>(
        &self,
        _client: &C,
        context: &ConnectionContext,
    ) -> Response<'a>
    where
        C: ClientInfo,
    {
        // Get current branch - session override or repository default
        let branch = match context.session_branch() {
            Some(b) => b.to_string(),
            None => {
                // Get repository's default_branch from storage
                match self
                    .storage
                    .repository_management()
                    .get_repository(&context.tenant_id, &context.repository)
                    .await
                {
                    Ok(Some(repo)) => repo.config.default_branch.clone(),
                    _ => "main".to_string(), // Fallback if repo not found
                }
            }
        };

        debug!("Extended query: Current effective branch: {}", branch);

        let mut row = Row::new();
        row.insert(
            "branch".to_string(),
            raisin_models::nodes::properties::PropertyValue::String(branch),
        );

        let rows = vec![row];
        let columns = infer_schema_from_rows(&rows);
        let encoder = ResultEncoder::new();
        let schema = encoder.encode_schema(&columns);

        Response::Query(ResultEncoder::build_query_response(rows, schema))
    }

    // ── Generic SET value extraction ─────────────────────────────────────

    /// Extract value from SET command: SET x = 'value' or SET x TO 'value'
    pub(crate) fn extract_value_from_set_query(query: &str) -> Option<String> {
        let query_lower = query.to_lowercase();

        // Find the start of the value
        let value_start = if let Some(pos) = query_lower.find(" = '") {
            pos + 4
        } else if let Some(pos) = query_lower.find(" to '") {
            pos + 5
        } else if let Some(pos) = query_lower.find(" = ") {
            // Handle unquoted values
            let start = pos + 3;
            let value_portion = &query[start..];
            let end = value_portion.find(|c: char| c.is_whitespace() || c == ';');
            return Some(match end {
                Some(e) => value_portion[..e].trim().to_string(),
                None => value_portion.trim().to_string(),
            });
        } else if let Some(pos) = query_lower.find(" to ") {
            // Handle unquoted values with TO
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

        // Find the end quote (use original query for correct case)
        let value_portion = &query[value_start..];
        let value_end = value_portion.find('\'')?;

        Some(value_portion[..value_end].to_string())
    }

    // ── SHOW command handler ─────────────────────────────────────────────

    /// Handle SHOW commands - returns appropriate PostgreSQL configuration values.
    ///
    /// This handles common SHOW parameters that JDBC drivers query during
    /// connection initialization, including transaction isolation level.
    pub(crate) fn handle_show_command<'a>(&self, query_lower: &str) -> Response<'a> {
        // Extract the parameter being shown
        if let Some(param) = query_lower.strip_prefix("show ") {
            let param = param.trim().trim_matches(|c| c == '\'' || c == '"');

            // Common SHOW commands - includes PostgreSQL JDBC driver initialization params
            let value = match param {
                // Basic server info
                "server_version" => "14.0",
                "server_encoding" => "UTF8",
                "client_encoding" => "UTF8",
                "is_superuser" => "off",
                "session_authorization" => "raisin",
                "timezone" => "UTC",
                "integer_datetimes" => "on",
                "standard_conforming_strings" => "on",
                // Transaction isolation (JDBC driver sends this during connection init)
                "transaction isolation level"
                | "transaction_isolation"
                | "default_transaction_isolation" => "read committed",
                // Additional JDBC compatibility params
                "max_identifier_length" => "63",
                "datestyle" => "ISO, MDY",
                "intervalstyle" => "postgres",
                "extra_float_digits" => "3",
                "application_name" => "",
                "search_path" => "public",
                _ => {
                    warn!("Unknown SHOW parameter: {}", param);
                    return Response::Error(Box::new(ErrorInfo::new(
                        "ERROR".to_string(),
                        "42704".to_string(), // undefined_object
                        format!("unrecognized configuration parameter \"{}\"", param),
                    )));
                }
            };

            let mut row = Row::new();
            row.insert(
                param.to_string(),
                raisin_models::nodes::properties::PropertyValue::String(value.to_string()),
            );

            let rows = vec![row];
            let columns = infer_schema_from_rows(&rows);
            // SHOW commands use text format (simple query semantics)
            let encoder = ResultEncoder::new();
            let schema = encoder.encode_schema(&columns);

            return Response::Query(ResultEncoder::build_query_response(rows, schema));
        }

        // Shouldn't reach here, but return empty query response as fallback
        Response::EmptyQuery
    }
}
