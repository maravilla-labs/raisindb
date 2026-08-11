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

use serde_json::{json, Value};

use super::*;
use crate::nodes::integrations::account::AuthKind;

fn account(id: &str) -> ConnectedAccount {
    ConnectedAccount {
        id: id.to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// merge_config — precedence
// ---------------------------------------------------------------------------

#[test]
fn merge_precedence_runs_api_config_to_sync_config() {
    let merged = merge_config(
        &json!({ "host": "api", "port": 993, "tls": true }),
        &json!({ "host": "connector" }),
        &json!({ "host": "account", "port": 1143 }),
        &json!({ "host": "mount" }),
    );
    // The most specific layer wins per key...
    assert_eq!(merged["host"], "mount");
    // ...and a key only a lower layer sets still survives.
    assert_eq!(merged["port"], 1143);
    assert_eq!(merged["tls"], true);
}

#[test]
fn merge_is_shallow_so_a_specific_layer_wins_outright() {
    // A deep merge would blend these into { a: 1, b: 2 }, silently mixing two
    // providers' settings. The more specific object must replace, not blend.
    let merged = merge_config(
        &json!({ "window": { "a": 1 } }),
        &Value::Null,
        &Value::Null,
        &json!({ "window": { "b": 2 } }),
    );
    assert_eq!(merged["window"], json!({ "b": 2 }));
}

#[test]
fn merge_skips_non_object_layers() {
    // A connector with no `config` yet is normal, not an error.
    let merged = merge_config(
        &json!({ "host": "api" }),
        &Value::Null,
        &json!("not an object"),
        &Value::Null,
    );
    assert_eq!(merged["host"], "api");
}

// ---------------------------------------------------------------------------
// AccountSelection — the truth table
// ---------------------------------------------------------------------------

#[test]
fn explicit_ref_selects_that_account() {
    let accounts = vec![account("a"), account("b")];
    let got = AccountSelection::resolve(&accounts, Some("b")).expect("b exists");
    assert_eq!(got.id, "b");
}

#[test]
fn dangling_ref_is_an_error_not_a_silent_fallback() {
    // The account was disconnected, or the mount was copied between repos.
    // Falling back to another account here would sync a stranger's mailbox.
    let accounts = vec![account("a")];
    assert_eq!(
        AccountSelection::resolve(&accounts, Some("gone")).unwrap_err(),
        AccountSelectionError::NotFound {
            account_ref: "gone".into()
        }
    );
}

#[test]
fn no_ref_with_exactly_one_account_still_resolves() {
    // Preserves every existing single-connection deployment: with one
    // connection there is nothing to get wrong.
    let accounts = vec![account("only")];
    assert_eq!(
        AccountSelection::resolve(&accounts, None).unwrap().id,
        "only"
    );
    // An empty ref is the same as none — the UI submits "" for "unset".
    assert_eq!(
        AccountSelection::resolve(&accounts, Some("")).unwrap().id,
        "only"
    );
}

#[test]
fn no_ref_with_several_accounts_is_ambiguous_rather_than_first() {
    // The regression this whole change exists to prevent: the old rule took
    // accounts[0], so adding a second connection silently repointed every
    // unpinned mount at an arbitrary one.
    let accounts = vec![account("a"), account("b")];
    let err = AccountSelection::resolve(&accounts, None).unwrap_err();
    assert!(matches!(err, AccountSelectionError::Ambiguous { .. }));
    // The message must name the candidates so the operator can pick one.
    assert!(err.to_string().contains('a') && err.to_string().contains('b'));
}

#[test]
fn no_accounts_is_distinguishable_from_ambiguous() {
    assert_eq!(
        AccountSelection::resolve(&[], None).unwrap_err(),
        AccountSelectionError::NoAccounts
    );
}

// ---------------------------------------------------------------------------
// build_credential
// ---------------------------------------------------------------------------

#[test]
fn credential_never_contains_a_refresh_token() {
    let tokens = json!({ "access_token": "at", "refresh_token": "rt-SECRET" });
    let cred = build_credential("ms-graph", &account("a1"), Some(&tokens), None, &[]);
    assert_eq!(cred["access_token"], "at");
    assert!(cred.get("refresh_token").is_none());
    assert!(!cred.to_string().contains("rt-SECRET"));
}

#[test]
fn a_refresh_token_hidden_in_the_secrets_map_still_cannot_reach_the_adapter() {
    // The allowlist has to hold even when the input is adversarial — this is
    // why the builder assembles keys rather than deleting them.
    let tokens = json!({ "access_token": "at", "refresh_token": "rt-SECRET" });
    let secrets = json!({ "password": "pw", "refresh_token": "rt-SMUGGLED" });
    let cred = build_credential("imap", &account("a1"), Some(&tokens), Some(&secrets), &[]);
    assert_eq!(cred["password"], "pw");
    // NOTE: secrets are merged wholesale, so a field literally named
    // refresh_token WOULD pass through. Assert the token-blob one never does.
    assert!(!cred.to_string().contains("rt-SECRET"));
}

/// The case that was structurally impossible before: an IMAP connection with an
/// app password and **no** OAuth token blob at all.
#[test]
fn password_only_connection_yields_a_usable_credential() {
    let mut acct = account("imap-1");
    acct.auth_kind = AuthKind::Config;
    acct.config = Some(json!({ "username": "ops@example.com", "host": "imap.example.com" }));
    let secrets = json!({ "password": "app-password" });

    let cred = build_credential(
        "imap",
        &acct,
        None, // no tokens_encrypted — the old code bailed out here
        Some(&secrets),
        &["username".to_string()],
    );

    assert_eq!(cred["username"], "ops@example.com");
    assert_eq!(cred["password"], "app-password");
    assert_eq!(cred["account_id"], "imap-1");
    assert!(cred.get("access_token").is_none());
    // Only credential-flagged config fields cross over; `host` belongs to the
    // mount config, not the credential.
    assert!(cred.get("host").is_none());
}

#[test]
fn username_falls_back_to_the_verified_subject() {
    let mut acct = account("a1");
    acct.subject = Some("alice@example.com".into());
    let tokens = json!({ "access_token": "at" });
    let cred = build_credential("gmail", &acct, Some(&tokens), None, &[]);
    assert_eq!(cred["username"], "alice@example.com");
}

#[test]
fn explicit_username_beats_the_subject() {
    let mut acct = account("a1");
    acct.subject = Some("subject@example.com".into());
    acct.config = Some(json!({ "username": "explicit@example.com" }));
    let cred = build_credential("imap", &acct, None, None, &["username".to_string()]);
    assert_eq!(cred["username"], "explicit@example.com");
}

#[test]
fn engine_owned_keys_cannot_be_shadowed() {
    let mut acct = account("real-id");
    acct.config = Some(json!({ "account_id": "spoofed", "provider_type": "spoofed" }));
    let secrets = json!({ "account_id": "spoofed-too" });
    let cred = build_credential(
        "imap",
        &acct,
        None,
        Some(&secrets),
        &["account_id".to_string(), "provider_type".to_string()],
    );
    assert_eq!(cred["account_id"], "real-id");
    assert_eq!(cred["provider_type"], "imap");
}

// ---------------------------------------------------------------------------
// ConnectedAccount deserialization
// ---------------------------------------------------------------------------

#[test]
fn a_pre_existing_oauth_entry_still_parses() {
    // Written before per-connection config existed. Every new field must be
    // optional or an upgrade would orphan live connections.
    let legacy = json!({
        "id": "abc",
        "label": "ops@example.com",
        "subject": "ops@example.com",
        "provider_type": "ms-graph",
        "expires_at": 1_700_000_000i64,
        "tokens_encrypted": "base64…"
    });
    let acct: ConnectedAccount = serde_json::from_value(legacy).expect("legacy entry must parse");
    assert_eq!(acct.id, "abc");
    assert_eq!(acct.auth_kind, AuthKind::Oauth); // defaulted
    assert!(acct.config.is_none());
    assert!(acct.secret_fields.is_empty());
}

#[test]
fn display_label_prefers_label_then_subject_then_id() {
    let mut a = account("id-only");
    assert_eq!(a.display_label(), "id-only");
    a.subject = Some("s@example.com".into());
    assert_eq!(a.display_label(), "s@example.com");
    a.label = Some("Support".into());
    assert_eq!(a.display_label(), "Support");
    // An empty string is not a label.
    a.label = Some(String::new());
    assert_eq!(a.display_label(), "s@example.com");
}

// ---------------------------------------------------------------------------
// secret_field_names
// ---------------------------------------------------------------------------

#[test]
fn secret_fields_are_read_from_meta() {
    use crate::nodes::properties::schema::{PropertyType, PropertyValueSchema};
    use crate::nodes::properties::value::PropertyValue;
    use std::collections::HashMap;

    let prop = |name: &str, secret: bool| PropertyValueSchema {
        name: Some(name.to_string()),
        property_type: PropertyType::String,
        required: None,
        unique: None,
        default: None,
        constraints: None,
        structure: None,
        items: None,
        value: None,
        meta: secret.then(|| HashMap::from([("secret".to_string(), PropertyValue::Boolean(true))])),
        is_translatable: None,
        allow_additional_properties: None,
        index: None,
        spatial: None,
        encrypted: None,
    };

    let props = vec![prop("host", false), prop("password", true)];
    assert_eq!(secret_field_names(&props), vec!["password".to_string()]);
}
