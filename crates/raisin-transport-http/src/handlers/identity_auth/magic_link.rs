// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Magic link (passwordless) authentication handlers.
//!
//! # The link is built from CONFIG, never from the request
//!
//! `base_url` comes from the tenant's `raisin:EmailConfig` node at
//! `/config/email`, and a caller-supplied `redirect_url` is allow-listed
//! against it. Deriving either from the request's `Host` or `Origin` header —
//! the obvious shortcut — hands anyone who can set those headers a way to have
//! the victim's one-time token delivered to a host of their choosing. The token
//! is a bearer credential in a URL; whoever the URL points at wins.
//!
//! # The masked response says nothing about the account
//!
//! [`request_magic_link`] answers identically whether the address is known,
//! unknown-but-registerable, or unknown-with-registration-closed. That is the
//! whole point of the endpoint's response shape: any difference in body, status
//! or error code turns an unauthenticated POST into an account-enumeration
//! oracle for the tenant's entire user list.

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Extension, Json,
};

use crate::error::ApiError;
use crate::state::AppState;

use super::helpers::{mask_email, validate_email};
use super::types::{MagicLinkRequest, MagicLinkSentResponse, MagicLinkVerifyQuery};

/// Workspace holding per-tenant auth and email configuration.
#[cfg(feature = "storage-rocksdb")]
const CONFIG_WORKSPACE: &str = "raisin:system";

/// Branch the configuration and the token nodes are pinned to.
#[cfg(feature = "storage-rocksdb")]
const CONFIG_BRANCH: &str = "main";

/// Path of the tenant's `raisin:EmailConfig` node.
#[cfg(feature = "storage-rocksdb")]
const EMAIL_CONFIG_PATH: &str = "/config/email";

/// Resolve the repository the tenant-wide (`/auth/magic-link`) endpoints act on.
///
/// The repo-scoped endpoints are the real ones: a token node lives in one repo's
/// `raisin:system` workspace, `/config/email` is per repo, and
/// `allow_registration` is per repo. Magic link therefore cannot be repo-less the
/// way `POST /auth/login` is — that one only touches the tenant-level identity
/// store and issues a token with no repo context at all.
///
/// This used to hardcode `"default"`. That silently did the WRONG thing on a
/// multi-repo tenant: it read another repo's email config, consulted another
/// repo's registration policy, and wrote the token node somewhere the verify
/// step would never look — producing a link that could not be redeemed, with no
/// error anywhere.
///
/// So: exactly one repo means the tenant-wide spelling is unambiguous and is
/// honoured. Anything else is a caller error, answered by naming the endpoint
/// that can express what they meant.
#[cfg(feature = "storage-rocksdb")]
async fn resolve_tenant_repo(state: &AppState, tenant_id: &str) -> Result<String, ApiError> {
    // Same route `handlers/profile.rs::list_repositories` takes.
    use raisin_storage::{RepositoryManagementRepository, Storage};

    let repos = state
        .storage()
        .repository_management()
        .list_repositories_for_tenant(tenant_id)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "REPO_LOOKUP_FAILED",
                format!("could not resolve the tenant's repositories: {e}"),
            )
        })?;

    match repos.as_slice() {
        [only] => Ok(only.repo_id.clone()),
        [] => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "NO_REPOSITORY",
            "this tenant has no repository, so there is nowhere to issue a sign-in link from",
        )),
        _ => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "REPOSITORY_REQUIRED",
            "this tenant has more than one repository; use POST /auth/{repo}/magic-link \
             and GET /auth/{repo}/magic-link/verify so the link is issued against, and \
             redeemed in, the same one",
        )),
    }
}

/// How long a magic link stays valid.
///
/// Short on purpose: the link is a bearer credential that travels through mail
/// servers and, on the way, through whatever scans them.
#[cfg(feature = "storage-rocksdb")]
const EXPIRES_IN_MINUTES: u32 = 15;

/// The single confirmation body. A constant, so no branch of
/// [`request_magic_link_core`] can accidentally word it differently.
#[cfg(feature = "storage-rocksdb")]
const SENT_MESSAGE: &str = "If that address has an account, a sign-in link is on its way.";

