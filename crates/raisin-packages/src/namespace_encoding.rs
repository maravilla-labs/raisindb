/// Filesystem-safe encoding for namespaced names (e.g. workspace names, node types).
///
/// Names containing colons like `raisin:access_control` are invalid on Windows.
/// We encode the namespace prefix with a leading underscore and replace the colon
/// with a double underscore (`__`):
///
/// - `raisin:access_control` → `_raisin__access_control`
/// - `myapp:Article` → `_myapp__Article`
/// - `default` → `default` (no colon, unchanged)
///
/// The double underscore makes the encoding unambiguous — names with single
/// underscores (like `my_app:thing`) round-trip correctly.
///
/// This encoding is applied ONLY at filesystem/ZIP boundaries.
/// HTTP APIs, SQL queries, and internal storage always use logical names with colons.

/// Encode a namespaced name for filesystem use.
/// `raisin:access_control` → `_raisin__access_control`
pub fn encode_namespace(name: &str) -> String {
    if let Some(pos) = name.find(':') {
        let prefix = &name[..pos];
        let rest = &name[pos + 1..];
        format!("_{prefix}__{rest}")
    } else {
        name.to_string()
    }
}

/// Decode a filesystem-encoded name back to its logical namespaced form.
/// `_raisin__access_control` → `raisin:access_control`
pub fn decode_namespace(name: &str) -> String {
    // Only decode names that start with a single underscore (not double)
    if name.starts_with('_') && !name.starts_with("__") {
        // Find the double underscore (colon separator)
        if let Some(pos) = name[1..].find("__") {
            let prefix = &name[1..pos + 1];
            let rest = &name[pos + 3..];
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
            "_raisin__access_control"
        );
        assert_eq!(encode_namespace("myapp:Article"), "_myapp__Article");
        assert_eq!(encode_namespace("default"), "default");
        assert_eq!(encode_namespace("functions"), "functions");
    }

    #[test]
    fn test_decode() {
        assert_eq!(
            decode_namespace("_raisin__access_control"),
            "raisin:access_control"
        );
        assert_eq!(decode_namespace("_myapp__Article"), "myapp:Article");
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
            "my_app:some_thing",
        ];
        for name in names {
            assert_eq!(decode_namespace(&encode_namespace(name)), name);
        }
    }

    #[test]
    fn test_double_underscore_passthrough() {
        assert_eq!(decode_namespace("__internal"), "__internal");
    }

    #[test]
    fn test_underscore_in_prefix_roundtrip() {
        // Names with underscores in the prefix should round-trip correctly
        assert_eq!(
            encode_namespace("my_app:thing"),
            "_my_app__thing"
        );
        assert_eq!(
            decode_namespace("_my_app__thing"),
            "my_app:thing"
        );
    }
}
