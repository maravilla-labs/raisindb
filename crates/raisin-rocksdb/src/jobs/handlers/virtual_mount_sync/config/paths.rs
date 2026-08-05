//! Path templating, segment sanitization, glob filtering, and timestamp parsing.

use super::items::ExternalItem;
use serde_json::Value;

/// Parse an ISO-8601 / RFC-3339 timestamp into Unix epoch seconds.
pub fn parse_iso_epoch(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// Characters that may not appear inside a single path SEGMENT.
///
/// A resolved value is untrusted provider text (a subject line, a folder name),
/// so a `/` in it would silently invent hierarchy and a `\0` would corrupt the
/// storage key encoding. Both are replaced rather than rejected: dropping the
/// item would be a worse outcome than a slightly-renamed folder.
fn sanitize_segment(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string()
}

/// Resolve a [`SyncConfig::path_template`] against one item.
///
/// Returns `None` when the template is empty or any placeholder cannot be
/// resolved, so the caller keeps the flat provider path. Failing closed matters:
/// a half-resolved template would scatter items across folders named after the
/// literal `{conversation_id}`, and because the materializer matches an existing
/// node by `__external_id` and keeps its current path, undoing that afterwards
/// is not a simple re-sync.
pub fn resolve_path_template(template: &str, item: &ExternalItem) -> Option<String> {
    if template.trim().is_empty() {
        return None;
    }
    let meta = item.metadata.as_ref();
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let end = rest[start..].find('}')? + start;
        let token = &rest[start + 1..end];
        rest = &rest[end + 1..];

        // `{key:FMT}` formats an ISO-8601 value with a strftime pattern.
        let (key, fmt) = match token.split_once(':') {
            Some((k, f)) => (k, Some(f)),
            None => (token, None),
        };

        let raw = match key {
            "name" => Some(item.name.clone()),
            "external_id" => Some(item.external_id.clone()),
            _ => meta.and_then(|m| m.get(key)).and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            }),
        }?;

        let value = match fmt {
            Some(f) => chrono::DateTime::parse_from_rfc3339(&raw)
                .ok()
                .map(|dt| dt.format(f).to_string())?,
            None => raw,
        };
        let cleaned = sanitize_segment(&value);
        if cleaned.is_empty() {
            return None;
        }
        out.push_str(&cleaned);
    }
    out.push_str(rest);

    // Collapse accidental empty segments (`a//b`, a leading slash) so the result
    // is always a clean relative path.
    let joined = out
        .split('/')
        .filter(|seg| !seg.trim().is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

/// Apply include/exclude globs to a relative path. Exclude wins over include.
/// An empty `include` list includes everything; a non-empty one requires a
/// match.
pub fn passes_filters(rel_path: &str, include: &[String], exclude: &[String]) -> bool {
    for pat in exclude {
        if glob_matches(pat, rel_path) {
            return false;
        }
    }
    if include.is_empty() {
        return true;
    }
    include.iter().any(|p| glob_matches(p, rel_path))
}

fn glob_matches(pattern: &str, path: &str) -> bool {
    match glob::Pattern::new(pattern) {
        Ok(p) => p.matches(path),
        // A malformed pattern never matches (fail closed for include, open for
        // exclude is handled by the caller's ordering).
        Err(_) => false,
    }
}

pub(super) fn normalize_path(p: &str) -> String {
    let trimmed = p.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}
