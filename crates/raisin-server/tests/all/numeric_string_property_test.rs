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

//! A property declared `String` keeps numeric-looking text, end to end.
//!
//! Postal codes, jersey and article numbers, phone numbers and identity subjects
//! are strings whose SPELLING is the point. A decimal and a string are the same
//! bytes on the wire — `rust_decimal` is built `serde-str` and `PropertyValue`'s
//! untagged ladder puts `Decimal` ahead of `String` — so the ladder claimed every
//! numeric-looking string, on the way in AND on the way back out of storage.
//!
//! Two distinct failures came from that one fact, and both are covered here
//! against a REAL server over the node REST API, which is the ingestion boundary
//! that package installs and importers use:
//!
//! - `"05"` was stored and read back as `5`. Silent data loss, on live content.
//! - `"20457"` was refused outright with "is declared String but the value is
//!   Decimal", which made German postcodes unwritable.
//!
//! The acceptance shape is the one the field asked for: for a property declared
//! `String`, the value round-trips byte-identically.

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{ServerConfig, ServerHandle};
use helpers::sql_geo::{bootstrap_admin, http_post, http_put};
use serde_json::{json, Value};
use std::time::Duration;

/// `sql_geo` has `http_post`/`http_put` but no GET, and it is shared with other
/// suites — so this one lives here rather than growing the common helper.
async fn http_get(base_url: &str, path: &str, token: &str) -> Result<Value, String> {
    let response = reqwest::Client::new()
        .get(format!("{base_url}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|_| text)
}

const REPO: &str = "numstr";
const BRANCH: &str = "main";
const WS: &str = "people";
const NODE_TYPE: &str = "test:Address";
const PORT: u16 = 8146;

async fn provision(base_url: &str, token: &str) {
    http_post(
        base_url,
        "/api/repositories",
        token,
        json!({ "repo_id": REPO, "description": "numeric string test repo",
                "default_branch": BRANCH }),
    )
    .await
    .expect("create repository");

    http_put(
        base_url,
        &format!("/api/workspaces/{REPO}/{WS}"),
        token,
        json!({
            "name": WS,
            "description": "Addresses",
            "allowed_node_types": [NODE_TYPE, "raisin:Folder"],
            "allowed_root_node_types": [NODE_TYPE, "raisin:Folder"],
            "depends_on": [],
            "config": { "default_branch": BRANCH, "node_type_pins": {} }
        }),
    )
    .await
    .expect("create workspace");

    http_post(
        base_url,
        &format!("/api/management/{REPO}/{BRANCH}/nodetypes"),
        token,
        json!({
            "node_type": {
                "name": NODE_TYPE,
                "description": "An address",
                // Every one of these is a String on purpose.
                "properties": [
                    { "name": "postal_code", "type": "String" },
                    { "name": "jersey_number", "type": "String" },
                    { "name": "amount_label", "type": "String" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "Create test NodeType", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Write one address through the node REST API and read it straight back.
async fn round_trip(base_url: &str, token: &str, name: &str, postal_code: &str) -> String {
    http_post(
        base_url,
        &format!("/api/repository/{REPO}/{BRANCH}/head/{WS}/"),
        token,
        json!({
            "name": name,
            "path": format!("/{name}"),
            "node_type": NODE_TYPE,
            "properties": { "postal_code": postal_code }
        }),
    )
    .await
    .unwrap_or_else(|e| panic!("writing postal_code {postal_code:?} was refused: {e}"));

    let read = http_get(
        base_url,
        &format!("/api/repository/{REPO}/{BRANCH}/head/{WS}/{name}"),
        token,
    )
    .await
    .unwrap_or_else(|e| panic!("reading {name} failed: {e}"));

    read.pointer("/properties/postal_code")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| {
            panic!(
                "postal_code did not come back as a JSON string: {:?}",
                read.pointer("/properties/postal_code")
            )
        })
}

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all numeric_string_property_test -- --ignored --nocapture
async fn a_string_property_keeps_numeric_text_through_the_node_api() {
    let server = ServerHandle::start(ServerConfig::new(PORT))
        .await
        .expect("failed to start server");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let token = bootstrap_admin(&server.base_url).await;
    provision(&server.base_url, &token).await;

    for (name, value) in CASES {
        let back = round_trip(&server.base_url, &token, name, value).await;
        assert_eq!(
            back, value,
            "postal_code {value:?} came back as {back:?} — the string was not preserved"
        );
    }
}

/// Every measured case from the field audit, on one `String`-declared property.
///
/// The two groups matter for different reasons. A value whose decimal spelling
/// would CHANGE (a leading zero or `+`, an exponent, something past 96 bits) is
/// kept a string by the deserializer itself. A value that is already canonical
/// (`76133`, a 9-digit DUNS, `1.5`) is genuinely indistinguishable from a decimal
/// on the wire, so the DECLARATION settles it in the validator — and because the
/// rendering is byte-identical either way, that is lossless. Both groups must
/// come back exactly as written.
const CASES: [(&str, &str); 20] = [
    // Postal codes: 8 of 14 countries audited were refused outright.
    ("de-karlsruhe", "76133"),
    ("de-leipzig", "01067"),
    ("de-hamburg", "20457"),
    ("it-roma", "00184"),
    ("us-ny", "10001"),
    ("jp", "1000001"),
    ("gb", "W1A 0AX"),
    ("pl", "00-001"),
    ("br", "01310-100"),
    // Phone numbers. Canonical E.164 was the one spelling that could not be
    // stored, while the loosely formatted local version beside it saved fine.
    ("e164", "+41442345678"),
    ("intl-00", "0044123456"),
    ("no-plus", "41442345678"),
    ("local", "0442345678"),
    ("spaced-phone", "+41 44 234 56 78"),
    ("hyphen-phone", "+41-44-234"),
    // Identifiers that are always digits.
    ("duns", "123456789"),
    ("jersey", "05"),
    // Shapes that decimal parsing would rewrite.
    ("exponent", "12e5"),
    ("decimalish", "1.5"),
    (
        "beyond-96-bits",
        "99999999999999999999999999999999999999999",
    ),
];

/// The SQL write path, which is what the Studio editor uses.
///
/// It stored these correctly all along — it does no coercion — so this is a
/// regression guard rather than a fix: whatever the node API now does, the two
/// paths must agree, or the fix has only moved the inconsistency.
#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all numeric_string_property_test -- --ignored --nocapture
async fn the_sql_path_stores_the_same_strings_as_the_node_api() {
    let server = ServerHandle::start(ServerConfig::new(PORT + 1))
        .await
        .expect("failed to start server");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let token = bootstrap_admin(&server.base_url).await;
    provision(&server.base_url, &token).await;

    for (name, value) in CASES {
        let sql_name = format!("sql-{name}");
        helpers::sql_geo::sql_ws(
            PORT + 1,
            REPO,
            BRANCH,
            &format!(
                "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES \
                 ('{sql_name}','/{sql_name}','{NODE_TYPE}', \
                 '{{\"postal_code\":\"{value}\"}}'::JSONB)"
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("SQL insert of {value:?} failed: {e}"));

        let read = http_get(
            &server.base_url,
            &format!("/api/repository/{REPO}/{BRANCH}/head/{WS}/{sql_name}"),
            &token,
        )
        .await
        .unwrap_or_else(|e| panic!("reading {sql_name} failed: {e}"));

        let back = read
            .pointer("/properties/postal_code")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{value:?} did not come back as a JSON string"));
        assert_eq!(
            back, value,
            "SQL path: {value:?} came back as {back:?} — the two write paths disagree"
        );
    }
}
