// SPDX-License-Identifier: BSL-1.1

//! PostgreSQL system query handling (version, SET, SHOW commands).

use crate::auth::{ApiKeyValidator, ConnectionContext};
use crate::result_encoder::{infer_schema_from_rows, ResultEncoder};
use pgwire::api::results::Response;
use pgwire::api::results::Tag;
use pgwire::api::ClientInfo;
use pgwire::error::ErrorInfo;
use raisin_sql_execution::Row;
use raisin_storage::Storage;
use tracing::{debug, warn};

use super::handler::RaisinSimpleQueryHandler;

impl<S, V, P> RaisinSimpleQueryHandler<S, V, P>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    /// Handle PostgreSQL system queries that need special treatment
    ///
    /// Returns Some(Response) if the query is a system query that was handled,
    /// None if the query should be passed to the normal execution pipeline.
    pub(super) async fn handle_system_query<'a, C>(
        &self,
        query: &str,
        client: &C,
        context: &ConnectionContext,
    ) -> Option<Response<'a>>
    where
        C: ClientInfo,
    {
        let query_lower = query.trim().to_lowercase();

        // Handle SELECT version()
        if query_lower.starts_with("select version()") || query_lower == "select version()" {
            debug!("Handling SELECT version() system query");
            return Some(self.create_version_response());
        }

        // Handle SET app.user = '<jwt>' - set identity context
        if query_lower.starts_with("set local app.user") || query_lower.starts_with("set app.user")
        {
            debug!("Handling SET app.user command: {}", query);
            return Some(self.handle_set_identity_user(query, client, context).await);
        }

        // Handle RESET app.user - clear identity context
        if query_lower.starts_with("reset app.user") {
            debug!("Handling RESET app.user command");
            self.auth_handler.clear_identity_auth(client);
            return Some(Response::Execution(Tag::new("RESET")));
        }

        // Handle SET app.branch = 'x' / SET LOCAL app.branch = 'x'
        if query_lower.starts_with("set local app.branch")
            || query_lower.starts_with("set app.branch")
        {
            debug!("Handling SET app.branch command: {}", query);
            return Some(self.handle_set_branch(query, client, &query_lower));
        }

        // Handle USE BRANCH 'x' / USE LOCAL BRANCH 'x'
        if query_lower.starts_with("use branch") || query_lower.starts_with("use local branch") {
            debug!("Handling USE BRANCH command: {}", query);
            return Some(self.handle_use_branch(query, client, &query_lower));
        }

        // Handle SHOW app.branch / SHOW CURRENT BRANCH
        if query_lower.starts_with("show app.branch")
            || query_lower.starts_with("show current branch")
        {
            debug!("Handling SHOW branch command");
            return Some(self.handle_show_branch(client, context).await);
        }

        // Handle other SET commands - acknowledge but don't execute
        if query_lower.starts_with("set ") {
            debug!("Acknowledging SET command: {}", query);
            return Some(Response::Execution(Tag::new("SET")));
        }

        // Handle SHOW commands with default values
        if query_lower.starts_with("show ") {
            debug!("Handling SHOW command: {}", query);
            return self.handle_show_command(&query_lower);
        }

        None
    }

    /// Create a response for SELECT version() query
    pub(super) fn create_version_response<'a>(&self) -> Response<'a> {
        let version = format!(
            "RaisinDB {} on {} (PostgreSQL 14.0 compatible)",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        );

        let mut row = Row::new();
        row.insert(
            "version".to_string(),
            raisin_models::nodes::properties::PropertyValue::String(version),
        );

        let rows = vec![row];
        let columns = infer_schema_from_rows(&rows);
        let schema = ResultEncoder::new().encode_schema(&columns);

        Response::Query(ResultEncoder::build_query_response(rows, schema))
    }

    /// Handle SHOW commands
    pub(super) fn handle_show_command<'a>(&self, query_lower: &str) -> Option<Response<'a>> {
        if let Some(param) = query_lower.strip_prefix("show ") {
            let param = param.trim().trim_matches(|c| c == '\'' || c == '"');

            let value = match param {
                "server_version" => "14.0",
                "server_encoding" => "UTF8",
                "client_encoding" => "UTF8",
                "is_superuser" => "off",
                "session_authorization" => "raisin",
                "timezone" => "UTC",
                "integer_datetimes" => "on",
                "standard_conforming_strings" => "on",
                "transaction isolation level"
                | "transaction_isolation"
                | "default_transaction_isolation" => "read committed",
                "max_identifier_length" => "63",
                "datestyle" => "ISO, MDY",
                "intervalstyle" => "postgres",
                "extra_float_digits" => "3",
                "application_name" => "",
                "search_path" => "public",
                _ => {
                    warn!("Unknown SHOW parameter: {}", param);
                    return Some(Response::Error(Box::new(ErrorInfo::new(
                        "ERROR".to_string(),
                        "42704".to_string(),
                        format!("unrecognized configuration parameter \"{}\"", param),
                    ))));
                }
            };

            let mut row = Row::new();
            row.insert(
                param.to_string(),
                raisin_models::nodes::properties::PropertyValue::String(value.to_string()),
            );

            let rows = vec![row];
            let columns = infer_schema_from_rows(&rows);
            let schema = ResultEncoder::new().encode_schema(&columns);

            return Some(Response::Query(ResultEncoder::build_query_response(
                rows, schema,
            )));
        }

        None
    }
}
