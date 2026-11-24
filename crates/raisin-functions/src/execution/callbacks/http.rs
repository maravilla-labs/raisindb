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

//! HTTP operation callbacks for function execution.
//!
//! These callbacks implement the `raisin.http.*` API available to JavaScript functions.

use std::sync::Arc;

use serde_json::Value;

use crate::api::HttpRequestCallback;

/// The egress rules every outbound function request is held to.
///
/// Empty `allowed_hosts` on purpose: WHICH hosts a function may name is its own
/// `network_policy`'s question, already answered before we get here. This only
/// decides which ADDRESSES those names are allowed to resolve to.
fn function_egress_policy() -> raisin_mcp_protocol::client::EgressPolicy {
    raisin_mcp_protocol::client::EgressPolicy {
        allowed_hosts: Vec::new(),
        allow_private_addresses: false,
    }
}

/// Refuse a URL that names a non-public address LITERALLY, e.g.
/// `http://127.0.0.1/` or `http://169.254.169.254/`.
///
/// Only the literal-IP case. A hostname is deliberately left alone here and
/// judged by [`PublicOnlyResolver`] instead, because reqwest resolves it itself
/// and only the resolver's answer is the one actually dialled — see that type
/// for why checking here as well would be both redundant and unsound.
///
/// This half is still needed because reqwest does NOT consult a resolver for a
/// URL that already carries an IP: there is no name to resolve, so the resolver
/// never runs and this is the only thing standing in the way.
async fn refuse_non_public_address(url: &str) -> Result<(), raisin_error::Error> {
    let parsed = url::Url::parse(url).map_err(|e| {
        raisin_error::Error::Validation(format!("invalid request URL `{url}`: {e}"))
    })?;
    if parsed.host().is_none() {
        return Err(raisin_error::Error::Validation(format!(
            "request URL `{url}` has no host"
        )));
    }
    match literal_address_refusal(&parsed) {
        Some(reason) => Err(raisin_error::Error::Validation(format!(
            "URL not allowed by egress policy: {reason}"
        ))),
        None => Ok(()),
    }
}

/// Why `url` must not be dialled because it names a non-public address
/// LITERALLY, or `None` when it may (a hostname is judged later, at resolution
/// time, by [`PublicOnlyResolver`]).
///
/// Shared by the first request and by EVERY redirect hop: reqwest follows a
/// redirect to `http://169.254.169.254/` without resolving anything, so the
/// resolver never sees it and this is the only check standing in the way.
fn literal_address_refusal(url: &url::Url) -> Option<String> {
    let addr = match url.host()? {
        url::Host::Ipv4(a) => std::net::IpAddr::V4(a),
        url::Host::Ipv6(a) => std::net::IpAddr::V6(a),
        url::Host::Domain(_) => return None,
    };
    function_egress_policy()
        .validate_resolved(&addr.to_string(), &[addr])
        .err()
        .map(|e| e.to_string())
}

/// Redirects a function fetch may follow before it is refused.
const MAX_FUNCTION_REDIRECTS: usize = 5;

/// The redirect policy of [`guarded_http_client`]: every hop is held to the same
/// egress rule as the first URL. A hop to a hostname is covered by the resolver;
/// a hop to a LITERAL non-public IP is refused here (reqwest's default policy
/// followed it, which let a public page that answers `302 →
/// http://169.254.169.254/` hand an internal response to function code).
fn guarded_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_FUNCTION_REDIRECTS {
            return attempt.error(format!(
                "too many redirects (more than {MAX_FUNCTION_REDIRECTS})"
            ));
        }
        match literal_address_refusal(attempt.url()) {
            Some(reason) => {
                attempt.error(format!("redirect not allowed by egress policy: {reason}"))
            }
            None => attempt.follow(),
        }
    })
}

/// A DNS resolver that refuses to hand back a non-public address.
///
/// # Why a resolver and not a pre-flight check
///
/// Validating a hostname before the request is a time-of-check/time-of-use bug.
/// The pre-flight resolves `evil.example.com` to a public address and allows it;
/// reqwest then resolves the name AGAIN, and an attacker serving a short-TTL
/// record answers `127.0.0.1` the second time. Two lookups, and only the second
/// one decides where the socket goes — the check was advisory. That is DNS
/// rebinding.
///
/// Installing the check INSIDE resolution removes the gap: there is exactly one
/// lookup, and the addresses this returns are the addresses reqwest connects to.
/// Nothing re-resolves afterwards.
///
/// EVERY address must pass, not merely one — `validate_resolved` enforces that.
/// A host answering with both a public and a private record would otherwise be
/// dialled on whichever the connector happened to pick.
///
/// A literal IP in the URL never reaches here (there is no name to resolve),
/// which is what [`refuse_non_public_address`] covers.
#[derive(Debug, Default)]
struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            // Port is irrelevant to the address check; 443 is a placeholder that
            // only satisfies `lookup_host`'s signature.
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 443))
                .await?
                .collect();

            let ips: Vec<std::net::IpAddr> = addrs.iter().map(|s| s.ip()).collect();
            function_egress_policy()
                .validate_resolved(&host, &ips)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The HTTP clients every `raisin.http.fetch` goes through: one that follows
