// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! TypeScript declaration backend.
//!
//! ONE renderer produces the namespace body used by BOTH `.d.ts` outputs
//! (`sdks/ts/function-wasm/src/generated/raisin.d.ts` and
//! `packages/raisindb-functions-types/raisin.generated.d.ts`); only the header
//! differs. Two renderers would drift, which is the bug class CLAUDE.md names.

use super::model::{self, by_category, is_optional};
use crate::runtime::bindings::registry::{ApiMethodDescriptor, ArgType, ReturnType};

/// TypeScript type for an argument, per the ABI type-mapping table.
fn arg_type(t: ArgType) -> &'static str {
    match t {
        ArgType::String => "string",
        ArgType::OptionalString => "string | null",
        ArgType::Json | ArgType::OptionalJson => "any",
        ArgType::U32 | ArgType::OptionalU32 | ArgType::I64 | ArgType::OptionalI64 => "number",
        ArgType::Bool => "boolean",
        ArgType::OptionalBool => "boolean | null",
        ArgType::StringArray => "string[]",
        ArgType::JsonArray => "any[]",
    }
}

/// TypeScript result type. Every binding is async on the JS surface.
fn ret(r: ReturnType) -> &'static str {
    match r {
        ReturnType::Json => "Promise<any>",
        ReturnType::OptionalJson => "Promise<any | null>",
        ReturnType::JsonArray => "Promise<any[]>",
        ReturnType::Bool => "Promise<boolean>",
        ReturnType::I64 => "Promise<number>",
        ReturnType::String => "Promise<string>",
        ReturnType::Void => "Promise<void>",
    }
}

fn signature(m: &ApiMethodDescriptor) -> String {
    let params: Vec<String> = m
        .args
        .iter()
        .enumerate()
        .map(|(i, a)| {
            // `?` is only legal while every following argument is optional too.
            let trailing = m.args[i..].iter().all(|x| is_optional(x.arg_type));
            let opt = if is_optional(a.arg_type) && trailing {
                "?"
            } else {
                ""
            };
            format!("{}{}: {}", a.name, opt, arg_type(a.arg_type))
        })
        .collect();
    format!(
        "function {}({}): {};",
        m.js_name,
        params.join(", "),
        ret(m.return_type)
    )
}

/// Render `declare namespace raisin { ... }` from the registry.
pub fn render_namespace() -> String {
    let mut out = String::from("declare namespace raisin {\n");
    let grouped = by_category();
    let mut i = 0usize;
    while i < grouped.len() {
        let path = model::namespace_path(grouped[i].0);
        if path.len() == 1 {
            out.push_str(&format!("  namespace {} {{\n", path[0]));
            for m in &grouped[i].1 {
                out.push_str(&format!("    {}\n", signature(m)));
            }
            out.push_str("  }\n\n");
            i += 1;
            continue;
        }
        let outer = path[0].clone();
        out.push_str(&format!("  namespace {} {{\n", outer));
        while i < grouped.len() {
            let p = model::namespace_path(grouped[i].0);
            if p.len() != 2 || p[0] != outer {
                break;
            }
            out.push_str(&format!("    namespace {} {{\n", p[1]));
            for m in &grouped[i].1 {
                out.push_str(&format!("      {}\n", signature(m)));
            }
            out.push_str("    }\n");
            i += 1;
        }
        out.push_str("  }\n\n");
    }
    out.push_str("}\n");
    out
}

/// `packages/raisindb-functions-types/raisin.generated.d.ts`.
///
/// v1 ships BESIDE the hand-written `raisin.d.ts` (40 % of which is prose the
/// registry cannot produce); the parity check between them is a warning, not
/// an assertion, until they are reconciled.
pub fn render_package_dts() -> String {
    let mut out = model::header("//");
    out.push_str(
        "//\n\
         // Generated companion to the hand-written `raisin.d.ts`. It describes the\n\
         // `raisin.*` surface exactly as the bindings registry defines it, with no\n\
         // prose and no hand-authored helper types.\n\n",
    );
    out.push_str(&render_namespace());
    out
}

/// `sdks/ts/function-wasm/src/generated/raisin.d.ts`.
pub fn render_sdk_dts() -> String {
    let mut out = model::header("//");
    out.push_str(
        "//\n\
         // The `raisin.*` surface available inside a WebAssembly TypeScript\n\
         // function. It is the SAME registry surface QuickJS functions see, which\n\
         // is what makes a QuickJS function componentize without source changes.\n\
         // Not available here: `setTimeout`, native `fetch`, and the\n\
         // `__raisin_internal.temp_*` Resource helpers.\n\n",
    );
    out.push_str(&render_namespace());
    out
}
