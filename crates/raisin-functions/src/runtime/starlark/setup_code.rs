// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Starlark setup code generation for the raisin namespace

use crate::runtime::bindings::methods::registry;
use crate::types::ExecutionContext;

/// Generate Starlark setup code that creates the raisin namespace with full API
pub(super) fn generate_setup_code(context: &ExecutionContext) -> String {
    // Escape strings for Starlark
    let escape_str = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");

    // Build the wrapper functions for all API methods
    let reg = registry();
    let mut wrapper_funcs = String::new();

    for method in reg.methods() {
        // Skip internal category
        if method.category == "internal" {
            continue;
        }

        // Build argument list
        let args: Vec<String> = method
            .args
            .iter()
            .map(|a| {
                use crate::runtime::bindings::registry::ArgType;
                match a.arg_type {
                    ArgType::OptionalString
                    | ArgType::OptionalJson
                    | ArgType::OptionalU32
                    | ArgType::OptionalI64
                    | ArgType::OptionalBool => format!("{} = None", a.name),
                    ArgType::JsonArray => format!("{} = []", a.name),
                    _ => a.name.to_string(),
                }
            })
            .collect();
        let args_str = args.join(", ");

        // Build the call arguments
        let call_args: Vec<&str> = method.args.iter().map(|a| a.name).collect();
        let call_args_str = call_args.join(", ");

        wrapper_funcs.push_str(&format!(
            r#"def _{}({}):
    result = __raisin_call("{}", [{}])
    decoded = json_decode(result)
    if type(decoded) == "dict" and decoded.get("error"):
        fail("API Error [{}]: " + str(decoded.get("message", "Unknown error")))
    return decoded

"#,
            method.internal_name,
            args_str,
            method.internal_name,
            call_args_str,
            method.internal_name
        ));
    }

    // Add log helper functions
    wrapper_funcs.push_str(
        r#"
# Log helper functions
def _log_debug(*args):
    __raisin_log("debug", list(args))

def _log_info(*args):
    __raisin_log("info", list(args))

def _log_warn(*args):
    __raisin_log("warn", list(args))

def _log_error(*args):
    __raisin_log("error", list(args))

"#,
    );

    // Build the raisin namespace struct
    let mut namespace_code = String::new();
    namespace_code.push_str("# Build the raisin namespace\n");
    namespace_code.push_str("raisin = struct(\n");

    // Add context
    namespace_code.push_str(&format!(
        r#"    context = struct(
        tenant_id = "{}",
        repo_id = "{}",
        branch = "{}",
        workspace = "{}",
        actor = "{}",
        execution_id = "{}",
    ),
"#,
        escape_str(&context.tenant_id),
        escape_str(&context.repo_id),
        escape_str(&context.branch),
        escape_str(context.workspace_id.as_deref().unwrap_or("default")),
        escape_str(&context.actor),
        escape_str(&context.execution_id),
    ));

    // Add log struct to raisin namespace
    namespace_code.push_str(
        r#"    log = struct(
        debug = _log_debug,
        info = _log_info,
        warn = _log_warn,
        error = _log_error,
    ),
"#,
    );

    // Group methods by category and build sub-structs
    let categories = reg.categories();
    for category in &categories {
        // Skip internal, admin, context (context is manually added above with execution info),
        // and notify (added as direct method, not nested in a struct)
        if *category == "internal"
            || category.starts_with("admin_")
            || *category == "context"
            || *category == "notify"
        {
            continue;
        }

        let methods = reg.methods_by_category(category);
        if methods.is_empty() {
            continue;
        }

        // Special handling for http
        if *category == "http" {
            namespace_code.push_str("    http = struct(\n");
            namespace_code.push_str("        request = _http_request,\n");
            namespace_code.push_str("        get = lambda url, headers={}, params={}: _http_request(\"GET\", url, {\"headers\": headers, \"params\": params}),\n");
            namespace_code.push_str("        post = lambda url, json=None, headers={}: _http_request(\"POST\", url, {\"headers\": headers, \"body\": json}),\n");
            namespace_code.push_str("        put = lambda url, json=None, headers={}: _http_request(\"PUT\", url, {\"headers\": headers, \"body\": json}),\n");
            namespace_code.push_str("        patch = lambda url, json=None, headers={}: _http_request(\"PATCH\", url, {\"headers\": headers, \"body\": json}),\n");
            namespace_code.push_str("        delete = lambda url, headers={}: _http_request(\"DELETE\", url, {\"headers\": headers}),\n");
            namespace_code.push_str("    ),\n");
            continue;
        }

        namespace_code.push_str(&format!("    {} = struct(\n", category));
        for method in &methods {
            namespace_code.push_str(&format!(
                "        {} = _{},\n",
                method.py_name, method.internal_name
            ));
        }
        namespace_code.push_str("    ),\n");
    }

    // Add admin namespace
    let admin_nodes = reg.methods_by_category("admin_nodes");
    let admin_sql = reg.methods_by_category("admin_sql");
    if !admin_nodes.is_empty() || !admin_sql.is_empty() {
        namespace_code.push_str("    admin = struct(\n");
        if !admin_nodes.is_empty() {
            namespace_code.push_str("        nodes = struct(\n");
            for method in &admin_nodes {
                namespace_code.push_str(&format!(
                    "            {} = _{},\n",
                    method.py_name, method.internal_name
                ));
            }
            namespace_code.push_str("        ),\n");
        }
        if !admin_sql.is_empty() {
            namespace_code.push_str("        sql = struct(\n");
            for method in &admin_sql {
                namespace_code.push_str(&format!(
                    "            {} = _{},\n",
                    method.py_name, method.internal_name
                ));
            }
            namespace_code.push_str("        ),\n");
        }
        namespace_code.push_str("    ),\n");
    }

    // Add notify as a direct method (not nested in a struct)
    // This allows raisin.notify({...}) instead of raisin.notify.notify({...})
    let notify_methods = reg.methods_by_category("notify");
    for method in &notify_methods {
        namespace_code.push_str(&format!(
            "    {} = _{},\n",
            method.py_name, method.internal_name
        ));
    }

    namespace_code.push_str(")\n\n");

    // Add top-level log variable for convenience (log.debug instead of raisin.log.debug)
    namespace_code.push_str(
        r#"# Top-level log for convenience
log = struct(
    debug = _log_debug,
    info = _log_info,
    warn = _log_warn,
    error = _log_error,
)
"#,
    );

    // Combine everything
    format!(
        r#"# RaisinDB Runtime Setup
# Auto-generated - provides full raisin.* API

{}
{}
"#,
        wrapper_funcs, namespace_code
    )
}
