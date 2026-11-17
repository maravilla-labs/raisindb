// SPDX-License-Identifier: BSL-1.1

//! `return_to` for the connector OAuth flow: where the browser goes once the
//! consent is over, instead of the admin console.
//!
//! A control plane (flightdeck) hands an org operator a consent link and wants
//! them back on its own page afterwards. Without this the callback always ends
//! on `/admin/{repo}/integrations`, a console the operator may not even have a
//! login for.
//!
//! # The rules
//!
//! - **Allow-listed origins only.** `return_to` must be an absolute `https` URL
//!   whose exact origin (scheme, host, port) is in
//!   `[integrations] oauth_return_origins` (env
//!   `RAISIN_INTEGRATIONS_OAUTH_RETURN_ORIGINS`, comma-separated). No list means
//!   every `return_to` is refused. A refused one is a 400 at `oauth/start`,
//!   never a silent fallback: a caller who asked to come back and silently did
//!   not would see a flow that "works" and strands the operator.
//! - **Exact origin, parsed.** The check compares the parsed origin, so
//!   `https://www.maravilla.cloud.evil.com` and
//!   `https://www.maravilla.cloud@evil.com` are refused, and userinfo is refused
//!   outright.
//! - **Nothing secret travels.** The redirect carries `integration_status` and,
//!   on failure, `error` / `error_description`. Never the code, the state, a
//!   token or an account id. The response sets `Referrer-Policy: no-referrer`,
//!   because the callback URL itself carries the authorization code and
//!   `state` in its query, and a Referer header would hand both to the
//!   `return_to` origin.

use std::sync::RwLock;

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// Environment variable read when the server config sets no list.
pub const RETURN_ORIGINS_ENV: &str = "RAISIN_INTEGRATIONS_OAUTH_RETURN_ORIGINS";

/// Longest `return_to` accepted, before the outcome params are added.
const MAX_RETURN_TO_LEN: usize = 2048;

/// Set by the server from `[integrations] oauth_return_origins`.
static CONFIGURED: RwLock<Option<Vec<String>>> = RwLock::new(None);

/// Install the allow-list. Called once at startup by the server; an empty list
/// leaves the environment variable in charge.
pub fn configure_oauth_return_origins(origins: Vec<String>) {
    if let Ok(mut g) = CONFIGURED.write() {
        *g = if origins.is_empty() {
            None
        } else {
            Some(origins)
        };
    }
}

/// The allow-list in force: the configured list, else the environment's.
fn allowed_origins() -> Vec<String> {
    if let Some(list) = CONFIGURED.read().ok().and_then(|g| g.clone()) {
        return list;
    }
    parse_origin_list(&std::env::var(RETURN_ORIGINS_ENV).unwrap_or_default())
}

