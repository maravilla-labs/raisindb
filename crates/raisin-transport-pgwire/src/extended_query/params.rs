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

//! Parameter binding and extraction for the extended query protocol.
//!
//! Converts pgwire portal parameters into JSON values and substitutes
//! them into SQL placeholders using the shared `substitute_params` logic.

use crate::auth::ApiKeyValidator;
use crate::error::{PgWireTransportError, Result};
use pgwire::api::portal::Portal;
use pgwire::api::Type;
use raisin_sql::substitute_params;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::Value as JsonValue;
use tracing::{debug, warn};

use super::statement::RaisinStatement;
use super::RaisinExtendedQueryHandler;

impl<S, V, P> RaisinExtendedQueryHandler<S, V, P>
where
    S: Storage + TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    /// Bind parameters to SQL by replacing placeholders with values.
    ///
    /// This uses the same parameter substitution logic as the HTTP transport
    /// (raisin_sql::substitute_params) to ensure consistent behavior.
    ///
    /// # Arguments
    ///
    /// * `portal` - The portal containing the statement and bound parameters
    ///
    /// # Returns
    ///
    /// The SQL string with parameters substituted, or an error
    pub(crate) fn bind_parameters(&self, portal: &Portal<RaisinStatement>) -> Result<String> {
        let sql = &portal.statement.statement.sql;
        let param_count = portal.parameter_len();

        debug!("Binding {} parameters to SQL: {}", param_count, sql);

        if param_count == 0 {
            // No parameters to bind
            return Ok(sql.clone());
        }

        // Convert pgwire parameters to JSON values for substitute_params
        let mut json_params: Vec<JsonValue> = Vec::with_capacity(param_count);

        for i in 0..param_count {
            // Get the parameter type from the statement
            let param_type = portal
                .statement
                .parameter_types
                .get(i)
                .cloned()
                .unwrap_or(Type::TEXT);

            let json_value = self.extract_parameter_as_json(portal, i, &param_type)?;
            debug!(
                "Parameter ${}: {:?} (type: {:?})",
                i + 1,
                json_value,
                param_type
            );
            json_params.push(json_value);
        }

        // Use the same substitution logic as HTTP transport
        let result = substitute_params(sql, &json_params).map_err(|e| {
            PgWireTransportError::internal(format!("Parameter substitution failed: {}", e))
        })?;

        debug!("SQL after parameter binding: {}", result);
        Ok(result)
    }

    /// Extract a parameter from the portal and convert it to a JSON value.
    ///
    /// This handles the parameter extraction based on the PostgreSQL type.
    fn extract_parameter_as_json(
        &self,
        portal: &Portal<RaisinStatement>,
        index: usize,
        param_type: &Type,
    ) -> Result<JsonValue> {
        match *param_type {
            Type::BOOL => {
                let value = portal.parameter::<bool>(index, param_type).map_err(|e| {
                    PgWireTransportError::internal(format!(
                        "Failed to extract BOOL parameter {}: {}",
                        index, e
                    ))
                })?;
                Ok(value.map(JsonValue::Bool).unwrap_or(JsonValue::Null))
            }
            Type::INT2 => {
                let value = portal.parameter::<i16>(index, param_type).map_err(|e| {
                    PgWireTransportError::internal(format!(
                        "Failed to extract INT2 parameter {}: {}",
                        index, e
                    ))
                })?;
                Ok(value
                    .map(|v| JsonValue::Number(v.into()))
                    .unwrap_or(JsonValue::Null))
            }
            Type::INT4 => {
                let value = portal.parameter::<i32>(index, param_type).map_err(|e| {
                    PgWireTransportError::internal(format!(
                        "Failed to extract INT4 parameter {}: {}",
                        index, e
                    ))
                })?;
                Ok(value
                    .map(|v| JsonValue::Number(v.into()))
                    .unwrap_or(JsonValue::Null))
            }
            Type::INT8 => {
                let value = portal.parameter::<i64>(index, param_type).map_err(|e| {
                    PgWireTransportError::internal(format!(
                        "Failed to extract INT8 parameter {}: {}",
                        index, e
                    ))
                })?;
                Ok(value
                    .map(|v| JsonValue::Number(v.into()))
                    .unwrap_or(JsonValue::Null))
            }
            Type::FLOAT4 => {
                let value = portal.parameter::<f32>(index, param_type).map_err(|e| {
                    PgWireTransportError::internal(format!(
                        "Failed to extract FLOAT4 parameter {}: {}",
                        index, e
                    ))
                })?;
                Ok(value
                    .and_then(|v| serde_json::Number::from_f64(v as f64))
                    .map(JsonValue::Number)
                    .unwrap_or(JsonValue::Null))
            }
            Type::FLOAT8 => {
                let value = portal.parameter::<f64>(index, param_type).map_err(|e| {
                    PgWireTransportError::internal(format!(
                        "Failed to extract FLOAT8 parameter {}: {}",
                        index, e
                    ))
                })?;
                Ok(value
                    .and_then(serde_json::Number::from_f64)
                    .map(JsonValue::Number)
                    .unwrap_or(JsonValue::Null))
            }
            Type::TEXT | Type::VARCHAR => {
                let value = portal.parameter::<String>(index, param_type).map_err(|e| {
                    PgWireTransportError::internal(format!(
                        "Failed to extract TEXT/VARCHAR parameter {}: {}",
                        index, e
                    ))
                })?;
                // For TEXT parameters, try to detect if value is actually a number
                // This handles cases where JDBC drivers send LIMIT/OFFSET as text
                if let Some(ref s) = value {
                    // Try integer first
                    if let Ok(n) = s.parse::<i64>() {
                        debug!("TEXT parameter {} looks like integer: {}", index, n);
                        return Ok(JsonValue::Number(n.into()));
                    }
                    // Try float
                    if let Ok(n) = s.parse::<f64>() {
                        if let Some(num) = serde_json::Number::from_f64(n) {
                            debug!("TEXT parameter {} looks like float: {}", index, n);
                            return Ok(JsonValue::Number(num));
                        }
                    }
                }
                Ok(value.map(JsonValue::String).unwrap_or(JsonValue::Null))
            }
            Type::UUID => {
                let value = portal.parameter::<String>(index, param_type).map_err(|e| {
                    PgWireTransportError::internal(format!(
                        "Failed to extract UUID parameter {}: {}",
                        index, e
                    ))
                })?;
                Ok(value.map(JsonValue::String).unwrap_or(JsonValue::Null))
            }
            _ => {
                // For unsupported types, try to get as string
                warn!(
                    "Unsupported parameter type {:?} at index {}, treating as TEXT",
                    param_type, index
                );
                let value = portal.parameter::<String>(index, param_type).map_err(|e| {
                    PgWireTransportError::internal(format!(
                        "Failed to extract parameter {} as TEXT: {}",
                        index, e
                    ))
                })?;
                Ok(value.map(JsonValue::String).unwrap_or(JsonValue::Null))
            }
        }
    }
}
