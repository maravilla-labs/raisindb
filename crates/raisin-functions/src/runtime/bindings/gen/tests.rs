// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Freshness tests for the generated SDK bindings.
//!
//! The generated files are committed, so the only thing that can go wrong is
//! that someone adds a registry method and forgets to regenerate. These tests
//! turn that into a `cargo test -p raisin-functions --lib` failure.

use super::*;
use crate::runtime::bindings::gen::model::exported_methods;

fn stale_of(kind: OutputKind) -> Vec<&'static str> {
    let root = repo_root();
    let stale = stale_files(&root);
    render_all()
        .into_iter()
        .filter(|f| f.kind == kind && stale.contains(&f.path))
        .map(|f| f.path)
        .collect()
}

/// The committed Rust/Go/TypeScript bindings must equal what the registry
/// renders right now.
#[test]
fn generated_sdk_bindings_are_fresh() {
    let stale = stale_of(OutputKind::Binding);
    assert!(
        stale.is_empty(),
        "generated SDK bindings are out of date: {:?}\n{}",
        stale,
        REGENERATE_HINT
    );
}

/// Every SDK-local WIT copy must be byte-identical to the source of truth.
#[test]
fn sdk_wit_copies_match_source() {
    let stale = stale_of(OutputKind::Wit);
    assert!(
        stale.is_empty(),
        "SDK copies of {} are out of date: {:?}\n{}",
        WIT_SOURCE_PATH,
        stale,
        REGENERATE_HINT
    );
    assert!(
        WIT_SOURCE.contains("export handler: func(name: string, input: string)"),
        "the WIT export is name-routed; one artifact carries N handlers"
    );
}

/// The TS SDK ships `api_wrapper.js` verbatim — that is what makes a QuickJS
/// function componentize with zero source changes.
#[test]
fn ts_sdk_api_wrapper_is_fresh() {
    let stale = stale_of(OutputKind::ApiWrapper);
    assert!(
        stale.is_empty(),
        "the TS SDK copy of quickjs/api_wrapper.js is out of date: {:?}\n{}",
        stale,
        REGENERATE_HINT
    );
}

/// Non-fatal parity report between the generated declarations and the
/// hand-written `raisin.d.ts` (B1-dts). The hand-written file is ~40 % prose
/// and security notes the registry cannot produce, so replacing it is a
/// follow-up; until then a divergence is a WARNING, and this assertion is
/// flipped to a hard one once the two are reconciled.
#[test]
fn generated_dts_parity_with_handwritten_is_warned_not_asserted() {
    let handwritten = repo_root().join("packages/raisindb-functions-types/raisin.d.ts");
    let Ok(source) = std::fs::read_to_string(&handwritten) else {
        eprintln!(
            "WARNING [B1-dts]: {} not found; parity not checked",
            handwritten.display()
        );
        return;
    };

    let mut missing: Vec<String> = Vec::new();
    for m in exported_methods() {
        if !source.contains(&format!("function {}(", m.js_name)) {
            missing.push(format!("{}.{}", m.category, m.js_name));
        }
    }
    missing.sort();
    missing.dedup();

    if !missing.is_empty() {
        eprintln!(
            "WARNING [B1-dts]: {} registry methods are absent from the hand-written \
             raisin.d.ts: {}\nThe generated companion is raisin.generated.d.ts; reconcile \
             the two, then make this a hard assertion.",
            missing.len(),
            missing.join(", ")
        );
    }
}

/// A smoke test on the rendered shape, so a refactor that silently empties a
/// backend fails here rather than in a downstream SDK build.
#[test]
fn every_backend_renders_the_same_method_set() {
    let methods = exported_methods();
    assert!(methods.len() > 50, "expected the full registry surface");

    let rust = rust::render();
    let go = go::render();
    let dts = dts::render_namespace();
    for m in methods {
        assert!(
            rust.contains(&format!("crate::host::call(\"{}\"", m.internal_name)),
            "Rust SDK is missing {}",
            m.internal_name
        );
        assert!(
            go.contains(&format!("\"{}\"", m.internal_name)),
            "Go SDK is missing {}",
            m.internal_name
        );
        assert!(
            dts.contains(&format!("function {}(", m.js_name)),
            "TS declarations are missing {}",
            m.internal_name
        );
    }
    // Excluded methods must not appear anywhere.
    assert!(!rust.contains("crate::host::call(\"context_get\""));
    assert!(!go.contains("\"context_get\""));
}
