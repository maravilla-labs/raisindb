//! String conversion and error formatting utilities

/// Sanitize a package name to be valid
pub(crate) fn sanitize_package_name(name: &str) -> String {
    let mut result = String::new();
    let mut first = true;

    for c in name.chars() {
        if first {
            if c.is_alphabetic() {
                result.push(c.to_ascii_lowercase());
                first = false;
            }
        } else if c.is_alphanumeric() || c == '-' || c == '_' {
            result.push(c.to_ascii_lowercase());
        } else if c == ' ' {
            result.push('-');
        }
    }

    if result.is_empty() {
        "my-package".to_string()
    } else {
        result
    }
}

/// Suggest a valid node type name from an invalid one
pub(crate) fn suggest_node_type_name(invalid: &str) -> String {
    // If it has no namespace, add "custom:"
    if !invalid.contains(':') {
        let pascal = to_pascal_case(invalid);
        return format!("custom:{}", pascal);
    }

    // If it has a namespace, convert the type part to PascalCase
    let parts: Vec<&str> = invalid.splitn(2, ':').collect();
    if parts.len() == 2 {
        let namespace = parts[0].to_lowercase();
        let type_part = to_pascal_case(parts[1]);
        return format!("{}:{}", namespace, type_part);
    }

    // Fallback
    format!("custom:{}", to_pascal_case(invalid))
}

/// Convert a string to PascalCase
pub(crate) fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in s.chars() {
        if c == '_' || c == '-' || c == ' ' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c.to_ascii_lowercase());
        }
    }

    if result.is_empty() {
        "Unknown".to_string()
    } else {
        result
    }
}

/// Format serde YAML error into a user-friendly message
pub(crate) fn format_serde_error(e: &serde_yaml::Error, _yaml_str: &str) -> String {
    let msg = e.to_string();

    // Check for common issues and provide helpful hints
    if msg.contains("missing field") {
        return format!("Missing required field: {}", msg);
    }

    if msg.contains("unknown field") {
        return format!("Unknown field: {}", msg);
    }

    // Special case: 'type' vs '$type' confusion for FieldSchema
    if msg.contains("$type") || msg.contains("unknown variant") {
        return format!(
            "{}. Note: Fields must use '$type' (not 'type') to specify the field type, e.g., '$type: TextField'",
            msg
        );
    }

    msg
}
