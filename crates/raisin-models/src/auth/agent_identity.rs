// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! The namespaced vocabulary for [`AuthContext::agent`](super::AuthContext::agent)
//! — the non-human principal that *initiated* a write.
//!
//! # Grammar
//!
//! ```text
//! <kind>:<id>[@<origin-kind>:<origin-id>]
//! ```
//!
//! | Case                                   | Value                                                      |
//! |----------------------------------------|------------------------------------------------------------|
//! | MCP tool call                          | `mcp:studio-admin`                                          |
//! | AI agent, invoked directly             | `agent:/agents/triage-bot`                                  |
//! | AI agent fired by a trigger            | `agent:/agents/triage-bot@trigger:/triggers/on-order-created`|
//! | Standalone trigger → plain function    | `trigger:/triggers/sweep-expired-holds`                     |
//! | Inline trigger (no trigger node exists)| `trigger:fn:/lib/studio/on-order-created`                   |
//! | Flow instance write                    | `flow:/flows/publish-approval`                              |
//! | One-shot scheduled invocation          | `schedule:/lib/studio/send-digest`                          |
//! | Plain system job                       | `job:replication-gc`                                        |
//!
//! # Why this shape
//!
//! * **Kind first** so the value is a prefix scan: `LIKE 'agent:%'` and
//!   `LIKE 'trigger:/triggers/x%'` are the only efficient query form in this
//!   engine. Putting the origin first would destroy that.
//! * **Node paths, not instance ids**, because paths replicate and are stable
//!   across the cluster while nanoid instance ids are neither — and because
//!   bounded cardinality keeps "group by agent" meaningful. Per-run identity
//!   already lives in the job and flow-instance records.
//! * **One `@`, at most.** A chain deeper than two hops records the nearest
//!   actor and its immediate origin; `@` cannot occur in a node path, so the
//!   split is unambiguous. [`with_origin`] enforces the bound by truncating an
//!   already-composed origin to its head.
//! * **One string, no schema change**: `AuthContext.agent` and `AuditLog.agent`
//!   are `Option<String>`, map-encoded everywhere, so nothing here perturbs a
//!   persisted layout.
//!
//! `mcp:<slug>` predates this module and is unchanged — it is the pattern the
//! rest of the vocabulary was generalized from.

/// An AI agent identified by its node path.
pub fn agent(agent_path: &str) -> String {
    format!("agent:{}", agent_path)
}

/// A standalone `raisin:Trigger` node, identified by its path.
pub fn trigger(trigger_path: &str) -> String {
    format!("trigger:{}", trigger_path)
}

/// A trigger declared inline on a function node. Such triggers have no node of
/// their own, so the function they live on is their only stable identity.
pub fn inline_trigger(function_path: &str) -> String {
    format!("trigger:fn:{}", function_path)
}

/// A flow definition, identified by its path.
pub fn flow(flow_path: &str) -> String {
    format!("flow:{}", flow_path)
}

/// A scheduled (cron / one-shot) invocation of the given target path.
pub fn schedule(target_path: &str) -> String {
    format!("schedule:{}", target_path)
}

/// A plain internal system job, identified by a stable job name.
pub fn job(job_name: &str) -> String {
    format!("job:{}", job_name)
}

/// Compose `actor` with the origin that caused it: `"<actor>@<origin-head>"`.
///
/// If `origin` is itself composed, only its head is kept, so the result always
/// contains exactly one `@` — the nearest actor plus its immediate origin.
/// Returns `actor` unchanged when `origin` is `None` or blank.
pub fn with_origin(actor: String, origin: Option<&str>) -> String {
    match origin.map(str::trim).filter(|o| !o.is_empty()) {
        Some(origin) => {
            let head = origin.split('@').next().unwrap_or(origin);
            format!("{}@{}", actor, head)
        }
        None => actor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_render_as_documented() {
        assert_eq!(agent("/agents/triage-bot"), "agent:/agents/triage-bot");
        assert_eq!(trigger("/triggers/on-order"), "trigger:/triggers/on-order");
        assert_eq!(inline_trigger("/lib/x"), "trigger:fn:/lib/x");
        assert_eq!(flow("/flows/publish"), "flow:/flows/publish");
        assert_eq!(schedule("/lib/digest"), "schedule:/lib/digest");
        assert_eq!(job("replication-gc"), "job:replication-gc");
    }

    #[test]
    fn origin_is_appended_once() {
        assert_eq!(
            with_origin(agent("/agents/bot"), Some("trigger:/triggers/t")),
            "agent:/agents/bot@trigger:/triggers/t"
        );
    }

    #[test]
    fn an_already_composed_origin_is_truncated_to_its_head() {
        // Two hops is the documented bound: keep the nearest origin, drop the
        // rest of the chain (it is recoverable from the job/flow records).
        let composed = with_origin(
            agent("/agents/bot"),
            Some("flow:/flows/f@trigger:/triggers/t"),
        );
        assert_eq!(composed, "agent:/agents/bot@flow:/flows/f");
        assert_eq!(composed.matches('@').count(), 1);
    }

    #[test]
    fn a_missing_or_blank_origin_leaves_the_actor_alone() {
        assert_eq!(with_origin(agent("/a"), None), "agent:/a");
        assert_eq!(with_origin(agent("/a"), Some("   ")), "agent:/a");
    }

    #[test]
    fn kind_stays_the_leftmost_token_so_prefix_scans_work() {
        let v = with_origin(agent("/agents/bot"), Some("trigger:/triggers/t"));
        assert!(v.starts_with("agent:"), "LIKE 'agent:%' must still match");
    }
}
