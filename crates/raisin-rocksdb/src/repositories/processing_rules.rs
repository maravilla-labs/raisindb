//! Processing rules repository implementation backed by RocksDB.
//!
//! Stores AI processing rules per repository in a dedicated column family.
//! Rules control how content is processed for embedding generation, PDF
//! text extraction, image captioning, etc.

use crate::cf;
use crate::cf_handle;
use raisin_ai::{ProcessingRule, ProcessingRuleSet};
use raisin_error::{Error as RaisinError, Result};
use raisin_storage::scope::RepoScope;
use raisin_storage::ProcessingRulesRepository;
use rocksdb::DB;
use std::sync::Arc;

/// Re-export the column family name for external use.
pub use crate::cf::PROCESSING_RULES as CF_PROCESSING_RULES;

/// RocksDB implementation of the ProcessingRulesRepository trait.
///
/// Stores rules as a serialized ProcessingRuleSet per tenant/repo combination.
/// Key format: `{tenant_id}\0{repo_id}`
#[derive(Clone)]
pub struct ProcessingRulesRepositoryImpl {
    db: Arc<DB>,
}

impl ProcessingRulesRepositoryImpl {
    /// Create a new processing rules repository.
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Build the storage key for a tenant/repo combination.
    fn build_key(tenant_id: &str, repo_id: &str) -> Vec<u8> {
        let mut key = Vec::with_capacity(tenant_id.len() + repo_id.len() + 1);
        key.extend_from_slice(tenant_id.as_bytes());
        key.push(0); // null separator
        key.extend_from_slice(repo_id.as_bytes());
        key
    }

    /// Serialize a ProcessingRuleSet to bytes.
    fn serialize(rules: &ProcessingRuleSet) -> Result<Vec<u8>> {
        // Use named fields to avoid tuple ordering issues with skipped Option fields.
        rmp_serde::to_vec_named(rules).map_err(|e| {
            RaisinError::storage(format!("Failed to serialize processing rules: {}", e))
        })
    }

    /// Deserialize bytes to a ProcessingRuleSet.
    fn deserialize(bytes: &[u8]) -> Result<ProcessingRuleSet> {
        rmp_serde::from_slice(bytes).map_err(|e| {
            RaisinError::storage(format!("Failed to deserialize processing rules: {}", e))
        })
    }
}

impl ProcessingRulesRepository for ProcessingRulesRepositoryImpl {
    async fn get_rules(&self, scope: RepoScope<'_>) -> Result<Option<ProcessingRuleSet>> {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let cf = cf_handle(&self.db, cf::PROCESSING_RULES)?;
        let key = Self::build_key(tenant_id, repo_id);

        match self.db.get_cf(&cf, &key) {
            Ok(Some(bytes)) => Ok(Some(Self::deserialize(&bytes)?)),
            Ok(None) => Ok(None),
            Err(e) => Err(RaisinError::storage(format!(
                "Failed to get processing rules: {}",
                e
            ))),
        }
    }

    async fn set_rules(&self, scope: RepoScope<'_>, rules: &ProcessingRuleSet) -> Result<()> {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let cf = cf_handle(&self.db, cf::PROCESSING_RULES)?;
        let key = Self::build_key(tenant_id, repo_id);
        let bytes = Self::serialize(rules)?;

        self.db
            .put_cf(&cf, &key, &bytes)
            .map_err(|e| RaisinError::storage(format!("Failed to set processing rules: {}", e)))?;

        tracing::debug!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            rule_count = rules.rules.len(),
            "Stored processing rules"
        );

