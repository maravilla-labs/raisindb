// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Network policy and URL matching for RaisinFunctionApi

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    /// Check if URL is allowed by network policy.
    ///
    /// # The empty-list divergence (issue #13)
    ///
    /// `http_enabled: true` with an EMPTY `allowed_urls` is ALLOWED here and
    /// DENIED by
    /// [`NetworkPolicy::is_url_allowed`](crate::types::NetworkPolicy::is_url_allowed),
    /// the sibling gate on the sandbox path. The strict reading is the one we
    /// want everywhere, but flipping this side today would instantly deny every
    /// deployed function that has `http_enabled: true` and never listed a URL —
    /// unrestricted outbound is the behaviour those functions were written
    /// against. Unifying the DEFAULT is a deliberate follow-up needing a
    /// deprecation window; the WARN below is the first half of it, so the
    /// exposure is visible in logs before anything starts failing.
    ///
    /// The MATCHER is no longer part of the divergence: both sides now call
    /// [`crate::types::glob_match`], so `*` means the same thing in both.
    pub(crate) fn is_url_allowed(&self, url: &str) -> bool {
        tracing::trace!(
            url = url,
            http_enabled = self.network_policy.http_enabled,
            allowed_urls = ?self.network_policy.allowed_urls,
            "is_url_allowed - checking"
        );

        if !self.network_policy.http_enabled {
            tracing::trace!("is_url_allowed - BLOCKED: http_enabled is false");
            return false;
        }

        // If no allowed URLs specified, all are allowed (when http_enabled).
        // Kept for compatibility, warned about because it is an unbounded
        // outbound grant — see the issue #13 note above.
        if self.network_policy.allowed_urls.is_empty() {
            // ExecutionContext carries no function NAME, so the execution id
            // plus tenant/repo is the best handle a reader has for tracing this
            // back to the node that needs an allowed_urls list.
            tracing::warn!(
                url = url,
                tenant_id = %self.context.tenant_id,
                repo_id = %self.context.repo_id,
                execution_id = %self.context.execution_id,
                "network_policy has http_enabled: true with an EMPTY allowed_urls, so this \
                 function may call ANY host. This permissive default is deprecated (issue #13) \
                 and will become a denial; list the hosts in network_policy.allowed_urls."
            );
            return true;
        }

        // Check against allowlist with glob matching
        for pattern in &self.network_policy.allowed_urls {
            let matches = Self::glob_match(pattern, url);
            tracing::trace!(
                pattern = pattern,
                url = url,
                matches = matches,
                "is_url_allowed - pattern check"
            );
            if matches {
                tracing::trace!(pattern = pattern, "is_url_allowed - ALLOWED by pattern");
                return true;
            }
        }

        tracing::trace!("is_url_allowed - BLOCKED: no pattern matched");
        false
    }

    /// Glob matching for URL patterns.
    ///
    /// Supports:
    /// - `*` matches any characters except `/`
    /// - `**` matches any characters including `/`
    ///
    /// This used to be a second, hand-rolled implementation that disagreed with
    /// the `glob::Pattern` matcher the policy types used (issue #13). It is now
    /// a thin alias for [`crate::types::glob_match`] — kept as an associated
    /// function because the adapter tests and `imap`/`crypto` call sites address
    /// it that way, but there is exactly one matcher behind it.
    pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
        crate::types::glob_match(pattern, text)
    }
}
