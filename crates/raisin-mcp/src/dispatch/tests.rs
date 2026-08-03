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

//! Tests for the dispatcher and the widget contract.

#[cfg(test)]
mod ui_tests {
    use super::super::ui::*;
    use super::super::*;
    use crate::protocol::META_PROTOCOL_VERSION;
    use serde_json::{json, Value};

    #[test]
    fn ui_resource_uri_strips_leading_slash() {
        assert_eq!(
            ui_resource_uri("assets", "/widgets/x/index.html"),
            "ui://assets/widgets/x/index.html"
        );
    }

    fn request_with_meta(meta: Value) -> crate::protocol::JsonRpcRequest {
        crate::protocol::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "tools/list".to_string(),
            params: Some(json!({ "_meta": meta })),
        }
    }

    #[test]
    fn current_revision_requires_per_request_negotiation() {
        // 2026-07-28 removed `initialize`, so both fields are required on every
        // request and may not be remembered from an earlier one.
        let ok = RequestMeta::from_request(&request_with_meta(json!({
            META_PROTOCOL_VERSION: PROTOCOL_VERSION,
            META_CLIENT_CAPABILITIES: {},
        })));
        assert!(Dispatcher::negotiate(&ok).is_ok());

        let no_caps = RequestMeta::from_request(&request_with_meta(json!({
            META_PROTOCOL_VERSION: PROTOCOL_VERSION,
        })));
        assert!(Dispatcher::negotiate(&no_caps).is_err());
    }

    #[test]
    fn legacy_clients_are_not_asked_for_fields_their_revision_lacks() {
        // Claude Desktop 1.24012.9 tops out at 2025-11-25 and sends no
        // per-request `_meta` at all. Demanding it would refuse every client
        // that exists today at its first message after `initialize`.
        let legacy = RequestMeta::from_request(&request_with_meta(json!({
            META_PROTOCOL_VERSION: "2025-11-25",
        })));
        assert!(Dispatcher::negotiate(&legacy).is_ok());

        let absent = RequestMeta::from_request(&crate::protocol::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "tools/list".to_string(),
            params: None,
        });
        assert!(Dispatcher::negotiate(&absent).is_ok());
    }

    #[test]
    fn an_unknown_revision_is_refused_with_the_supported_list() {
        let meta = RequestMeta::from_request(&request_with_meta(json!({
            META_PROTOCOL_VERSION: "1999-01-01",
            META_CLIENT_CAPABILITIES: {},
        })));
        let err = Dispatcher::negotiate(&meta).unwrap_err();
        assert_eq!(err.code(), -32022);
        // The client recovers by picking from `supported` and retrying, so the
        // data member is load-bearing, not decoration.
        let data = err.data().expect("-32022 must carry data");
        assert_eq!(data["requested"], json!("1999-01-01"));
        assert!(data["supported"]
            .as_array()
            .unwrap()
            .contains(&json!(PROTOCOL_VERSION)));
    }

    #[test]
    fn ui_extension_is_declared_only_when_the_client_declares_it() {
        let meta = RequestMeta::from_request(&request_with_meta(json!({
            META_PROTOCOL_VERSION: PROTOCOL_VERSION,
            META_CLIENT_CAPABILITIES: {
                "extensions": { UI_EXTENSION_ID: { "mimeTypes": [UI_MIME_TYPE] } }
            },
        })));
        assert!(meta.supports_ui());

        let without = RequestMeta::from_request(&request_with_meta(json!({
            META_PROTOCOL_VERSION: PROTOCOL_VERSION,
            META_CLIENT_CAPABILITIES: {},
        })));
        assert!(!without.supports_ui());

        // A client offering only some other mime type has not offered ours.
        let other = RequestMeta::from_request(&request_with_meta(json!({
            META_PROTOCOL_VERSION: PROTOCOL_VERSION,
            META_CLIENT_CAPABILITIES: {
                "extensions": { UI_EXTENSION_ID: { "mimeTypes": ["text/plain"] } }
            },
        })));
        assert!(!other.supports_ui());
    }

    #[test]
    fn each_mode_declares_its_own_mime_type() {
        // Both modes must be served AND listed as what they are. `uri-list`
        // used to be parsed and then ignored: every binding was served inline
        // as html, so a multi-file widget reached the host as `srcdoc` and its
        // relative asset URLs resolved against a null origin.
        assert_eq!(ui_mime_type(UiMode::Html), "text/html;profile=mcp-app");
        assert_eq!(ui_mime_type(UiMode::UriList), "text/uri-list");
        assert_ne!(ui_mime_type(UiMode::Html), ui_mime_type(UiMode::UriList));
    }

    #[test]
    fn server_origin_is_defined_before_the_first_script() {
        let html = "<!doctype html><html><head><meta charset=\"utf-8\" />\
                    <script type=\"module\">boot()</script></head><body></body></html>";
        let out = inject_server_origin(
            html.to_string(),
            Some("https://solutas.rdb.maravilla.cloud"),
        );

        let global = out.find("__RAISIN_SERVER_ORIGIN__").unwrap();
        let boot = out.find("boot()").unwrap();
        assert!(global < boot, "widget code must not run before the global");
        assert!(out
            .contains(r#"window.__RAISIN_SERVER_ORIGIN__="https://solutas.rdb.maravilla.cloud";"#));
    }

    #[test]
    fn server_origin_precedes_a_head_less_fragment() {
        // Bundlers emit documents with no <head>; appending would define the
        // global after the code that reads it.
        let out = inject_server_origin(
            "<script>boot()</script>".to_string(),
            Some("https://x.test"),
        );
        assert!(out.find("__RAISIN_SERVER_ORIGIN__").unwrap() < out.find("boot()").unwrap());
    }

    #[test]
    fn header_tag_is_not_mistaken_for_head() {
        let out = inject_server_origin("<header>hi</header>".to_string(), Some("https://x.test"));
        assert!(out.starts_with("<script>window.__RAISIN_SERVER_ORIGIN__"));
    }

    #[test]
    fn origin_is_escaped_and_absent_base_changes_nothing() {
        let out = inject_server_origin(
            "<head></head>".to_string(),
            Some("https://x.test/\"</script>"),
        );
        // The injected <script> must contain exactly one `</script>` — its own
        // terminator. JSON quoting alone does NOT achieve this: the HTML parser
        // finds `</script` before JS is tokenized, so an origin carrying one
        // would close the element early and spill the rest as markup.
        assert_eq!(out.matches("</script>").count(), 1);
        assert!(out.contains("\\u003C/script\\u003E"), "must be \\u-escaped");

        let html = "<head></head><script>boot()</script>";
        assert_eq!(inject_server_origin(html.to_string(), None), html);
    }

    #[test]
    fn own_origin_is_added_alongside_declared_domains() {
        // The regression: a binding that declares any csp used to REPLACE the
        // default, dropping the server's own origin. A package authored against
        // localhost then shipped to a deployment whose widget could reach
        // nothing — every image and API call blocked by the host's sandbox.
        let declared = json!({
            "connectDomains": ["http://localhost:5173"],
            "resourceDomains": ["http://localhost:8080"],
            "frameDomains": ["http://localhost:5173"],
        });
        let merged =
            csp_with_own_origin(declared, Some("https://solutas.rdb.maravilla.cloud")).unwrap();

        assert_eq!(
            merged["connectDomains"],
            json!([
                "http://localhost:5173",
                "https://solutas.rdb.maravilla.cloud"
            ])
        );
        assert_eq!(
            merged["resourceDomains"],
            json!([
                "http://localhost:8080",
                "https://solutas.rdb.maravilla.cloud"
            ])
        );
        // frameDomains is NOT widened: framing the server's own origin is not
        // implied by serving the widget, and granting it would be a privilege
        // the author never asked for.
        assert_eq!(merged["frameDomains"], json!(["http://localhost:5173"]));
    }

    #[test]
    fn own_origin_is_not_duplicated() {
        let declared = json!({ "connectDomains": ["https://example.test"] });
        let merged = csp_with_own_origin(declared, Some("https://example.test")).unwrap();
        assert_eq!(merged["connectDomains"], json!(["https://example.test"]));
    }

    #[test]
    fn undeclared_csp_still_gets_the_server_origin() {
        let merged = csp_with_own_origin(Value::Null, Some("https://example.test")).unwrap();
        assert_eq!(merged["connectDomains"], json!(["https://example.test"]));
        assert_eq!(merged["resourceDomains"], json!(["https://example.test"]));
    }

    #[test]
    fn no_base_and_no_declaration_says_nothing() {
        assert!(csp_with_own_origin(Value::Null, None).is_none());
        // ...but a declaration alone is still passed through verbatim.
        let declared = json!({ "connectDomains": ["https://example.test"] });
        assert_eq!(
            csp_with_own_origin(declared.clone(), None).unwrap(),
            declared
        );
    }
}

#[cfg(test)]
mod security_tests {
    use super::super::ui::*;
    use serde_json::json;

    // ---- CSP origin sanitization -----------------------------------------
    //
    // These values reach a real `Content-Security-Policy` header that the HOST
    // builds by joining them. Author-declared origins come from node
    // properties; the server's own origin can come from `x-forwarded-host`.
    // Neither is trustworthy enough to interpolate verbatim.

    #[test]
    fn a_plain_origin_survives_sanitization() {
        assert_eq!(
            sanitize_origin("https://api.example.com"),
            Some("https://api.example.com".to_string())
        );
        // Wildcard subdomains are a legal CSP source expression.
        assert_eq!(
            sanitize_origin("https://*.example.com"),
            Some("https://*.example.com".to_string())
        );
        // Explicit ports are legal.
        assert_eq!(
            sanitize_origin("http://localhost:8080"),
            Some("http://localhost:8080".to_string())
        );
    }

    /// THE injection cases. A `;` or a newline ends one CSP directive and
    /// starts another, so either would let a node property rewrite the whole
    /// policy — e.g. re-opening `script-src` on an attacker's origin.
    #[test]
    fn csp_delimiters_and_newlines_are_rejected() {
        for bad in [
            "https://evil.com; script-src *",
            "https://evil.com\r\nX-Injected: 1",
            "https://evil.com\nscript-src *",
            "https://a.com,https://b.com",
            "'unsafe-inline'",
            "https://evil.com \"",
            "https://evil.com'",
        ] {
            assert_eq!(sanitize_origin(bad), None, "must reject: {bad:?}");
        }
    }

    /// A CSP source is an ORIGIN. A bare host would be read as a relative
    /// source and match more than the author meant; a path changes matching
    /// semantics.
    #[test]
    fn non_origins_are_rejected() {
        for bad in [
            "example.com",
            "https://example.com/path",
            "https://example.com?q=1",
            "https://example.com#f",
            "javascript://evil",
            "data:",
            "",
            "   ",
        ] {
            assert_eq!(sanitize_origin(bad), None, "must reject: {bad:?}");
        }
    }

    #[test]
    fn sanitize_csp_lists_drops_only_the_bad_entries() {
        let declared = json!({
            "connectDomains": ["https://good.example", "https://evil.com; script-src *"],
            "resourceDomains": ["https://cdn.example"],
        });
        let cleaned = sanitize_csp_lists(declared);
        assert_eq!(
            cleaned["connectDomains"],
            json!(["https://good.example"]),
            "the injecting entry is dropped, the good one survives"
        );
        assert_eq!(cleaned["resourceDomains"], json!(["https://cdn.example"]));
    }

    /// A malformed origin must never fail the request — a widget that renders
    /// with a narrower CSP is strictly better than one that 500s.
    #[test]
    fn an_entirely_bad_list_yields_an_empty_list_not_an_error() {
        let cleaned = sanitize_csp_lists(json!({ "connectDomains": ["nonsense", "also; bad"] }));
        assert_eq!(cleaned["connectDomains"], json!([]));
    }
}