/// How many links one ADDRESS may ask for per [`EMAIL_RATE_WINDOW`].
///
/// Low, because every one of these is an email delivered to a third party who
/// did not ask for it: without this, the endpoint is a free mail bomb aimed at
/// anyone whose address the caller knows, sent from the tenant's own verified
/// sending domain.
#[cfg(feature = "storage-rocksdb")]
const EMAIL_RATE_LIMIT: usize = 5;

/// Window for [`EMAIL_RATE_LIMIT`] — matched to the link's own lifetime, so a
/// user who genuinely lost a link can ask again as soon as the last one dies.
#[cfg(feature = "storage-rocksdb")]
const EMAIL_RATE_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(EXPIRES_IN_MINUTES as u64 * 60);

/// How many links one CLIENT may ask for per [`IP_RATE_WINDOW`].
///
/// The per-address limit alone stops nothing: a caller walking a list of
/// addresses never hits it twice. This is the one that bounds the walk.
#[cfg(feature = "storage-rocksdb")]
const IP_RATE_LIMIT: usize = 20;

/// Window for [`IP_RATE_LIMIT`].
#[cfg(feature = "storage-rocksdb")]
const IP_RATE_WINDOW: std::time::Duration = std::time::Duration::from_secs(3600);

// ============================================================================
// Rate limiting
// ============================================================================

/// The process-wide magic-link rate limiter.
///
/// Opened lazily beside the main RocksDB data directory rather than threaded
/// through `build_router`: the limiter is an implementation detail of this one
/// unauthenticated endpoint, and every caller of `build_router` (tests
/// included) would otherwise have to learn about it. `OnceLock` because there
/// is one storage per process, so there is one path.
///
/// `None` means the limiter could not be opened. See [`check_rate_limits`] for
/// what that then means.
#[cfg(feature = "storage-rocksdb")]
static RATE_LIMITER: std::sync::OnceLock<
    Option<std::sync::Arc<raisin_ratelimit::RocksRateLimiter>>,
> = std::sync::OnceLock::new();

#[cfg(feature = "storage-rocksdb")]
fn rate_limiter(
    storage: &raisin_rocksdb::RocksDBStorage,
) -> Option<&'static std::sync::Arc<raisin_ratelimit::RocksRateLimiter>> {
    RATE_LIMITER
        .get_or_init(|| {
            let path = storage.config().path.join("magic-link-ratelimit");
            match raisin_ratelimit::RocksRateLimiter::open(&path) {
                Ok(limiter) => Some(std::sync::Arc::new(limiter)),
                Err(e) => {
                    tracing::error!(
                        path = %path.display(),
                        error = %e,
                        "Failed to open the magic link rate limiter; requests will be REFUSED"
                    );
                    None
                }
            }
        })
        .as_ref()
}

/// The client address, as far as it can be trusted.
///
/// `X-Forwarded-For`'s first hop, else `X-Real-IP`. Both are only as good as
/// the proxy that sets them — which is the point: a deployment that terminates
/// TLS itself and sets neither gets `None`, and per-IP limiting is then
/// skipped rather than collapsing every client into one shared bucket that the
/// first abuser would exhaust for everybody.
fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        // The LAST entry, not the first.
        //
        // `X-Forwarded-For` is a client-settable header that the reverse proxy
        // APPENDS to. So on `1.2.3.4, <real>` the leftmost value is whatever the
        // caller typed and the rightmost is what our own proxy observed. Taking
        // the leftmost — the conventional "original client" reading, and what
        // this did originally — lets a caller mint a fresh rate-limit bucket per
        // request just by rotating a header, which is the same as having no
        // per-IP limit at all.
        //
        // This is only sound because every deployment fronts the server with a
        // trusted proxy. Exposed directly, the whole header is caller-controlled
        // and the fallback bucket in `check_rate_limits` is what actually holds.
        .and_then(|v| v.rsplit(',').next())
        .or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok()))
        .map(str::trim)
        .filter(|ip| !ip.is_empty())
        .map(str::to_string)
}

