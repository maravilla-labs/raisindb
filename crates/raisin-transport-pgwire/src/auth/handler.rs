// SPDX-License-Identifier: BSL-1.1

//! Authentication handler for pgwire connections.

use async_trait::async_trait;
use futures::sink::{Sink, SinkExt};
use pgwire::api::auth::{
    finish_authentication, save_startup_parameters_to_metadata, LoginInfo, ServerParameterProvider,
    StartupHandler,
};
use pgwire::api::{ClientInfo, PgWireConnectionState};
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use pgwire::messages::response::ErrorResponse;
use pgwire::messages::startup::Authentication;
use pgwire::messages::{PgWireBackendMessage, PgWireFrontendMessage};
use raisin_models::auth::AuthContext;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, RwLock};

use super::context::ConnectionContext;
use super::validator::ApiKeyValidator;

/// Authentication handler for RaisinDB pgwire connections
///
/// Implements pgwire's `StartupHandler` trait and performs API key-based
/// authentication using cleartext password authentication.
pub struct RaisinAuthHandler<V, P>
where
    V: ApiKeyValidator,
    P: ServerParameterProvider,
{
    validator: Arc<V>,
    parameter_provider: Arc<P>,
    contexts: Arc<RwLock<HashMap<String, ConnectionContext>>>,
}

