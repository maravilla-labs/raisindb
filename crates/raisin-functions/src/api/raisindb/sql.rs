// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! SQL operation implementations for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_sql_query(&self, sql: &str, params: Vec<Value>) -> Result<Value> {
        let callback = self.callbacks.sql_query.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("SQL query callback not configured".to_string())
        })?;

        callback(sql.to_string(), params).await
    }

    pub(crate) async fn impl_sql_execute(&self, sql: &str, params: Vec<Value>) -> Result<i64> {
        let callback = self.callbacks.sql_execute.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("SQL execute callback not configured".to_string())
        })?;

        callback(sql.to_string(), params).await
    }
}
