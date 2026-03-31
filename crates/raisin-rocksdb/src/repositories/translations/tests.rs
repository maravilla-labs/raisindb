//! Tests for translation key encoding.

#[cfg(test)]
mod key_encoding_tests {
    use super::super::keys;
    use raisin_hlc::HLC;

    #[test]
    fn test_translation_key_encoding() {
        let revision = HLC::new(42, 0);
        let key = keys::translation_key(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node123",
            "fr-FR",
            &revision,
        );

        let key_str = String::from_utf8_lossy(&key[..key.len() - 16]).to_string();
        assert!(key_str.contains("tenant1"));
        assert!(key_str.contains("repo1"));
        assert!(key_str.contains("main"));
        assert!(key_str.contains("workspace1"));
        assert!(key_str.contains("translations"));
        assert!(key_str.contains("node123"));
        assert!(key_str.contains("fr-FR"));
    }

    #[test]
    fn test_block_translation_key_encoding() {
        let revision = HLC::new(100, 0);
        let key = keys::block_translation_key(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node123",
            "block-uuid-456",
            "de-DE",
            &revision,
        );

        let key_str = String::from_utf8_lossy(&key[..key.len() - 16]).to_string();
        assert!(key_str.contains("block_trans"));
        assert!(key_str.contains("block-uuid-456"));
        assert!(key_str.contains("de-DE"));
    }

    #[test]
    fn test_translation_index_key_encoding() {
        let revision = HLC::new(200, 0);
        let key = keys::translation_index_key("tenant1", "repo1", "es-MX", &revision, "node789");

        // NOTE: For translation_index_key, the HLC is in the MIDDLE of the key, not at the end
        // Key format: {tenant}\0{repo}\0translation_index\0{locale}\0{~revision}\0{node_id}
        // So we check the full key (from_utf8_lossy handles the HLC bytes gracefully)
        let key_str = String::from_utf8_lossy(&key).to_string();
        assert!(key_str.contains("translation_index"));
        assert!(key_str.contains("es-MX"));
        assert!(key_str.contains("node789"));
    }
}
