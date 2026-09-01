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
    /// Expire the NODES themselves — the mailbox pattern, where synced items
    /// older than [`Self::ttl_seconds`] are deleted from the workspace.
    ///
    /// Nothing to do with the file bytes: see [`Self::cache_content`] for those.
    /// The two are kept apart deliberately, because conflating them would mean
    /// enabling a cache also started deleting a tenant's documents.
    #[serde(default)]
    pub ephemeral: bool,
    /// How long a synced NODE survives, in seconds. Only with [`Self::ephemeral`].
    #[serde(default)]
    pub ttl_seconds: Option<u64>,

    /// May this deployment hold copies of the mount's file bytes?
    ///
    /// A mount syncs metadata only; the bytes behind a file are fetched when
    /// something needs them — to extract text, render a thumbnail, serve a
    /// preview. This is the switch that allows that at all, because fetching
    /// means pulling someone else's storage onto this disk and a mount can be
    /// far larger than the deployment wants to hold. Off, the mount stays
    /// metadata-only and its files are findable by name alone.
    ///
    /// Separate from [`Self::ephemeral`] on purpose. That one deletes NODES;
    /// this one governs BYTES, and a mount routinely wants one without the
    /// other — a drive whose documents should be searchable but never expire is
    /// `cache_content` without `ephemeral`.
    #[serde(default)]
    pub cache_content: bool,
    /// How long a cached copy of the bytes outlives its last use, in seconds.
    ///
    /// Long enough that the jobs which follow one another — extract, thumbnail,
    /// embed — all still find it, and that browsing does not pay a provider
    /// round-trip per click. Short enough that a large drive does not
    /// accumulate. `Some(0)` drops the bytes as soon as processing is done;
    /// `None` keeps them, which is what a small published mount wants.
    #[serde(default)]
    pub content_ttl_seconds: Option<u64>,
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
    /// Whether a full walk may DELETE mount-owned nodes it did not see.
    ///
    /// On by default, because for a file-shaped provider the walk is the whole
    /// truth: `list` enumerates everything, so "not seen" does mean "gone
    /// upstream", and without this a rename or a remote delete would leave the
    /// node behind forever.
    ///
    /// Turn it OFF for a mount whose `list` is NOT authoritative for its own
    /// content. IMAP is the case that forced this: its `opList` returns
    /// MAILBOXES only, by design — messages arrive through `get_changes` — so
    /// after a full walk `seen` holds folder ids and nothing else. It is not
    /// empty, so the empty-reconcile guard above does not fire; the walk is not
    /// truncated, resumed or stopped on an ordinary forced run; and the stale
    /// filter exempts only commands. Every message node was therefore staged
    /// for delete, and they did not come back: `get_changes` is `fetchSince`
    /// the highest uid already seen, so the next delta returns only NEW mail.
    /// One click of the console's "full" button — or a remap, or the
    /// CursorInvalid fallback — emptied the mailbox tree.
    ///
    /// This is a MOUNT setting rather than an adapter capability because it is
    /// a statement about this mount's layout, and the engine must be able to
    /// act on it without domain knowledge of what the provider's `list` covers.
    ///
    /// THE TRADE, stated plainly so nobody sets this expecting a free win: with
    /// it off, NOTHING on the walk path ever prunes this mount again. Upstream
    /// deletions can then only arrive through the delta path — and for the very
    /// adapter that forced this field they do not, because IMAP's
    /// `opGetChanges` emits `type: "created"` items only. So a message deleted
    /// in the mailbox stays in the tree until `ephemeral` / `ttl_seconds` ages
    /// it out, which is why the documented IMAP mount layout is a cache. Losing
    /// deletions of already-deleted mail is still far cheaper than deleting
    /// every live message on one click of the console's "full" button.
    #[serde(default = "default_reconcile_deletes")]
    pub reconcile_deletes: bool,
    /// Node types this mount treats as FOLDERS, beyond the engine's own
    /// `raisin:Folder`.
    ///
    /// A folder has no bytes, so it must never be mistaken for a node whose
    /// upload has not finished — that mistake defers its create forever and
    /// misfiles everything inside it. It is a mount setting rather than engine
    /// knowledge on purpose: a product with its own container type names it
    /// here, and that type's name never has to appear in this engine.
    #[serde(default)]
    pub folder_node_types: Vec<String>,
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

