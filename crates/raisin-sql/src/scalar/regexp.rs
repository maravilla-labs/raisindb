//! Regular-expression kernels: REGEXP_MATCH, REGEXP_REPLACE, REGEXP_LIKE.
//!
//! Patterns use Rust `regex` syntax, which covers the PostgreSQL ARE syntax
//! people actually write (classes, groups, anchors, `\d`, `\w`, lazy
//! quantifiers). Backreferences and lookaround are not supported and fail at
//! compile time with the engine's message.
//!
//! Compiled patterns are cached: a per-row `REGEXP_LIKE(name, '^a')` would
//! otherwise recompile the same pattern once per row.

use super::args::{arity, text, KernelError};
use super::{Kernel, KernelCategory, KernelResult};
use crate::analyzer::Literal;
use regex::Regex;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

macro_rules! kernel {
    ($name:expr, $sig:expr, $f:expr) => {
        Kernel {
            name: $name,
            aliases: &[],
            category: KernelCategory::String,
            signature: $sig,
            deterministic: true,
            strict: true,
            func: $f,
        }
    };
}

pub(super) static KERNELS: &[Kernel] = &[
    kernel!(
        "REGEXP_MATCH",
        "REGEXP_MATCH(text, pattern[, flags]) -> JSONB",
        regexp_match
    ),
    kernel!(
        "REGEXP_REPLACE",
        "REGEXP_REPLACE(text, pattern, replacement[, flags]) -> TEXT",
        regexp_replace
    ),
    kernel!(
        "REGEXP_LIKE",
        "REGEXP_LIKE(text, pattern[, flags]) -> BOOLEAN",
        regexp_like
    ),
];

const CACHE_CAPACITY: usize = 256;

static CACHE: Mutex<Option<HashMap<String, Arc<Regex>>>> = Mutex::new(None);

/// Compile (or fetch) a pattern. Flags: `i` case-insensitive, `n`/`m`
/// newline-sensitive, `s` dot-all, `x` extended, `g` (global, consumed by
/// REGEXP_REPLACE, ignored here).
pub(super) fn compile(name: &str, pattern: &str, flags: &str) -> Result<Arc<Regex>, KernelError> {
    let key = format!("{}\u{0}{}", flags, pattern);
    if let Ok(mut guard) = CACHE.lock() {
        if let Some(hit) = guard.get_or_insert_with(HashMap::new).get(&key) {
            return Ok(hit.clone());
        }
    }
    let mut builder = regex::RegexBuilder::new(pattern);
    for flag in flags.chars() {
        match flag {
            'i' => {
                builder.case_insensitive(true);
            }
            'n' | 'm' => {
                builder.multi_line(true);
            }
            's' => {
                builder.dot_matches_new_line(true);
            }
            'x' => {
                builder.ignore_whitespace(true);
            }
            'g' => {}
            other => return Err(format!("{}: unknown regexp flag '{}'", name, other)),
        }
    }
    let compiled = Arc::new(
        builder
            .build()
            .map_err(|e| format!("{}: invalid regular expression: {}", name, e))?,
    );
    if let Ok(mut guard) = CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        if map.len() >= CACHE_CAPACITY {
            map.clear();
        }
        map.insert(key, compiled.clone());
    }
    Ok(compiled)
}

fn flags<'a>(name: &str, args: &'a [Literal], i: usize) -> Result<&'a str, KernelError> {
    if args.len() > i {
        text(name, args, i)
    } else {
        Ok("")
    }
}

/// First match as a JSON array of capture groups (the whole match when the
/// pattern has no groups); NULL when nothing matches — PostgreSQL semantics,
/// with `text[]` rendered as a JSON array because that is the one array type
/// a RaisinDB result row can carry.
fn regexp_match(args: &[Literal]) -> KernelResult {
    arity("REGEXP_MATCH", args, 2, 3)?;
    let s = text("REGEXP_MATCH", args, 0)?;
    let re = compile(
        "REGEXP_MATCH",
        text("REGEXP_MATCH", args, 1)?,
        flags("REGEXP_MATCH", args, 2)?,
    )?;
    let Some(caps) = re.captures(s) else {
        return Ok(Literal::Null);
    };
    let groups: Vec<serde_json::Value> = if caps.len() == 1 {
        vec![serde_json::Value::String(caps[0].to_string())]
    } else {
        caps.iter()
            .skip(1)
            .map(|m| match m {
                Some(m) => serde_json::Value::String(m.as_str().to_string()),
                None => serde_json::Value::Null,
            })
            .collect()
    };
    Ok(Literal::JsonB(serde_json::Value::Array(groups)))
}

/// Replaces the FIRST match unless the flags contain `g`, as in PostgreSQL.
/// `\1`..`\9` and `\&` in the replacement refer to groups / the whole match.
fn regexp_replace(args: &[Literal]) -> KernelResult {
    arity("REGEXP_REPLACE", args, 3, 4)?;
    let s = text("REGEXP_REPLACE", args, 0)?;
    let flag_str = flags("REGEXP_REPLACE", args, 3)?;
    let re = compile("REGEXP_REPLACE", text("REGEXP_REPLACE", args, 1)?, flag_str)?;
    let replacement = convert_replacement(text("REGEXP_REPLACE", args, 2)?);
    let out = if flag_str.contains('g') {
        re.replace_all(s, replacement.as_str())
    } else {
        re.replace(s, replacement.as_str())
    };
    Ok(Literal::Text(out.into_owned()))
}

/// PostgreSQL spells groups `\1` and the whole match `\&`; `regex` spells
/// them `${1}` and `${0}`, and treats `$` itself specially, so escape it.
fn convert_replacement(pg: &str) -> String {
    let mut out = String::with_capacity(pg.len());
    let mut chars = pg.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '$' => out.push_str("$$"),
            '\\' => match chars.next() {
                Some(d) if d.is_ascii_digit() => {
                    out.push_str("${");
                    out.push(d);
                    out.push('}');
                }
                Some('&') => out.push_str("${0}"),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            },
            other => out.push(other),
        }
    }
    out
}

fn regexp_like(args: &[Literal]) -> KernelResult {
    arity("REGEXP_LIKE", args, 2, 3)?;
    let re = compile(
        "REGEXP_LIKE",
        text("REGEXP_LIKE", args, 1)?,
        flags("REGEXP_LIKE", args, 2)?,
    )?;
    Ok(Literal::Boolean(re.is_match(text("REGEXP_LIKE", args, 0)?)))
}
