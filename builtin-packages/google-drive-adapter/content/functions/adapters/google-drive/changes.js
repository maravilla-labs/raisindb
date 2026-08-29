/**
 * The delta feed. Account-wide at Drive, mount-scoped here: `paths.js` decides
 * which changes belong to this mount and where they land.
 */

import { enc } from "./common.js";
import { DRIVE, driveFetch } from "./http.js";
import { FILE_FIELDS, toExternalItem } from "./items.js";
import { changeRelativePath, newPathCache } from "./paths.js";

export function opGetChanges(credential, mount, params) {
  var token = params.since_token;
  // First delta call: baseline. Fetch a start token and report no changes —
  // the engine has already run a full reconcile for the initial state.
  if (!token) {
    var startResp = driveFetch(
      credential,
      "GET",
      DRIVE + "/changes/startPageToken?supportsAllDrives=true",
      { context: "get_changes(start)" }
    );
    return { items: [], next_token: startResp.body.startPageToken };
  }

  var url =
    DRIVE +
    "/changes?pageToken=" +
    enc(token) +
    "&fields=" +
    enc(
      "newStartPageToken,nextPageToken,changes(fileId,removed,file(" + FILE_FIELDS + "))"
    ) +
    "&pageSize=200&supportsAllDrives=true&includeItemsFromAllDrives=true&includeRemoved=true";
  var resp = driveFetch(credential, "GET", url, { context: "get_changes" });
  var body = resp.body || {};
  var changes = body.changes || [];
  var cache = newPathCache();
  var items = [];
  for (var i = 0; i < changes.length; i++) {
    var c = changes[i];
    if (c.removed || (c.file && c.file.trashed)) {
      // A deletion carries no path and needs none — the engine stages it by
      // `external_id` — and a removed file has no metadata left to walk anyway.
      // Deletions are NOT subtree-filtered for that reason: an id from outside
      // the mount matches no node and stages nothing.
      items.push({ type: "deleted", item: { external_id: c.fileId }, relative_path: "" });
      continue;
    }
    if (!c.file || !c.file.id) continue;
    var rel = changeRelativePath(credential, mount, cache, c.file);
    // Outside the mount. The feed is account-wide, so this is the ordinary case
    // for most changes, not an error.
    if (rel === null) continue;
    items.push({ type: "updated", item: toExternalItem(c.file), relative_path: rel });
  }
  // Durable, resumable cursor: prefer nextPageToken while paging, else the new start token.
  // `has_more` says explicitly whether to keep paging now (nextPageToken) or
  // stop with a caught-up cursor — token identity is not a reliable signal.
  var next = body.nextPageToken || body.newStartPageToken || token;
  return { items: items, next_token: next, has_more: Boolean(body.nextPageToken) };
}
