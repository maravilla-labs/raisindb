//! Regular-expression operators (`~`, `~*`, `!~`, `!~*`, `SIMILAR TO`) and
//! quantified comparisons (`= ANY(...)`, `> ALL(...)`).

use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{BinaryOperator, Literal, TypedExpr};
use regex::Regex;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::core::eval_expr;
use super::helpers::compare_literals;

/// Compiled patterns, keyed by (source, case-insensitive). Bounded: when it
/// fills up it is cleared rather than evicted by policy — a query's patterns
/// are few and repeat per row, which is the case this serves.
const REGEX_CACHE_CAPACITY: usize = 256;

static REGEX_CACHE: Mutex<Option<HashMap<(String, bool), Arc<Regex>>>> = Mutex::new(None);

fn compile_cached(source: &str, case_insensitive: bool) -> Result<Arc<Regex>, Error> {
    let key = (source.to_string(), case_insensitive);
    let mut guard = REGEX_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(re) = cache.get(&key) {
        return Ok(Arc::clone(re));
    }
    let re = regex::RegexBuilder::new(source)
        .case_insensitive(case_insensitive)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| Error::Validation(format!("invalid regular expression '{source}': {e}")))?;
    let re = Arc::new(re);
    if cache.len() >= REGEX_CACHE_CAPACITY {
        cache.clear();
    }
    cache.insert(key, Arc::clone(&re));
    Ok(re)
}

fn literal_text(lit: &Literal) -> Option<String> {
    match lit {
        Literal::Null => None,
        Literal::Text(s) | Literal::Uuid(s) | Literal::Path(s) => Some(s.clone()),
        Literal::Int(i) => Some(i.to_string()),
        Literal::BigInt(i) => Some(i.to_string()),
        Literal::Double(d) => Some(d.to_string()),
        Literal::Boolean(b) => Some(b.to_string()),
        Literal::JsonB(serde_json::Value::String(s)) => Some(s.clone()),
        Literal::JsonB(v) => Some(v.to_string()),
        Literal::Timestamp(ts) => Some(ts.to_rfc3339()),
        _ => None,
    }
}

/// `expr ~ pattern` and friends. NULL on either side yields NULL, as in
/// PostgreSQL.
pub(super) fn eval_regex(
    expr: &TypedExpr,
    pattern: &TypedExpr,
    case_insensitive: bool,
    negated: bool,
    similar_to: bool,
    row: &Row,
) -> Result<Literal, Error> {
    let text = literal_text(&eval_expr(expr, row)?);
    let pat = literal_text(&eval_expr(pattern, row)?);
    let (Some(text), Some(pat)) = (text, pat) else {
        return Ok(Literal::Null);
    };
    let source = if similar_to {
        raisin_sql::analyzer::regex_pattern::similar_to_regex(&pat)
    } else {
        pat
    };
    let re = compile_cached(&source, case_insensitive)?;
    let matched = re.is_match(&text);
    Ok(Literal::Boolean(matched != negated))
}

/// Decode the array side of ANY/ALL into literals.
///
/// Accepts a JSONB array, a JSON-array text (what a bound `$1` array
/// parameter is rendered as), a PostgreSQL `{a,b,c}` array text, and a
/// numeric vector.
fn array_elements(value: Literal) -> Result<Vec<Literal>, Error> {
    let from_json = |items: Vec<serde_json::Value>| -> Vec<Literal> {
        items
            .into_iter()
            .map(|v| match v {
                serde_json::Value::Null => Literal::Null,
                serde_json::Value::Bool(b) => Literal::Boolean(b),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        Literal::BigInt(i)
                    } else {
                        Literal::Double(n.as_f64().unwrap_or(f64::NAN))
                    }
                }
                serde_json::Value::String(s) => Literal::Text(s),
                other => Literal::JsonB(other),
            })
            .collect()
    };
    match value {
        Literal::Null => Ok(Vec::new()),
        Literal::JsonB(serde_json::Value::Array(items)) => Ok(from_json(items)),
        Literal::JsonB(other) => Err(Error::Validation(format!(
            "ANY/ALL expects an array, got JSON {other}"
        ))),
        Literal::Vector(v) => Ok(v.into_iter().map(|f| Literal::Double(f as f64)).collect()),
        Literal::Text(s) => {
            let trimmed = s.trim();
            if trimmed.starts_with('[') {
                match serde_json::from_str::<serde_json::Value>(trimmed) {
                    Ok(serde_json::Value::Array(items)) => Ok(from_json(items)),
                    _ => Err(Error::Validation(format!(
                        "ANY/ALL expects a JSON array, got '{s}'"
                    ))),
                }
            } else if let Some(inner) = trimmed.strip_prefix('{').and_then(|t| t.strip_suffix('}'))
            {
                if inner.trim().is_empty() {
                    return Ok(Vec::new());
                }
                Ok(inner
                    .split(',')
                    .map(|item| {
                        let item = item.trim();
                        let item = item
                            .strip_prefix('"')
                            .and_then(|i| i.strip_suffix('"'))
                            .unwrap_or(item);
                        if item.eq_ignore_ascii_case("null") {
                            Literal::Null
                        } else {
                            Literal::Text(item.to_string())
                        }
                    })
                    .collect())
            } else {
                Err(Error::Validation(format!(
                    "ANY/ALL expects an array value (ARRAY[...], '[...]' or '{{a,b}}'), got '{s}'"
                )))
            }
        }
        other => Err(Error::Validation(format!(
            "ANY/ALL expects an array value, got {other:?}"
        ))),
    }
}

