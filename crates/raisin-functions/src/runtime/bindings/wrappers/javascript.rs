// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! JavaScript wrapper code generator
//!
//! Generates the JavaScript wrapper code that creates the `raisin` global
//! object with all the user-facing APIs.

use crate::runtime::bindings::methods::registry;
use crate::runtime::bindings::registry::ReturnType;

/// Generate the JavaScript wrapper code
///
/// This creates the `globalThis.raisin` object that exposes all APIs
/// in a user-friendly format. Internal functions are called via
/// `__raisin_internal.*` and results are parsed/formatted appropriately.
pub fn generate_js_wrapper() -> String {
    let reg = registry();
    let mut code = String::with_capacity(8192);

    code.push_str("// RaisinDB JavaScript API Wrapper\n");
    code.push_str("// Auto-generated from shared bindings registry\n\n");
    code.push_str(
        r#"const __raisinParseInternalError = (raw, methodName) => {
  if (typeof raw !== 'string') {
    throw new Error(`Invalid response from ${methodName}`);
  }
  if (raw.startsWith('{')) {
    try {
      const parsed = JSON.parse(raw);
      if (parsed && parsed.error === true) {
        throw new Error(parsed.message || `Internal API call failed (${methodName})`);
      }
    } catch (err) {
      if (err instanceof Error) throw err;
    }
  }
  return raw;
};

"#,
    );
    code.push_str(
        r#"const __raisinParseJson = (raw, methodName) => {
  const value = __raisinParseInternalError(raw, methodName);
  try {
    return JSON.parse(value);
  } catch (err) {
    throw new Error(`Invalid JSON response from ${methodName}: ${String(err)}`);
  }
};

