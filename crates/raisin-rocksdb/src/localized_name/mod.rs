//! The localized name index (plan Phase 12): localized URL lookup.
//!
//! `/produits/chaise` in `fr` resolves to the node at `/products/chair` with
//! one seek per segment, independent of workspace size. Built like the other
//! internal derived indexes: maintained INLINE in the write batch of every
//! funnel by one writer ([`sync`]), rebuilt by a native streaming repair
//! ([`rebuild`], `RepairKind::LocalizedNames`), gated by a fingerprinted
//! fail-closed state record ([`state`]) with an always-correct row-level
//! fallback ([`lookup`]). No functions, triggers or workflows involved.
//!
//! # Shape
//!
//! `cf::LOCALIZED_NAME_INDEX`, branch-scoped ([`keys`]): a FORWARD claim per
//! `(locale, parent_id, name, node)` and a REVERSE row per `(node, locale)`
//! naming the node's current `(parent_id, name)`. One segment per node per
//! parent, so moving or renaming an ancestor writes nothing; moving a node
//! rewrites its own rows only.
//!
//! # A name is a TRANSLATED node name (owner decision, 2026-10-04)
//!
//! A node's segment in a locale is the reserved overlay pointer
//! `/__node_name` of its translation there (SQL: the `__node_name` column,
//! written with `UPDATE … FOR LOCALE … SET __node_name = …`), or — without one
//! — its canonical name. A `Hidden` overlay hides the node there. The default
//! language never has localized names, and no base-node property is ever a
//! source. One selector: `indexing::localized_node_names`.
//!
//! # A claim is a hint, the selector decides
//!
//! The lookup re-derives every candidate's segment through THE selector
//! (`indexing::localized_node_names`) at the read revision before answering, and
//! checks the node is live there. So a stale claim (an out-of-order apply, a
//! delete that wrote no tombstones, a checkpoint from an older peer, a
//! collision) can never be SERVED; the index only has to be COMPLETE, which
//! the inline writers and the rebuild guarantee from `built_from_rev` on.
//!
//! # On by default (owner decision, 2026-10-04)
//!
//! The index is maintained for every repository, and each branch's initial
//! build is queued automatically in the background ([`auto`]: after the job
//! system starts — every link queues the next pending branch — and whenever a
//! lookup finds a branch not built: a fork, a configuration change, a
//! checkpoint ingest, a merge from an unbuilt source). Until a branch's build is
//! `Ready` under the current configuration, lookups take the fallback.
//! `RAISIN_LOCALIZED_NAME_INDEX=0` (`false`/`off`/`no`) switches the index
//! off: writers stop, builds stop, every lookup falls back, and at start every
//! state record is reset to `NotBuilt` so switching it back on rebuilds.

pub mod auto;
mod availability;
pub(crate) mod catch_up;
pub mod config;
pub mod config_change;
pub mod keys;
pub mod lookup;
pub(crate) mod plan;
mod reads;
pub mod rebuild;
pub mod rows;
pub mod source;
pub mod state;
pub(crate) mod sync;
pub mod unique;

pub use config::NameConfig;
pub use keys::NameScope;
pub use lookup::{LocalizedLookup, Resolution};
pub use state::{Availability, BuildStatus, LocalizedNameState};

use std::sync::OnceLock;

/// The environment switch (default ON).
pub const LOCALIZED_NAME_INDEX_ENV: &str = "RAISIN_LOCALIZED_NAME_INDEX";

/// Whether the index is maintained and used in this process (read once).
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var(LOCALIZED_NAME_INDEX_ENV)
            .map(|v| {
                !matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "off" | "no"
                )
            })
            .unwrap_or(true)
    })
}