/// `left <op> ANY(array)` / `left <op> ALL(array)`.
///
/// NULL elements never compare true. `ANY` over an empty array is false,
/// `ALL` over an empty array is true; a NULL left operand yields NULL.
pub(super) fn eval_quantified(
    left: &TypedExpr,
    op: BinaryOperator,
    right: &TypedExpr,
    all: bool,
    row: &Row,
) -> Result<Literal, Error> {
    let left_val = eval_expr(left, row)?;
    if matches!(left_val, Literal::Null) {
        return Ok(Literal::Null);
    }
    let elements = array_elements(eval_expr(right, row)?)?;
    let mut result = all;
    for element in elements {
        if matches!(element, Literal::Null) {
            if all {
                result = false;
            }
            continue;
        }
        let hit = compare_literals(&left_val, &element, op)?;
        if all && !hit {
            return Ok(Literal::Boolean(false));
        }
        if !all && hit {
            return Ok(Literal::Boolean(true));
        }
    }
    Ok(Literal::Boolean(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::analyzer::DataType;

    fn lit(l: Literal) -> TypedExpr {
        TypedExpr::literal(l)
    }

    fn text(s: &str) -> TypedExpr {
        lit(Literal::Text(s.into()))
    }

    #[test]
    fn regex_match_and_flags() {
        let row = Row::new();
        let t = text("Hello World");
        assert_eq!(
            eval_regex(&t, &text("^Hello"), false, false, false, &row).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            eval_regex(&t, &text("^hello"), false, false, false, &row).unwrap(),
            Literal::Boolean(false)
        );
        assert_eq!(
            eval_regex(&t, &text("^hello"), true, false, false, &row).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            eval_regex(&t, &text("World$"), false, true, false, &row).unwrap(),
            Literal::Boolean(false)
        );
        // SIMILAR TO is anchored: 'Hello' alone must not match 'Hello World'
        assert_eq!(
            eval_regex(&t, &text("Hello"), false, false, true, &row).unwrap(),
            Literal::Boolean(false)
        );
        assert_eq!(
            eval_regex(&t, &text("Hello%"), false, false, true, &row).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            eval_regex(&lit(Literal::Null), &text("x"), false, false, false, &row).unwrap(),
            Literal::Null
        );
    }

    #[test]
    fn quantified_over_json_array_and_text_forms() {
        let row = Row::new();
        let arr = TypedExpr::new(
            raisin_sql::analyzer::Expr::Literal(Literal::JsonB(serde_json::json!(["a", "b"]))),
            DataType::JsonB,
        );
        assert_eq!(
            eval_quantified(&text("a"), BinaryOperator::Eq, &arr, false, &row).unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            eval_quantified(&text("z"), BinaryOperator::Eq, &arr, false, &row).unwrap(),
            Literal::Boolean(false)
        );
        // JSON text, as a bound array parameter is rendered
        assert_eq!(
            eval_quantified(
                &text("b"),
                BinaryOperator::Eq,
                &text("[\"a\",\"b\"]"),
                false,
                &row
            )
            .unwrap(),
            Literal::Boolean(true)
        );
        // PostgreSQL array text
        assert_eq!(
            eval_quantified(
                &text("b"),
                BinaryOperator::NotEq,
                &text("{a,c}"),
                true,
                &row
            )
            .unwrap(),
            Literal::Boolean(true)
        );
        // numeric ALL
        let nums = TypedExpr::new(
            raisin_sql::analyzer::Expr::Literal(Literal::JsonB(serde_json::json!([1, 2, 3]))),
            DataType::JsonB,
        );
        assert_eq!(
            eval_quantified(
                &lit(Literal::BigInt(5)),
                BinaryOperator::Gt,
                &nums,
                true,
                &row
            )
            .unwrap(),
            Literal::Boolean(true)
        );
        assert_eq!(
            eval_quantified(
                &lit(Literal::BigInt(2)),
                BinaryOperator::Gt,
                &nums,
                true,
                &row
            )
            .unwrap(),
            Literal::Boolean(false)
        );
    }
}
