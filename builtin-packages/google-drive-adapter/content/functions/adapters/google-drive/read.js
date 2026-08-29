/**
 * The READ operations: list, get (by id or by path) and get_content.
 */

import { enc } from "./common.js";
import { DRIVE, driveFetch, raiseForStatus } from "./http.js";
import { FILE_FIELDS, toExternalItem } from "./items.js";

export function opList(credential, mount, params) {
  var folderId = params.folder_id || mount.remote_root;
  var pageSize = params.limit && params.limit > 0 ? Math.min(params.limit, 1000) : 200;
  var q = "'" + folderId + "' in parents and trashed = false";
  var url =
    DRIVE +
    "/files?q=" +
    enc(q) +
    "&fields=" +
    enc("nextPageToken,files(" + FILE_FIELDS + ")") +
    "&pageSize=" +
    pageSize +
    "&supportsAllDrives=true&includeItemsFromAllDrives=true";
  if (params.cursor) url += "&pageToken=" + enc(params.cursor);

  var resp = driveFetch(credential, "GET", url, { context: "list" });
  var files = (resp.body && resp.body.files) || [];
  var items = files.map(toExternalItem);
  return { items: items, next_cursor: (resp.body && resp.body.nextPageToken) || null };
}

export function opGet(credential, mount, params) {
  if (params.item_id) {
    var url =
      DRIVE +
      "/files/" +
      enc(params.item_id) +
      "?fields=" +
      enc(FILE_FIELDS) +
      "&supportsAllDrives=true";
    var resp = driveFetch(credential, "GET", url, { context: "get", rawStatusOk: true });
    if (resp.status === 404) return null;
    raiseForStatus(resp, "get");
    if (resp.body && resp.body.trashed) return null;
    return toExternalItem(resp.body);
  }
  if (params.path) {
    return getByPath(credential, mount, params.path);
  }
  return null;
}

// Resolve a path relative to remote_root by walking one segment at a time.
function getByPath(credential, mount, relPath) {
  var parts = relPath.split("/").filter(function (p) {
    return p.length > 0;
  });
  var parent = mount.remote_root;
  var found = null;
  for (var i = 0; i < parts.length; i++) {
    var q =
      "'" + parent + "' in parents and name = '" +
      parts[i].replace(/'/g, "\\'") +
      "' and trashed = false";
    var url =
      DRIVE + "/files?q=" + enc(q) + "&fields=" + enc("files(" + FILE_FIELDS + ")") +
      "&pageSize=1&supportsAllDrives=true&includeItemsFromAllDrives=true";
    var resp = driveFetch(credential, "GET", url, { context: "get(path)" });
    var files = (resp.body && resp.body.files) || [];
    if (!files.length) return null;
    found = files[0];
    parent = found.id;
  }
  return found ? toExternalItem(found) : null;
}

// v1 mounts link via web_url/download_url and never call get_content. It is
// implemented for opt-in content sync: binary files download via alt=media;
// Google-native docs export to a portable mime type.
export function opGetContent(credential, params) {
  var meta = driveFetch(
    credential,
    "GET",
    DRIVE + "/files/" + enc(params.item_id) + "?fields=" + enc("mimeType,name") +
      "&supportsAllDrives=true",
    { context: "get_content(meta)" }
  ).body;

  if (meta.mimeType && meta.mimeType.indexOf("application/vnd.google-apps.") === 0) {
    var exportMime = exportMimeFor(meta.mimeType);
    var ex = driveFetch(
      credential,
      "GET",
      DRIVE + "/files/" + enc(params.item_id) + "/export?mimeType=" + enc(exportMime),
      { context: "get_content(export)" }
    );
    return { content: bodyToString(ex.body), mime_type: exportMime };
  }
  var dl = driveFetch(
    credential,
    "GET",
    DRIVE + "/files/" + enc(params.item_id) + "?alt=media&supportsAllDrives=true",
    { context: "get_content(media)" }
  );
  return { content: bodyToString(dl.body), mime_type: meta.mimeType || "application/octet-stream" };
}

function exportMimeFor(googleMime) {
  if (googleMime.indexOf("spreadsheet") >= 0) return "text/csv";
  if (googleMime.indexOf("presentation") >= 0) return "text/plain";
  return "text/plain";
}

function bodyToString(body) {
  return typeof body === "string" ? body : JSON.stringify(body);
}