/// redirects (the default) and one that does not (`redirect: "manual"`).
///
/// Separate from [`shared_http_client`] deliberately. The resolver above refuses
/// private addresses outright, and other users of the shared client — AI
/// providers, MCP backends — may legitimately be pointed at an internal host by
/// an operator. Narrowing this to the one untrusted caller avoids breaking them,
/// and function calls still share a connection pool with each other.
///
/// [`shared_http_client`]: crate::execution::types::shared_http_client
fn guarded_http_client(follow_redirects: bool) -> reqwest::Client {
    static FOLLOWING: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    static MANUAL: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let slot = if follow_redirects {
        &FOLLOWING
    } else {
        &MANUAL
    };
    slot.get_or_init(|| {
        client_builder(follow_redirects)
            .dns_resolver(std::sync::Arc::new(PublicOnlyResolver))
            .build()
            .unwrap_or_else(|e| {
                // A client that cannot be built with the guard must not fall
                // back to one without it.
                panic!("failed to build the guarded HTTP client: {e}")
            })
    })
    .clone()
}

/// The client's redirect behaviour: every hop held to the egress policy, or
/// none followed (the 3xx response and its `location` header are returned).
fn client_builder(follow_redirects: bool) -> reqwest::ClientBuilder {
    reqwest::Client::builder().redirect(if follow_redirects {
        guarded_redirect_policy()
    } else {
        reqwest::redirect::Policy::none()
    })
}

/// Whether a fetch follows redirects: yes, unless the options say
/// `redirect: "manual"` (the Fetch API spelling) or `followRedirects: false`
/// (also `follow_redirects`, the spelling the Rust SDK serialises).
fn follows_redirects(options: &Value) -> bool {
    if options
        .get("redirect")
        .and_then(Value::as_str)
        .is_some_and(|r| r.eq_ignore_ascii_case("manual"))
    {
        return false;
    }
    !["followRedirects", "follow_redirects"]
        .iter()
        .any(|k| options.get(*k).and_then(Value::as_bool) == Some(false))
}

