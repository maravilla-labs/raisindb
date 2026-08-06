//! The `raisin:VirtualMount` node: its `sync_config`, `write_config`, and the
//! parsed mount as a whole.

use raisin_models::nodes::Node;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::paths::normalize_path;
use super::props::{
    opt_bool, opt_string, parse_object, parse_object_checked, raw_value, req_string,
};
use super::state::MountState;
use super::{
    DEFAULT_BATCH_MAX_BYTES, DEFAULT_BATCH_SIZE, DEFAULT_INTERVAL_SECS, DEFAULT_MAX_ITEMS,
    DEFAULT_MAX_ITEM_FAILURES, DEGRADE_THRESHOLD,
};

/// Parsed `sync_config` object of a `raisin:VirtualMount`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// `"poll" | "webhook" | "hybrid"`. `webhook` mounts are skipped by the
    /// periodic driver (they are driven by inbound webhooks instead).
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
    #[serde(default)]
    pub include_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    #[serde(default = "default_max_items")]
    pub max_items_per_sync: u64,
    /// Items written per transaction during an import.
    ///
    /// One batch is one revision, one branch-HEAD bump, one RocksDB write, one
    /// snapshot job and one replication record — instead of one of each per item,
    /// which is what made importing a real mailbox unusably slow.
    ///
    /// A batch also flushes when `batch_max_bytes` is reached, so this is an
    /// upper bound rather than the actual size; see [`DEFAULT_BATCH_MAX_BYTES`]
    /// for why the byte budget is the one that must not be removed.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// Byte budget for one batch (see [`DEFAULT_BATCH_MAX_BYTES`]).
    #[serde(default = "default_batch_max_bytes")]
    pub batch_max_bytes: usize,
    /// Folder hierarchy for materialized items, as a template over the item's
    /// fields — e.g. `"{conversation_id}/{name}"` groups mail into one folder
    /// per thread, `"{date:%Y}/{date:%m}/{name}"` into year/month.
    ///
    /// Empty (the default) keeps the flat provider-derived path, which for mail
    /// meant every message landing directly under the mount path named after its
    /// opaque provider id — unusable in a tree once a mailbox is real-sized.
    ///
    /// Placeholders resolve against the item's `metadata` object, plus the
    /// built-ins `{name}` and `{external_id}`. `{key:FMT}` formats an ISO-8601
    /// metadata value with a strftime pattern. An unresolvable placeholder falls
    /// back to the flat path rather than producing a half-built one.
    #[serde(default)]
    pub path_template: String,
    /// Allow a full reconcile to delete the ENTIRE mount subtree when the
    /// provider returns zero items.
    ///
    /// Off by default. An empty listing is ambiguous: it means either "the
    /// folder really is empty now" or "something went wrong in a way that did
    /// not raise an error" — a permissions change, a silently-failing filter, a
    /// provider hiccup. Acting on the first reading destroys every synced node;
    /// acting on the second leaves some stale ones, which the next good sync
    /// cleans up. Deleted content is not recoverable, stale content is, so the
    /// safe reading is the default.
    ///
    /// Turn it on for a mount whose source folder is legitimately emptied and
    /// where that must propagate.
    #[serde(default)]
    pub allow_empty_reconcile: bool,
    /// How many item rejections to tolerate before giving up the run, while
    /// nothing at all has been written.
    ///
    /// A mount whose target workspace forbids the node type its mapper produces
    /// rejects EVERY item. Counting those to the end of the budget burned the
    /// full wall-clock timeout and reported `OK` with a failure tally beside it;
    /// the operator saw a healthy-looking mount that could never work. Stopping
    /// early turns it into a `misconfigured` status carrying the provider's own
    /// rejection message.
    #[serde(default = "default_max_item_failures")]
    pub max_item_failures: usize,
}

fn default_max_item_failures() -> usize {
    DEFAULT_MAX_ITEM_FAILURES
}

