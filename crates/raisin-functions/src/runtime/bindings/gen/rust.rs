// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Rust guest-SDK backend: renders `sdks/rust/raisin-sdk/src/generated.rs`.
//!
//! The generated code depends on exactly two hand-written SDK modules:
//! `crate::host::call(method, args_json) -> Result<String>` (the WIT gateway)
//! and `crate::wire::decode_*` (envelope-aware decoders).

use super::model::{self, by_category};
use crate::runtime::bindings::registry::{ApiMethodDescriptor, ArgType, ReturnType};

/// Rust type for an argument, per the ABI type-mapping table.
fn arg_type(t: ArgType) -> &'static str {
    match t {
        ArgType::String => "&str",
        ArgType::OptionalString => "Option<&str>",
        ArgType::Json => "impl ::serde::Serialize",
        ArgType::OptionalJson => "Option<::serde_json::Value>",
        ArgType::U32 => "u32",
        ArgType::OptionalU32 => "Option<u32>",
        ArgType::I64 => "i64",
        ArgType::OptionalI64 => "Option<i64>",
        ArgType::Bool => "bool",
        ArgType::OptionalBool => "Option<bool>",
        ArgType::StringArray => "&[String]",
        ArgType::JsonArray => "&[::serde_json::Value]",
    }
}

/// `(return type, decoder fn)` for the plain variant.
fn ret(r: ReturnType) -> (&'static str, &'static str) {
    match r {
        ReturnType::Json => ("::serde_json::Value", "decode_json"),
        ReturnType::OptionalJson => ("Option<::serde_json::Value>", "decode_optional_json"),
        ReturnType::JsonArray => ("Vec<::serde_json::Value>", "decode_json_array"),
        ReturnType::Bool => ("bool", "decode_bool"),
        ReturnType::I64 => ("i64", "decode_i64"),
        ReturnType::String => ("String", "decode_string"),
        ReturnType::Void => ("()", "decode_void"),
    }
}

/// `(return type, decoder fn)` for the typed `_as::<T>` variant, if there is one.
fn ret_as(r: ReturnType) -> Option<(&'static str, &'static str)> {
    match r {
        ReturnType::Json => Some(("T", "decode_json_as")),
        ReturnType::OptionalJson => Some(("Option<T>", "decode_optional_json_as")),
        ReturnType::JsonArray => Some(("Vec<T>", "decode_json_array_as")),
        _ => None,
    }
}

/// Argument identifier: snake_case, keyword-escaped.
fn rust_arg_ident(name: &str) -> String {
    model::rust_ident(&model::snake_case(name))
}

fn signature(m: &ApiMethodDescriptor, name: &str, ret_ty: &str, generic: bool) -> String {
    let params: Vec<String> = m
        .args
        .iter()
        .map(|a| format!("{}: {}", rust_arg_ident(a.name), arg_type(a.arg_type)))
        .collect();
    let bound = if generic {
        "<T: ::serde::de::DeserializeOwned>"
    } else {
        ""
    };
    format!(
        "    pub fn {}{}({}) -> crate::Result<{}> {{\n",
        model::rust_ident(name),
        bound,
        params.join(", "),
        ret_ty
    )
}

fn body(m: &ApiMethodDescriptor, decoder: &str, indent: &str) -> String {
    let mut out = String::new();
    if m.args.is_empty() {
        out.push_str(&format!("{i}let args = \"[]\".to_string();\n", i = indent));
    } else {
        out.push_str(&format!(
            "{i}let args = ::serde_json::Value::Array(vec![\n",
            i = indent
        ));
        for a in &m.args {
            out.push_str(&format!(
                "{i}    ::serde_json::to_value({})?,\n",
                rust_arg_ident(a.name),
                i = indent
            ));
        }
        out.push_str(&format!("{i}])\n{i}.to_string();\n", i = indent));
    }
    out.push_str(&format!(
        "{i}let raw = crate::host::call(\"{}\", &args)?;\n{i}crate::wire::{}(&raw)\n",
        m.internal_name,
        decoder,
        i = indent
    ));
    out
}

fn render_method(m: &ApiMethodDescriptor, out: &mut String, pad: &str) {
    let (ret_ty, decoder) = ret(m.return_type);
    out.push_str(&format!(
        "{pad}    /// `raisin.{}.{}` — registry method `{}`.\n",
        model::namespace_path(m.category).join("."),
        m.js_name,
        m.internal_name
    ));
    let sig = signature(m, m.py_name, ret_ty, false);
    out.push_str(&format!("{pad}{}", sig));
    out.push_str(&body(m, decoder, &format!("{pad}        ")));
    out.push_str(&format!("{pad}    }}\n\n"));

    if let Some((as_ty, as_decoder)) = ret_as(m.return_type) {
        out.push_str(&format!(
            "{pad}    /// Typed form of `{}`. `T` is inferred from the binding site\n\
             {pad}    /// (turbofish is unavailable when the method takes a JSON argument).\n",
            model::rust_ident(m.py_name)
        ));
        let sig = signature(m, &format!("{}_as", m.py_name), as_ty, true);
        out.push_str(&format!("{pad}{}", sig));
        out.push_str(&body(m, as_decoder, &format!("{pad}        ")));
        out.push_str(&format!("{pad}    }}\n\n"));
    }
}

/// Render the whole generated Rust SDK surface.
pub fn render() -> String {
    let mut out = model::header("//");
    out.push_str(
        "//!\n\
         //! Typed wrappers over the single WIT gateway `host.call(method, args)`.\n\
         //! Every function serialises its arguments into a positional JSON array\n\
         //! and decodes the reply through `crate::wire`, which turns an\n\
         //! `{\"error\": true, ...}` envelope into an `Err` (defensive rule: a host\n\
         //! that answers Ok with an error envelope is still an error).\n\
         \n\
         #![allow(clippy::too_many_arguments)]\n\n",
    );

    // Group by top-level namespace so `admin_nodes` / `admin_sql` land inside
    // one `admin` module.
    let grouped = by_category();
    let mut i = 0usize;
    while i < grouped.len() {
        let path = model::namespace_path(grouped[i].0);
        if path.len() == 1 {
            out.push_str(&format!("pub mod {} {{\n", model::rust_ident(&path[0])));
            for m in &grouped[i].1 {
                render_method(m, &mut out, "");
            }
            out.push_str("}\n\n");
            i += 1;
            continue;
        }
        // Nested: consume every consecutive category sharing this outer name.
        let outer = path[0].clone();
        out.push_str(&format!("pub mod {} {{\n", model::rust_ident(&outer)));
        while i < grouped.len() {
            let p = model::namespace_path(grouped[i].0);
            if p.len() != 2 || p[0] != outer {
                break;
            }
            out.push_str(&format!("    pub mod {} {{\n", model::rust_ident(&p[1])));
            for m in &grouped[i].1 {
                render_method(m, &mut out, "    ");
            }
            out.push_str("    }\n\n");
            i += 1;
        }
        out.push_str("}\n\n");
    }
    out
}