        Ok(())
    }

    async fn delete_rules(&self, scope: RepoScope<'_>) -> Result<()> {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let cf = cf_handle(&self.db, cf::PROCESSING_RULES)?;
        let key = Self::build_key(tenant_id, repo_id);

        self.db.delete_cf(&cf, &key).map_err(|e| {
            RaisinError::storage(format!("Failed to delete processing rules: {}", e))
        })?;

        tracing::debug!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            "Deleted processing rules"
        );

        Ok(())
    }

    async fn get_rule(
        &self,
        scope: RepoScope<'_>,
        rule_id: &str,
    ) -> Result<Option<ProcessingRule>> {
        let rules = self.get_rules(scope).await?;
        Ok(rules.and_then(|r| r.get_rule(rule_id).cloned()))
    }

    async fn upsert_rule(&self, scope: RepoScope<'_>, rule: &ProcessingRule) -> Result<()> {
        let mut rules = self.get_rules(scope).await?.unwrap_or_default();

        // Check if rule exists
        if let Some(existing) = rules.get_rule_mut(&rule.id) {
            // Update existing rule
            *existing = rule.clone();
        } else {
            // Add new rule
            rules.add_rule(rule.clone());
        }

        self.set_rules(scope, &rules).await
    }

    async fn delete_rule(&self, scope: RepoScope<'_>, rule_id: &str) -> Result<()> {
        let mut rules = self.get_rules(scope).await?.unwrap_or_default();

        if rules.remove_rule(rule_id).is_some() {
            self.set_rules(scope, &rules).await?;
        }

        Ok(())
    }

    async fn reorder_rules(&self, scope: RepoScope<'_>, rule_ids: &[String]) -> Result<()> {
        let mut rules = self.get_rules(scope).await?.unwrap_or_default();

        rules.reorder(rule_ids);
        self.set_rules(scope, &rules).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_ai::{ProcessingSettings, RuleMatcher};
    use raisin_storage::scope::RepoScope;
    use tempfile::TempDir;

    fn create_test_db() -> (Arc<DB>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cfs = vec![CF_PROCESSING_RULES];
        let db = DB::open_cf(&opts, temp_dir.path(), cfs).unwrap();

        (Arc::new(db), temp_dir)
    }

    #[tokio::test]
    async fn test_get_set_rules() {
        let (db, _temp_dir) = create_test_db();
        let repo = ProcessingRulesRepositoryImpl::new(db);

        let scope = RepoScope { tenant_id: "tenant1", repo_id: "repo1" };

        // Initially no rules
        let rules = repo.get_rules(scope).await.unwrap();
        assert!(rules.is_none());

        // Set rules
        let mut rule_set = ProcessingRuleSet::new();
        rule_set.add_rule(
            ProcessingRule::new("test-rule", "Test Rule")
                .with_order(1)
                .with_matcher(RuleMatcher::All),
        );

        repo.set_rules(scope, &rule_set).await.unwrap();

        // Get rules back
        let rules = repo.get_rules(scope).await.unwrap().unwrap();
        assert_eq!(rules.rules.len(), 1);
        assert_eq!(rules.rules[0].id, "test-rule");
    }

    #[tokio::test]
    async fn test_upsert_rule() {
        let (db, _temp_dir) = create_test_db();
        let repo = ProcessingRulesRepositoryImpl::new(db);
        let scope = RepoScope { tenant_id: "tenant1", repo_id: "repo1" };

        // Add first rule
        let rule1 = ProcessingRule::new("rule1", "Rule 1")
            .with_order(1)
            .with_matcher(RuleMatcher::All);
        repo.upsert_rule(scope, &rule1).await.unwrap();

        // Add second rule
        let rule2 = ProcessingRule::new("rule2", "Rule 2")
            .with_order(2)
            .with_matcher(RuleMatcher::MimeType {
                mime_type: "application/pdf".to_string(),
            });
        repo.upsert_rule(scope, &rule2).await.unwrap();

        // Verify both rules exist
        let rules = repo.get_rules(scope).await.unwrap().unwrap();
        assert_eq!(rules.rules.len(), 2);

        // Update rule1
        let rule1_updated = ProcessingRule::new("rule1", "Rule 1 Updated")
            .with_order(10)
            .with_matcher(RuleMatcher::All);
        repo.upsert_rule(scope, &rule1_updated)
            .await
            .unwrap();

        // Verify update
        let rules = repo.get_rules(scope).await.unwrap().unwrap();
        assert_eq!(rules.rules.len(), 2);
        let rule = rules.get_rule("rule1").unwrap();
        assert_eq!(rule.name, "Rule 1 Updated");
    }

    #[tokio::test]
    async fn test_delete_rule() {
        let (db, _temp_dir) = create_test_db();
        let repo = ProcessingRulesRepositoryImpl::new(db);
        let scope = RepoScope { tenant_id: "tenant1", repo_id: "repo1" };

        // Add rules
        let rule1 = ProcessingRule::new("rule1", "Rule 1");
        let rule2 = ProcessingRule::new("rule2", "Rule 2");
        repo.upsert_rule(scope, &rule1).await.unwrap();
        repo.upsert_rule(scope, &rule2).await.unwrap();

        // Delete one rule
        repo.delete_rule(scope, "rule1").await.unwrap();

        // Verify only rule2 remains
        let rules = repo.get_rules(scope).await.unwrap().unwrap();
        assert_eq!(rules.rules.len(), 1);
        assert_eq!(rules.rules[0].id, "rule2");
    }

    #[tokio::test]
    async fn test_reorder_rules() {
        let (db, _temp_dir) = create_test_db();
        let repo = ProcessingRulesRepositoryImpl::new(db);
        let scope = RepoScope { tenant_id: "tenant1", repo_id: "repo1" };

        // Add rules in order 1, 2, 3
        for i in 1..=3 {
            let rule =
                ProcessingRule::new(format!("rule{}", i), format!("Rule {}", i)).with_order(i);
            repo.upsert_rule(scope, &rule).await.unwrap();
        }

        // Reorder to 3, 1, 2
        repo.reorder_rules(
            scope,
            &[
                "rule3".to_string(),
                "rule1".to_string(),
                "rule2".to_string(),
            ],
        )
        .await
        .unwrap();

        // Verify new order
        let rules = repo.get_rules(scope).await.unwrap().unwrap();
        assert_eq!(rules.rules[0].id, "rule3");
        assert_eq!(rules.rules[1].id, "rule1");
        assert_eq!(rules.rules[2].id, "rule2");
    }

    #[tokio::test]
    async fn test_chunking_roundtrip() {
        use raisin_ai::{ChunkingConfig, OverlapConfig, ProcessingSettings, SplitterType};
        let (db, _temp_dir) = create_test_db();
        let repo = ProcessingRulesRepositoryImpl::new(db);

        let chunking = ChunkingConfig {
            chunk_size: 512,
            overlap: OverlapConfig::Tokens(50),
            splitter: SplitterType::Recursive,
            tokenizer_id: None,
        };

        let settings = ProcessingSettings {
            chunking: Some(chunking),
            ..Default::default()
        };

        let rule = ProcessingRule::new("chunk-rule", "Chunk Rule").with_settings(settings);

        let scope = RepoScope { tenant_id: "tenant1", repo_id: "repo1" };
        repo.upsert_rule(scope, &rule).await.unwrap();

        let rules = repo.get_rules(scope).await.unwrap().unwrap();
        let loaded_rule = rules.get_rule("chunk-rule").unwrap();
        assert!(loaded_rule.settings.chunking.is_some());
        assert_eq!(
            loaded_rule.settings.chunking.as_ref().unwrap().chunk_size,
            512
        );
    }

    #[tokio::test]
    async fn test_optional_settings_roundtrip() {
        use raisin_ai::ProcessingSettings;
        let (db, _temp_dir) = create_test_db();
        let repo = ProcessingRulesRepositoryImpl::new(db);
        let scope = RepoScope { tenant_id: "tenant1", repo_id: "repo1" };

        let settings = ProcessingSettings {
            generate_image_embedding: Some(true),
            ..Default::default()
        };

        let rule = ProcessingRule::new("image-rule", "Image Rule").with_settings(settings);

        repo.upsert_rule(scope, &rule).await.unwrap();

        let rules = repo.get_rules(scope).await.unwrap().unwrap();
        let loaded_rule = rules.get_rule("image-rule").unwrap();
        assert_eq!(loaded_rule.settings.generate_image_embedding, Some(true));
    }
}
