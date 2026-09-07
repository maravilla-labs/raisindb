//! SQL-standard function syntax that sqlparser represents as dedicated AST
//! nodes rather than `Function` calls:
//!
//! - `SUBSTRING(x FROM a FOR b)` / `SUBSTR(x, a, b)`  → `SUBSTR(x, a[, b])`
//! - `TRIM([BOTH|LEADING|TRAILING] [c] FROM x)`       → `BTRIM|LTRIM|RTRIM(x[, c])`
//! - `EXTRACT(field FROM x)`                          → `DATE_PART('field', x)`
//! - `POSITION(a IN b)`                               → `STRPOS(b, a)`
//! - `CEIL(x)` / `FLOOR(x)`                           → `CEIL(x)` / `FLOOR(x)`
//! - `TIMESTAMP '...'` / `DATE '...'`                 → `CAST('...' AS ...)`
//!
//! Each form is lowered onto the named function so the registry, the constant
//! folder and the executor see exactly one spelling.

use crate::analyzer::{
    error::AnalysisError,
    semantic::{AnalyzerContext, Result},
    typed_expr::{Literal, TypedExpr},
};
use sqlparser::ast::{
    CeilFloorKind, DataType as SqlDataType, DateTimeField, Expr as SqlExpr, TrimWhereField,
    TypedString, Value,
};

impl<'a> AnalyzerContext<'a> {
    pub(in crate::analyzer::semantic) fn analyze_substring(
        &self,
        expr: &SqlExpr,
        from: Option<&SqlExpr>,
        length: Option<&SqlExpr>,
    ) -> Result<TypedExpr> {
        let mut args = vec![self.analyze_expr(expr)?];
        args.push(match from {
            Some(from) => self.analyze_expr(from)?,
            None => TypedExpr::literal(Literal::Int(1)),
        });
        if let Some(length) = length {
            args.push(self.analyze_expr(length)?);
        }
        self.analyze_scalar_call("SUBSTR", args)
    }

    pub(in crate::analyzer::semantic) fn analyze_trim(
        &self,
        expr: &SqlExpr,
        trim_where: Option<&TrimWhereField>,
        trim_what: Option<&SqlExpr>,
        trim_characters: Option<&[SqlExpr]>,
    ) -> Result<TypedExpr> {
        let name = match trim_where {
            Some(TrimWhereField::Leading) => "LTRIM",
            Some(TrimWhereField::Trailing) => "RTRIM",
            Some(TrimWhereField::Both) | None => "BTRIM",
        };
        let mut args = vec![self.analyze_expr(expr)?];
        // `TRIM(x, 'c')` (the MySQL-flavoured form) carries its characters in
        // `trim_characters`; the SQL form carries them in `trim_what`.
        let chars = trim_what.or_else(|| trim_characters.and_then(|c| c.first()));
        if let Some(chars) = chars {
            args.push(self.analyze_expr(chars)?);
        }
        self.analyze_scalar_call(name, args)
    }

    pub(in crate::analyzer::semantic) fn analyze_extract(
        &self,
        field: &DateTimeField,
        expr: &SqlExpr,
    ) -> Result<TypedExpr> {
        let field_name = match field {
            DateTimeField::Custom(ident) => ident.value.to_lowercase(),
            other => other.to_string().to_lowercase(),
        };
        let args = vec![
            TypedExpr::literal(Literal::Text(field_name)),
            self.analyze_expr(expr)?,
        ];
        self.analyze_scalar_call("DATE_PART", args)
    }

    /// `POSITION(needle IN haystack)` — note the argument order flips for
    /// `STRPOS(haystack, needle)`.
    pub(in crate::analyzer::semantic) fn analyze_position(
        &self,
        needle: &SqlExpr,
        haystack: &SqlExpr,
    ) -> Result<TypedExpr> {
        let args = vec![self.analyze_expr(haystack)?, self.analyze_expr(needle)?];
        self.analyze_scalar_call("STRPOS", args)
    }

    pub(in crate::analyzer::semantic) fn analyze_ceil_floor(
        &self,
        name: &str,
        expr: &SqlExpr,
        kind: &CeilFloorKind,
    ) -> Result<TypedExpr> {
        match kind {
            CeilFloorKind::DateTimeField(DateTimeField::NoDateTime) => {}
            CeilFloorKind::Scale(Value::Number(n, _)) if n == "0" => {}
            other => {
                return Err(AnalysisError::UnsupportedExpression(format!(
                    "{}(expr TO/scale {:?}) is not supported; use DATE_TRUNC or ROUND",
                    name, other
                )))
            }
        }
        self.analyze_scalar_call(name, vec![self.analyze_expr(expr)?])
    }

    /// `DATE '2024-01-01'`, `TIMESTAMP '2024-01-01 10:00'`: a typed literal is
    /// a cast of the string.
    pub(in crate::analyzer::semantic) fn analyze_typed_string(
        &self,
        typed: &TypedString,
    ) -> Result<TypedExpr> {
        let text = match &typed.value.value {
            Value::SingleQuotedString(s)
            | Value::DoubleQuotedString(s)
            | Value::EscapedStringLiteral(s) => s.clone(),
            other => {
                return Err(AnalysisError::UnsupportedExpression(format!(
                    "typed literal value {:?}",
                    other
                )))
            }
        };
        let data_type: &SqlDataType = &typed.data_type;
        let literal = SqlExpr::Value(Value::SingleQuotedString(text).into());
        self.analyze_cast(&literal, data_type)
    }
}
