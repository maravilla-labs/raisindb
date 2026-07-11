/**
 * Google Drive virtual-node adapter.
 *
 * Implements the frozen adapter contract (docs/reference/virtual-node-adapters.md)
 * over the Google Drive v3 REST API using the synchronous `raisin.http.fetch`
 * binding. The sync engine invokes this function directly, decrypts the account
 * credential just before the call, and materializes returned items into nodes.
 *
 * Entrypoint: handler(input) — exactly one argument.
 *   input = { operation, params, credential, mount }
 *
 * Token lifecycle is owned entirely by the engine: `credential.access_token` is
 * a current, decrypted token; there is NO refresh_token and no refresh logic
 * here. If a token is rejected, throw `auth_expired` and let the engine handle
 * the reconnect/refresh cycle.
 */

var DRIVE = "https://www.googleapis.com/drive/v3";
var UPLOAD = "https://www.googleapis.com/upload/drive/v3";
var FOLDER_MIME = "application/vnd.google-apps.folder";

// Fields requested for every file so ExternalItem can be built without extra calls.
var FILE_FIELDS =
  "id,name,mimeType,size,parents,createdTime,modifiedTime,version," +
  "md5Checksum,webViewLink,webContentLink,trashed,shared,iconLink";

function coded(message, code) {
  var e = new Error(message);
  e.code = code;
  return e;
}

// Throw the reserved error codes the engine dispatches on. Never swallow an
// auth failure into an empty result — that reads as "everything was deleted".
function raiseForStatus(resp, context) {
  var status = resp.status;
  if (status >= 200 && status < 300) return;

  var body = resp.body || {};
  var reason = "";
  try {
    if (body && body.error && body.error.errors && body.error.errors.length) {
      reason = body.error.errors[0].reason || "";
    }
  } catch (_) {
    reason = "";
  }

  if (status === 401) {
    throw coded("Google Drive rejected the access token", "auth_expired");
  }
  if (status === 429) {
    throw coded("Google Drive rate limit exceeded", "rate_limited");
  }
  if (
    status === 403 &&
    (reason === "rateLimitExceeded" ||
      reason === "userRateLimitExceeded" ||
      reason === "dailyLimitExceeded")
  ) {
    throw coded("Google Drive usage limit exceeded", "rate_limited");
  }
  var msg =
    (body && body.error && body.error.message) ||
    "Google Drive request failed (" + status + ")";
  throw new Error(context + ": " + msg);
}

// Single authorized request. `raisin.http.fetch` is synchronous and returns
// { status, headers, body }.
function driveFetch(credential, method, url, opts) {
  opts = opts || {};
  var headers = { Authorization: "Bearer " + credential.access_token };
  if (opts.headers) {
    for (var k in opts.headers) headers[k] = opts.headers[k];
  }
  var request = { method: method, headers: headers };
  if (opts.body !== undefined) request.body = opts.body;
  var resp = raisin.http.fetch(url, request);
  if (!opts.rawStatusOk || (resp.status !== 404 && resp.status !== 412)) {
    raiseForStatus(resp, opts.context || method + " " + url);
  }
  return resp;
}

function enc(v) {
  return encodeURIComponent(v);
}

function toExternalItem(f) {
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

// ---- operations -----------------------------------------------------------

function opCapabilities() {
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
  };
}

