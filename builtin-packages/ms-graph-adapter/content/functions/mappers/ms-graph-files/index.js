/**
 * Microsoft 365 (OneDrive) files mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop (and, for to_external, inside the write drain under the
 * mount lease). Returning null skips the item.
 *
 * Bidirectional dispatch (adapter contract §6.0), same shape as the calendar
 * mapper:
 *
 *   to_node             { external_item, mount }          -> { node_type, name?, properties } | null
 *   to_external         { node, mount, fields?, intent? } -> { payload } | null
 *   mapper_capabilities { mount }                         -> { to_external: true }
 *
 * An absent operation means to_node, so the engine's read path is unchanged.
 *
 * `to_external` emits the driveItem METADATA only — a name and a conflict
 * behaviour. The BYTES never pass through here: they travel beside the payload
 * as the engine's `content` (base64 for a small file, streamed by the engine for
 * a large one), because a mapper that had to carry megabytes through
 * JSON.stringify would be neither pure nor affordable.
 *
 * Mapping:
 *   - driveItem folders -> raisin:Folder
 *   - driveItem files   -> raisin:Asset
 *
 * The ExternalItem's `name`/`external_id` are the Graph driveItem id (so distinct
 * items never collide on a path); the human filename is carried in
 * metadata.filename and used here for the title/name. Link-only in v1:
 * web_url/download_url are carried through, no content is inlined. The engine
 * stamps the reserved __virtual/__mount_id/__external_id/__etag/__synced_at
 * properties on top of whatever is returned, so they are not set here.
 */

function handler(input) {
  switch (input && input.operation) {
    case "to_external":
      return toExternal(input.node, input.mount, input.fields, input.intent);
    case "mapper_capabilities":
      return { to_external: true };
    case "to_node":
    default:
      return toNode(input);
  }
}

// ---- to_node --------------------------------------------------------------

function toNode(input) {
  var item = input.external_item;
  if (!item || !item.external_id) return null;

  var meta = item.metadata || {};
  var filename = meta.filename || item.name;

  if (item.is_folder) {
    return {
      node_type: "raisin:Folder",
      name: filename,
      properties: {
        title: filename,
        icon: "folder",
        provider: "ms-graph",
        parent_id: item.parent_id || null,
        created_at: item.created_at || null,
        modified_at: item.modified_at || null,
        provider_metadata: meta,
      },
    };
  }

  return {
    node_type: "raisin:Asset",
    name: filename,
    properties: {
      title: filename,
      mimeType: item.mime_type || null,
      size: item.size_bytes != null ? item.size_bytes : null,
      // The human-openable link. Durable: this is the item's page in
      // OneDrive/SharePoint, and it survives as long as the item does.
      web_url: item.web_url || null,
      // SHORT-LIVED, and deliberately not the way to read this file.
      //
      // Graph's `@microsoft.graph.downloadUrl` is pre-authenticated and expires
      // in roughly an hour, while this node is only rewritten when its etag
      // changes — so a settled file carries a link that has been dead for
      // weeks and still looks durable. It is kept because it is genuinely
      // useful in the minutes after a sync (a preview, a quick hand-off) and
      // removing a published property would break consumers.
      //
      // To READ THE BYTES, ask the engine for the content instead: the
      // adapter's `get_content` mints a fresh URL per call and the engine
      // downloads it binary-safely. To judge how stale this copy is, read the
      // engine's own `__synced_at` on the node — deliberately NOT a timestamp
      // minted here, which would make this mapper answer differently for
      // identical input and rewrite the node on every remap.
      download_url: item.download_url || null,
      provider: "ms-graph",
      provider_kind: "file",
      parent_id: item.parent_id || null,
      created_at: item.created_at || null,
      modified_at: item.modified_at || null,
      // Provider-specific passthrough is preserved verbatim.
      provider_metadata: meta,
    },
  };
}

// ---- to_external ----------------------------------------------------------

// No list at all means "the whole object" — a create.
function allowed(fields, name) {
  if (!fields || !fields.length) return true;
  return fields.indexOf(name) !== -1;
}

function str(v) {
  return typeof v === "string" && v ? v : null;
}

/**
 * The driveItem metadata for a create or an update.
 *
 * ALWAYS non-null for a file node, and that is deliberate rather than lax. The
 * engine skips an item whose to_external answers null, so a CONTENT-ONLY push —
 * new bytes, unchanged name — would be dropped before the adapter ever saw it if
 * this returned null for an empty payload the way the calendar mapper does. The
 * conflict behaviour is therefore always emitted, which also means the adapter
 * never has to guess it.
 */
function toExternal(node, mount, fields, intent) {
  if (!node) return null;
  var props = node.properties || {};
  var creating = intent === "create";

  // A FOLDER is creatable now (`driveCreate` has a folder branch and
  // `can_create_folders` says so), and it is announced EXPLICITLY rather than
  // inferred from the absence of bytes: the adapter falls back to that
  // inference, but a mapper that knows the node type should not make the
  // adapter guess. An existing folder node is still renameable through the
  // ordinary PATCH below.
  // `raisin:Folder` is the engine's own container type and always counts. A
  // mount may name ADDITIONAL container types in
  // `sync_config.folder_node_types` — which is how a product with its own
  // folder type (a CMS folder, say) mounts a drive without that type's name
  // living in this mapper, this adapter, or the engine. Same list the engine
  // reads when it decides whether a node is waiting for bytes, so the two
  // cannot disagree about what a folder is.
  var extra = (mount && mount.sync_config && mount.sync_config.folder_node_types) || [];
  var isFolder =
    node.node_type === "raisin:Folder" || extra.indexOf(node.node_type) !== -1;

  var payload = {};
  if (isFolder && creating) payload.is_folder = true;

  // `title` is the node property the engine knows how to gate; `name` is what
  // Graph calls it. The node's own name is the fallback because to_node writes
  // the filename there — for a node this mapper imported the two agree, and for
  // a locally-born one the name is what the author typed.
  var name = str(props.title) || str(node.name);
  if (name && allowed(fields, "title")) payload.name = name;

  // WHY `rename` ON CREATE. The file already sitting at that name may not be
  // ours: a mirror create is a locally-born node landing in a drive full of
  // documents this mount never imported, and `replace` would destroy a
  // stranger's file while reporting success. Graph answers with the real,
  // possibly renamed item and the engine adopts THAT id.
  //
  // On update the item is addressed by its own id, so `replace` means "these are
  // the new bytes for this file", which is what an update is.
  payload["@microsoft.graph.conflictBehavior"] = creating ? "rename" : "replace";

  // A create with no name at all cannot be issued — Graph has nowhere to put the
  // file. Null rather than a guess: the engine records the item as failed with a
  // stated reason, which is something an author can act on.
  if (creating && !payload.name) return null;

  return { payload: payload };
}