fn default_reconcile_deletes() -> bool {
    true
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
            cache_content: false,
            content_ttl_seconds: None,
            max_items_per_sync: default_max_items(),
            batch_size: default_batch_size(),
            batch_max_bytes: default_batch_max_bytes(),
            path_template: String::new(),
            folder_node_types: Vec::new(),
            allow_empty_reconcile: false,
            reconcile_deletes: default_reconcile_deletes(),
            max_item_failures: default_max_item_failures(),
        }
    }
}

/// Parsed `write_config` object of a `raisin:VirtualMount`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteConfig {
    /// `"off" | "write_through"`. The older, mode-less switch: a deprecated
    /// spelling of `mode: "mirror"`, resolved as one (see
    /// [`Self::wants_mirror`]) rather than as a mode of its own.
    #[serde(default = "default_writeback")]
    pub writeback: String,
    /// Which write mode this mount drives:
    /// `"off" | "state_only" | "mirror" | "submit"`.
    ///
    /// A property of the MOUNT, not of the adapter — the same Graph adapter
    /// serves a `state_only` inbox and a `submit` outbox. An unrecognized value
    /// is a typo: a mount is never more writable than the engine understands it
    /// to be, so it is REFUSED out loud rather than quietly demoted to `off`
    /// (see [`Self::unsupported_mode_reason`]).
    #[serde(default = "default_write_mode")]
    pub mode: String,
    /// The update allow-list: node properties a local edit may push.
    ///
    /// Read by `state_only` and `mirror` alike. Empty means nothing is pushable,
    /// so writeback stays off for every mount that has not opted in field by
    /// field. There is no "all fields" value on purpose — the engine is
    /// domain-blind and cannot know which node properties a provider accepts as
    /// writes, and `state_only` exists precisely because most of the record (a
    /// mail body, its sender, its date) is immutable at the provider.
    #[serde(default)]
    pub mutable_fields: Vec<String>,
    /// `"remote_wins" | "local_wins" | "error" | "resolver_function"`.
    ///
    /// Absent means `remote_wins` — abandon the push and let the next delta
    /// overwrite the local value — unless the mount sets a `resolver_function`,
    /// which is unambiguous on its own. Resolution and the refusal rules live in
    /// [`write::conflict`](crate::jobs::handlers::virtual_mount_sync). An
    /// unrecognized value is REFUSED, never silently demoted: `local_wins`
    /// overwrites another writer irreversibly, so a typo must not be able to
    /// resolve to it *or* away from it.
    #[serde(default)]
    pub conflict: Option<String>,

    // ---- mirror: what a local delete or move means remotely (§8) ----
    //
    // All `Option`, all absent by default, because absent means "take the
    // adapter's recommendation" and an empty string is not the same answer as a
    // chosen policy. Resolution and the refusal rules live in `write::policy`.
    /// `"detach" | "trash" | "purge"`. Overrides the adapter's
    /// `default_delete_policy`. `purge` is never a default at any layer.
    #[serde(default)]
    pub delete_policy: Option<String>,
    /// `"push" | "detach" | "reject"`. A move is an `update` carrying the new
    /// parent/folder field — there is no `move` operation.
    #[serde(default)]
    pub move_policy: Option<String>,

    // ---- blast-radius rails (§9) ----
    /// Absolute floor for how many deletes one drain may push.
    ///
    /// A FLOOR, not a cap: the effective allowance is
    /// `max(this, max_delete_ratio × mount size)`, so a large mount is governed
    /// by the ratio and a tiny one by this. See `write::guard`.
    #[serde(default)]
    pub max_deletes_per_run: Option<usize>,
    /// Proportional allowance, as a fraction of the mount's live node count.
    #[serde(default)]
    pub max_delete_ratio: Option<f64>,
    /// Require an explicit confirmation for a delete whose originating
    /// transaction was wide enough to be a bulk operation, regardless of how
    /// many deletes reach any one drain. Defaults to `true`.
    #[serde(default)]
    pub require_confirmation_on_bulk: Option<bool>,

    // ---- submit: which nodes under the mount path are COMMANDS (§5) ----
    /// Node types this outbox treats as commands.
    ///
    /// The `submit` drain's provenance rule, and it has no alternative: a
    /// command is authored by a USER and carries no `__mount_id` /
    /// `__external_id` until the drain stamps one on it, so the ownership check
    /// every other write path makes cannot be made here. Without a type filter
    /// the drain claims any node under the outbox path whose `status` happens to
    /// read `queued` — a task, a draft record, an agent's scratch node — writes
    /// engine properties onto it, and with a permissive `to_external` sends it.
    ///
    /// Empty means [`DEFAULT_COMMAND_NODE_TYPES`]: the command types the engine
    /// ships. A mount whose outbox holds its own type declares it here.
    #[serde(default)]
    pub command_node_types: Vec<String>,

    // ---- mirror: which locally-authored nodes may be CREATED remotely ----
    /// Node types this mount may create at the provider.
    ///
    /// The same provenance problem as [`Self::command_node_types`], and the same
    /// answer. A node the user has just authored under the mount path carries no
    /// `__mount_id` and no `__external_id` — that is exactly what makes it a
    /// create candidate — so the ownership check every other write path makes is
    /// unavailable here, and a type filter is the only thing standing between
    /// "the user added an event to the calendar mount" and "the drain uploaded
    /// the folder note somebody left under it".
    ///
    /// **Empty means no local create**, which is why there is no default list.
    /// Unlike a command type, which is unambiguously an instruction to act, an
    /// ordinary content node under a mount path is ambiguous by nature: the read
    /// path deliberately tolerates non-mount content there
    /// (`SyncIndex.by_path`). Guessing wrong pushes private content to a third
    /// party, which is not recoverable by fixing the config afterwards. So a
    /// mount that wants local create says so, type by type, in the same spirit
    /// as `mutable_fields` having no "all fields" value.
    #[serde(default)]
    pub create_node_types: Vec<String>,
}

