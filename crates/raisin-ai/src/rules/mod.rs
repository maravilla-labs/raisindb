//! Content processing rules for AI-powered asset handling.
//!
//! This module defines the rule system that controls how content is processed
//! for embedding generation, PDF extraction, and image captioning. Rules are
//! matched in priority order (first-match-wins) and can target content by:
//!
//! - Node type (e.g., "raisin:Asset")
//! - Path pattern (glob syntax: "/docs/**")
//! - MIME type (e.g., "application/pdf")
//! - Combined matchers (AND logic)
//!
//! # Example
//!
//! ```rust
//! use raisin_ai::rules::{ProcessingRule, RuleMatcher, ProcessingSettings};
//!
//! // Rule for PDF documents in the /documents path
//! let pdf_rule = ProcessingRule {
//!     id: "pdf-docs".to_string(),
//!     name: "PDF Documents".to_string(),
//!     order: 1,
//!     enabled: true,
//!     matcher: RuleMatcher::Combined { matchers: vec![
//!         RuleMatcher::Path { pattern: "/documents/**".to_string() },
//!         RuleMatcher::MimeType { mime_type: "application/pdf".to_string() },
//!     ]},
//!     settings: ProcessingSettings::default(),
//! };
//! ```

mod matcher;
mod settings;
#[cfg(test)]
mod tests;

pub use matcher::{RuleMatchContext, RuleMatcher};
pub use settings::ProcessingSettings;

use serde::{Deserialize, Serialize};

// TODO: Verify if we can use REL as base format for this.

/// Default value for enabled field.
fn default_enabled() -> bool {
    true
}

/// A content processing rule that defines how nodes should be processed.
///
/// Rules are evaluated in order by the `order` field (lower = higher priority).
/// The first matching rule's settings are applied. Rules can be enabled/disabled
/// without deletion for easy testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingRule {
    /// Unique identifier for the rule.
    #[serde(default)]
    pub id: String,

    /// Human-readable name for the rule.
    #[serde(default)]
    pub name: String,

    /// Priority order for rule matching (lower = higher priority).
    /// UI allows drag-and-drop reordering.
    #[serde(default)]
    pub order: i32,

    /// Whether this rule is active.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Condition that must be satisfied for this rule to apply.
    #[serde(default)]
    pub matcher: RuleMatcher,

    /// Processing settings to apply when this rule matches.
    #[serde(default)]
    pub settings: ProcessingSettings,
}

impl ProcessingRule {
    /// Create a new rule with a given ID and name.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            order: 0,
            enabled: true,
            matcher: RuleMatcher::All,
            settings: ProcessingSettings::default(),
        }
    }

    /// Set the priority order.
    pub fn with_order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }

    /// Set the matcher.
    pub fn with_matcher(mut self, matcher: RuleMatcher) -> Self {
        self.matcher = matcher;
        self
    }

    /// Set the processing settings.
    pub fn with_settings(mut self, settings: ProcessingSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Check if this rule matches the given node context.
    pub fn matches(&self, context: &RuleMatchContext) -> bool {
        if !self.enabled {
            return false;
        }
        self.matcher.matches(context)
    }
}

/// A collection of processing rules for a repository.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessingRuleSet {
    /// Rules sorted by order (ascending).
    #[serde(default)]
    pub rules: Vec<ProcessingRule>,
}

impl ProcessingRuleSet {
    /// Create an empty rule set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a rule to the set.
    pub fn add_rule(&mut self, rule: ProcessingRule) {
        self.rules.push(rule);
        self.sort_rules();
    }

    /// Remove a rule by ID.
    pub fn remove_rule(&mut self, id: &str) -> Option<ProcessingRule> {
        if let Some(pos) = self.rules.iter().position(|r| r.id == id) {
            Some(self.rules.remove(pos))
        } else {
            None
        }
    }

    /// Get a rule by ID.
    pub fn get_rule(&self, id: &str) -> Option<&ProcessingRule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// Get a mutable reference to a rule by ID.
    pub fn get_rule_mut(&mut self, id: &str) -> Option<&mut ProcessingRule> {
        self.rules.iter_mut().find(|r| r.id == id)
    }

    /// Reorder rules by providing a new order of IDs.
    /// Rules not in the list keep their relative order at the end.
    pub fn reorder(&mut self, rule_ids: &[String]) {
        for (new_order, id) in rule_ids.iter().enumerate() {
            if let Some(rule) = self.get_rule_mut(id) {
                rule.order = new_order as i32;
            }
        }
        self.sort_rules();
    }

    /// Sort rules by order.
    fn sort_rules(&mut self) {
        self.rules.sort_by_key(|r| r.order);
    }

    /// Find the first matching rule for a given context.
    ///
    /// Returns the matching rule if found, following first-match-wins semantics.
    pub fn find_matching_rule(&self, context: &RuleMatchContext) -> Option<&ProcessingRule> {
        self.rules.iter().find(|r| r.matches(context))
    }

    /// Get settings for a given context.
    ///
    /// Returns the settings from the first matching rule, or default settings if none match.
    pub fn get_settings(&self, context: &RuleMatchContext) -> ProcessingSettings {
        self.find_matching_rule(context)
            .map(|r| r.settings.clone())
            .unwrap_or_default()
    }

    /// Create a default rule set with common patterns.
    pub fn default_rules() -> Self {
        let mut set = Self::new();

        set.add_rule(
            ProcessingRule::new("pdf-default", "PDF Documents")
                .with_order(10)
                .with_matcher(RuleMatcher::MimeType {
                    mime_type: "application/pdf".to_string(),
                })
                .with_settings(ProcessingSettings::pdf()),
        );

        set.add_rule(
            ProcessingRule::new("image-default", "Image Assets")
                .with_order(20)
                .with_matcher(RuleMatcher::Combined {
                    matchers: vec![
                        RuleMatcher::NodeType("raisin:Asset".to_string()),
                        RuleMatcher::MimeType {
                            mime_type: "image/".to_string(),
                        },
                    ],
                })
                .with_settings(ProcessingSettings::image()),
        );

        set.add_rule(
            ProcessingRule::new("default", "Default")
                .with_order(100)
                .with_matcher(RuleMatcher::All)
                .with_settings(ProcessingSettings::default()),
        );

        set
    }
}