fn default_mode() -> String {
    "poll".to_string()
}
fn default_interval() -> u64 {
    DEFAULT_INTERVAL_SECS
}
fn default_max_items() -> u64 {
    DEFAULT_MAX_ITEMS
}
fn default_batch_size() -> usize {
    DEFAULT_BATCH_SIZE
}
fn default_batch_max_bytes() -> usize {
    DEFAULT_BATCH_MAX_BYTES
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            interval_seconds: default_interval(),
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            ephemeral: false,
            ttl_seconds: None,
            max_items_per_sync: default_max_items(),
            batch_size: default_batch_size(),
            batch_max_bytes: default_batch_max_bytes(),
            path_template: String::new(),
            allow_empty_reconcile: false,
            max_item_failures: default_max_item_failures(),
        }
    }
}

/// Parsed `write_config` object of a `raisin:VirtualMount`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteConfig {
    /// `"off" | "write_through"`. The older, mode-less switch: it asks for a
    /// full `mirror`, which is not implemented yet.
    #[serde(default = "default_writeback")]
    pub writeback: String,
    /// Which write mode this mount drives: `"off" | "state_only"`.
    ///
    /// A property of the MOUNT, not of the adapter — the same IMAP adapter
    /// serves a `state_only` inbox and (later) a `submit` outbox. `mirror` and
    /// `submit` are declared in the docs but not implemented here yet, and an
    /// unrecognized value is a typo: a mount is never more writable than the
    /// engine understands it to be, and both cases are REFUSED out loud rather
    /// than quietly demoted to `off` (see [`Self::unsupported_mode_reason`]).
    #[serde(default = "default_write_mode")]
    pub mode: String,
    /// The `state_only` allow-list: node properties a local edit may push.
    ///
    /// Empty means nothing is pushable, so writeback stays off for every mount
    /// that has not opted in field by field. There is no "all fields" value on
    /// purpose — `state_only` exists precisely because most of the record
    /// (a mail body, its sender, its date) is immutable at the provider.
    #[serde(default)]
    pub mutable_fields: Vec<String>,
    /// `"remote_wins" | "error"`.
    #[serde(default)]
    pub conflict: Option<String>,
}

fn default_writeback() -> String {
    "off".to_string()
}

fn default_write_mode() -> String {
    "off".to_string()
}

impl Default for WriteConfig {
    fn default() -> Self {
        Self {
            writeback: default_writeback(),
            mode: default_write_mode(),
            mutable_fields: Vec::new(),
            conflict: None,
        }
    }
}

impl WriteConfig {
    /// Whether this mount asks for write-through writeback (a `mirror`).
    pub fn wants_write_through(&self) -> bool {
        self.writeback.eq_ignore_ascii_case("write_through")
    }

    /// Whether this mount asks for `state_only` writes.
    pub fn wants_state_only(&self) -> bool {
        self.mode.eq_ignore_ascii_case("state_only")
    }

    /// Why this mount's `mode` cannot be honoured, when it is neither `off` nor
    /// the one mode the engine implements.
    ///
    /// `None` for `off`/absent (an operator's choice, and silent) and for
    /// `state_only` (which the caller resolves properly, against the adapter).
    /// Everything else answers here: `mirror` and `submit` are documented in
    /// `docs/reference/virtual-node-adapters.md` §10.2, so an operator can
    /// configure one by following the docs — and used to get a mount that did
    /// nothing at all, with `writeback_supported` absent and not one line in the
    /// log. A typo behaved identically. Both are now refusals with a reason,
    /// which is the same channel a `state_only` mount's shortfall arrives on.
    pub fn unsupported_mode_reason(&self) -> Option<String> {
        let mode = self.mode.trim();
        if mode.is_empty() || mode.eq_ignore_ascii_case("off") || self.wants_state_only() {
            return None;
        }
        if mode.eq_ignore_ascii_case("mirror") || mode.eq_ignore_ascii_case("submit") {
            return Some(format!("write mode '{mode}' is not implemented yet"));
        }
        Some(format!(
            "write mode '{mode}' is not recognized; expected 'off' or 'state_only'"
        ))
    }

