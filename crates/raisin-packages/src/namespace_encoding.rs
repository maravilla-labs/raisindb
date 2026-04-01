/// Filesystem-safe encoding for namespaced names (e.g. workspace names, node types).
///
/// Names containing colons like `raisin:access_control` are invalid on Windows.
/// Following the Apache Jackrabbit FileVault convention, we encode the namespace
/// prefix by wrapping it in underscores and removing the colon:
///
/// - `raisin:access_control` → `_raisin_access_control`
/// - `myapp:Article` → `_myapp_Article`
/// - `default` → `default` (no colon, unchanged)
///
/// This encoding is applied ONLY at filesystem/ZIP boundaries.
/// HTTP APIs, SQL queries, and internal storage always use logical names with colons.

/// Encode a namespaced name for filesystem use.
/// `raisin:access_control` → `_raisin_access_control`
pub fn encode_namespace(name: &str) -> String {
    if let Some(pos) = name.find(':') {
        let prefix = &name[..pos];
        let rest = &name[pos + 1..];
        format!("_{prefix}_{rest}")
    } else {
        name.to_string()
    }
}

/// Decode a filesystem-encoded name back to its logical namespaced form.
/// `_raisin_access_control` → `raisin:access_control`
pub fn decode_namespace(name: &str) -> String {
    // Only decode names that start with a single underscore (not double)
    if name.starts_with('_') && !name.starts_with("__") {
        // Find the second underscore (end of prefix)
        if let Some(pos) = name[1..].find('_') {
            let prefix = &name[1..pos + 1];
            let rest = &name[pos + 2..];
            return format!("{prefix}:{rest}");
        }
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode() {
        assert_eq!(
            encode_namespace("raisin:access_control"),
            "_raisin_access_control"
        );
        assert_eq!(encode_namespace("myapp:Article"), "_myapp_Article");
        assert_eq!(encode_namespace("default"), "default");
        assert_eq!(encode_namespace("functions"), "functions");
    }

    #[test]
    fn test_decode() {
        assert_eq!(
            decode_namespace("_raisin_access_control"),
            "raisin:access_control"
        );
        assert_eq!(decode_namespace("_myapp_Article"), "myapp:Article");
        assert_eq!(decode_namespace("default"), "default");
        assert_eq!(decode_namespace("functions"), "functions");
    }

    #[test]
    fn test_roundtrip() {
        let names = [
            "raisin:access_control",
            "myapp:Article",
            "default",
            "raisin:Folder",
        ];
        for name in names {
            assert_eq!(decode_namespace(&encode_namespace(name)), name);
        }
    }

    #[test]
    fn test_double_underscore_passthrough() {
        assert_eq!(decode_namespace("__internal"), "__internal");
    }
}
