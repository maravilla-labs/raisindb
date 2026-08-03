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
    fn every_widget_is_served_as_the_one_apps_mime_type() {
        // SEP-1865 defines exactly one content type for a widget. `mode` used
        // to pick between this and `text/uri-list` — the spec's `externalUrl`,
        // listed under "Content Types (deferred from MVP)" and therefore
        // renderable by no conformant host, which is why such a widget arrived
        // in ChatGPT as a bare url. There is no longer a second answer to pick.
        assert_eq!(UI_MIME_TYPE, "text/html;profile=mcp-app");
        // No space after the `;` — the string is matched verbatim against the
        // client's advertised `mimeTypes`.
        assert!(!UI_MIME_TYPE.contains("; "));
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

#[cfg(test)]
mod conformance_tests {
    use super::super::ui::*;
    use crate::server::{UiBinding, UiCsp, UiMode, UiPermissions};

    fn binding_json(v: serde_json::Value) -> UiBinding {
        serde_json::from_value(v).expect("binding should parse")
    }

    /// SEP-1865 models a permission request as the PRESENCE of an empty object.
    /// `{"camera": true}` is what a YAML author naturally writes and what hosts
    /// ignore, so the typed field normalizes it on the way out.
    #[test]
    fn permissions_serialize_as_empty_objects() {
        let ui = binding_json(serde_json::json!({
            "entry": "/w/index.html",
            "permissions": { "camera": true, "clipboardWrite": {} }
        }));
        let out = serde_json::to_value(ui.permissions.unwrap()).unwrap();
        assert_eq!(
            out,
            serde_json::json!({ "camera": {}, "clipboardWrite": {} }),
            "`true` must normalize to `{{}}`, and an omitted key must stay absent"
        );
    }

    /// A permission is granted by presence, so `false` is the one spelling that
    /// must not silently grant it.
    #[test]
    fn a_false_permission_is_rejected_rather_than_granted() {
        let parsed: Result<UiBinding, _> = serde_json::from_value(serde_json::json!({
            "entry": "/w/index.html",
            "permissions": { "camera": false }
        }));
        assert!(parsed.is_err(), "`camera: false` must not parse as a grant");
    }

    /// `mode` was required with no default. A deployed node carrying
    /// `mode: uri-list` must still parse, because a failed UiBinding parse
    /// cascades through CustomTool and assemble_registry and takes down the
    /// whole server — not just the widget.
    #[test]
    fn a_deployed_uri_list_binding_still_parses_and_mode_is_now_optional() {
        let legacy = binding_json(serde_json::json!({
            "mode": "uri-list", "entry": "/w/index.html"
        }));
        assert_eq!(legacy.mode, Some(UiMode::UriList));

        let modern = binding_json(serde_json::json!({ "entry": "/w/index.html" }));
        assert_eq!(modern.mode, None, "authors may now omit mode entirely");
    }

    /// CSP lists are camelCase on the wire, but the Rust field names are
    /// snake_case and hand-written YAML uses both. Either must parse, or a
    /// mistyped key yields a silently empty CSP.
    #[test]
    fn csp_domains_parse_in_either_casing() {
        let camel: UiCsp =
            serde_json::from_value(serde_json::json!({ "connectDomains": ["https://a.test"] }))
                .unwrap();
        let snake: UiCsp =
            serde_json::from_value(serde_json::json!({ "connect_domains": ["https://a.test"] }))
                .unwrap();
        assert_eq!(camel.connect_domains, snake.connect_domains);
        assert_eq!(camel.connect_domains, vec!["https://a.test".to_string()]);
    }

    /// The sandbox domain is a bare hostname, not an origin — hosts publish it
    /// as `{hash}.claudemcpcontent.com`. A scheme or path means the author
    /// misunderstood, so reject rather than reinterpret.
    #[test]
    fn sandbox_domain_accepts_a_hostname_and_rejects_an_origin() {
        assert_eq!(
            sanitize_domain("a904794854a047f6.claudemcpcontent.com"),
            Some("a904794854a047f6.claudemcpcontent.com".to_string())
        );
        for bad in [
            "https://x.example.com",
            "x.example.com/path",
            "no-dot",
            "evil.com; script-src *",
            "",
        ] {
            assert_eq!(sanitize_domain(bad), None, "must reject: {bad:?}");
        }
    }

    /// Empty permissions must not emit an empty object into `_meta.ui`.
    #[test]
    fn absent_permissions_emit_nothing() {
        assert!(UiPermissions::default().is_empty());
    }

    /// A `<base href>` is what lets a widget keep its RELATIVE asset urls once
    /// it is delivered inline instead of served same-origin. Without one they
    /// resolve against the host's sandbox origin and 404.
    #[test]
    fn base_href_points_at_the_entry_documents_directory() {
        assert_eq!(
            widget_base_href(
                "https://sol.rdb.example.cloud",
                "studio",
                "main",
                "assets",
                "mcp-widgets/studio/index.html"
            ),
            "https://sol.rdb.example.cloud/resources/studio/main/assets/mcp-widgets/studio/"
        );
        // Trailing slash is load-bearing: without it the browser treats the
        // last segment as a file name and drops it when resolving.
        assert!(widget_base_href("https://x.test", "r", "b", "ws", "index.html").ends_with('/'));
    }

    /// A trailing slash on the configured base must not double up.
    #[test]
    fn base_href_tolerates_a_trailing_slash_on_the_public_base() {
        assert_eq!(
            widget_base_href("https://x.test/", "r", "b", "ws", "w/index.html"),
            "https://x.test/resources/r/b/ws/w/"
        );
    }

    /// The migration guarantee: a binding written against same-origin serving
    /// (`mode: uri-list`) keeps working when it is delivered inline instead.
    /// A spec-clean binding gets a spec-clean document with nothing injected.
    #[test]
    fn base_href_defaults_on_only_for_a_legacy_uri_list_binding() {
        let binding = |mode, base_href| UiBinding {
            mode,
            base_href,
            resource: None,
            entry: "w/index.html".into(),
            workspace: None,
            name: None,
            description: None,
            csp: None,
            permissions: None,
            domain: None,
            prefers_border: None,
            visibility: None,
        };
        assert!(binding(Some(UiMode::UriList), None).wants_base_href());
        assert!(!binding(Some(UiMode::Html), None).wants_base_href());
        assert!(!binding(None, None).wants_base_href());
        // An explicit value always wins over the mode-derived default.
        assert!(binding(Some(UiMode::Html), Some(true)).wants_base_href());
        assert!(!binding(Some(UiMode::UriList), Some(false)).wants_base_href());
    }

    /// HTML resolves against the FIRST `<base href>`, so injecting ours ahead
    /// of an author's would silently override theirs.
    #[test]
    fn an_authors_own_base_tag_is_left_alone() {
        let html = "<html><head><base href=\"https://mine.test/\"></head></html>";
        let out = inject_widget_preamble(html.to_string(), None, Some("https://ours.test/"));
        assert!(!out.contains("ours.test"));
        assert_eq!(out, html);
    }

    /// `<base>` must precede the origin script, and both must precede the
    /// document's own scripts — a base that lands after markup above it is
    /// too late to affect that markup.
    #[test]
    fn the_base_tag_precedes_the_origin_script_and_both_precede_the_document() {
        let out = inject_widget_preamble(
            "<html><head><script>boot()</script></head></html>".to_string(),
            Some("https://x.test"),
            Some("https://x.test/resources/r/b/ws/w/"),
        );
        let base = out.find("<base").expect("base injected");
        let origin = out
            .find("__RAISIN_SERVER_ORIGIN__")
            .expect("origin injected");
        let boot = out.find("boot()").expect("document preserved");
        assert!(base < origin, "base must come first: {out}");
        assert!(origin < boot, "preamble must precede the document: {out}");
    }

    /// The href embeds a workspace and asset path read from node properties,
    /// so it is author-controlled and lands in an attribute context.
    #[test]
    fn base_href_is_escaped_for_its_attribute_context() {
        let out = inject_widget_preamble(
            "<html><head></head></html>".to_string(),
            None,
            Some("https://x.test/a\"><script>alert(1)</script>/"),
        );
        assert!(!out.contains("<script>alert(1)"), "escaped: {out}");
        assert!(out.contains("&quot;"));
    }

    /// No public base means no base href — a widget with its own fallback must
    /// keep working rather than get a broken absolute base.
    #[test]
    fn nothing_is_injected_without_a_public_base() {
        let html = "<html><head></head></html>".to_string();
        assert_eq!(inject_widget_preamble(html.clone(), None, None), html);
    }

    /// `resources/templates/list` is standard MCP (since 2024-11-05) and part
    /// of the `resources` capability, so a client that sees `resources`
    /// advertised may call it while discovering. This server answered
    /// `-32601 unknown method`, and a client that treats a discovery error as
    /// fatal never went on to `resources/read` — so a working widget was never
    /// fetched and the host reported it as a failure to load the app.
    ///
    /// For a server with no templates the correct answer is an EMPTY LIST.
    /// Never an error: advertising a capability and then refusing its methods
    /// is what broke this.
    #[test]
    fn resource_templates_list_answers_instead_of_erroring() {
        use crate::dispatch::Dispatcher;
        use crate::registry::ToolRegistry;
        use crate::server::{DataPolicy, McpServerDescriptor};

        let descriptor = McpServerDescriptor {
            name: "T".into(),
            version: "1.0.0".into(),
            slug: "t".into(),
            instructions: None,
            public: true,
            scopes: Vec::new(),
            data_policy: DataPolicy {
                workspaces: vec!["stories".into()],
                operations: Vec::new(),
                resources: true,
            },
            custom_tools: Vec::new(),
            ui_resources: Vec::new(),
        };
        let dispatcher = Dispatcher::new(descriptor, ToolRegistry::new());

        let result = dispatcher
            .handle_resource_templates_list()
            .expect("must answer, never error");
        let templates = result["resourceTemplates"]
            .as_array()
            .expect("resourceTemplates must be an array");
        // No resource provider wired here, so the list is empty — and that is
        // still a valid answer.
        assert!(templates.is_empty());
    }
}