/// Create http_request callback: `raisin.http.fetch(method, url, options)`
///
/// `options.redirect: "manual"` (or `followRedirects: false`) returns a 3xx
/// response as it is, with its `location` header, instead of following it.
pub fn create_http_request(_http_client: reqwest::Client) -> HttpRequestCallback {
    Arc::new(move |method: String, url: String, options: Value| {
        // NOT the client passed in: function fetches go through the
        // resolver-guarded one. The parameter is kept so callers do not have to
        // change, and so the shared client stays available if this ever needs to
        // become configurable.
        let client = guarded_http_client(follows_redirects(&options));

        Box::pin(async move {
            // BEFORE anything is built or sent.
            refuse_non_public_address(&url).await?;

            // Parse options
            let headers = options.get("headers").and_then(|v| v.as_object()).cloned();
            let body = options.get("body").cloned();
            let timeout_ms = options
                .get("timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(30_000);

            // Build request
            let mut request = match method.to_uppercase().as_str() {
                "GET" => client.get(&url),
                "POST" => client.post(&url),
                "PUT" => client.put(&url),
                "DELETE" => client.delete(&url),
                "PATCH" => client.patch(&url),
                "HEAD" => client.head(&url),
                _ => {
                    return Err(raisin_error::Error::Validation(format!(
                        "Unsupported HTTP method: {}",
                        method
                    )))
                }
            };

            // Set timeout
            request = request.timeout(std::time::Duration::from_millis(timeout_ms));

            // Add headers
            if let Some(hdrs) = headers {
                for (key, value) in hdrs {
                    if let Some(val_str) = value.as_str() {
                        request = request.header(&key, val_str);
                    }
                }
            }

            // Add body.
            //
            // `bodyBase64` carries real bytes. Without it an ArrayBuffer body
            // was base64-encoded on the way in and then sent AS THAT TEXT, so
            // uploading a file transmitted its base64 rather than its content.
            if let Some(encoded) = options.get("bodyBase64").and_then(|v| v.as_str()) {
                use base64::Engine as _;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded.as_bytes())
                    .map_err(|e| {
                        raisin_error::Error::Validation(format!(
                            "request bodyBase64 is not valid base64: {e}"
                        ))
                    })?;
                request = request.body(bytes);
            } else if let Some(body_val) = body {
                if let Some(body_str) = body_val.as_str() {
                    request = request.body(body_str.to_string());
                } else {
                    // JSON body
                    request = request.json(&body_val);
                }
            }

            // Execute request
            let response = request
                .send()
                .await
                .map_err(|e| raisin_error::Error::Backend(format!("HTTP request failed: {}", e)))?;

            // Build response object
            let status = response.status().as_u16();
            let headers: serde_json::Map<String, Value> = response
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        Value::String(v.to_str().unwrap_or("").to_string()),
                    )
                })
                .collect();

            // Read BYTES, not text. `Response::text()` decodes lossily, so
            // every byte that is not valid UTF-8 became U+FFFD — which made
            // fetching a PDF, an image or any other binary silently return
            // corrupted data, with `arrayBuffer()` re-encoding the damage.
            let body_bytes = response.bytes().await.map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to read response body: {}", e))
            })?;

            // The shape is decided by the bytes themselves rather than by a
            // caller flag: valid UTF-8 keeps the existing string/JSON body
            // (so nothing about the common path changes, and `arrayBuffer()`
            // can round-trip it exactly through TextEncoder), and anything
            // else is handed over as base64. A caller may force base64 with
            // `responseType: "base64"` when it wants the exact bytes of
            // something that merely happens to be valid UTF-8.
            let force_base64 = options
                .get("responseType")
                .and_then(|v| v.as_str())
                .is_some_and(|t| t.eq_ignore_ascii_case("base64"));

            let as_text = if force_base64 {
                None
            } else {
                std::str::from_utf8(&body_bytes).ok()
            };

            match as_text {
                Some(text) => {
                    let body_value = serde_json::from_str::<Value>(text)
                        .unwrap_or_else(|_| Value::String(text.to_string()));
                    Ok(serde_json::json!({
                        "status": status,
                        "headers": headers,
                        "body": body_value
                    }))
                }
                None => {
                    use base64::Engine as _;
                    let encoded =
                        base64::engine::general_purpose::STANDARD.encode(body_bytes.as_ref());
                    Ok(serde_json::json!({
                        "status": status,
                        "headers": headers,
                        // Empty rather than absent: existing callers that read
                        // `.body` unconditionally get a string, not undefined.
                        "body": "",
                        "body_base64": encoded
                    }))
                }
            }
        })
    })
}

#[cfg(test)]
mod egress_guard_tests {
    use super::{refuse_non_public_address, PublicOnlyResolver};

    /// The addresses an SSRF actually aims at, named LITERALLY in the URL.
    ///
    /// `169.254.169.254` is the one that matters most: on every major cloud it
    /// serves instance credentials to anything that can issue a plain GET.
    ///
    /// These never reach the resolver — there is no name to resolve — so this
    /// pre-flight is the only thing standing in the way.
    #[tokio::test]
    async fn non_public_addresses_are_refused() {
        for url in [
            "http://127.0.0.1:8080/admin",
            // `localhost` is a NAME, not a literal — it belongs to the resolver
            // test below, not here.
            "http://169.254.169.254/latest/meta-data/iam/security-credentials/",
            "http://10.0.0.5/internal",
            "http://192.168.1.1/",
            "http://172.16.0.1/",
            // Loopback wearing an IPv6 hat — the form a naive string check misses.
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
        ] {
            assert!(
                refuse_non_public_address(url).await.is_err(),
                "{url} must be refused"
            );
        }
    }

    /// The guard must not become a general outage: a public host still passes,
    /// and WHICH public hosts are permitted stays the function network_policy's
    /// question, not this one's.
    #[tokio::test]
    async fn a_public_address_is_permitted() {
        assert!(refuse_non_public_address("https://93.184.216.34/")
            .await
            .is_ok());
    }

    /// A malformed URL is refused rather than passed through to reqwest, so the
    /// guard cannot be skipped by handing it something it fails to parse.
    #[tokio::test]
    async fn an_unparseable_url_is_refused() {
        assert!(refuse_non_public_address("not a url").await.is_err());
        assert!(refuse_non_public_address("").await.is_err());
    }

