// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Processing rules repository trait definition.
//!
//! This module contains the `ProcessingRulesRepository` trait which controls
//! how content is handled for embedding generation, PDF text extraction,
//! image captioning, etc.

use raisin_error::Result;

use crate::scope::RepoScope;

/// Repository for AI processing rules per repository.
///
/// Processing rules control how content is handled for embedding generation,
/// PDF text extraction, image captioning, etc. Rules are stored at the
/// repository level and evaluated in priority order (first-match-wins).
///
/// All methods take a `RepoScope` (tenant + repo).
pub trait ProcessingRulesRepository: Send + Sync {
    /// Get the processing rules for a repository.
    ///
    /// # Arguments
    /// * `scope` - Repository scope (tenant + repo)
    ///
    /// # Returns
    /// The processing rule set for the repository, or None if not configured.
    fn get_rules(
        &self,
        scope: RepoScope<'_>,
    ) -> impl std::future::Future<Output = Result<Option<raisin_ai::ProcessingRuleSet>>> + Send;

    /// Set the processing rules for a repository.
    ///
    /// # Arguments
    /// * `scope` - Repository scope (tenant + repo)
    /// * `rules` - The processing rule set to store
    fn set_rules(
        &self,
        scope: RepoScope<'_>,
        rules: &raisin_ai::ProcessingRuleSet,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Delete processing rules for a repository.
    ///
    /// # Arguments
    /// * `scope` - Repository scope (tenant + repo)
    fn delete_rules(
        &self,
        scope: RepoScope<'_>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Get a single rule by ID.
    ///
    /// # Arguments
    /// * `scope` - Repository scope (tenant + repo)
    /// * `rule_id` - Rule identifier
    ///
    /// # Returns
    /// The rule if found, or None.
    fn get_rule(
        &self,
        scope: RepoScope<'_>,
        rule_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<raisin_ai::ProcessingRule>>> + Send;

    /// Add or update a single rule.
    ///
    /// # Arguments
    /// * `scope` - Repository scope (tenant + repo)
    /// * `rule` - The rule to upsert
    fn upsert_rule(
        &self,
        scope: RepoScope<'_>,
        rule: &raisin_ai::ProcessingRule,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Delete a single rule by ID.
    ///
    /// # Arguments
    /// * `scope` - Repository scope (tenant + repo)
    /// * `rule_id` - Rule identifier
    fn delete_rule(
        &self,
        scope: RepoScope<'_>,
        rule_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Reorder rules by providing a new order of IDs.
    ///
    /// # Arguments
    /// * `scope` - Repository scope (tenant + repo)
    /// * `rule_ids` - List of rule IDs in the desired order
    fn reorder_rules(
        &self,
        scope: RepoScope<'_>,
        rule_ids: &[String],
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}
