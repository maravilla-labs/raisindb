/**
 * Default Google Drive mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 * Bidirectional, dispatched on `input.operation` (adapter contract §6.0):
 *
 *   to_node     input  = { external_item, mount }
 *               return = { node_type, name?, properties } | null
 *   to_external input  = { node, mount, fields? }
 *               return = { payload, external_id? } | null
 *
 * An absent `operation` means `to_node`, so nothing that called this mapper
 * before the write path existed changes behaviour.
 *
 * BOTH directions live here rather than the reverse one living inside the
 * adapter. The mapper exists so a user can reshape nodes without forking the
 * adapter — and the moment someone points a mount at a custom mapper, an adapter
 * with its own built-in reverse mapping writes the wrong fields, silently. One
 * relationship, one file.
 *
 * Mapping:
 *   - folders                        -> raisin:Folder
 *   - Google Docs / Sheets / Slides  -> raisin:Asset (kind reflects the doc type)
 *   - every other file               -> raisin:Asset
 *
 * The engine stamps the reserved __virtual/__mount_id/__external_id/__etag/
 * __synced_at properties on top of whatever is returned, so they are not set here.
 * v1 links only: web_url/download_url are carried through, no content is inlined.
 */

var GOOGLE_DOC_KINDS = {
  "application/vnd.google-apps.document": "google-doc",
  "application/vnd.google-apps.spreadsheet": "google-sheet",
  "application/vnd.google-apps.presentation": "google-slides",
  "application/vnd.google-apps.form": "google-form",
  "application/vnd.google-apps.drawing": "google-drawing",
};

// Node property -> Drive metadata field, for the reverse direction. Exactly the
// fields `opCapabilities().mutable_fields` declares; anything else Drive either
// rejects or computes itself, and a PATCH carrying it bumps the file `version`
// for nothing — which on a mirror is one revision per file per drain, forever.
var WRITABLE = { title: "name" };

function handler(input) {
  switch (input.operation) {
    // Probed once per run by the engine to decide whether this mount is
    // writable at all, so the console can say why rather than showing a control
    // that silently does nothing.
    case "mapper_capabilities":
      return { to_external: true };
    case "to_external":
      return toExternal(input.node, input.fields);
    // `to_node`, and absent — see the header.
  }
  return toNode(input.external_item);
}

/**
 * One node back into a Drive metadata patch.
 *
 * `fields` is the engine's allow-list (the mount's `mutable_fields` narrowed by
 * the adapter's). Emitting only those keys is what keeps a field-scoped update
 * from becoming a whole-object overwrite.
 *
 * Returns null — "not writable" — for a folder, for a node with no writable
 * field in the request, and for anything that resolves to an empty patch. Null
 * parks the intent with a stated reason instead of sending a guess, and an empty
 * PATCH is the one request that costs a revision and achieves nothing.
 */
function toExternal(node, fields) {
  if (!node) return null;
  var props = node.properties || {};
  var allowed = fields && fields.length ? fields : Object.keys(WRITABLE);

  var payload = {};
  for (var i = 0; i < allowed.length; i++) {
    var driveField = WRITABLE[allowed[i]];
    if (!driveField) continue;
    var value = props[allowed[i]];
    // An absent or blank name is not an instruction to clear it — Drive has no
    // nameless file, and sending "" renames it to nothing.
    if (value === undefined || value === null || value === "") continue;
    payload[driveField] = value;
  }
  for (var _ in payload) {
    return { payload: payload, external_id: props.__external_id || undefined };
  }
  return null;
}

function toNode(item) {
  if (!item || !item.external_id) return null;

  if (item.is_folder) {
    return {
      node_type: "raisin:Folder",
      name: item.name,
      properties: {
        title: item.name,
        icon: "folder",
      },
    };
  }

  var meta = item.metadata || {};
  var googleKind = GOOGLE_DOC_KINDS[item.mime_type] || null;

  var properties = {
    title: item.name,
    mimeType: item.mime_type || null,
    size: item.size_bytes != null ? item.size_bytes : null,
    // Link-only in v1 — the human-openable and direct-download URLs.
    web_url: item.web_url || null,
    download_url: item.download_url || null,
    provider: "google-drive",
    provider_kind: googleKind || "file",
    created_at: item.created_at || null,
    modified_at: item.modified_at || null,
    // Provider-specific passthrough is preserved verbatim.
    provider_metadata: meta,
  };

  return {
    node_type: "raisin:Asset",
    name: item.name,
    properties: properties,
  };
}
