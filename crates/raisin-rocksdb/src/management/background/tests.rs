//! Tests for background jobs configuration.

use super::*;

#[test]
fn test_default_config() {
    let config = BackgroundJobsConfig::default();
    assert!(config.integrity_check_enabled);
    assert!(config.compaction_enabled);
    assert!(!config.backup_enabled);
    assert!(config.self_heal_enabled);
    assert_eq!(config.max_concurrent_jobs, 2);
}
