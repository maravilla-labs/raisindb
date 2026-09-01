//! What THIS process can actually do, injected from above at startup.
//!
//! # Why a registry rather than a call
//!
//! [`super::tasks::plan_tasks`] takes availability as a predicate because the
//! crate that knows the answer sits ABOVE this one: plugins are registered in
//! `raisin-functions`, which depends on `raisin-rocksdb`, which depends on
//! `raisin-ai`. Cargo rejects package cycles regardless of features, so
//! `raisin-ai` can never ask `plugin_method_available` directly.
//!
//! Injection alone was not enough, though, because the one caller that matters
//! could not answer either. The `AssetProcessing` enqueue path lives in
//! `raisin-rocksdb` — also below `raisin-functions` — and so passed `|_| false`,
//! hard-coded. That is honest in a stock build and WRONG in a build that loaded
//! a plugin: every delegated task was reported `plugin_missing` on a server that
//! could perform it, so a `.docx` rule was permanently dead no matter what the
//! operator configured.
//!
//! This module is the inversion that fixes it, and it is the pattern this
//! server already uses twice — `configure_mcp_client` and
//! `configure_platform_hooks`, both installed process-wide from `main.rs` so
//! every path shares one policy. `main.rs` installs a probe backed by the plugin
//! registry after loading plugins; everything below can then ask.
//!
//! # It holds a closure, not a snapshot
//!
//! The probe is invoked per question, so it reflects the registry as it stands
//! rather than as it stood at install time. Plugins are registered at startup
//! and never after, so the distinction is currently theoretical — but a snapshot
//! taken one line too early would fail silently and look exactly like no plugin,
//! which is the failure this module exists to end.
//!
//! # A build that installs nothing behaves as before
//!
//! [`capability_available`] answers `false` until a probe is installed. The
//! public server installs one that reports its own (usually empty) plugin set,
//! so stock behaviour is unchanged and no caller needs a `cfg`.

use std::sync::{Arc, OnceLock};

/// Answers "does this process service `<ns>.<method>`?".
pub type CapabilityProbe = Arc<dyn Fn(&str) -> bool + Send + Sync>;

fn slot() -> &'static OnceLock<CapabilityProbe> {
    static PROBE: OnceLock<CapabilityProbe> = OnceLock::new();
    &PROBE
}

/// Install the process-wide capability probe. Call once at startup, AFTER
/// plugins are loaded and BEFORE any node is processed.
///
/// Returns `false` if a probe was already installed, in which case the existing
/// one is kept. A second install is a wiring mistake — two answers to "can this
/// server convert a document" is the drift this registry exists to prevent — so
/// the caller should log it rather than assume it took effect.
pub fn install_capability_probe(probe: CapabilityProbe) -> bool {
    slot().set(probe).is_ok()
}

/// True when a probe has been installed. Distinguishes "nothing can run here"
/// from "nobody has said yet", which are the same answer from
/// [`capability_available`] and very different diagnoses.
pub fn capability_probe_installed() -> bool {
    slot().get().is_some()
}

/// Does this process service `method` (e.g. `"media.doc.toMarkdown"`)?
///
/// `false` when no probe is installed — the same answer the hard-coded
/// `|_| false` gave, so an un-wired build degrades to the previous behaviour
/// rather than to an optimistic one that would plan work nothing performs.
pub fn capability_available(method: &str) -> bool {
    match slot().get() {
        Some(probe) => probe(method),
        None => false,
    }
}

/// [`super::tasks::plan_tasks`] against this process's installed capabilities.
///
/// The form every caller inside the engine should use. Passing an ad-hoc
/// predicate is still possible and is what the tests do, but a second
/// production answer to "what can run here" is exactly the mirrored path this
/// module replaced.
pub fn plan_tasks_here(tasks: &[String]) -> super::tasks::PipelinePlan {
    super::tasks::plan_tasks(tasks, capability_available)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The probe is a process-global `OnceLock`, so these assertions describe
    // the un-installed state and must not install one — a test that did would
    // race every other test in this binary and win only sometimes.
    #[test]
    fn absent_probe_answers_false_not_true() {
        if !capability_probe_installed() {
            assert!(!capability_available("media.doc.toMarkdown"));
        }
    }

    #[test]
    fn a_plugin_task_is_blocked_without_a_probe() {
        use super::super::tasks::BlockedReason;

        if capability_probe_installed() {
            return;
        }
        let plan = plan_tasks_here(&["doc_to_markdown".to_string()]);
        assert!(plan.runnable.is_empty());
        assert_eq!(
            plan.blocked[0].blocked,
            BlockedReason::PluginMissing {
                method: "media.doc.toMarkdown".to_string()
            }
        );
    }
}
