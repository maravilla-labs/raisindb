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

//! The registered-function inventory, and the coverage ledger checked against it.
//!
//! The function list is **not** hardcoded here. It is scanned out of the
//! analyzer's registration source at compile time via [`include_str!`], so a
//! function added to (or removed from) `register_geospatial` changes this list
//! automatically. A hardcoded list is exactly how a conformance suite silently
//! stops being one: the brief that produced this file quoted "49 ST_* functions"
//! and the real number was already different.

use std::collections::{BTreeMap, BTreeSet};

/// The analyzer's geospatial registration source, embedded at compile time.
///
/// `include_str!` resolves relative to *this* file:
/// `crates/raisin-server/tests/all/st_conformance/` + `../../../../` =
/// `crates/`.
const BUILTINS_SRC: &str =
    include_str!("../../../../raisin-sql/src/analyzer/functions/builtins_system.rs");

/// Every `ST_*` name the analyzer registers, scanned from the source of
/// `register_geospatial`.
///
/// Deliberately a dumb scanner rather than a parser: it looks for
/// `name: "ST_..."` between the start of `fn register_geospatial` and the end of
/// the file. Over-matching would only ever *add* a function to the required set,
/// which fails loudly; it cannot silently drop one.
pub fn registered_functions() -> BTreeSet<String> {
    let body = match BUILTINS_SRC.find("fn register_geospatial") {
        Some(idx) => &BUILTINS_SRC[idx..],
        None => panic!(
            "register_geospatial not found in builtins_system.rs — the geospatial \
             registration moved. Update st_conformance::coverage::BUILTINS_SRC."
        ),
    };

    const NEEDLE: &str = "name: \"ST_";
    let mut found = BTreeSet::new();
    // `match_indices` yields (offset, matched_text) — the matched text is the
    // needle itself, NOT the remainder, so the offset is what we need here.
    for (offset, _) in body.match_indices(NEEDLE) {
        // Skip past `name: "` to land on `ST_`, then take up to the closing quote.
        let rest = &body[offset + "name: \"".len()..];
        if let Some(end) = rest.find('"') {
            found.insert(rest[..end].to_string());
        }
    }

    assert!(
        found.len() > 40,
        "scanned only {} ST_* functions out of builtins_system.rs — the scanner \
         is broken, not the registry",
        found.len()
    );
    found
}

/// Records which functions the suite actually exercised, and how.
#[derive(Default)]
pub struct Coverage {
    /// function name -> the assertions made against it
    hits: BTreeMap<String, Vec<String>>,
}

impl Coverage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `func` was exercised by an assertion described by `what`.
    pub fn record(&mut self, func: &str, what: &str) {
        self.hits
            .entry(func.to_ascii_uppercase())
            .or_default()
            .push(what.to_string());
    }

    /// Record every `ST_*` name appearing in a SQL string.
    ///
    /// This is what makes the ledger honest with modest effort: a test writes the
    /// SQL it means to run and every function in it is credited, so a function
    /// used only incidentally (`ST_ASGEOJSON` wrapping a result) still counts as
    /// exercised, and no separate bookkeeping can drift from the SQL.
    pub fn record_sql(&mut self, sql: &str, what: &str) {
        for func in scan_st_names(sql) {
            self.record(&func, what);
        }
    }

    pub fn exercised(&self) -> BTreeSet<String> {
        self.hits.keys().cloned().collect()
    }

    pub fn assertion_count(&self) -> usize {
        self.hits.values().map(|v| v.len()).sum()
    }

    /// Print the ledger and return the set of registered-but-unexercised names.
    ///
    /// A gap is printed, not swallowed — the caller decides whether it fails the
    /// test. Silence here would defeat the whole point of the exercise.
    pub fn report(&self) -> BTreeSet<String> {
        let registered = registered_functions();
        let exercised = self.exercised();

        println!("\n================ ST_* CONFORMANCE COVERAGE ================");
        println!(
            "registered: {}   exercised: {}   assertions: {}",
            registered.len(),
            registered.intersection(&exercised).count(),
            self.assertion_count()
        );
        println!("-----------------------------------------------------------");

        for func in &registered {
            match self.hits.get(func) {
                Some(cases) => println!("  [COVERED] {:<24} {} case(s)", func, cases.len()),
                None => println!("  [ *GAP* ] {:<24} NOT EXERCISED", func),
            }
        }

        // Anything exercised that is not registered means the suite is testing a
        // name the analyzer does not know — a typo in a test, or a function whose
        // registration was removed while its tests stayed.
        let unknown: BTreeSet<String> = exercised.difference(&registered).cloned().collect();
        if !unknown.is_empty() {
            println!("-----------------------------------------------------------");
            for func in &unknown {
                println!("  [UNKNOWN] {:<24} exercised but NOT registered", func);
            }
        }

        let gaps: BTreeSet<String> = registered.difference(&exercised).cloned().collect();
        println!("===========================================================");
        if gaps.is_empty() {
            println!("RESULT: every registered ST_* function was exercised.\n");
        } else {
            println!(
                "RESULT: {} registered function(s) NOT exercised: {:?}\n",
                gaps.len(),
                gaps
            );
        }
        gaps
    }
}

/// Extract every `ST_<IDENT>` token that is used as a call in `sql`.
///
/// Requires a following `(` so a bare mention in a comment or an alias named
/// `st_thing` is not credited.
fn scan_st_names(sql: &str) -> BTreeSet<String> {
    let upper = sql.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let mut out = BTreeSet::new();
    let mut i = 0usize;

    while let Some(rel) = upper[i..].find("ST_") {
        let start = i + rel;
        // Must not be preceded by an identifier character, or `DIST_ST_X` matches.
        let preceded_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        let mut end = start + 3;
        while end < bytes.len() && is_ident_byte(bytes[end]) {
            end += 1;
        }
        // Must be a call: the next non-space byte is `(`.
        let mut probe = end;
        while probe < bytes.len() && bytes[probe] == b' ' {
            probe += 1;
        }
        if preceded_ok && end > start + 3 && probe < bytes.len() && bytes[probe] == b'(' {
            out.insert(upper[start..end].to_string());
        }
        i = start + 3;
    }
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_finds_calls_and_ignores_non_calls() {
        let found = scan_st_names("SELECT ST_X(g), ST_Y (g), st_area(p) AS st_total FROM t");
        assert!(found.contains("ST_X"));
        assert!(found.contains("ST_Y"));
        assert!(found.contains("ST_AREA"));
        // `st_total` is an alias, not a call.
        assert!(!found.contains("ST_TOTAL"));
    }

    #[test]
    fn registry_scan_finds_the_known_core() {
        let r = registered_functions();
        for expected in [
            "ST_POINT",
            "ST_AREA",
            "ST_BUFFER",
            "ST_RELATE",
            "ST_TRANSFORM",
            "ST_3DDWITHIN",
        ] {
            assert!(r.contains(expected), "{expected} missing from scan");
        }
    }
}
