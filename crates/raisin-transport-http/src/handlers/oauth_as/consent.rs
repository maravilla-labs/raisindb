// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Renders the login + consent page served by `GET /authorize`.
//!
//! The page is a self-contained HTML form that POSTs back to `/authorize` with
//! the resource owner's credentials and every authorization parameter carried
//! through as a hidden field, so no server-side login session is required.

use raisin_auth::authserver::ValidatedAuthorizationRequest;

/// HTML-escape a string for safe interpolation into an attribute or text node.
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Emit a hidden `<input>` for `name`/`value` when the value is present.
fn hidden(name: &str, value: Option<&str>) -> String {
    match value {
        Some(v) => format!(
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">\n",
            escape(name),
            escape(v)
        ),
        None => String::new(),
    }
}

/// Render the login + consent form for a validated authorization request.
///
/// `repo` is the repository the MCP resource lives under, threaded through so
/// the POST handler provisions the user node and resolves permissions in the
/// right repository.
pub fn render_consent_form(validated: &ValidatedAuthorizationRequest, repo: &str) -> String {
    let scopes_display = if validated.requested_scopes.is_empty() {
        "<em>basic access</em>".to_string()
    } else {
        validated
            .requested_scopes
            .iter()
            .map(|s| format!("<li><code>{}</code></li>", escape(s)))
            .collect::<String>()
    };

    let mut hidden_fields = String::new();
    hidden_fields.push_str(&hidden("response_type", Some("code")));
    hidden_fields.push_str(&hidden("client_id", Some(&validated.client_id)));
    hidden_fields.push_str(&hidden("redirect_uri", Some(&validated.redirect_uri)));
    hidden_fields.push_str(&hidden("state", validated.state.as_deref()));
    hidden_fields.push_str(&hidden("code_challenge", Some(&validated.code_challenge)));
    hidden_fields.push_str(&hidden(
        "code_challenge_method",
        Some(&validated.code_challenge_method.to_string()),
    ));
    if !validated.requested_scopes.is_empty() {
        hidden_fields.push_str(&hidden(
            "scope",
            Some(&validated.requested_scopes.join(" ")),
        ));
    }
    hidden_fields.push_str(&hidden("resource", Some(&validated.resource)));
    hidden_fields.push_str(&hidden("repo", Some(repo)));

    format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>Authorize access</title>\n\
<style>\n\
body{{font-family:system-ui,sans-serif;max-width:28rem;margin:3rem auto;padding:0 1rem;}}\n\
h1{{font-size:1.25rem;}} ul{{padding-left:1.25rem;}} code{{background:#f3f3f3;padding:.1rem .3rem;border-radius:.2rem;}}\n\
label{{display:block;margin:.75rem 0 .25rem;}} input[type=email],input[type=password]{{width:100%;padding:.5rem;box-sizing:border-box;}}\n\
button{{margin-top:1.25rem;padding:.6rem 1.2rem;font-size:1rem;cursor:pointer;}}\n\
.resource{{color:#555;font-size:.9rem;word-break:break-all;}}\n\
</style>\n\
</head>\n\
<body>\n\
<h1>Authorize MCP access</h1>\n\
<p>An application is requesting access to the resource:</p>\n\
<p class=\"resource\"><code>{resource}</code></p>\n\
<p>Requested scopes:</p>\n\
<ul>{scopes}</ul>\n\
<form method=\"post\" action=\"/authorize\">\n\
{hidden}\
<label for=\"email\">Email</label>\n\
<input id=\"email\" type=\"email\" name=\"email\" autocomplete=\"username\" required>\n\
<label for=\"password\">Password</label>\n\
<input id=\"password\" type=\"password\" name=\"password\" autocomplete=\"current-password\" required>\n\
<button type=\"submit\">Sign in and authorize</button>\n\
</form>\n\
</body>\n\
</html>",
        resource = escape(&validated.resource),
        scopes = scopes_display,
        hidden = hidden_fields,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_auth::authserver::CodeChallengeMethod;

    fn validated() -> ValidatedAuthorizationRequest {
        ValidatedAuthorizationRequest {
            client_id: "client-1".to_string(),
            redirect_uri: "http://127.0.0.1:9000/cb".to_string(),
            state: Some("xyz".to_string()),
            code_challenge: "abc".to_string(),
            code_challenge_method: CodeChallengeMethod::S256,
            requested_scopes: vec!["reader".to_string()],
            resource: "https://db.example.com/mcp/repo/main/srv".to_string(),
        }
    }

    #[test]
    fn form_carries_all_parameters() {
        let html = render_consent_form(&validated(), "repo");
        assert!(html.contains("name=\"client_id\" value=\"client-1\""));
        assert!(html.contains("name=\"code_challenge\" value=\"abc\""));
        assert!(html.contains("name=\"code_challenge_method\" value=\"S256\""));
        assert!(html.contains("name=\"resource\""));
        assert!(html.contains("name=\"repo\" value=\"repo\""));
        assert!(html.contains("name=\"state\" value=\"xyz\""));
        assert!(html.contains("<code>reader</code>"));
    }

    #[test]
    fn escaping_prevents_injection() {
        let mut v = validated();
        v.state = Some("\"><script>alert(1)</script>".to_string());
        let html = render_consent_form(&v, "repo");
        assert!(!html.contains("<script>alert(1)"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
