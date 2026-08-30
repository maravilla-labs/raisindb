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

/// Include/exclude globs, COMPILED ONCE for a run.
///
/// Why this type exists at all: `glob::Pattern::new` was called on every
/// comparison, so filtering one item cost `p` pattern compilations for `p`
/// patterns — and once exclusion became INHERITED (see [`PathFilter::excluded`])
/// the ancestor chain multiplied that by path depth: `(d+1)*p` compilations per
/// item, in the hottest loop this engine has. A drive mount of 20k files at
/// depth 5 with two patterns paid 240k parser runs per full walk for an answer
/// that never changes within a run. Compiled here the cost is `p` compilations
/// TOTAL per run; matching stays `(d+1)*p` cheap matches per item.
///
/// Build it ONCE per run and hand the same value to the walk, the delta and the
/// reconciler — that shared instance is also what stops those three drifting
/// apart, which is the bug class this filter has already produced twice.
pub struct PathFilter {
    include: Vec<glob::Pattern>,
    /// Whether the mount DECLARED any include pattern, as opposed to none
    /// surviving compilation.
    ///
    /// Load-bearing: an empty include list means "include everything", but a
    /// list whose every pattern is MALFORMED must keep meaning "nothing
    /// matches" — that is what the old per-call `glob_matches` did, returning
    /// false for an unparseable pattern. Without this flag a typo'd include
    /// pattern would flip the filter from syncing nothing to syncing the whole
    /// provider.
    include_declared: bool,
    exclude: Vec<glob::Pattern>,
}

impl PathFilter {
    /// Compile the mount's patterns. A malformed pattern is dropped, which
    /// matches the previous per-call behaviour: it never matched anything.
    pub fn new(include: &[String], exclude: &[String]) -> Self {
        Self {
            include: compile_all(include),
            include_declared: !include.is_empty(),
            exclude: compile_all(exclude),
        }
    }

    /// Apply include/exclude globs to a relative path. Exclude wins over
    /// include. No include patterns means everything is included; a non-empty
    /// one requires a match.
    ///
    /// Exclusion is INHERITED (see [`Self::excluded`]); inclusion is not. An
    /// include pattern describes the items to keep — `*.pdf` — and a folder is
    /// not one of them, so testing a folder against the include list would
    /// refuse to descend into every folder on a mount that has one.
    pub fn passes(&self, rel_path: &str) -> bool {
        if self.excluded(rel_path) {
            return false;
        }
        if !self.include_declared {
            return true;
        }
        self.include.iter().any(|p| p.matches(rel_path))
    }

    /// Does this path, or ANY folder above it, match an exclude pattern?
    ///
    /// Exclusion has to be inherited, and it has to be inherited on BOTH sync
    /// paths. `exclude: ["Archive"]` reads to an operator as "do not sync the
    /// Archive folder", and on a full walk that is what happened by accident:
    /// the folder itself failed the leaf test, so it was never staged. But the
    /// walk still descended into it and the delta computed the whole path and
    /// tested only the LEAF, so `Archive/x.pdf` — which the glob `Archive` does
    /// not match — was admitted by both. The excluded folder's entire contents
    /// synced anyway.
    ///
    /// Testing every ancestor segment is what makes the two paths give one
    /// answer for one item. A delta/walk disagreement is the class of bug that
    /// relocates a node on every alternate run; this one imported data an
    /// operator had asked to be left alone.
    pub fn excluded(&self, rel_path: &str) -> bool {
        if self.exclude.is_empty() {
            return false;
        }
        // The WHOLE path, verbatim, FIRST. This is the test that stood here
        // before exclusion became inherited, and keeping it makes the new rule a
        // strict superset: nothing a live mount's pattern excluded yesterday can
        // start syncing today. It is load-bearing because the ancestor walk
        // below NORMALISES the path — it skips empty segments — so a `rel_path`
        // a mapper emitted as `/Archive/x.pdf` is rebuilt as `Archive/x.pdf`,
        // and a pattern written `/Archive/x.pdf` would then match nothing at
        // all: the filter would fail OPEN and sync exactly what it was
        // configured to keep out.
        if self.exclude.iter().any(|pat| pat.matches(rel_path)) {
            return true;
        }
        // Walk the ancestor chain: `a/b/c` is tested as `a`, then `a/b`, then
        // `a/b/c`. Empty segments are skipped so a leading or doubled slash
        // cannot produce a `""` prefix that a pattern like `*` would match.
        let mut prefix = String::with_capacity(rel_path.len());
        for segment in rel_path.split('/').filter(|s| !s.trim().is_empty()) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            if self.exclude.iter().any(|pat| pat.matches(&prefix)) {
                return true;
            }
        }
        false
    }
}

/// Compile what compiles; drop what does not.
///
/// A malformed pattern never matched under the old per-call `glob_matches`
/// either (it returned `false` on a parse error), so dropping it here changes
/// nothing except how often we discover it. It is logged once per run rather
/// than never, because a silently ignored `exclude` pattern is a mount syncing
/// data an operator believes is filtered out.
fn compile_all(patterns: &[String]) -> Vec<glob::Pattern> {
    patterns
        .iter()
        .filter_map(|p| match glob::Pattern::new(p) {
            Ok(compiled) => Some(compiled),
            Err(e) => {
                tracing::warn!(
                    pattern = %p,
                    error = %e,
                    "ignoring a malformed sync filter pattern; it matches nothing"
                );
                None
            }
        })
        .collect()
}

/// One-shot form of [`PathFilter::passes`], for callers outside the per-item
/// hot path. Compiles the patterns on every call, so it belongs in tests and
/// one-off checks only; inside a run use a [`PathFilter`] built once — that is
/// the whole reason the type exists.
pub fn passes_filters(rel_path: &str, include: &[String], exclude: &[String]) -> bool {
    PathFilter::new(include, exclude).passes(rel_path)
}

/// One-shot form of [`PathFilter::excluded`]. Same caveat as
/// [`passes_filters`]: it compiles, so never call it per item.
pub fn excluded(rel_path: &str, exclude: &[String]) -> bool {
    PathFilter::new(&[], exclude).excluded(rel_path)
}

/// A node's path relative to its mount, or `None` when it is not under the
/// mount at all.
///
/// The reconciler needs it to ask the SAME exclusion question the walk asked:
/// the walk filters mount-relative paths (`Archive/x.pdf`), while the index
/// holds absolute workspace paths (`/drive/Archive/x.pdf`).
pub fn mount_relative<'a>(mount_path: &str, node_path: &'a str) -> Option<&'a str> {
    if mount_path == "/" {
        return Some(node_path.trim_start_matches('/'));
    }
    if node_path == mount_path {
        return Some("");
    }
    node_path
        .strip_prefix(mount_path)
        .and_then(|rest| rest.strip_prefix('/'))
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
