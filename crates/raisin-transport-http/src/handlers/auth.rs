// SPDX-License-Identifier: BSL-1.1

//! Authentication HTTP handlers
//!
//! These endpoints manage admin user authentication for the admin console and API access.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Html,
    Extension, Json,
};
use raisin_models::admin_user::{AdminAccessFlags, AdminInterface};
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, state::AppState};

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::AdminClaims;

/// Request to authenticate a user
#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    /// Username
    pub username: String,

    /// Password
    pub password: String,

    /// Interface requesting access (console, cli, or api)
    #[serde(default = "default_interface")]
    pub interface: AdminInterface,
}

fn default_interface() -> AdminInterface {
    AdminInterface::Console
}

/// Response from authentication
#[derive(Debug, Serialize)]
pub struct AuthResponse {
    /// JWT token
    pub token: String,

    /// User ID
    pub user_id: String,

    /// Username
    pub username: String,

    /// Whether the user must change password on first login
    pub must_change_password: bool,

    /// Token expiry time (Unix timestamp)
    pub expires_at: i64,

    /// Admin access flags (console, cli, api, pgwire, impersonate)
    pub access_flags: AdminAccessFlags,
}

/// Request to change password
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    /// Current password
    pub old_password: String,

    /// New password
    pub new_password: String,
}

/// Authenticate a user with username and password
///
/// # Endpoint
/// POST /api/raisindb/sys/{tenant_id}/auth
///
/// # Body
/// ```json
/// {
///   "username": "admin",
///   "password": "password123",
///   "interface": "console"
/// }
/// ```
///
/// # Response
/// ```json
/// {
///   "token": "eyJ0eXAiOiJKV1QiLCJhbGc...",
///   "user_id": "uuid",
///   "username": "admin",
///   "must_change_password": false,
///   "expires_at": 1234567890
/// }
/// ```
#[cfg(feature = "storage-rocksdb")]
pub async fn authenticate(
    State(state): State<AppState>,
    Path(tenant_id): Path<String>,
    Json(req): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    // Authenticate user (errors automatically converted via From impl)
    let (user, token) = auth_service
        .authenticate(&tenant_id, &req.username, &req.password, req.interface)
        .map_err(ApiError::from)?;

    // Decode token to get expiry
    let claims = auth_service
        .validate_token(&token)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(AuthResponse {
        token,
        user_id: user.user_id,
        username: user.username,
        must_change_password: user.must_change_password,
        expires_at: claims.exp,
        access_flags: user.access_flags,
    }))
}

/// Change password for the authenticated user
///
/// # Endpoint
/// POST /api/raisindb/sys/{tenant_id}/auth/change-password
///
/// # Headers
/// Authorization: Bearer {jwt_token}
///
/// # Body
/// ```json
/// {
///   "old_password": "current_password",
///   "new_password": "new_secure_password123!"
/// }
/// ```
///
/// # Security
/// This endpoint requires authentication via JWT middleware.
/// The username is extracted from the validated JWT token.
#[cfg(feature = "storage-rocksdb")]
pub async fn change_password(
    State(state): State<AppState>,
    Path(tenant_id): Path<String>,
    Extension(claims): Extension<AdminClaims>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    // Extract username from JWT claims (set by auth middleware)
    let username = &claims.username;

    // Verify tenant_id matches the token
    if claims.tenant_id != tenant_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "TENANT_MISMATCH",
            "Token tenant does not match requested tenant",
        ));
    }

    auth_service
        .change_password(&tenant_id, username, &req.old_password, &req.new_password)
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

/// Query parameters for CLI auth page
#[derive(Debug, Deserialize)]
pub struct CliAuthQuery {
    /// Optional callback port (if not using default 9999)
    pub port: Option<u16>,
}

/// Form data for CLI login
#[derive(Debug, Deserialize)]
pub struct CliLoginForm {
    pub username: String,
    pub password: String,
    /// Callback port
    pub port: u16,
}

/// Default CLI callback port
const DEFAULT_CLI_PORT: u16 = 9999;

/// Brand colors from admin-console theme
const PRIMARY_COLOR: &str = "#B8754E";
const SECONDARY_COLOR: &str = "#D97706";
const ACCENT_COLOR: &str = "#EA580C";