/// Split a comma- or whitespace-separated origin list.
pub fn parse_origin_list(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// The canonical origin of an allow-list entry, or `None` for an entry that
/// is not a bare https origin (a path, a query or userinfo makes it ambiguous,
/// so it is ignored rather than guessed at).
fn canonical_origin(entry: &str) -> Option<String> {
    let u = url::Url::parse(entry.trim()).ok()?;
    if u.scheme() != "https"
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || !(u.path().is_empty() || u.path() == "/")
    {
        return None;
    }
    Some(u.origin().ascii_serialization())
}

/// Validate `return_to` against an allow-list. Pure: the caller passes the list.
///
/// Returns the normalised URL to store, or the reason it is refused.
pub fn validate_return_to(return_to: &str, allowed: &[String]) -> Result<String, String> {
    let raw = return_to.trim();
    if raw.len() > MAX_RETURN_TO_LEN {
        return Err(format!(
            "return_to is longer than {MAX_RETURN_TO_LEN} characters"
        ));
    }
    let allowed: Vec<String> = allowed.iter().filter_map(|e| canonical_origin(e)).collect();
    if allowed.is_empty() {
        return Err(
            "return_to is not accepted: this server allows no return origins \
             (set [integrations] oauth_return_origins)"
                .to_string(),
        );
    }
    let u = url::Url::parse(raw).map_err(|_| "return_to must be an absolute URL".to_string())?;
    if u.scheme() != "https" {
        return Err("return_to must use https".to_string());
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err("return_to must not carry userinfo".to_string());
    }
    if u.host_str().is_none() {
        return Err("return_to must name a host".to_string());
    }
    let origin = u.origin().ascii_serialization();
    if !allowed.iter().any(|a| *a == origin) {
        return Err(format!("return_to origin {origin} is not allowed"));
    }
    Ok(u.to_string())
}

/// [`validate_return_to`] against the allow-list in force.
pub fn validate_configured(return_to: &str) -> Result<String, String> {
    validate_return_to(return_to, &allowed_origins())
}

/// The keys this module owns on the `return_to` URL.
const OWNED_KEYS: [&str; 3] = ["integration_status", "error", "error_description"];

/// `return_to` with the outcome merged into its query. Existing parameters the
/// caller put there survive; any that collide with ours are replaced.
///
/// `failure` is `(code, description)`, already clamped by the caller.
pub fn outcome_url(return_to: &str, failure: Option<(&str, &str)>) -> Option<String> {
    let mut u = url::Url::parse(return_to).ok()?;
    let kept: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| !OWNED_KEYS.contains(&k.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    {
        let mut q = u.query_pairs_mut();
        q.clear();
        for (k, v) in &kept {
            q.append_pair(k, v);
        }
        match failure {
            None => {
                q.append_pair("integration_status", "connected");
            }
            Some((code, description)) => {
                q.append_pair("integration_status", "error");
                q.append_pair("error", code);
                if !description.is_empty() {
                    q.append_pair("error_description", description);
                }
            }
        }
    }
    Some(u.to_string())
}

/// A 302 to `location` that leaks nothing of the callback URL onward.
pub fn redirect(location: &str) -> Response {
    let mut resp = StatusCode::FOUND.into_response();
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(location) {
        h.insert(header::LOCATION, v);
    }
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow() -> Vec<String> {
        vec!["https://www.maravilla.cloud".to_string()]
    }

    #[test]
    fn an_allowed_origin_passes_with_its_path_and_query() {
        assert_eq!(
            validate_return_to(
                "https://www.maravilla.cloud/orgs/acme/stripe?step=4",
                &allow()
            )
            .unwrap(),
            "https://www.maravilla.cloud/orgs/acme/stripe?step=4"
        );
        // A trailing slash or an explicit default port in the list is the same origin.
        let list = vec!["https://www.maravilla.cloud:443/".to_string()];
        assert!(validate_return_to("https://www.maravilla.cloud/x", &list).is_ok());
    }

    #[test]
    fn other_origins_are_refused() {
        for bad in [
            "https://evil.com/",
            "https://maravilla.cloud/",
            "https://www.maravilla.cloud:8443/",
            "https://www.maravilla.cloud.evil.com/",
            "https://www.maravilla.cloud@evil.com/",
            "https://evil.com@www.maravilla.cloud/",
            "https://user:pw@www.maravilla.cloud/",
            "//www.maravilla.cloud/x",
            "/orgs/acme",
            "javascript:alert(1)",
        ] {
            assert!(
                validate_return_to(bad, &allow()).is_err(),
                "{bad} was accepted"
            );
        }
    }

    #[test]
    fn http_is_refused_even_for_the_allowed_host() {
        let err = validate_return_to("http://www.maravilla.cloud/x", &allow()).unwrap_err();
        assert!(err.contains("https"));
    }

    #[test]
    fn no_allow_list_refuses_everything() {
        let err = validate_return_to("https://www.maravilla.cloud/x", &[]).unwrap_err();
        assert!(err.contains("allows no return origins"));
        // Entries that are not bare https origins do not count as a list.
        let junk = vec![
            "http://www.maravilla.cloud".to_string(),
            "https://www.maravilla.cloud/path".to_string(),
            "*".to_string(),
        ];
        assert!(validate_return_to("https://www.maravilla.cloud/x", &junk).is_err());
    }

    #[test]
    fn an_overlong_return_to_is_refused() {
        let long = format!(
            "https://www.maravilla.cloud/{}",
            "a".repeat(MAX_RETURN_TO_LEN)
        );
        assert!(validate_return_to(&long, &allow()).is_err());
    }

    #[test]
    fn the_env_list_splits_on_commas_and_spaces() {
        assert_eq!(
            parse_origin_list(" https://a.test, https://b.test  https://c.test,"),
            vec!["https://a.test", "https://b.test", "https://c.test"]
        );
        assert!(parse_origin_list("").is_empty());
    }

    #[test]
    fn success_merges_the_status_into_the_existing_query() {
        let u = outcome_url("https://www.maravilla.cloud/orgs/acme?step=4", None).unwrap();
        assert_eq!(
            u,
            "https://www.maravilla.cloud/orgs/acme?step=4&integration_status=connected"
        );
    }

    #[test]
    fn failure_carries_the_clamped_error_and_replaces_stale_keys() {
        let u = outcome_url(
            "https://www.maravilla.cloud/x?error=old&integration_status=connected&keep=1",
            Some(("access_denied", "The user denied & left")),
        )
        .unwrap();
        let parsed = url::Url::parse(&u).unwrap();
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("keep".into(), "1".into()),
                ("integration_status".into(), "error".into()),
                ("error".into(), "access_denied".into()),
                ("error_description".into(), "The user denied & left".into()),
            ]
        );
    }

    #[test]
    fn the_redirect_is_a_302_that_sends_no_referrer() {
        let r = redirect("https://www.maravilla.cloud/x?integration_status=connected");
        assert_eq!(r.status(), StatusCode::FOUND);
        assert_eq!(
            r.headers().get(header::LOCATION).unwrap(),
            "https://www.maravilla.cloud/x?integration_status=connected"
        );
        assert_eq!(
            r.headers().get(header::REFERRER_POLICY).unwrap(),
            "no-referrer"
        );
    }
}