"#,
    );

    // Start building the raisin object
    code.push_str("globalThis.raisin = {\n");

    // Group methods by category
    let categories = reg.categories();

    for category in &categories {
        // Skip internal methods
        if *category == "internal" {
            continue;
        }

        let methods = reg.methods_by_category(category);
        if methods.is_empty() {
            continue;
        }

        // Handle admin categories specially (nested under admin)
        if category.starts_with("admin_") {
            // These will be handled in the admin section
            continue;
        }

        // Handle notify as a direct method, not a namespace
        if *category == "notify" {
            // These will be added as direct methods below
            continue;
        }

        code.push_str(&format!("  {}: {{\n", category));

        for method in &methods {
            code.push_str(&generate_js_method(method));
        }

        code.push_str("  },\n\n");
    }

    // Generate notify as a direct method (convenience function)
    // Uses options object pattern: raisin.notify({ title, body, recipient, ... })
    let notify_methods = reg.methods_by_category("notify");
    if !notify_methods.is_empty() {
        code.push_str("  // Notification convenience method\n");
        code.push_str("  // Usage: raisin.notify({ title, body, recipient, recipientId, priority, link, data })\n");
        for method in &notify_methods {
            code.push_str(&format!("  async {}(options) {{\n", method.js_name));
            code.push_str(&format!(
                "    const result = __raisin_internal.{}(JSON.stringify(options));\n",
                method.internal_name
            ));
            code.push_str(&format!(
                "    return __raisinParseJson(result, '{}');\n",
                method.internal_name
            ));
            code.push_str("  },\n\n");
        }
    }

    // Generate admin namespace with nested nodes/sql
    if !reg.methods_by_category("admin_nodes").is_empty()
        || !reg.methods_by_category("admin_sql").is_empty()
    {
        code.push_str(
            "  // Admin methods bypass RLS (requires requiresAdmin: true in function metadata)\n",
        );
        code.push_str("  admin: {\n");

        let admin_nodes = reg.methods_by_category("admin_nodes");
        if !admin_nodes.is_empty() {
            code.push_str("    nodes: {\n");
            for method in &admin_nodes {
                code.push_str(&generate_js_method_indented(method, 6));
            }
            code.push_str("    },\n");
        }

        let admin_sql = reg.methods_by_category("admin_sql");
        if !admin_sql.is_empty() {
            code.push_str("    sql: {\n");
            for method in &admin_sql {
                code.push_str(&generate_js_method_indented(method, 6));
            }
            code.push_str("    },\n");
        }

        code.push_str("  },\n\n");
    }

    // Add context as a getter
    code.push_str("  // Execution context\n");
    code.push_str("  get context() {\n");
    code.push_str("    return JSON.parse(__raisin_internal.context_get());\n");
    code.push_str("  },\n\n");

    // Add HTTP convenience methods
    code.push_str("  // HTTP convenience methods\n");
    code.push_str("  get http() {\n");
    code.push_str("    return {\n");
    code.push_str("      request: async (method, url, options = {}) => {\n");
    code.push_str("        const result = __raisin_internal.http_request(method, url, JSON.stringify(options));\n");
    code.push_str("        return __raisinParseJson(result, 'http_request');\n");
    code.push_str("      },\n");
    code.push_str("      get: async (url, options = {}) => {\n");
    code.push_str("        return await raisin.http.request('GET', url, options);\n");
    code.push_str("      },\n");
    code.push_str("      post: async (url, options = {}) => {\n");
    code.push_str("        return await raisin.http.request('POST', url, options);\n");
    code.push_str("      },\n");
    code.push_str("      put: async (url, options = {}) => {\n");
    code.push_str("        return await raisin.http.request('PUT', url, options);\n");
    code.push_str("      },\n");
    code.push_str("      patch: async (url, options = {}) => {\n");
    code.push_str("        return await raisin.http.request('PATCH', url, options);\n");
    code.push_str("      },\n");
    code.push_str("      delete: async (url, options = {}) => {\n");
    code.push_str("        return await raisin.http.request('DELETE', url, options);\n");
    code.push_str("      },\n");
    code.push_str("    };\n");
    code.push_str("  },\n\n");

    // Add asAdmin() helper
    code.push_str("  // Admin escalation helper\n");
    code.push_str("  asAdmin() {\n");
    code.push_str("    const allowed = __raisinParseInternalError(__raisin_internal.allowsAdminEscalation(), 'allowsAdminEscalation');\n");
    code.push_str("    if (allowed !== 'true') {\n");
    code.push_str("      throw new Error('Admin escalation not allowed. Add requiresAdmin: true to function metadata.');\n");
    code.push_str("    }\n");
    code.push_str("    return raisin.admin;\n");
    code.push_str("  },\n");

    code.push_str("};\n");

    code
}

/// Generate a single method wrapper
fn generate_js_method(method: &crate::runtime::bindings::registry::ApiMethodDescriptor) -> String {
    generate_js_method_indented(method, 4)
}

