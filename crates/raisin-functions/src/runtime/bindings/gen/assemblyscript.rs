// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AssemblyScript guest-SDK backend: renders
//! `sdks/assemblyscript/assembly/generated.ts`.
//!
//! AssemblyScript has no JSON in its standard library and bundling one would
//! make every artifact pay for it, so the generated surface is deliberately
//! STRING-IN, STRING-OUT: arguments are composed into a JSON array by
//! `argsOf`, and a result comes back as the raw JSON the host produced. A
//! caller that wants typed values parses with whatever library it chose.
//!
//! That also keeps this backend honest about the one thing it cannot do: it
//! cannot hand back a `Value` the way the Rust SDK does, so it does not
//! pretend to.

use super::model::{self, by_category};
use crate::runtime::bindings::registry::{ApiMethodDescriptor, ArgType};

/// AssemblyScript type for an argument.
///
/// Optionals are `string | null` rather than a wrapper: AssemblyScript has
/// nullable reference types, and `null` is exactly what the gateway expects
/// for an absent optional.
fn arg_type(t: ArgType) -> &'static str {
    match t {
        ArgType::String => "string",
        ArgType::OptionalString => "string | null",
        // A JSON argument arrives already encoded: the caller owns the shape,
        // so the SDK takes the text and splices it into the argument array.
        ArgType::Json | ArgType::JsonArray => "string",
        ArgType::OptionalJson => "string | null",
        ArgType::U32 => "u32",
        ArgType::OptionalU32 => "i64",
        ArgType::I64 => "i64",
        ArgType::OptionalI64 => "i64",
        ArgType::Bool => "bool",
        ArgType::OptionalBool => "i32",
        ArgType::StringArray => "string[]",
    }
}

/// How one argument is turned into its JSON-array element.
fn encode_arg(name: &str, t: ArgType) -> String {
    match t {
        ArgType::String => format!("jsonString({name})"),
        ArgType::OptionalString => format!("jsonStringOrNull({name})"),
        // Already JSON text.
        ArgType::Json | ArgType::JsonArray => name.to_string(),
        ArgType::OptionalJson => format!("({name} == null ? \"null\" : {name}!)"),
        ArgType::U32 | ArgType::I64 => format!("{name}.toString()"),
        // A negative sentinel means "absent": AssemblyScript has no Option, and
        // every optional numeric in the registry is a limit or a count.
        ArgType::OptionalU32 | ArgType::OptionalI64 => {
            format!("({name} < 0 ? \"null\" : {name}.toString())")
        }
        ArgType::Bool => format!("({name} ? \"true\" : \"false\")"),
        ArgType::OptionalBool => {
            format!("({name} < 0 ? \"null\" : ({name} != 0 ? \"true\" : \"false\"))")
        }
        ArgType::StringArray => format!("jsonStringArray({name})"),
    }
}

/// AssemblyScript reserves a few words the registry also uses as argument
/// names. Suffix rather than rename, so the generated signature still reads
/// like the documented API.
fn ident(name: &str) -> String {
    const RESERVED: &[&str] = &[
        "function",
        "class",
        "namespace",
        "export",
        "import",
        "type",
        "const",
        "let",
        "var",
        "new",
        "delete",
        "in",
        "instanceof",
        "typeof",
        "void",
        "null",
        "true",
        "false",
        "this",
        "super",
        "return",
        "if",
        "else",
        "for",
        "while",
        "do",
        "switch",
        "case",
        "default",
    ];
    if RESERVED.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

fn render_method(m: &ApiMethodDescriptor, out: &mut String) {
    let params: Vec<String> = m
        .args
        .iter()
        .map(|a| format!("{}: {}", ident(a.name), arg_type(a.arg_type)))
        .collect();
    let encoded: Vec<String> = m
        .args
        .iter()
        .map(|a| encode_arg(&ident(a.name), a.arg_type))
        .collect();
    let args_expr = if encoded.is_empty() {
        "\"[]\"".to_string()
    } else {
        format!("argsOf([{}])", encoded.join(", "))
    };

    out.push_str(&format!(
        "  /** `raisin.{}.{}` — registry method `{}`. Returns raw JSON. */\n",
        model::namespace_path(m.category).join("."),
        m.js_name,
        m.internal_name
    ));
    out.push_str(&format!(
        "  export function {}({}): string {{\n    return call({:?}, {});\n  }}\n\n",
        ident(m.js_name),
        params.join(", "),
        m.internal_name,
        args_expr
    ));
}

/// Render the whole generated AssemblyScript surface.
pub fn render() -> String {
    let mut out = model::header("//");
    out.push_str(
        "\nimport { call } from \"./abi\";\n\n\
         // --- JSON argument helpers -------------------------------------------------\n\
         // AssemblyScript ships no JSON encoder, so the few shapes the gateway needs\n\
         // are built here rather than pulling a library into every artifact.\n\n\
         function jsonString(s: string): string {\n  \
           let out = \"\\\"\";\n  \
           for (let i = 0; i < s.length; i++) {\n    \
             const c = s.charCodeAt(i);\n    \
             if (c == 0x22) out += \"\\\\\\\"\";\n    \
             else if (c == 0x5c) out += \"\\\\\\\\\";\n    \
             else if (c == 0x0a) out += \"\\\\n\";\n    \
             else if (c == 0x0d) out += \"\\\\r\";\n    \
             else if (c == 0x09) out += \"\\\\t\";\n    \
             else if (c < 0x20) out += \"\\\\u\" + c.toString(16).padStart(4, \"0\");\n    \
             else out += String.fromCharCode(c);\n  \
           }\n  \
           return out + \"\\\"\";\n\
         }\n\n\
         function jsonStringOrNull(s: string | null): string {\n  \
           return s == null ? \"null\" : jsonString(s!);\n\
         }\n\n\
         function jsonStringArray(items: string[]): string {\n  \
           let out = \"[\";\n  \
           for (let i = 0; i < items.length; i++) {\n    \
             if (i > 0) out += \",\";\n    \
             out += jsonString(items[i]);\n  \
           }\n  \
           return out + \"]\";\n\
         }\n\n\
         function argsOf(parts: string[]): string {\n  \
           let out = \"[\";\n  \
           for (let i = 0; i < parts.length; i++) {\n    \
             if (i > 0) out += \",\";\n    \
             out += parts[i];\n  \
           }\n  \
           return out + \"]\";\n\
         }\n\n",
    );

    for (category, methods) in by_category() {
        let ns = model::namespace_path(category);
        // A nested namespace (`admin.nodes`) becomes nested AssemblyScript
        // namespaces, so the call site reads the same as in every other SDK.
        for part in &ns {
            out.push_str(&format!("export namespace {} {{\n", part));
        }
        for m in methods {
            render_method(m, &mut out);
        }
        for _ in &ns {
            out.push_str("}\n");
        }
        out.push('\n');
    }
    out
}