/// The command types RaisinDB ships, used when a `submit` mount declares none.
///
/// Both are engine-defined (`nodetype_init.rs`): `status` defaults to `draft`,
/// `action` is required, and both carry the `raisin:VirtualNode` mixin that
/// declares the `__write_seq` counter the at-most-once claim is written into.
pub const DEFAULT_COMMAND_NODE_TYPES: &[&str] = &["raisin:OutboundMail", "raisin:CalendarAction"];

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
            delete_policy: None,
            move_policy: None,
            max_deletes_per_run: None,
            max_delete_ratio: None,
            require_confirmation_on_bulk: None,
            command_node_types: Vec::new(),
            create_node_types: Vec::new(),
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

    /// Whether this mount is an OUTBOX: its members are commands, and a
    /// `draft -> queued` transition issues one.
    ///
    /// Nothing about a `submit` mount is a mirror of a remote object, which is
    /// why it reads none of the fields the other two modes share — no
    /// `mutable_fields`, no `delete_policy`, no `move_policy`, no conflict
    /// policy. A command has a status lifecycle and a terminal state, and that
    /// is the whole of it.
    pub fn wants_submit(&self) -> bool {
        self.mode.eq_ignore_ascii_case("submit")
    }

    /// Whether this mount asks for a `mirror`: the node **is** the remote
    /// object, so updates and deletes propagate.
    ///
    /// The deprecated `writeback: "write_through"` switch means exactly this
    /// (§8), so it resolves here rather than through a second path — one mode
    /// with two spellings, not two modes. An explicit `mode` wins over the alias
    /// so a mount that says `mode: "state_only"` is not promoted to a mirror by
    /// a leftover `writeback` value.
    pub fn wants_mirror(&self) -> bool {
        if self.mode.eq_ignore_ascii_case("mirror") {
            return true;
        }
        let mode = self.mode.trim();
        (mode.is_empty() || mode.eq_ignore_ascii_case("off")) && self.wants_write_through()
    }

    /// Whether this mount asks for outbound writes of ANY kind.
    ///
    /// Deliberately wider than [`Self::wants_state_only`], and the difference
    /// matters for change DETECTION. The reconcile walk is what makes a local
    /// delete visible at all, and a delete becomes unrecoverable once the MVCC
    /// pre-image behind it is collected — so a mount whose pushes the engine
    /// cannot perform (today: `submit`, or any mode whose adapter falls short)
    /// must still have its watermark advanced and its deletes recorded. Gating
    /// detection on what the engine can currently PUSH would mean the first
    /// mount to gain a mode has no history to act on.
    ///
    /// A mode the engine does not recognize at all is a typo, and answers
    /// `false`: nothing about it says the operator wants writes.
    pub fn wants_any_write(&self) -> bool {
        let mode = self.mode.trim();
        self.wants_write_through()
            || mode.eq_ignore_ascii_case("state_only")
            || mode.eq_ignore_ascii_case("mirror")
            || mode.eq_ignore_ascii_case("submit")
    }

    /// Why this mount's `mode` cannot be honoured, when it is neither `off` nor
    /// one of the modes the engine implements.
    ///
    /// `None` for `off`/absent (an operator's choice, and silent) and for the
    /// modes the caller resolves properly against the adapter (`state_only`,
    /// `mirror`, `submit`). Everything else is a TYPO, and a typo is refused
    /// with a reason rather than demoted to `off` — a mount that followed the
    /// documentation and then did nothing at all, with `writeback_supported`
    /// absent and not one line in the log, is the outcome this prevents.
    pub fn unsupported_mode_reason(&self) -> Option<String> {
        let mode = self.mode.trim();
        if mode.is_empty()
            || mode.eq_ignore_ascii_case("off")
            || self.wants_state_only()
            || self.wants_mirror()
            || self.wants_submit()
        {
            return None;
        }
        Some(format!(
            "write mode '{mode}' is not recognized; expected 'off', 'state_only', \
             'mirror' or 'submit'"
        ))
    }

    /// The fields this mount declares pushable, empty unless it drives a write
    /// mode that pushes field updates. Reading them under any other mode would
    /// let a `mode: off` mount push the moment an adapter declared `can_update`.
    ///
    /// A `mirror` reads the same list as a `state_only`. That is not a shortcut:
    /// the engine is domain-blind and cannot know which node properties a
    /// provider will accept as writes, so the allow-list is how the intent is
    /// stated in BOTH modes. What a mirror adds over a `state_only` is delete
    /// propagation, not an implicit "send everything" — which would be a guess,
    /// and one that reaches the provider.
    /// Whether a node of this type is one of this outbox's commands.
    ///
    /// Read ONLY by the `submit` drain. It is the provenance rule for a path
    /// where the usual one — `__mount_id` on the node — cannot apply, and it
    /// fails CLOSED: an unrecognized type under the outbox path is ordinary user
    /// content, left untouched, rather than something to claim and send.
    pub fn is_command_type(&self, node_type: &str) -> bool {
        if self.command_node_types.is_empty() {
            return DEFAULT_COMMAND_NODE_TYPES.contains(&node_type);
        }
        self.command_node_types.iter().any(|t| t == node_type)
    }

    /// Whether this mount may create a node of `node_type` at the provider.
    ///
    /// Only meaningful for `mirror`; every other mode answers false, so a
    /// `state_only` mailbox cannot upload a locally-authored node however its
    /// `create_node_types` is filled in. There is no default list — see
    /// [`Self::create_node_types`] for why an empty one means "create nothing".
    pub fn creates_locally(&self) -> bool {
        self.wants_mirror() && !self.create_node_types.is_empty()
    }

    /// Whether one node type is creatable by this mount.
    pub fn is_creatable_type(&self, node_type: &str) -> bool {
        self.creates_locally() && self.create_node_types.iter().any(|t| t == node_type)
    }

    pub fn declared_mutable_fields(&self) -> &[String] {
        if self.wants_state_only() || self.wants_mirror() {
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
    /// Function node that decides what a REFUSED push means for this mount
    /// (`write_config.conflict: "resolver_function"`).
    ///
    /// Declared beside `mapping_function` rather than inside `write_config`
    /// because it is the same kind of thing — a function node this mount owns —
    /// and it gets the same rule: **no integration-level fallback**. An adapter
    /// serves many mounts and cannot have an opinion about which of two
    /// conflicting edits to a particular mount's data is the right one;
    /// `adapter_function` falls back precisely because the opposite is true of
    /// it.
    pub resolver_function: Option<String>,
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
            resolver_function: opt_string(node, "resolver_function"),
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