    /// A HOSTNAME is deliberately NOT judged by the pre-flight.
    ///
    /// Not an oversight — judging it here would be the time-of-check bug the
    /// resolver exists to remove. `localhost` sails past this check and is
    /// stopped at resolution instead, which is the only point where the answer
    /// is the one actually dialled.
    #[tokio::test]
    async fn a_hostname_is_left_for_the_resolver() {
        assert!(
            refuse_non_public_address("http://localhost:5432/")
                .await
                .is_ok(),
            "the pre-flight judges literal IPs only"
        );
    }

    /// The resolver refuses a name that resolves to loopback.
    ///
    /// This is the rebinding case: the name looks innocuous and only its ANSWER
    /// is disqualifying. Because the check runs inside resolution, the addresses
    /// returned here are the addresses reqwest dials — there is no second lookup
    /// for an attacker to answer differently.
    #[tokio::test]
    async fn the_resolver_refuses_a_name_that_resolves_to_loopback() {
        use reqwest::dns::Resolve;
        use std::str::FromStr;

        let name = reqwest::dns::Name::from_str("localhost").expect("valid name");
        let result = PublicOnlyResolver.resolve(name).await;
        assert!(
            result.is_err(),
            "localhost resolves to 127.0.0.1/::1 and must be refused at resolution"
        );
    }

    /// ...and still returns addresses for a name that resolves publicly, so the
    /// guard is not simply an outage.
    #[tokio::test]
    async fn the_resolver_allows_a_publicly_resolving_name() {
        use reqwest::dns::Resolve;
        use std::str::FromStr;

        let name = reqwest::dns::Name::from_str("example.com").expect("valid name");
        match PublicOnlyResolver.resolve(name).await {
            Ok(addrs) => {
                let found: Vec<std::net::SocketAddr> = addrs.collect();
                assert!(!found.is_empty(), "a public name must yield addresses");
            }
            // No DNS in the sandbox is not a failure of the guard.
            Err(e) => eprintln!("skipped: resolution unavailable ({e})"),
        }
    }

    /// The check every REDIRECT hop goes through: a literal non-public address is
    /// refused (reqwest does not resolve a literal IP, so the resolver never sees
    /// it), a hostname is left to the resolver, and a public literal passes.
    #[test]
    fn redirect_hops_to_literal_private_addresses_are_refused() {
        for bad in [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:8088/api",
            "http://10.0.0.5/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
        ] {
            let url = url::Url::parse(bad).unwrap();
            assert!(
                super::literal_address_refusal(&url).is_some(),
                "{bad} must be refused as a redirect target"
            );
        }
        for ok in ["https://example.com/next", "https://93.184.216.34/"] {
            let url = url::Url::parse(ok).unwrap();
            assert!(
                super::literal_address_refusal(&url).is_none(),
                "{ok} must be allowed"
            );
        }
        assert_eq!(super::MAX_FUNCTION_REDIRECTS, 5);
    }
}

#[cfg(test)]
mod redirect_option_tests {
    use super::{client_builder, follows_redirects};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn redirects_are_followed_unless_the_options_say_manual() {
        assert!(follows_redirects(&json!({})));
        assert!(follows_redirects(&json!({ "redirect": "follow" })));
        assert!(!follows_redirects(&json!({ "redirect": "manual" })));
        assert!(!follows_redirects(&json!({ "followRedirects": false })));
        assert!(!follows_redirects(&json!({ "follow_redirects": false })));
        assert!(follows_redirects(&json!({ "followRedirects": true })));
    }

    /// `/a` answers 302 to `/b`, `/b` answers 200. Built without the public
    /// resolver (the test server is on loopback); the redirect behaviour is
    /// the one the function clients use.
    #[tokio::test]
    async fn manual_returns_the_redirect_and_follow_follows_it() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let reply = if req.starts_with("GET /a") {
                        format!(
                            "HTTP/1.1 302 Found\r\nlocation: http://localhost:{port}/b\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        )
                    } else {
                        "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok"
                            .to_string()
                    };
                    let _ = sock.write_all(reply.as_bytes()).await;
                });
            }
        });
        let url = format!("http://127.0.0.1:{port}/a");

        let manual = client_builder(false).build().unwrap();
        let r = manual.get(&url).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 302);
        assert_eq!(
            r.headers().get("location").unwrap().to_str().unwrap(),
            format!("http://localhost:{port}/b")
        );

        let following = client_builder(true).build().unwrap();
        let r = following.get(&url).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 200);
    }
}
