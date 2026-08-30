/**
 * The delta feed. Account-wide at Drive, mount-scoped here: `paths.js` decides
 * which changes belong to this mount and where they land.
 */

import { coded, enc } from "./common.js";
import { DRIVE, driveFetch, errorReason, raiseForStatus } from "./http.js";
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
  // 400/404 come back RAW so this function — the only one that knows the token
  // is a delta cursor — can classify them. Left to raiseForStatus they are a
  // plain Error, which AdapterError::classify files as Transient: the engine
  // then re-sends the SAME dead cursor on every drain, forever, importing
  // nothing and emitting only a repeating transient error, because nothing ever
  // says `cursor_invalid` and so no full reconcile is ever scheduled. This is
  // the incident ms-graph already fixed for its 410/resyncRequired (http.js
  // there). Google documents no expiry for Drive page tokens, but 400
  // invalidPageToken is observed in the wild (issuetracker 196413673) and the
  // documented recovery is exactly getStartPageToken + reconcile, which is what
  // `cursor_invalid` makes the engine do.
  var resp = driveFetch(credential, "GET", url, {
    context: "get_changes",
    rawStatusOk: true,
    rawStatuses: [400, 404],
  });
  if (resp.status === 400 || resp.status === 404) {
    // `invalid` is read as a token reason ONLY because the pageToken is the one
    // variable in the URL above — the fields selection, the page size and the
    // shared-drive flags are literals — and Drive answers a rejected pageToken
    // with the bare reason `invalid` / message "Invalid Value". That is not a
    // safe reading of a Drive 400 in general (a bad `fields` value reports the
    // same reason), so if this URL ever takes a caller-supplied parameter, drop
    // `invalid` from this list: a malformed request reported as cursor_invalid
    // would silently re-baseline and full-walk on every run instead of saying
    // what is wrong.
    var reason = errorReason(resp);
    if (resp.status === 404 || reason === "invalidPageToken" || reason === "invalid") {
      throw coded(
        "get_changes: Google Drive rejected the delta pageToken (" +
          resp.status +
          " " +
          (reason || "not found") +
          "). The cursor is unusable; re-baseline from changes.getStartPageToken " +
          "and reconcile.",
        "cursor_invalid"
      );
    }
    // Any other 400 is a malformed request, not a dead cursor — hand it back to
    // the single mapping point so it reads the same as everywhere else.
    raiseForStatus(resp, "get_changes");
  }
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
