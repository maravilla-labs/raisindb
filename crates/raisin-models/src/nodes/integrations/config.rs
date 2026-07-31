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

//! Parsing a `raisin:Integration` node and layering its configuration.

use serde_json::{Map, Value};

use super::account::ConnectedAccount;
use crate::nodes::properties::schema::PropertyValueSchema;
use crate::nodes::Node;

/// A parsed `raisin:Integration` node — only the fields consumers need.
#[derive(Debug, Clone, Default)]
pub struct IntegrationConfig {
    pub provider_type: String,
    pub adapter_function: Option<String>,
    pub accounts: Vec<ConnectedAccount>,
    /// Legacy free-form connection settings. Predates `config_type`; retained
    /// verbatim and kept as the lowest merge layer so shipped templates (IMAP
    /// host/port/tls) keep working untouched.
    pub api_config: Value,
    /// Connector-level values validated against `config_type`.
    pub config: Value,
    /// NodeType naming the connector-level config schema.
    pub config_type: Option<String>,
    /// NodeType naming the per-connection config schema.
    pub connection_config_type: Option<String>,
    /// base64 AES-256-GCM of the connector-level `{ field: value }` secrets.
    pub config_secrets_encrypted: Option<String>,
}

impl IntegrationConfig {
    /// Parse a `raisin:Integration` node.
    ///
    /// Returns `Err` only when `provider_type` is missing — everything else is
    /// optional, so a v1 connector node parses into the v2 shape with empty
    /// config fields.
    pub fn from_node(node: &Node) -> Result<Self, String> {
        let provider_type = string_prop(node, "provider_type")
            .ok_or_else(|| "integration is missing provider_type".to_string())?;

        // A malformed connected_accounts array must not take the whole connector
        // down; an unparseable entry is dropped and the rest still resolve.
        let accounts = raw_value(node, "connected_accounts")
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| serde_json::from_value::<ConnectedAccount>(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(Self {
            provider_type,
            adapter_function: string_prop(node, "adapter_function"),
            accounts,
            api_config: raw_value(node, "api_config"),
            config: raw_value(node, "config"),
            config_type: string_prop(node, "config_type"),
            connection_config_type: string_prop(node, "connection_config_type"),
            config_secrets_encrypted: string_prop(node, "config_secrets_encrypted"),
        })
    }

    /// Resolve the effective adapter path for a mount, honouring its override.
    pub fn adapter_for(&self, mount_override: Option<&str>) -> Option<String> {
        mount_override
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| self.adapter_function.clone())
    }
}

/// Layer the four configuration sources an adapter can be configured from.
///
/// Lowest to highest precedence:
///
/// 1. `api_config` — legacy template defaults (IMAP host/port/tls).
/// 2. `config` — connector-level, from `config_type`.
/// 3. account `config` — per-connection, from `connection_config_type`. This is
///    the layer that lets one connector hold two Entra tenants or two mailboxes.
/// 4. `sync_config` — the mount's own overrides.
///
/// The merge is **shallow, per top-level key** — matching what the IMAP adapter
/// already does by hand (`sync_config` wins over `api_config`, key by key), so
/// adding layers 2 and 3 underneath cannot change existing behaviour. A deep
/// merge would silently blend two providers' nested objects instead of letting
/// the more specific one win outright.
///
/// Non-object layers are skipped rather than erroring: a connector with no
/// `config` yet is the normal case, not a fault.
pub fn merge_config(
    api_config: &Value,
    connector_config: &Value,
    account_config: &Value,
    sync_config: &Value,
) -> Value {
    let mut out = Map::new();
    for layer in [api_config, connector_config, account_config, sync_config] {
        if let Value::Object(map) = layer {
            for (k, v) in map {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}

/// Names of the properties a config NodeType flags as secret (`meta.secret`).
///
/// Used to route values away from the plaintext `config` object and into the
/// encrypted blob, and to render write-only fields in the console. Reading it
/// from `meta` is not incidental: `PropertyValueSchema` drops top-level
/// `title`/`description`/`enum`, so `meta` is the only free-form carrier that
/// survives a schema round trip.
pub fn secret_field_names(properties: &[PropertyValueSchema]) -> Vec<String> {
    properties
        .iter()
        .filter(|p| meta_flag(p, "secret"))
        .filter_map(|p| p.name.clone())
        .collect()
}

/// Whether a property carries a truthy `meta.<key>` flag.
pub(super) fn meta_flag(schema: &PropertyValueSchema, key: &str) -> bool {
    use crate::nodes::properties::value::PropertyValue;
    schema
        .meta
        .as_ref()
        .and_then(|m| m.get(key))
        .is_some_and(|v| matches!(v, PropertyValue::Boolean(true)))
}

/// Read a non-empty string property.
fn string_prop(node: &Node, key: &str) -> Option<String> {
    use crate::nodes::properties::value::PropertyValue;
    match node.properties.get(key)? {
        PropertyValue::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Read a property as raw JSON, defaulting to `Null`.
fn raw_value(node: &Node, key: &str) -> Value {
    node.properties
        .get(key)
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null)
}
