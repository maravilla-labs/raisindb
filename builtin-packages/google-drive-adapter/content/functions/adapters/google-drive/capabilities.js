/**
 * What this adapter declares it can do. Read by the engine before it schedules
 * anything, so every flag here is a promise some other module must keep.
 */

export function opCapabilities() {
  return {
    can_read: true,
    can_write: true,
    can_create_folders: true,
    supports_changes: true,
    supports_webhooks: false,
    supports_search: false,
    supports_push: false,
    default_ttl: null,
    max_file_size: null,

    // ---- write path ----
    // Declared because they are implemented below and dispatched in `handler`.
    // A capability the engine cannot see is a capability the engine will not
    // use.
    //
    // Each is demanded only by the mount that would USE it, not by every mirror
    // mount — `write/plan.rs::resolve_mirror` asks for `can_create` only when
    // `write_config.create_node_types` is non-empty and for `can_delete` only
    // when the resolved delete policy actually pushes (a `detach` mount never
    // calls `delete`). So omitting one here does not make the whole mount
    // read-only; it silently removes exactly the operation it names from the
    // mounts configured to want it, which is the harder failure to see.
    can_create: true,
    can_update: true,
    can_delete: true,
    can_submit: false,

    // THE BYTE CHANNEL. Without it the engine sends metadata only and a
    // "mirrored" file arrives at Drive as a name with no content — which is
    // what this adapter did for as long as its upload path read
    // `params.content` as a string the engine has never sent.
    //
    // Declaring it also changes when a create is ATTEMPTED: the engine defers
    // any create whose node has no bytes yet (`write::content::content_pending`)
    // rather than minting an empty file the next walk would call synced.
    accepts_content: true,

    // What a local edit may push. Drive files are content plus one writable
    // piece of metadata worth mirroring — the name. The node property is
    // `title`, which is what the default mapper writes; the reverse mapper
    // turns it back into Drive's `name`. Everything else the mapper emits is
    // provider-computed (size, checksums, links, timestamps) and a PATCH
    // carrying it would be rejected or silently ignored.
    mutable_fields: ["title"],

    // A locally-created FOLDER becomes a real Drive folder (`opCreate`'s folder
    // branch, and the default mapper's `to_external` emits the folder mime type
    // on a create so the mapper — not the adapter — stays the authority on what
    // a node translates to). This flag is what makes the engine offer
    // raisin:Folder as a creatable type at all.
    //
    // KNOWN GAP, stated rather than papered over: the engine's create drain
    // defers every candidate whose node carries no file bytes while
    // `accepts_content` is declared, and a folder never carries any — so a
    // folder create is currently issued only by a mount that does not take
    // content. Nothing here throws when the engine does ask; the branch is real.
    // The deferral is `write/content.rs::content_pending`, and it is the same
    // for the ms-graph adapter.

    // `detach` for files (§9.5): a local delete removes the node and leaves the
    // Drive file alone. Deliberately NOT `trash` — a mount is frequently a
    // read-mostly view of a shared Drive folder, and a node deleted to tidy a
    // workspace must not bin a colleague's file. A mount whose deletes really
    // should propagate sets `write_config.delete_policy` explicitly, and gets
    // `trash` (recoverable) or `purge` (not) by name.
    default_delete_policy: "detach",
    default_move_policy: "detach",
    // Drive has a real trash: `trashed: true` is reversible from the UI for 30
    // days. Declaring this is what lets a mount choose `trash` at all — without
    // it the engine REFUSES the policy rather than quietly promoting it to a
    // permanent delete.
    supports_trash: true,
    supports_idempotency_key: false,
  };
}