/// Serve CLI login page
///
/// # Endpoint
/// GET /auth/cli or GET /auth/cli?port=<port>
///
/// This serves an HTML login page for CLI authentication.
/// After successful login, sends token to CLI callback and shows success page.
#[cfg(feature = "storage-rocksdb")]
pub async fn cli_auth_page(Query(query): Query<CliAuthQuery>) -> Html<String> {
    // Use provided port or default
    let port = query.port.unwrap_or(DEFAULT_CLI_PORT);

    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>RaisinDB CLI Login</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            background: linear-gradient(135deg, #1a1a1a 0%, #2d2d2d 100%);
        }}
        .container {{
            background: rgba(255, 255, 255, 0.05);
            backdrop-filter: blur(10px);
            border: 1px solid rgba(184, 117, 78, 0.2);
            border-radius: 16px;
            padding: 2.5rem;
            width: 100%;
            max-width: 400px;
            box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.5);
        }}
        .logo {{
            text-align: center;
            margin-bottom: 2rem;
        }}
        .logo h1 {{
            font-size: 2rem;
            font-weight: 700;
            background: linear-gradient(135deg, {PRIMARY_COLOR}, {SECONDARY_COLOR}, {ACCENT_COLOR});
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
            background-clip: text;
        }}
        .logo p {{
            color: #888;
            font-size: 0.875rem;
            margin-top: 0.5rem;
        }}
        .form-group {{
            margin-bottom: 1.25rem;
        }}
        label {{
            display: block;
            color: #aaa;
            font-size: 0.875rem;
            margin-bottom: 0.5rem;
        }}
        input[type="text"],
        input[type="password"] {{
            width: 100%;
            padding: 0.75rem 1rem;
            background: rgba(0, 0, 0, 0.3);
            border: 1px solid rgba(255, 255, 255, 0.1);
            border-radius: 8px;
            color: #fff;
            font-size: 1rem;
            transition: border-color 0.2s, box-shadow 0.2s;
        }}
        input:focus {{
            outline: none;
            border-color: {PRIMARY_COLOR};
            box-shadow: 0 0 0 3px rgba(184, 117, 78, 0.2);
        }}
        button {{
            width: 100%;
            padding: 0.875rem;
            background: linear-gradient(135deg, {PRIMARY_COLOR}, {SECONDARY_COLOR});
            border: none;
            border-radius: 8px;
            color: #fff;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: transform 0.2s, box-shadow 0.2s;
        }}
        button:hover {{
            transform: translateY(-1px);
            box-shadow: 0 4px 12px rgba(184, 117, 78, 0.4);
        }}
        button:active {{
            transform: translateY(0);
        }}
        .error {{
            background: rgba(239, 68, 68, 0.1);
            border: 1px solid rgba(239, 68, 68, 0.3);
            color: #ef4444;
            padding: 0.75rem;
            border-radius: 8px;
            margin-bottom: 1rem;
            font-size: 0.875rem;
            display: none;
        }}
        .footer {{
            text-align: center;
            margin-top: 1.5rem;
            color: #666;
            font-size: 0.75rem;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="logo">
            <h1>🍇 RaisinDB</h1>
            <p>Sign in to continue to CLI</p>
        </div>

        <div id="error" class="error"></div>

        <form id="loginForm" action="/auth/cli/login" method="POST">
            <input type="hidden" name="port" value="{port}" />

            <div class="form-group">
                <label for="username">Username</label>
                <input type="text" id="username" name="username" required autofocus />
            </div>

            <div class="form-group">
                <label for="password">Password</label>
                <input type="password" id="password" name="password" required />
            </div>

            <button type="submit">Sign In</button>
        </form>

        <div class="footer">
            Authorizing CLI access to RaisinDB
        </div>
    </div>

    <script>
        // Add loading state on form submit (form posts directly, browser handles redirect)
        document.getElementById('loginForm').addEventListener('submit', function() {{
            const button = this.querySelector('button');
            button.textContent = 'Signing in...';
            button.disabled = true;
        }});
    </script>
</body>
</html>"#,
        port = port,
        PRIMARY_COLOR = PRIMARY_COLOR,
        SECONDARY_COLOR = SECONDARY_COLOR,
        ACCENT_COLOR = ACCENT_COLOR,
    ))
}

/// Handle CLI login form submission
///
/// # Endpoint
/// POST /auth/cli/login
///
/// Authenticates user, sends token to CLI callback, and shows success page.
#[cfg(feature = "storage-rocksdb")]
pub async fn cli_auth_login(
    State(state): State<AppState>,
    axum::Form(form): axum::Form<CliLoginForm>,
) -> Result<Html<String>, ApiError> {
    let auth_service = state
        .auth_service()
        .ok_or_else(|| ApiError::internal("Authentication service not available"))?;

    // Use default tenant for CLI auth
    let tenant_id = "default";

    // Authenticate user
    let (_user, token) = auth_service
        .authenticate(
            tenant_id,
            &form.username,
            &form.password,
            AdminInterface::Cli,
        )
        .map_err(ApiError::from)?;

    // Build callback URL with token
    let callback_url = format!(
        "http://localhost:{}/auth/callback?token={}",
        form.port,
        urlencoding::encode(&token)
    );

    // Send token to CLI callback in background (fire and forget)
    let client = reqwest::Client::new();
    let _ = client.get(&callback_url).send().await;

    // Return success page to browser
    Ok(Html(r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>RaisinDB CLI - Authenticated</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            background: linear-gradient(135deg, #1a1a1a 0%, #2d2d2d 100%);
        }
        .container {
            background: rgba(255, 255, 255, 0.05);
            backdrop-filter: blur(10px);
            border: 1px solid rgba(16, 185, 129, 0.3);
            border-radius: 16px;
            padding: 3rem;
            text-align: center;
            box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.5);
        }
        .success-icon {
            font-size: 4rem;
            margin-bottom: 1.5rem;
        }
        h1 {
            color: #10b981;
            font-size: 1.75rem;
            margin-bottom: 0.75rem;
        }
        p {
            color: #888;
            font-size: 1rem;
            line-height: 1.6;
        }
        .hint {
            margin-top: 1.5rem;
            padding-top: 1.5rem;
            border-top: 1px solid rgba(255, 255, 255, 0.1);
            color: #666;
            font-size: 0.875rem;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="success-icon">✓</div>
        <h1>Authentication Successful!</h1>
        <p>RaisinDB CLI has been authorized.</p>
        <p class="hint">You can close this window and return to your terminal.</p>
    </div>
</body>
</html>"#.to_string()))
}