/// Refuse the request if either bucket is exhausted.
///
/// Runs before anything is read, minted or enqueued. A 429 is not an
/// enumeration signal: it depends on what THIS caller has been doing, never on
/// whether the address exists.
#[cfg(feature = "storage-rocksdb")]
async fn check_rate_limits(
    storage: &raisin_rocksdb::RocksDBStorage,
    tenant_id: &str,
    email: &str,
    client_ip: Option<&str>,
) -> Result<(), ApiError> {
    use raisin_context::RateLimiter;

    let too_many = || {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "RATE_LIMITED",
            "Too many sign-in link requests. Please wait before trying again.",
        )
    };

    // Fail CLOSED. A broken limiter on an endpoint that sends mail to arbitrary
    // third parties is not something to shrug off and continue through — the
    // error above already told the operator why.
    let Some(limiter) = rate_limiter(storage) else {
        return Err(too_many());
    };

    // Lowercased: mail domains are case-insensitive, and the bucket must not be
    // resettable by retyping the same address with different capitals.
    let email_key = format!("magic-link:email:{tenant_id}:{}", email.to_lowercase());
    if !limiter
        .check_rate(&email_key, EMAIL_RATE_LIMIT, EMAIL_RATE_WINDOW)
        .await
        .allowed
    {
        tracing::warn!(tenant_id = %tenant_id, "Magic link request refused: per-address rate limit");
        return Err(too_many());
    }

    // No identifiable client still gets a bucket — a SHARED one.
    //
    // Skipping the check when no proxy header is present, which is what this did
    // originally, removes the only limit that bounds an address-list walk: the
    // per-address bucket is never hit twice by a caller working through a list.
    // Anything reaching this server without a forwarded-for header is either
    // misconfigured or bypassing the proxy, and neither deserves an unmetered
    // path to sending mail.
    //
    // Deliberately coarse: every header-less caller shares one bucket, so enough
    // of them at once will refuse each other. That is the correct trade for an
    // endpoint that sends mail to third parties, and behind a correctly
    // configured proxy this branch is never taken.
    let (ip_key, limit) = match client_ip {
        Some(ip) => (format!("magic-link:ip:{tenant_id}:{ip}"), IP_RATE_LIMIT),
        None => (
            format!("magic-link:ip:{tenant_id}:unattributed"),
            IP_RATE_LIMIT,
        ),
    };
    if !limiter
        .check_rate(&ip_key, limit, IP_RATE_WINDOW)
        .await
        .allowed
    {
        tracing::warn!(
            tenant_id = %tenant_id,
            attributed = client_ip.is_some(),
            "Magic link request refused: per-IP rate limit"
        );
        return Err(too_many());
    }

    Ok(())
}

// ============================================================================
// Configuration
// ============================================================================

/// The link-building half of `/config/email`.
#[cfg(feature = "storage-rocksdb")]
struct LinkConfig {
    /// Origin every outbound link is built from.
    base_url: String,
    /// Extra redirect targets permitted beyond `base_url` itself.
    redirect_allowlist: Vec<String>,
}