impl<V, P> RaisinAuthHandler<V, P>
where
    V: ApiKeyValidator,
    P: ServerParameterProvider,
{
    /// Create a new authentication handler
    pub fn new(validator: V, parameter_provider: P) -> Self {
        Self {
            validator: Arc::new(validator),
            parameter_provider: Arc::new(parameter_provider),
            contexts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the connection context for a client
    pub fn get_context<C>(&self, client: &C) -> Option<ConnectionContext>
    where
        C: ClientInfo,
    {
        let key = client.socket_addr().to_string();
        let contexts = self.contexts.read().ok()?;
        contexts.get(&key).cloned()
    }

    /// Store connection context for a client
    fn store_context<C>(&self, client: &C, context: ConnectionContext)
    where
        C: ClientInfo,
    {
        let key = client.socket_addr().to_string();
        if let Ok(mut contexts) = self.contexts.write() {
            contexts.insert(key, context);
        }
    }

    /// Remove connection context when client disconnects
    pub fn remove_context<C>(&self, client: &C)
    where
        C: ClientInfo,
    {
        let key = client.socket_addr().to_string();
        if let Ok(mut contexts) = self.contexts.write() {
            contexts.remove(&key);
        }
    }

    /// Set identity auth context for a client (via SET app.user = '<jwt>')
    pub fn set_identity_auth<C>(&self, client: &C, auth: AuthContext)
    where
        C: ClientInfo,
    {
        let key = client.socket_addr().to_string();
        if let Ok(mut contexts) = self.contexts.write() {
            if let Some(ctx) = contexts.get_mut(&key) {
                ctx.set_identity_auth(auth);
            }
        }
    }

    /// Clear identity auth context for a client (via RESET app.user)
    pub fn clear_identity_auth<C>(&self, client: &C)
    where
        C: ClientInfo,
    {
        let key = client.socket_addr().to_string();
        if let Ok(mut contexts) = self.contexts.write() {
            if let Some(ctx) = contexts.get_mut(&key) {
                ctx.clear_identity_auth();
            }
        }
    }

    /// Set session branch for a client (via USE BRANCH or SET app.branch)
    pub fn set_session_branch<C>(&self, client: &C, branch: String)
    where
        C: ClientInfo,
    {
        let key = client.socket_addr().to_string();
        if let Ok(mut contexts) = self.contexts.write() {
            if let Some(ctx) = contexts.get_mut(&key) {
                ctx.set_session_branch(branch);
            }
        }
    }

    /// Clear session branch for a client (returns to default)
    pub fn clear_session_branch<C>(&self, client: &C)
    where
        C: ClientInfo,
    {
        let key = client.socket_addr().to_string();
        if let Ok(mut contexts) = self.contexts.write() {
            if let Some(ctx) = contexts.get_mut(&key) {
                ctx.clear_session_branch();
            }
        }
    }

    /// Extract tenant_id and repository from login info
    fn parse_login_info(login: &LoginInfo) -> Option<(String, String)> {
        let tenant_id = login.user()?.to_string();
        let repository = login.database()?.to_string();
        Some((tenant_id, repository))
    }
}

#[async_trait]
impl<V, P> StartupHandler for RaisinAuthHandler<V, P>
where
    V: ApiKeyValidator + 'static,
    P: ServerParameterProvider + 'static,
{
    async fn on_startup<C>(
        &self,
        client: &mut C,
        message: PgWireFrontendMessage,
    ) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        match message {
            PgWireFrontendMessage::Startup(ref startup) => {
                save_startup_parameters_to_metadata(client, startup);
                client.set_state(PgWireConnectionState::AuthenticationInProgress);
                client
                    .send(PgWireBackendMessage::Authentication(
                        Authentication::CleartextPassword,
                    ))
                    .await?;
            }

            PgWireFrontendMessage::PasswordMessageFamily(pwd) => {
                let pwd = pwd.into_password()?;
                let api_key = pwd.password;

                let login_info = LoginInfo::from_client_info(client);

                let (provided_tenant_id, repository) = Self::parse_login_info(&login_info)
                    .ok_or_else(|| {
                        PgWireError::UserError(Box::new(ErrorInfo::new(
                            "FATAL".to_string(),
                            "28000".to_string(),
                            "Missing username or database in connection string".to_string(),
                        )))
                    })?;

                let validation_result = self
                    .validator
                    .validate_api_key_scoped(&api_key)
                    .await
                    .map_err(|e| {
                        PgWireError::UserError(Box::new(ErrorInfo::new(
                            "FATAL".to_string(),
                            "XX000".to_string(),
                            format!("Authentication error: {}", e),
                        )))
                    })?;

                let validated = validation_result.ok_or_else(|| {
                    PgWireError::UserError(Box::new(ErrorInfo::new(
                        "FATAL".to_string(),
                        "28P01".to_string(),
                        "Invalid API key".to_string(),
                    )))
                })?;

                let user_id = validated.user_id.clone();
                let api_key_tenant_id = validated.tenant_id.clone();

                if api_key_tenant_id != provided_tenant_id {
                    let error_info = ErrorInfo::new(
                        "FATAL".to_string(),
                        "28000".to_string(),
                        format!(
                            "Tenant ID mismatch: API key belongs to '{}' but connection uses '{}'",
                            api_key_tenant_id, provided_tenant_id
                        ),
                    );
                    let error = ErrorResponse::from(error_info);
                    client
                        .feed(PgWireBackendMessage::ErrorResponse(error))
                        .await?;
                    client.close().await?;
                    return Ok(());
                }

                // A SCOPED key works in its one repository and nowhere else,
                // and its scope is its whole grant: there is no admin user
                // whose pgwire flag could add to (or vouch for) it.
                if let Some(scope_repo) = validated.scope_repo.as_deref() {
                    if scope_repo != repository {
                        let error_info = ErrorInfo::new(
                            "FATAL".to_string(),
                            "28000".to_string(),
                            format!(
                                "This API key is scoped to repository '{}' and cannot connect to '{}'",
                                scope_repo, repository
                            ),
                        );
                        client
                            .feed(PgWireBackendMessage::ErrorResponse(ErrorResponse::from(
                                error_info,
                            )))
                            .await?;
                        client.close().await?;
                        return Ok(());
                    }
                    let Some(key_auth) = validated.key_auth.clone() else {
                        return Err(PgWireError::UserError(Box::new(ErrorInfo::new(
                            "FATAL".to_string(),
                            "XX000".to_string(),
                            "Scoped API key has no resolved context".to_string(),
                        ))));
                    };
                    let context = ConnectionContext::new(api_key_tenant_id, user_id, repository)
                        .with_key_auth(key_auth);
                    self.store_context(client, context);
                    finish_authentication(client, self.parameter_provider.as_ref()).await;
                    return Ok(());
                }

                let has_access = self
                    .validator
                    .has_pgwire_access(&api_key_tenant_id, &user_id)
                    .await
                    .map_err(|e| {
                        PgWireError::UserError(Box::new(ErrorInfo::new(
                            "FATAL".to_string(),
                            "XX000".to_string(),
                            format!("Error checking permissions: {}", e),
                        )))
                    })?;

                if !has_access {
                    let error_info = ErrorInfo::new(
                        "FATAL".to_string(),
                        "28000".to_string(),
                        "User does not have pgwire access permission".to_string(),
                    );
                    let error = ErrorResponse::from(error_info);
                    client
                        .feed(PgWireBackendMessage::ErrorResponse(error))
                        .await?;
                    client.close().await?;
                    return Ok(());
                }

                let context = ConnectionContext::new(api_key_tenant_id, user_id, repository);
                self.store_context(client, context);

                finish_authentication(client, self.parameter_provider.as_ref()).await;
            }

            _ => {}
        }

        Ok(())
    }
}