/// Generate a single method wrapper with custom indentation
fn generate_js_method_indented(
    method: &crate::runtime::bindings::registry::ApiMethodDescriptor,
    indent: usize,
) -> String {
    let indent_str = " ".repeat(indent);
    let mut code = String::new();

    // Build argument list
    let args: Vec<&str> = method.args.iter().map(|a| a.name).collect();
    let args_str = args.join(", ");

    // Build internal call arguments (serialize JSON args)
    let call_args: Vec<String> = method
        .args
        .iter()
        .map(|a| match a.arg_type {
            crate::runtime::bindings::registry::ArgType::Json
            | crate::runtime::bindings::registry::ArgType::OptionalJson
            | crate::runtime::bindings::registry::ArgType::JsonArray => {
                format!("JSON.stringify({})", a.name)
            }
            _ => a.name.to_string(),
        })
        .collect();
    let call_args_str = call_args.join(", ");

    // Generate method
    code.push_str(&format!(
        "{}async {}({}) {{\n",
        indent_str, method.js_name, args_str
    ));

    // Call internal function
    code.push_str(&format!(
        "{}  const result = __raisin_internal.{}({});\n",
        indent_str, method.internal_name, call_args_str
    ));

    // Parse result based on return type
    match method.return_type {
        ReturnType::Json | ReturnType::OptionalJson | ReturnType::JsonArray => {
            code.push_str(&format!(
                "{}  return __raisinParseJson(result, '{}');\n",
                indent_str, method.internal_name
            ));
        }
        ReturnType::Bool => {
            code.push_str(&format!(
                "{}  const value = __raisinParseInternalError(result, '{}');\n",
                indent_str, method.internal_name
            ));
            code.push_str(&format!(
                "{}  if (value === 'true') return true;\n",
                indent_str
            ));
            code.push_str(&format!(
                "{}  if (value === 'false') return false;\n",
                indent_str
            ));
            code.push_str(&format!(
                "{}  throw new Error(`Invalid boolean response from {}: ${{value}}`);\n",
                indent_str, method.internal_name
            ));
        }
        ReturnType::I64 => {
            code.push_str(&format!(
                "{}  const value = __raisinParseInternalError(result, '{}');\n",
                indent_str, method.internal_name
            ));
            code.push_str(&format!(
                "{}  const parsed = parseInt(value, 10);\n",
                indent_str
            ));
            code.push_str(&format!(
                "{}  if (!Number.isFinite(parsed)) {{ throw new Error(`Invalid integer response from {}: ${{value}}`); }}\n",
                indent_str, method.internal_name
            ));
            code.push_str(&format!("{}  return parsed;\n", indent_str));
        }
        ReturnType::String => {
            code.push_str(&format!(
                "{}  return __raisinParseJson(result, '{}');\n",
                indent_str, method.internal_name
            ));
        }
        ReturnType::Void => {
            code.push_str(&format!(
                "{}  const value = __raisinParseInternalError(result, '{}');\n",
                indent_str, method.internal_name
            ));
            code.push_str(&format!("{}  if (value !== 'true') {{\n", indent_str));
            code.push_str(&format!(
                "{}    let message = `Operation failed for {}`;\n",
                indent_str, method.internal_name
            ));
            code.push_str(&format!(
                "{}    if (value.startsWith('{{')) {{\n",
                indent_str
            ));
            code.push_str(&format!(
                "{}      try {{ const parsed = JSON.parse(value); message = parsed?.message || message; }} catch (_) {{}}\n",
                indent_str
            ));
            code.push_str(&format!("{}    }}\n", indent_str));
            code.push_str(&format!("{}    throw new Error(message);\n", indent_str));
            code.push_str(&format!("{}  }}\n", indent_str));
        }
    }

    code.push_str(&format!("{}}},\n", indent_str));

    code
}

/// Get the generated wrapper code as a static string (cached)
pub fn get_wrapper_code() -> &'static str {
    use std::sync::OnceLock;
    static WRAPPER: OnceLock<String> = OnceLock::new();
    WRAPPER.get_or_init(generate_js_wrapper)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_wrapper() {
        let code = generate_js_wrapper();

        // Should have basic structure
        assert!(code.contains("globalThis.raisin"));
        assert!(code.contains("nodes:"));
        assert!(code.contains("sql:"));
        assert!(code.contains("ai:"));

        // Should have admin methods
        assert!(code.contains("admin:"));

        // Should have context getter
        assert!(code.contains("get context()"));

        // Should have HTTP convenience methods (in getter)
        assert!(code.contains("get: async (url"));
        assert!(code.contains("post: async (url"));

        // Should have notify as a direct method
        assert!(code.contains("async notify("));
    }

    #[test]
    fn test_wrapper_has_all_categories() {
        let code = generate_js_wrapper();
        let reg = registry();

        for category in reg.categories() {
            if category == "internal" {
                continue;
            }
            if category.starts_with("admin_") {
                // Admin categories are nested
                assert!(code.contains("admin:"), "Should have admin namespace");
            } else if category == "notify" {
                // Notify is exposed as a direct method, not a namespace
                assert!(
                    code.contains("async notify("),
                    "Should have notify as direct method"
                );
            } else {
                assert!(
                    code.contains(&format!("{}:", category)),
                    "Missing category: {}",
                    category
                );
            }
        }
    }
}