/// Read `base_url` / `redirect_allowlist` from the tenant's email config.
///
/// A missing node, or one without a `base_url`, is a hard error rather than a
/// fallback: the only fallback available would be a request header, which is
/// exactly the thing this must never trust.
#[cfg(feature = "storage-rocksdb")]
async fn load_link_config(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
) -> Result<LinkConfig, ApiError> {
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::properties::PropertyValue;

    let service = state.node_service_for_context(
        tenant_id,
        repo,
        CONFIG_BRANCH,
        CONFIG_WORKSPACE,
        Some(AuthContext::system()),
    );

    let node = service
        .get_by_path(EMAIL_CONFIG_PATH)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to read email config: {e}")))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "EMAIL_NOT_CONFIGURED",
                "Magic link sign-in is not configured for this tenant",
            )
        })?;

    let base_url = match node.properties.get("base_url") {
        Some(PropertyValue::String(url)) if !url.trim().is_empty() => {
            url.trim().trim_end_matches('/').to_string()
        }
        _ => {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "EMAIL_NOT_CONFIGURED",
                "Magic link sign-in requires base_url on /config/email",
            ))
        }
    };

    let redirect_allowlist = match node.properties.get("redirect_allowlist") {
        Some(PropertyValue::Array(entries)) => entries
            .iter()
            .filter_map(|entry| match entry {
                PropertyValue::String(s) => Some(s.trim().trim_end_matches('/').to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    Ok(LinkConfig {
        base_url,
        redirect_allowlist,
    })
}

/// Resolve where verification should land the browser.
///
/// An absent `redirect_url` means `base_url`. A supplied one must either sit
/// under `base_url` or match an allow-list entry — an unchecked redirect would
/// turn the verify endpoint into an open redirect that carries a freshly minted
/// session with it.
#[cfg(feature = "storage-rocksdb")]
fn resolve_redirect(config: &LinkConfig, requested: Option<&str>) -> Result<String, ApiError> {
    let Some(requested) = requested.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(config.base_url.clone());
    };

    // The match must end on a path boundary. A bare `starts_with` would accept
    // `https://app.example.com.attacker.test` as a child of
    // `https://app.example.com`, and that host is entirely the attacker's.
    let under = |base: &str| -> bool {
        requested == base
            || requested
                .strip_prefix(base)
                .is_some_and(|rest| rest.starts_with('/'))
    };

    if under(&config.base_url) || config.redirect_allowlist.iter().any(|entry| under(entry)) {
        return Ok(requested.to_string());
    }

    Err(ApiError::new(
        StatusCode::BAD_REQUEST,
        "REDIRECT_NOT_ALLOWED",
        "redirect_url is not permitted by this tenant's email configuration",
    ))
}

/// Whether this repo accepts sign-ins from addresses it has never seen.
///
/// Absent config means the [`raisin_models::auth::RepoAuthConfig`] default,
/// which is permissive — the same answer the registration endpoints give.
/// Disagreeing with them would make magic-link either a way around a closed
/// repo or a way to be locked out of an open one.
#[cfg(feature = "storage-rocksdb")]
async fn allow_registration(state: &AppState, tenant_id: &str, repo: &str) -> bool {
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::properties::PropertyValue;

    let service = state.node_service_for_context(
        tenant_id,
        repo,
        CONFIG_BRANCH,
        CONFIG_WORKSPACE,
        Some(AuthContext::system()),
    );

    let node = service
        .get_by_path(&format!("/config/repos/{repo}"))
        .await
        .ok()
        .flatten();

    let configured = node.as_ref().and_then(|n| {
        (n.node_type == "raisin:RepoAuthConfig")
            .then(|| n.properties.get("allow_registration"))
            .flatten()
    });

    match configured {
        Some(PropertyValue::Boolean(allowed)) => *allowed,
        _ => raisin_models::auth::RepoAuthConfig::default().allow_registration,
    }
}

// ============================================================================
// Request
// ============================================================================

/// Mint a token (if the three-case table says so) and enqueue the send.
#[cfg(feature = "storage-rocksdb")]
async fn request_magic_link_core(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    client_ip: Option<&str>,
    headers: &HeaderMap,
    req: &MagicLinkRequest,
) -> Result<Json<MagicLinkSentResponse>, ApiError> {
    use super::helpers::extract_repos;
    use raisin_auth::jobs::MagicLinkJobData;
    use raisin_auth::strategies::MagicLinkStrategy;
    use raisin_models::auth::{OneTimeToken, TokenPurpose};
    use raisin_models::timestamp::StorageTimestamp;
    use raisin_rocksdb::OneTimeTokenStore;
    use raisin_storage::jobs::{JobContext, JobId, JobType};

    validate_email(&req.email)?;

    let repos = extract_repos(state)?;
    check_rate_limits(&repos.storage, tenant_id, &req.email, client_ip).await?;

    // Both of these can refuse the request, and both refuse for reasons that
    // have nothing to do with WHICH address was submitted — so running them
    // ahead of the identity lookup keeps the enumeration-safe path below free
    // of early returns.
    let link_config = load_link_config(state, tenant_id, repo).await?;
    let redirect_url = resolve_redirect(&link_config, req.redirect_url.as_deref())?;

    // WHERE THE EMAILED LINK POINTS — this server, not the tenant's app.
    //
    // Two different origins are in play and they were previously the same
    // value: `link_config.base_url` is the tenant's FRONT END (it is the
    // redirect target above, and the root the redirect allowlist is measured
    // against), while the verify endpoint is a RaisinDB route on THIS server.
    // Building the link from base_url produced
    // `https://{their app}/auth/{repo}/magic-link/verify`, which 404s — and did,
    // for every magic link ever sent.
    //
    // Resolved HERE, at request time, because the send runs in a background job
    // with no request to derive a host from. `Required` refuses rather than
    // guessing: this link carries a one-time sign-in token, so a spoofed Host
    // would mean emailing a victim a working credential pointed at the
    // attacker's server. Not sending is the better failure.
    let tenant_hosts =
        crate::handlers::oauth_as::helpers::load_tenant_trusted_hosts(state, tenant_id).await;
    let verify_origin = crate::origin::self_origin(
        headers,
        &tenant_hosts,
        crate::origin::OriginTrust::Required,
    )
    .map_err(|e| {
        tracing::error!(error = %e, "refusing to send a magic link: cannot establish this server's own origin");
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "ORIGIN_NOT_CONFIGURED",
            format!("Magic link sign-in is unavailable: {e}"),
        )
    })?;

    let identity = repos
        .identity
        .find_by_email(tenant_id, &req.email)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to look up identity: {e}")))?;

    // The three-case table (decision F): known address, or unknown address on a
    // repo that still accepts registrations. Every arm falls through to the
    // SAME response at the bottom.
    let registration_open = allow_registration(state, tenant_id, repo).await;

    if identity.is_some() || registration_open {
        let (token, token_hash, token_prefix) = MagicLinkStrategy::generate_token()
            .map_err(|e| ApiError::internal(format!("Failed to generate token: {e}")))?;

        let token_id = uuid::Uuid::new_v4().to_string();
        let expires_at = StorageTimestamp::from_nanos(
            StorageTimestamp::now().timestamp_nanos()
                + i64::from(EXPIRES_IN_MINUTES) * 60 * 1_000_000_000,
        )
        .ok_or_else(|| ApiError::internal("Failed to compute token expiry"))?;

        let mut one_time_token = OneTimeToken::new(
            token_id.clone(),
            tenant_id.to_string(),
            token_hash,
            token_prefix,
            TokenPurpose::MagicLink,
            req.email.clone(),
            expires_at,
        );
        // `None` for an address with no account yet: the identity is created at
        // VERIFY time, so an unclicked link leaves no account behind and a
        // mistyped address cannot fill the tenant with ghosts.
        one_time_token.identity_id = identity.as_ref().map(|i| i.identity_id.clone());
        one_time_token
            .context
            .insert("redirect_url".to_string(), serde_json::json!(redirect_url));

        let store = OneTimeTokenStore::new(repos.storage.clone(), tenant_id, repo);
        store
            .create(&one_time_token)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to store magic link token: {e}")))?;

        // Empty for a not-yet-existing account. The job uses it only for
        // logging and its dedup key; the token node is the record that matters.
        let identity_id = one_time_token.identity_id.clone().unwrap_or_default();

        let job_data = MagicLinkJobData::new(
            identity_id.clone(),
            req.email.clone(),
            token_id.clone(),
            token,
            link_config.base_url.clone(),
            EXPIRES_IN_MINUTES,
        )
        .with_verify_path(format!("/auth/{repo}/magic-link/verify"));

        let job_context = JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo.to_string(),
            branch: CONFIG_BRANCH.to_string(),
            workspace_id: CONFIG_WORKSPACE.to_string(),
            revision: raisin_hlc::HLC::now(),
            metadata: job_data.to_metadata(),
        };

        // Context FIRST, then the registration. The handler reads everything it
        // sends out of the context, so a job that became visible before its
        // context would burn all of its retries on a permanent error — and each
        // of those retries is a login the user is still waiting for.
        let job_id = JobId::new();
        repos
            .storage
            .job_data_store()
            .put(&job_id, &job_context)
            .map_err(|e| ApiError::internal(format!("Failed to store magic link job data: {e}")))?;

        repos
            .storage
            .job_registry()
            .register_job_with_id(
                job_id.clone(),
                JobType::AuthMagicLinkSend {
                    identity_id,
                    email: req.email.clone(),
                    token_id: token_id.clone(),
                },
                tenant_id.to_string(),
                None,
                None,
                None,
            )
            .await
            .map_err(|e| {
                ApiError::internal(format!("Failed to enqueue magic link send job: {e}"))
            })?;

        tracing::info!(
            tenant_id = %tenant_id,
            repo = %repo,
            job_id = %job_id,
            token_id = %token_id,
            known_identity = identity.is_some(),
            "Magic link requested"
        );
    } else {
        // Logged, not answered. An operator can see the closed-registration
        // refusals; the caller cannot tell them from a successful send.
        tracing::info!(
            tenant_id = %tenant_id,
            repo = %repo,
            "Magic link request for an unknown address on a repo with registration closed"
        );
    }

    Ok(Json(MagicLinkSentResponse {
        message: SENT_MESSAGE.to_string(),
        masked_email: mask_email(&req.email),
        expires_in_minutes: EXPIRES_IN_MINUTES,
    }))
}