function opList(credential, mount, params) {
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

function opGet(credential, mount, params) {
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
function opGetContent(credential, params) {
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

function opCreate(credential, params) {
  var metadata = { name: params.name, parents: [params.parent_id] };
  if (params.is_folder) {
    metadata.mimeType = FOLDER_MIME;
    var resp = driveFetch(credential, "POST", DRIVE + "/files?fields=" + enc(FILE_FIELDS), {
      headers: { "Content-Type": "application/json" },
      body: metadata,
      context: "create(folder)",
    });
    return toExternalItem(resp.body);
  }
  if (params.mime_type) metadata.mimeType = params.mime_type;
  if (params.content === undefined || params.content === null) {
    var r = driveFetch(credential, "POST", DRIVE + "/files?fields=" + enc(FILE_FIELDS), {
      headers: { "Content-Type": "application/json" },
      body: metadata,
      context: "create(file)",
    });
    return toExternalItem(r.body);
  }
  return multipartUpload(credential, "POST", UPLOAD + "/files", metadata, params);
}

function opUpdate(credential, params) {
  // Optimistic concurrency: compare the caller's expected etag (file `version`)
  // against the current remote value; mismatch is a hard conflict.
  if (params.etag) {
    var cur = driveFetch(
      credential,
      "GET",
      DRIVE + "/files/" + enc(params.item_id) + "?fields=version&supportsAllDrives=true",
      { context: "update(etag-check)" }
    ).body;
    var remoteEtag = cur.version != null ? String(cur.version) : null;
    if (remoteEtag !== null && remoteEtag !== String(params.etag)) {
      throw coded("etag mismatch on update", "conflict");
    }
  }

  var metadata = {};
  if (params.name !== undefined) metadata.name = params.name;
  if (params.mime_type !== undefined) metadata.mimeType = params.mime_type;

  if (params.content === undefined || params.content === null) {
    var resp = driveFetch(
      credential,
      "PATCH",
      DRIVE + "/files/" + enc(params.item_id) + "?fields=" + enc(FILE_FIELDS) +
        "&supportsAllDrives=true",
      { headers: { "Content-Type": "application/json" }, body: metadata, context: "update" }
    );
    return toExternalItem(resp.body);
  }
  return multipartUpload(
    credential,
    "PATCH",
    UPLOAD + "/files/" + enc(params.item_id),
    metadata,
    params
  );
}

// Multipart/related upload: JSON metadata part + raw content part in one body.
function multipartUpload(credential, method, base, metadata, params) {
  var boundary = "raisin-gdrive-" + Date.now();
  var body =
    "--" + boundary + "\r\n" +
    "Content-Type: application/json; charset=UTF-8\r\n\r\n" +
    JSON.stringify(metadata) + "\r\n" +
    "--" + boundary + "\r\n" +
    "Content-Type: " + (params.mime_type || "text/plain") + "\r\n\r\n" +
    params.content + "\r\n" +
    "--" + boundary + "--";
  var url = base + "?uploadType=multipart&fields=" + enc(FILE_FIELDS) + "&supportsAllDrives=true";
  var resp = driveFetch(credential, method, url, {
    headers: { "Content-Type": "multipart/related; boundary=" + boundary },
    body: body,
    context: "upload",
  });
  return toExternalItem(resp.body);
}

function opDelete(credential, params) {
  var resp = driveFetch(
    credential,
    "DELETE",
    DRIVE + "/files/" + enc(params.item_id) + "?supportsAllDrives=true",
    { context: "delete", rawStatusOk: true }
  );
  // Already-absent items delete idempotently.
  if (resp.status === 404) return { deleted: true };
  raiseForStatus(resp, "delete");
  return { deleted: true };
}

function opGetChanges(credential, mount, params) {
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
  var items = changes.map(function (c) {
    if (c.removed || (c.file && c.file.trashed)) {
      return {
        type: "deleted",
        item: { external_id: c.fileId },
        relative_path: "",
      };
    }
    var item = toExternalItem(c.file);
    return { type: "updated", item: item, relative_path: item.name };
  });
  // Durable, resumable cursor: prefer nextPageToken while paging, else the new start token.
  var next = body.nextPageToken || body.newStartPageToken || token;
  return { items: items, next_token: next };
}

// ---- dispatch -------------------------------------------------------------

function handler(input) {
  var operation = input.operation;
  var params = input.params || {};
  var credential = input.credential;
  var mount = input.mount || {};

  switch (operation) {
    case "capabilities":
      return opCapabilities();
    case "list":
      return opList(credential, mount, params);
    case "get":
      return opGet(credential, mount, params);
    case "get_content":
      return opGetContent(credential, params);
    case "create":
      return opCreate(credential, params);
    case "update":
      return opUpdate(credential, params);
    case "delete":
      return opDelete(credential, params);
    case "get_changes":
      return opGetChanges(credential, mount, params);
    default:
      throw new Error("Unsupported operation: " + operation);
  }
}
