/**
 * What a Drive file IS to the engine: the field set every call requests, and the
 * one translation from a Drive `files` resource to an ExternalItem.
 *
 * Shared by read, changes and the write receipts because the etag formula here
 * (`version`, falling back to `modifiedTime`) is what the engine's skip-write
 * compares against. A second derivation of it anywhere would make the run after
 * a write mismatch its own push and rebuild the node from remote.
 */

export var FOLDER_MIME = "application/vnd.google-apps.folder";

// Fields requested for every file so ExternalItem can be built without extra calls.
export var FILE_FIELDS =
  "id,name,mimeType,size,parents,createdTime,modifiedTime,version," +
  "md5Checksum,webViewLink,webContentLink,trashed,shared,iconLink";

export function toExternalItem(f) {
  var isFolder = f.mimeType === FOLDER_MIME;
  var parents = f.parents || [];
  return {
    external_id: f.id,
    name: f.name,
    mime_type: f.mimeType || null,
    size_bytes: f.size !== undefined ? Number(f.size) : null,
    is_folder: isFolder,
    parent_id: parents.length ? parents[0] : null,
    created_at: f.createdTime || null,
    modified_at: f.modifiedTime || null,
    // `version` is a monotonic per-file counter — stable when nothing changed,
    // which lets the engine's etag skip-write suppress needless revisions.
    etag: f.version != null ? String(f.version) : f.modifiedTime || null,
    web_url: f.webViewLink || null,
    // v1 mounts link only; download_url is a direct-content link, never inlined.
    download_url: f.webContentLink || null,
    metadata: {
      md5_checksum: f.md5Checksum || null,
      shared: f.shared || false,
      icon_link: f.iconLink || null,
      trashed: f.trashed || false,
      google_mime_type: f.mimeType || null,
    },
  };
}