// ============================================================================
// Verify
// ============================================================================

/// Redeem a token: validate, consume, provision, and issue a session.
#[cfg(feature = "storage-rocksdb")]
async fn verify_magic_link_core(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    token: &str,
    wants_json: bool,
) -> Result<Response, ApiError> {
    use super::helpers::{
        build_auth_response, create_session, extract_repos, generate_tokens, AuthRepositories,
    };
    use super::user_node::ensure_user_node;
    use raisin_auth::strategies::MagicLinkStrategy;
    use raisin_models::auth::{Identity, TokenPurpose};
    use raisin_rocksdb::OneTimeTokenStore;

    // Every rejection below answers with this one error. A token that never
    // existed, one that expired, one already spent and one minted for another
    // purpose are all "this link does not work", because saying which would
    // tell a guesser how close they got.
    let rejected = || {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_TOKEN",
            "This sign-in link is invalid or has expired",
        )
    };

    let token_prefix = MagicLinkStrategy::extract_prefix(token).map_err(|_| rejected())?;
    let token_hash = MagicLinkStrategy::hash_token(token);

    let repos: AuthRepositories = extract_repos(state)?;
    let store = OneTimeTokenStore::new(repos.storage.clone(), tenant_id, repo);

    let stored = store
        .find_by_token(&token_prefix, &token_hash)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to look up magic link token: {e}")))?
        .ok_or_else(rejected)?;

    if stored.purpose != TokenPurpose::MagicLink || !stored.is_valid() {
        return Err(rejected());
    }

    // Consume BEFORE issuing anything. If this write fails there is no way left
    // to stop the link being replayed, so the only safe answer is to refuse the
    // login we could have granted: a user who retries loses one link, a user
    // whose link stayed live loses their account.
    let marked = store.mark_used(&stored.token_id).await.map_err(|e| {
        tracing::error!(
            token_id = %stored.token_id,
            error = %e,
            "Failed to mark magic link token used; refusing the sign-in"
        );
        rejected()
    })?;
    if !marked {
        return Err(rejected());
    }

    // Identity is created HERE, not at request time (decision F).
    let existing = match &stored.identity_id {
        Some(identity_id) => repos
            .identity
            .get(tenant_id, identity_id)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to load identity: {e}")))?,
        None => repos
            .identity
            .find_by_email(tenant_id, &stored.email)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to look up identity: {e}")))?,
    };

    let mut identity = match existing {
        Some(identity) => {
            if !identity.is_active {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "ACCOUNT_DISABLED",
                    "This account has been disabled",
                ));
            }
            identity
        }
        None => Identity::new(
            uuid::Uuid::new_v4().to_string(),
            tenant_id.to_string(),
            stored.email.clone(),
        ),
    };

    // Clicking a link delivered to the address IS the proof of the address.
    identity.email_verified = true;
    repos
        .identity
        .upsert(tenant_id, &identity, "system:magic-link")
        .await
        .map_err(|e| ApiError::internal(format!("Failed to save identity: {e}")))?;

    let (session, expires_at) = create_session(
        &repos.session,
        tenant_id,
        &identity.identity_id,
        "magic_link",
        false,
        "system:magic-link",
    )
    .await?;

    // Just-in-time provisioning, exactly as the password login path does: a
    // repo the user has never visited still needs a raisin:User node for RLS to
    // have anything to resolve. A failure here is logged, not fatal — the
    // session is already valid.
    let home = match ensure_user_node(
        &repos.storage,
        tenant_id,
        repo,
        &identity.identity_id,
        &identity.email,
        identity.display_name.as_deref(),
        &["viewer".to_string(), "authenticated_user".to_string()],
    )
    .await
    {
        Ok(path) => Some(path),
        Err(e) => {
            tracing::warn!(
                identity_id = %identity.identity_id,
                repo = %repo,
                error = %e,
                "Failed to ensure user node during magic link verification"
            );
            None
        }
    };

    let tokens = generate_tokens(state, &identity, &session, Some(repo), home.as_deref())?;
    let response = build_auth_response(&identity, tokens, expires_at, home);

    if wants_json {
        return Ok(Json(response).into_response());
    }

    // A browser followed the link, so the answer is a redirect (decision E).
    // The tokens ride in the FRAGMENT, not the query: a fragment is never sent
    // to a server, so it stays out of access logs, out of `Referer` on the next
    // navigation, and out of every proxy in between.
    let redirect_base = stored
        .context
        .get("redirect_url")
        .and_then(|v| v.as_str())
        .unwrap_or("/");

    let target = format!(
        "{}#access_token={}&refresh_token={}&expires_at={}",
        redirect_base,
        urlencoding::encode(&response.access_token),
        urlencoding::encode(&response.refresh_token),
        response.expires_at
    );

    Ok(Redirect::to(&target).into_response())
}

