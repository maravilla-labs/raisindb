use super::*;

#[test]
fn test_parse_sync_config() {
    let yaml = r##"
remote:
  url: "https://raisindb.example.com"
  repo_id: "my-project"
  branch: "main"

defaults:
  mode: merge
  on_conflict: prefer_newer
  sync_deletions: true

filters:
  - root: /content/pages
    mode: merge
    include:
      - "**/*.yaml"
    exclude:
      - "drafts/**"
      - "**/.local"

  - root: /content/dev
    direction: local_only

  - root: /system
    direction: server_only

conflicts:
  "/content/pages/home":
    strategy: prefer_local
    backup: true
"##;

    let config: SyncConfig = serde_yaml::from_str(yaml).unwrap();

    assert!(config.remote.is_some());
    let remote = config.remote.as_ref().unwrap();
    assert_eq!(remote.url, "https://raisindb.example.com");
    assert_eq!(remote.repo_id, "my-project");
    assert_eq!(remote.branch, "main");

    assert_eq!(config.defaults.mode, SyncMode::Merge);
    assert_eq!(config.defaults.on_conflict, ConflictStrategy::PreferNewer);

    assert_eq!(config.filters.len(), 3);
    assert_eq!(config.filters[0].root, "/content/pages");
    assert_eq!(config.filters[1].direction, Some(SyncDirection::LocalOnly));
    assert_eq!(config.filters[2].direction, Some(SyncDirection::ServerOnly));

    assert!(config.conflicts.contains_key("/content/pages/home"));
}

#[test]
fn test_glob_matches() {
    use super::filter::glob_matches;

    // Basic patterns
    assert!(glob_matches("*.yaml", "test.yaml"));
    assert!(!glob_matches("*.yaml", "test.json"));

    // Recursive patterns
    assert!(glob_matches("**/*.yaml", "test.yaml"));
    assert!(glob_matches("**/*.yaml", "dir/test.yaml"));
    assert!(glob_matches("**/*.yaml", "dir/sub/test.yaml"));

    // Path patterns
    assert!(glob_matches("drafts/**", "drafts/file.yaml"));
    assert!(glob_matches("drafts/**", "drafts/sub/file.yaml"));
    assert!(!glob_matches("drafts/**", "other/file.yaml"));

    // Question mark
    assert!(glob_matches("test?.yaml", "test1.yaml"));
    assert!(!glob_matches("test?.yaml", "test12.yaml"));
}

#[test]
fn test_should_sync_path() {
    let yaml = r##"
filters:
  - root: /content
    include:
      - "**/*.yaml"
    exclude:
      - "drafts/**"

  - root: /local
    direction: local_only
"##;

    let config: SyncConfig = serde_yaml::from_str(yaml).unwrap();

    assert!(config.should_sync_path("/content/pages/home.yaml"));
    assert!(!config.should_sync_path("/content/pages/home.json"));
    assert!(!config.should_sync_path("/content/drafts/wip.yaml"));
    assert!(!config.should_sync_path("/local/dev/test.yaml"));
}

#[test]
fn test_sync_direction() {
    assert!(SyncDirection::Bidirectional.allows_push());
    assert!(SyncDirection::Bidirectional.allows_pull());

    assert!(!SyncDirection::LocalOnly.allows_push());
    assert!(!SyncDirection::LocalOnly.allows_pull());

    assert!(!SyncDirection::ServerOnly.allows_push());
    assert!(SyncDirection::ServerOnly.allows_pull());

    assert!(SyncDirection::PushOnly.allows_push());
    assert!(!SyncDirection::PushOnly.allows_pull());
}
