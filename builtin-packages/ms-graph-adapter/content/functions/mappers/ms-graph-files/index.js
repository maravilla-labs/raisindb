/**
 * Microsoft 365 (OneDrive) files mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 *   input  = { external_item: ExternalItem, mount: { mount_id, mount_path, sync_config } }
 *   return = { node_type, name?, properties } | null
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