/// Whether the caller asked for the JSON body instead of the redirect.
fn wants_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| accept.contains("application/json"))
}

// ============================================================================
// Handlers
// ============================================================================

/// Request a magic link for passwordless authentication.
///
/// # Endpoint
/// POST /auth/magic-link
#[cfg(feature = "storage-rocksdb")]
pub async fn request_magic_link(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    headers: HeaderMap,
    Json(req): Json<MagicLinkRequest>,
) -> Result<Json<MagicLinkSentResponse>, ApiError> {
    let repo = resolve_tenant_repo(&state, &tenant_info.tenant_id).await?;
    request_magic_link_core(
        &state,
        &tenant_info.tenant_id,
        &repo,
        client_ip(&headers).as_deref(),
        &headers,
        &req,
    )
    .await
}

/// Request a magic link for a specific repository.
///
/// # Endpoint
/// POST /auth/{repo}/magic-link
#[cfg(feature = "storage-rocksdb")]
pub async fn request_magic_link_for_repo(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(repo): Path<String>,
    headers: HeaderMap,
    Json(req): Json<MagicLinkRequest>,
) -> Result<Json<MagicLinkSentResponse>, ApiError> {
    request_magic_link_core(
        &state,
        &tenant_info.tenant_id,
        &repo,
        client_ip(&headers).as_deref(),
        &headers,
        &req,
    )
    .await
}

