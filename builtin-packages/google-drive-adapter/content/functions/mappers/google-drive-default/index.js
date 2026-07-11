/**
 * Default Google Drive mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 *   input  = { external_item: ExternalItem, mount: { mount_id, mount_path, sync_config } }
 *   return = { node_type, name?, properties } | null
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

function handler(input) {
  var item = input.external_item;
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
