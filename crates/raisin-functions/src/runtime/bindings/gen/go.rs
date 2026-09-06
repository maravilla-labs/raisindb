// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Go guest-SDK backend: renders `sdks/go/raisin/generated.go`.
//!
//! The generated code depends on the hand-written `host.go` helpers
//! `callJSON`, `callBool`, `callInt64`, `callString` and `callVoid`, each of
//! which marshals a positional argument slice, invokes the WIT gateway and
//! rejects an `{"error": true, ...}` envelope.

use super::model::{self, by_category};
use crate::runtime::bindings::registry::{ApiMethodDescriptor, ArgType, ReturnType};

/// Go type for an argument, per the ABI type-mapping table.
fn arg_type(t: ArgType) -> &'static str {
    match t {
        ArgType::String => "string",
        ArgType::OptionalString => "*string",
        ArgType::Json | ArgType::OptionalJson => "any",
        ArgType::U32 => "uint32",
        ArgType::OptionalU32 => "*uint32",
        ArgType::I64 => "int64",
        ArgType::OptionalI64 => "*int64",
        ArgType::Bool => "bool",
        ArgType::OptionalBool => "*bool",
        ArgType::StringArray => "[]string",
        ArgType::JsonArray => "[]any",
    }
}

/// `(result signature, helper)` for a return type.
fn ret(r: ReturnType) -> (&'static str, &'static str) {
    match r {
        ReturnType::Json | ReturnType::OptionalJson | ReturnType::JsonArray => {
            ("(json.RawMessage, error)", "callJSON")
        }
        ReturnType::Bool => ("(bool, error)", "callBool"),
        ReturnType::I64 => ("(int64, error)", "callInt64"),
        ReturnType::String => ("(string, error)", "callString"),
        ReturnType::Void => ("error", "callVoid"),
    }
}

const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
    "any",
    "len",
    "cap",
    "new",
    "make",
    "copy",
];

/// Escape a Go identifier that collides with a keyword or predeclared name.
fn go_ident(name: &str) -> String {
    if GO_KEYWORDS.contains(&name) {
        format!("{}_", name)
    } else {
        name.to_string()
    }
}

fn render_method(m: &ApiMethodDescriptor, receiver: &str, out: &mut String) {
    let (result, helper) = ret(m.return_type);
    let params: Vec<String> = m
        .args
        .iter()
        .map(|a| format!("{} {}", go_ident(a.name), arg_type(a.arg_type)))
        .collect();
    let call_args: Vec<String> = m.args.iter().map(|a| go_ident(a.name)).collect();
    let arg_slice = if call_args.is_empty() {
        "nil".to_string()
    } else {
        format!("[]any{{{}}}", call_args.join(", "))
    };
    out.push_str(&format!(
        "// {} calls the RaisinDB registry method {:?}.\n",
        model::pascal_case(m.js_name),
        m.internal_name
    ));
    out.push_str(&format!(
        "func ({}) {}({}) {} {{\n\treturn {}({:?}, {})\n}}\n\n",
        receiver,
        model::pascal_case(m.js_name),
        params.join(", "),
        result,
        helper,
        m.internal_name,
        arg_slice
    ));
}

/// Render the whole generated Go SDK surface.
pub fn render() -> String {
    let mut out = model::header("//");
    out.push_str("\npackage raisin\n\nimport \"encoding/json\"\n\n");

    for (category, methods) in by_category() {
        let name = model::pascal_case(category);
        out.push_str(&format!(
            "// {name}API is the `raisin.{}` namespace.\ntype {name}API struct{{}}\n\n\
             // {name} is the entry point for the `raisin.{}` namespace.\nvar {name} {name}API\n\n",
            model::namespace_path(category).join("."),
            model::namespace_path(category).join("."),
            name = name
        ));
        let receiver = format!("{}API", name);
        for m in methods {
            render_method(m, &receiver, &mut out);
        }
    }
    out
}