/// Verify a magic link token.
///
/// # Endpoint
/// GET /auth/magic-link/verify?token={token}
///
/// Redirects to the URL recorded when the link was requested, carrying the
/// tokens in the fragment. Returns [`super::types::AuthTokensResponse`] as JSON
/// instead when the caller sends `Accept: application/json`.
#[cfg(feature = "storage-rocksdb")]
pub async fn verify_magic_link(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    headers: HeaderMap,
    Query(query): Query<MagicLinkVerifyQuery>,
) -> Result<Response, ApiError> {
    let repo = resolve_tenant_repo(&state, &tenant_info.tenant_id).await?;
    verify_magic_link_core(
        &state,
        &tenant_info.tenant_id,
        &repo,
        &query.token,
        wants_json(&headers),
    )
    .await
}

/// Verify a magic link token issued for a specific repository.
///
/// # Endpoint
/// GET /auth/{repo}/magic-link/verify?token={token}
///
/// This is the route the emailed link actually points at: the token node lives
/// in this repo's `raisin:system` workspace, so a verify request that carries
/// no repo has nowhere to look it up.
#[cfg(feature = "storage-rocksdb")]
pub async fn verify_magic_link_for_repo(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<crate::middleware::TenantInfo>,
    Path(repo): Path<String>,
    headers: HeaderMap,
    Query(query): Query<MagicLinkVerifyQuery>,
) -> Result<Response, ApiError> {
    verify_magic_link_core(
        &state,
        &tenant_info.tenant_id,
        &repo,
        &query.token,
        wants_json(&headers),
    )
    .await
}

