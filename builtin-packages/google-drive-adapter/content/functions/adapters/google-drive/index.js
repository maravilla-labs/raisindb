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
  // The engine passes `credential: null` when no account is selected; guard so
  // that surfaces as a readable error rather than a TypeError. Plain Error on
  // purpose — a coded "auth_expired" would be rewritten by the host into
  // "credential is expired or was rejected", the wrong diagnosis here.
  if (!credential || !credential.access_token) {
    throw new Error(
      "no account credential — connect a Google account and select it for this connector or mount"
    );
  }
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

    // ---- write path ----
    // Declared because they are implemented below and dispatched in `handler`.
    // A capability the engine cannot see is a capability the engine will not
    // use: `write::plan::resolve` refuses a `mirror` mount whose adapter has
    // not said all three, so omitting them here is what makes the mount
    // read-only regardless of what this file can actually do.
    can_create: true,
    can_update: true,
    can_delete: true,
    can_submit: false,

    // What a local edit may push. Drive files are content plus one writable
    // piece of metadata worth mirroring — the name. The node property is
    // `title`, which is what the default mapper writes; the reverse mapper
    // turns it back into Drive's `name`. Everything else the mapper emits is
    // provider-computed (size, checksums, links, timestamps) and a PATCH
    // carrying it would be rejected or silently ignored.
    mutable_fields: ["title"],

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

/**
 * Optimistic concurrency, ONE implementation, used by every write that carries
 * a concurrency base.
 *
 * Drive has no conditional request — no `If-Match`, no `If-Unmodified-Since` on
 * `files.update`/`files.delete` — so the check is a READ-THEN-COMPARE against
 * the file's `version`, and it is worth being honest about what that buys and
 * what it does not:
 *
 *   * It catches the case this is actually for: the remote changed since the
 *     mount last read it, so the local value the engine is about to push (or the
 *     local delete it is about to propagate) was decided against a stale view.
 *   * It does NOT close the race. A change landing between the GET and the write
 *     is not seen. That is inherent to a provider with no conditional write and
 *     cannot be fixed here; the mount's conflict policy and the next delta are
 *     what recover from it.
 *
 * It lives here rather than inline in `opUpdate` because `opDelete` needs the
 * same guarantee and had NONE — the engine sends the pre-image's etag on every
 * delete and this adapter ignored it, so a file edited remotely after the last
 * sync was deleted anyway, with the operator's `max_delete_ratio` rails all
 * satisfied and no error anywhere. Two writes with two different answers to
 * "has this changed?" is the drift this codebase pays for most often.
 *
 * Returns `"gone"` when the file no longer exists, `"match"` otherwise; throws
 * `conflict` on a mismatch.
 */
function checkVersion(credential, itemId, etag, context) {
  if (etag === undefined || etag === null || etag === "") return "match";
  var resp = driveFetch(
    credential,
    "GET",
    DRIVE + "/files/" + enc(itemId) + "?fields=version&supportsAllDrives=true",
    { context: context, rawStatusOk: true }
  );
  // GONE, not a failure. Left to `raiseForStatus` this is a plain Error, i.e.
  // `Transient`, i.e. retried on every drain forever against an id that can
  // never come back.
  if (resp.status === 404) return "gone";
  raiseForStatus(resp, context);
  var cur = resp.body || {};
  var remoteEtag = cur.version != null ? String(cur.version) : null;
  if (remoteEtag !== null && remoteEtag !== String(etag)) {
    // The message text is load-bearing: `AdapterError::classify` scans for
    // auth_expired, rate_limited, cursor_invalid, config_error and THEN
    // conflict, so a message containing any earlier token is misclassified.
    throw coded("etag mismatch on " + context, "conflict");
  }
  return "match";
}

function opUpdate(credential, params) {
  // A vanished file SETTLES the node rather than failing it: the delta feed
  // reports the deletion and the engine removes the node on its own schedule.
  if (checkVersion(credential, params.item_id, params.etag, "update") === "gone") {
    return null;
  }

  // The write drain sends `params.payload` — the mount mapper's `to_external`
  // output, already provider-shaped and already narrowed to the mount's field
  // allow-list. `params.name` / `params.mime_type` are the older direct form,
  // kept because the adapter contract documents them for content sync; the
  // payload wins where both appear.
  var metadata = {};
  if (params.name !== undefined) metadata.name = params.name;
  if (params.mime_type !== undefined) metadata.mimeType = params.mime_type;
  if (params.payload && typeof params.payload === "object") {
    for (var pk in params.payload) metadata[pk] = params.payload[pk];
  }
  if (isEmptyObject(metadata) && (params.content === undefined || params.content === null)) {
    // An empty PATCH still bumps the file's `version`, which invalidates every
    // stored etag and makes the next delta re-deliver the file for no reason —
    // and on a mirror that is a revision per file per drain, forever.
    throw coded("update: refusing an empty PATCH body", "config_error");
  }

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

function isEmptyObject(v) {
  if (!v || typeof v !== "object") return true;
  for (var k in v) return false;
  return true;
}

/**
 * Delete, under the mount's resolved policy.
 *
 * `params.policy` is `"trash"` or `"purge"` — the engine never sends `"detach"`,
 * because detaching means not calling this at all. The distinction is not
 * cosmetic: `trashed: true` is reversible from the Drive UI for 30 days, and
 * `DELETE` is not reversible by anyone. An adapter that treated the two the same
 * would make `supports_trash` a lie and turn a recoverable operator mistake into
 * a permanent one.
 *
 * Absent policy means `purge`, which is what `delete` has always meant in this
 * contract. The engine always sends one.
 *
 * `params.etag` is the concurrency base captured from the node's MVCC pre-image
 * at detection time, and it is honoured here — see [`checkVersion`]. It used to
 * be accepted and ignored, which meant a file someone else edited after the last
 * sync was deleted anyway: the engine's blast-radius rails were all satisfied
 * (one node, one delete), so nothing anywhere reported that the thing destroyed
 * was not the thing the operator had seen.
 */
function opDelete(credential, params) {
  // Already gone is SUCCESS — a delete is the one operation whose desired end
  // state a 404 already satisfies.
  if (checkVersion(credential, params.item_id, params.etag, "delete") === "gone") {
    return { deleted: true };
  }
  if (params.policy === "trash") {
    var patched = driveFetch(
      credential,
      "PATCH",
      DRIVE + "/files/" + enc(params.item_id) + "?fields=id&supportsAllDrives=true",
      {
        headers: { "Content-Type": "application/json" },
        body: { trashed: true },
        context: "delete(trash)",
        rawStatusOk: true,
      }
    );
    if (patched.status === 404) return { deleted: true, trashed: true };
    raiseForStatus(patched, "delete(trash)");
    return { deleted: true, trashed: true };
  }

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
  // `has_more` says explicitly whether to keep paging now (nextPageToken) or
  // stop with a caught-up cursor — token identity is not a reliable signal.
  var next = body.nextPageToken || body.newStartPageToken || token;
  return { items: items, next_token: next, has_more: Boolean(body.nextPageToken) };
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
