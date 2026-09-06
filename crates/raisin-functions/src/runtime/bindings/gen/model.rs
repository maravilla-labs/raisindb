// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Shared model for the SDK binding generator.
//!
//! The generator has exactly ONE view of the registry: [`exported_methods`].
//! Every language backend renders that same list, so a method cannot reach the
//! Rust SDK and quietly miss the Go one.

use crate::runtime::bindings::registry::ApiMethodDescriptor;
use crate::runtime::bindings::{methods::registry, ArgType};

/// Registry categories that never reach a guest SDK, with the reason.
///
/// Mirrors `NOT_EXPOSED_IN_JS` in `methods/mod.rs`: the exclusion is a
/// deliberate decision, recorded here so adding a registry method forces one.
pub const EXCLUDED_CATEGORIES: &[(&str, &str)] = &[(
    "internal",
    "host-side plumbing (log, allowsAdminEscalation); guests use the WIT `log` import",
)];

/// Individual registry methods that never reach a guest SDK, with the reason.
pub const EXCLUDED_METHODS: &[(&str, &str)] = &[
    (
        "context_get",
        "guests read the context through the WIT `context` import, not a call",
    ),
    (
        "log",
        "guests log through the WIT `log` import (also excluded by category)",
    ),
];

/// Every registry method a guest SDK exposes, sorted by `(category, internal_name)`
/// so generated output is byte-stable across runs.
pub fn exported_methods() -> Vec<&'static ApiMethodDescriptor> {
    let mut methods: Vec<&'static ApiMethodDescriptor> = registry()
        .methods()
        .iter()
        .filter(|m| !is_excluded(m))
        .collect();
    methods.sort_by(|a, b| {
        a.category
            .cmp(b.category)
            .then_with(|| a.internal_name.cmp(b.internal_name))
    });
    methods
}

/// Exported methods grouped by category, categories in sorted order.
pub fn by_category() -> Vec<(&'static str, Vec<&'static ApiMethodDescriptor>)> {
    let mut out: Vec<(&'static str, Vec<&'static ApiMethodDescriptor>)> = Vec::new();
    for m in exported_methods() {
        match out.last_mut() {
            Some((cat, list)) if *cat == m.category => list.push(m),
            _ => out.push((m.category, vec![m])),
        }
    }
    out
}

/// Is this method kept off every guest SDK surface?
pub fn is_excluded(m: &ApiMethodDescriptor) -> bool {
    EXCLUDED_CATEGORIES.iter().any(|(c, _)| *c == m.category)
        || EXCLUDED_METHODS.iter().any(|(n, _)| *n == m.internal_name)
}

/// Is the argument optional (may be omitted / `null` on the wire)?
pub fn is_optional(t: ArgType) -> bool {
    matches!(
        t,
        ArgType::OptionalString
            | ArgType::OptionalJson
            | ArgType::OptionalU32
            | ArgType::OptionalI64
            | ArgType::OptionalBool
    )
}

/// `admin_nodes` -> `["admin", "nodes"]`, `nodes` -> `["nodes"]`.
///
/// Matches the shape the hand-written `raisin.d.ts` already publishes
/// (`raisin.admin.nodes.*`).
pub fn namespace_path(category: &str) -> Vec<String> {
    match category.strip_prefix("admin_") {
        Some(rest) => vec!["admin".to_string(), rest.to_string()],
        None => vec![category.to_string()],
    }
}

/// `admin_nodes` -> `AdminNodes`, `tx` -> `Tx`.
pub fn pascal_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper = true;
    for ch in s.chars() {
        if ch == '_' || ch == '-' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// `parentPath` -> `parent_path`. Argument names in the registry are camelCase
/// (they were written for the JS surface); Rust wants snake_case.
pub fn snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Rust keywords that would otherwise produce uncompilable identifiers
/// (`tx.move`, `nodes.delete`, ...). Escaped with the raw-identifier prefix.
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "become", "box", "break", "const", "continue", "crate", "do", "dyn",
    "else", "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in", "let", "loop",
    "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref", "return", "self",
    "static", "struct", "super", "trait", "true", "try", "type", "typeof", "unsafe", "unsized",
    "use", "virtual", "where", "while", "yield",
];

/// Escape a Rust identifier that collides with a keyword (`move` -> `r#move`).
pub fn rust_ident(name: &str) -> String {
    if RUST_KEYWORDS.contains(&name) {
        format!("r#{}", name)
    } else {
        name.to_string()
    }
}

/// Header stamped on every generated file, in that language's line-comment syntax.
pub fn header(comment: &str) -> String {
    format!(
        "{c} Code generated by `cargo run -p raisin-functions --bin gen-bindings`.\n\
         {c} DO NOT EDIT — edit `crates/raisin-functions/src/runtime/bindings/` and\n\
         {c} re-run `make gen-bindings` instead.\n",
        c = comment
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exclusion lists must not carry stale entries — same guard as
    /// `NOT_EXPOSED_IN_JS` in `methods/mod.rs`.
    #[test]
    fn gen_exclusions_have_no_stale_entries() {
        let reg = registry();
        for (category, _reason) in EXCLUDED_CATEGORIES {
            assert!(
                reg.categories().contains(category),
                "EXCLUDED_CATEGORIES lists '{}' which is not a registry category",
                category
            );
        }
        for (name, _reason) in EXCLUDED_METHODS {
            assert!(
                reg.find_by_internal_name(name).is_some(),
                "EXCLUDED_METHODS lists '{}' which is not in the registry",
                name
            );
        }
        assert!(
            !exported_methods().is_empty(),
            "the generator must export something"
        );
        for m in exported_methods() {
            assert!(
                !is_excluded(m),
                "'{}' leaked past the filter",
                m.internal_name
            );
        }
    }

    #[test]
    fn naming_helpers_match_the_published_surface() {
        assert_eq!(namespace_path("admin_nodes"), vec!["admin", "nodes"]);
        assert_eq!(namespace_path("tx"), vec!["tx"]);
        assert_eq!(pascal_case("admin_nodes"), "AdminNodes");
        assert_eq!(rust_ident("move"), "r#move");
        assert_eq!(snake_case("parentPath"), "parent_path");
        assert_eq!(snake_case("id"), "id");
        assert_eq!(rust_ident("get"), "get");
    }
}
