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

//! SQL parameter substitution for the functions runtime.
//!
//! The placeholder scan itself lives in `raisin_sql::substitute_params_with`;
//! this module supplies only the literal formatting the functions runtime
//! wants, which differs from the SQL layer's in one respect: a JSON array
//! becomes an `ARRAY[...]` literal here, where the SQL layer serializes it to a
//! quoted JSON string.
//!
//! There were three byte-identical copies of this substitution — in `sql.rs`,
//! `query_context.rs` and `transaction/helpers.rs` — and each hand-rolled the
//! scan as a `str::replace` per index. That is wrong for ten or more
//! parameters: replacing `$1` first also rewrites the `$1` prefix of `$10`,
//! leaving a stray digit, so `IN ($1, ..., $10)` became `IN ('a', ..., 'a'0)`
//! and the parser reported `Expected: ), found: 0`. Keep ONE scan.

use raisin_error::Error;
use serde_json::Value;

/// Substitute `$1`, `$2`, ... with escaped literals, functions-runtime style.
///
/// # Errors
/// Propagates [`raisin_sql::substitute_params_with`]: a `$0` placeholder, or a
/// placeholder index with no corresponding parameter.
pub(crate) fn substitute_params(sql: &str, params: &[Value]) -> Result<String, Error> {
    raisin_sql_execution::substitute_params_with(sql, params, &format_value)
}

/// Render one parameter as a SQL literal.
fn format_value(value: &Value) -> String {
    match value {
        Value::String(s) => quote(s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "NULL".to_string(),
        // A SQL array literal, not a JSON string — this is the one place the
        // functions runtime deliberately differs from the SQL layer.
        Value::Array(items) => format!(
            "ARRAY[{}]",
            items
                .iter()
                .map(json_value_to_sql)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(_) => quote(&value.to_string()),
    }
}

/// Render one ARRAY element as a SQL literal (nested containers become JSON).
fn json_value_to_sql(value: &Value) -> String {
    match value {
        Value::String(s) => quote(s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "NULL".to_string(),
        _ => quote(&value.to_string()),
    }
}

/// Single-quote a value, doubling embedded quotes (SQL standard escaping).
fn quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn substitutes_multi_digit_placeholders() {
        // The regression: with ten or more parameters, a per-index
        // `str::replace` rewrites the `$1` inside `$10` and leaves a `0`.
        let params: Vec<Value> = (1..=12).map(|i| json!(format!("/p{i}"))).collect();
        let placeholders = (1..=12)
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT path FROM ws WHERE path IN ({placeholders})");

        let out = substitute_params(&sql, &params).unwrap();

        assert_eq!(
            out,
            "SELECT path FROM ws WHERE path IN ('/p1', '/p2', '/p3', '/p4', '/p5', \
             '/p6', '/p7', '/p8', '/p9', '/p10', '/p11', '/p12')"
        );
        // Belt and braces: the old bug's signature was a digit orphaned
        // immediately after a closing quote.
        assert!(!out.contains("'0"), "stray digit left by prefix collision");
    }

    #[test]
    fn substitutes_strings_numbers_and_several_params() {
        assert_eq!(
            substitute_params(
                "SELECT * FROM content WHERE path = $1 AND id = $2 LIMIT $3",
                &[json!("/test"), json!("abc123"), json!(10)],
            )
            .unwrap(),
            "SELECT * FROM content WHERE path = '/test' AND id = 'abc123' LIMIT 10"
        );
    }

    #[test]
    fn formats_arrays_as_sql_array_literals() {
        let out = substitute_params("SELECT $1", &[json!(["a", 2, true])]).unwrap();
        assert_eq!(out, "SELECT ARRAY['a', 2, true]");
    }

    #[test]
    fn escapes_embedded_quotes() {
        let out = substitute_params("SELECT $1", &[json!("John's")]).unwrap();
        assert_eq!(out, "SELECT 'John''s'");
    }

    #[test]
    fn missing_parameter_is_an_error() {
        // Previously the placeholder was left in the SQL verbatim and failed
        // later, in the parser, with a message that named neither the query
        // nor the parameter.
        assert!(substitute_params("SELECT $2", &[json!("only-one")]).is_err());
    }
}