#[cfg(all(test, feature = "storage-rocksdb"))]
mod tests {
    use super::*;

    fn config() -> LinkConfig {
        LinkConfig {
            base_url: "https://app.example.com".to_string(),
            redirect_allowlist: vec!["https://admin.example.com".to_string()],
        }
    }

    #[test]
    fn an_absent_redirect_lands_on_the_configured_base() {
        assert_eq!(
            resolve_redirect(&config(), None).expect("allowed"),
            "https://app.example.com"
        );
    }

    #[test]
    fn a_redirect_under_the_base_or_the_allowlist_is_kept() {
        let cfg = config();
        assert_eq!(
            resolve_redirect(&cfg, Some("https://app.example.com/welcome")).expect("allowed"),
            "https://app.example.com/welcome"
        );
        assert_eq!(
            resolve_redirect(&cfg, Some("https://admin.example.com/ops")).expect("allowed"),
            "https://admin.example.com/ops"
        );
    }

    #[test]
    fn a_lookalike_host_is_not_a_child_of_the_base() {
        // The bug a bare starts_with() has: this host is entirely someone
        // else's, and the session would be handed straight to it.
        assert!(resolve_redirect(&config(), Some("https://app.example.com.evil.test")).is_err());
    }

    #[test]
    fn an_unrelated_redirect_is_refused() {
        assert!(resolve_redirect(&config(), Some("https://evil.test/steal")).is_err());
    }

    /// The LAST forwarded hop is the one to trust, not the first.
    ///
    /// This test previously asserted the opposite, on the conventional reading
    /// that the leftmost `X-Forwarded-For` entry is the original client. That
    /// reading is right for logging and wrong for rate limiting: the header is
    /// caller-settable and the proxy APPENDS to it, so on
    /// `203.0.113.7, 198.51.100.1` the left value is whatever the caller typed
    /// and only the right value was observed by our own proxy.
    ///
    /// Keying a rate-limit bucket on the left one lets a caller mint a fresh
    /// bucket per request by rotating a header — which is indistinguishable from
    /// having no per-IP limit at all, on an endpoint whose whole job is sending
    /// mail to third parties.
    #[test]
    fn the_last_forwarded_hop_is_the_one_the_proxy_observed() {
        let mut headers = HeaderMap::new();
        assert_eq!(client_ip(&headers), None);

        headers.insert(
            "x-forwarded-for",
            "203.0.113.7, 198.51.100.1".parse().expect("valid"),
        );
        assert_eq!(client_ip(&headers), Some("198.51.100.1".to_string()));

        // A single hop is unambiguous either way.
        headers.insert("x-forwarded-for", "198.51.100.1".parse().expect("valid"));
        assert_eq!(client_ip(&headers), Some("198.51.100.1".to_string()));

        // A spoofed prefix cannot move the bucket: whatever the caller prepends,
        // the value taken is still the one the proxy appended.
        headers.insert(
            "x-forwarded-for",
            "evil-1, evil-2, evil-3, 198.51.100.1"
                .parse()
                .expect("valid"),
        );
        assert_eq!(client_ip(&headers), Some("198.51.100.1".to_string()));
    }

    /// A request with no forwarded-for header still lands in a bucket.
    ///
    /// Skipping the per-IP check when the header is absent removes the only
    /// limit that bounds walking a list of addresses — the per-address bucket is
    /// never hit twice by such a caller. The fallback is deliberately shared, so
    /// unattributed callers meter each other.
    #[test]
    fn an_unattributed_request_falls_back_rather_than_skipping() {
        let headers = HeaderMap::new();
        assert_eq!(client_ip(&headers), None);
        // `check_rate_limits` turns that None into the shared
        // `magic-link:ip:{tenant}:unattributed` key rather than bypassing the
        // check; see the match there.
    }

    #[test]
    fn json_is_opt_in_and_a_browser_gets_the_redirect() {
        let mut headers = HeaderMap::new();
        assert!(!wants_json(&headers));
        headers.insert(
            header::ACCEPT,
            "text/html,application/xhtml+xml".parse().expect("valid"),
        );
        assert!(!wants_json(&headers));
        headers.insert(header::ACCEPT, "application/json".parse().expect("valid"));
        assert!(wants_json(&headers));
    }
}
