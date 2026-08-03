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

//! What a widget asks the HOST to permit: its sandbox CSP and its permissions.
//!
//! These are declarative only — RaisinDB never enforces them. The host builds
//! the real `Content-Security-Policy` for the iframe it creates, which is why
//! the values are sanitized before they go on the wire (`dispatch::ui`) rather
//! than trusted here.
//!
//! Distinct from `raisin:StaticSiteFolder.serving_config`, which governs
//! RaisinDB's OWN http responses. A widget delivered inline is never fetched
//! over http, so that config does not apply to it — only to the images and
//! fonts it loads afterwards.

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Content Security Policy domains a widget declares (MCP Apps SEP-1865).
///
/// Hosts build the sandbox CSP from these; omitted lists mean the secure
/// default (no external access of that kind).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiCsp {
    /// Origins for network requests (fetch/XHR/WebSocket) — CSP `connect-src`.
    #[serde(
        default,
        alias = "connect_domains",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub connect_domains: Vec<String>,
    /// Origins for static resources (images/scripts/styles/fonts/media).
    #[serde(
        default,
        alias = "resource_domains",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub resource_domains: Vec<String>,
    /// Origins for nested iframes — CSP `frame-src`.
    #[serde(
        default,
        alias = "frame_domains",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub frame_domains: Vec<String>,
    /// Allowed base URIs — CSP `base-uri`.
    #[serde(
        default,
        alias = "base_uri_domains",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub base_uri_domains: Vec<String>,
}

impl UiCsp {
    /// Whether no domain list is declared at all.
    pub fn is_empty(&self) -> bool {
        self.connect_domains.is_empty()
            && self.resource_domains.is_empty()
            && self.frame_domains.is_empty()
            && self.base_uri_domains.is_empty()
    }
}

/// One requested sandbox permission.
///
/// Serializes as `{}` — SEP-1865 models a permission request as the PRESENCE of
/// an empty object, not as a boolean. Deserializes leniently from `{}`, `true`
/// or `null` so hand-written YAML (`camera: true`) means what its author
/// intended instead of shipping a value hosts ignore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPermissionGrant;

impl Serialize for UiPermissionGrant {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_map(Some(0))?.end()
    }
}

impl<'de> Deserialize<'de> for UiPermissionGrant {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        // Accept anything truthy-shaped; the value carries no information, only
        // its presence does. `false` is the one spelling that must NOT grant.
        match Value::deserialize(deserializer)? {
            Value::Bool(false) | Value::Null => Err(serde::de::Error::custom(
                "permission is granted by presence; use `{}` or omit the key",
            )),
            _ => Ok(UiPermissionGrant),
        }
    }
}

/// Sandbox permissions a widget requests (SEP-1865 `_meta.ui.permissions`).
///
/// Only these four are defined by the spec. Unknown keys are ignored rather
/// than rejected — a stricter parse would fail the whole descriptor, and
/// through `CustomTool`/`assemble_registry` that takes down the entire server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<UiPermissionGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<UiPermissionGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geolocation: Option<UiPermissionGrant>,
    #[serde(
        default,
        rename = "clipboardWrite",
        alias = "clipboard_write",
        skip_serializing_if = "Option::is_none"
    )]
    pub clipboard_write: Option<UiPermissionGrant>,
}

impl UiPermissions {
    /// Whether no permission is requested at all.
    pub fn is_empty(&self) -> bool {
        self.camera.is_none()
            && self.microphone.is_none()
            && self.geolocation.is_none()
            && self.clipboard_write.is_none()
    }
}