    /// The fields this mount declares pushable, empty unless it is
    /// `state_only`. Reading them under any other mode would let a `mode: off`
    /// mount push the moment an adapter declared `can_update`.
    pub fn declared_mutable_fields(&self) -> &[String] {
        if self.wants_state_only() {
            &self.mutable_fields
        } else {
            &[]
        }
    }
}

/// A fully-parsed `raisin:VirtualMount` node.
#[derive(Debug, Clone)]
pub struct MountConfig {
    pub mount_id: String,
    pub integration_ref: String,
    pub account_ref: Option<String>,
    pub target_workspace: String,
    /// Branch the mount materializes into (the mount config itself always lives
    /// on the repo's config branch). Defaults to `"main"` when unset.
    pub target_branch: String,
    pub mount_path: String,
    pub remote_root: Option<String>,
    pub adapter_function: Option<String>,
    pub mapping_function: Option<String>,
    pub enabled: bool,
    pub sync_config: SyncConfig,
    /// The original `sync_config` object as authored on the node, retained
    /// verbatim. The typed [`SyncConfig`] above only captures the fields the
    /// engine itself acts on; adapters need the full object (host/port/tls/
    /// mailbox/username/auth and any provider-specific keys), so the raw value
    /// is what is forwarded to them in the mount snapshot.
    pub sync_config_raw: Value,
    pub write_config: WriteConfig,
    pub state: MountState,
}

impl MountConfig {
    /// Parse a `raisin:VirtualMount` node. Returns an error only when a
    /// required field is missing or malformed.
    pub fn from_node(node: &Node) -> Result<Self, String> {
        let target_workspace = req_string(node, "target_workspace")?;
        let mount_path = req_string(node, "mount_path")?;
        let integration_ref = req_string(node, "integration_ref")?;
        Ok(Self {
            mount_id: node.id.clone(),
            integration_ref,
            account_ref: opt_string(node, "account_ref"),
            target_workspace,
            target_branch: opt_string(node, "target_branch")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "main".to_string()),
            mount_path: normalize_path(&mount_path),
            remote_root: opt_string(node, "remote_root"),
            adapter_function: opt_string(node, "adapter_function"),
            mapping_function: opt_string(node, "mapping_function"),
            enabled: opt_bool(node, "enabled").unwrap_or(true),
            sync_config: parse_object(node, "sync_config").unwrap_or_default(),
            sync_config_raw: raw_value(node, "sync_config"),
            // Also NOT `unwrap_or_default()`. The default is writeback OFF, so a
            // corrupt `write_config` silently disables the push an operator
            // explicitly configured: no error, no `writeback_last_error`, no log
            // line — edits just accumulate locally and never reach the provider.
            // Absent is fine (a read-only mount); unparseable is fatal.
            write_config: parse_object_checked(node, "write_config")?.unwrap_or_default(),
            // NOT `unwrap_or_default()`. A corrupt `state` blob defaulting
            // silently is a mount that forgets its cursor, forgets that its
            // backfill finished, and resets `state_seq` to 0 — after which
            // every state write it makes is refused, forever, with the console
            // still showing it healthy. Absent is fine; unparseable is fatal.
            state: parse_object_checked(node, "state")?.unwrap_or_default(),
        })
    }

    /// Effective poll interval honoring failure backoff:
    /// `interval * 2^min(consecutive_failures, 5)`.
    pub fn effective_interval_secs(&self) -> u64 {
        let base = self.sync_config.interval_seconds.max(1);
        let shift = self.state.consecutive_failures.min(DEGRADE_THRESHOLD);
        base.saturating_mul(1u64 << shift)
    }
}
