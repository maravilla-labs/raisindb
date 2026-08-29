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
 *
 * WRITES are the full mirror set — create, update, delete — plus BYTES. Two
 * facts about Google shape that half and are worth knowing before reading it:
 *
 *   * `raisin.http.fetch` can send raw bytes (`bodyBase64`) but cannot assemble
 *     a multipart/related body around them, so every content write goes through
 *     a RESUMABLE UPLOAD SESSION: this adapter negotiates the session (an
 *     ordinary JSON call) and hands the ENGINE the URL to stream to, and the
 *     engine calls back with `finalize_upload`. That is also why the multipart
 *     path that used to live here is gone — it stringified `params.content`,
 *     which the engine sends as an OBJECT, so it could only ever have uploaded
 *     the literal text "[object Object]".
 *   * Drive's changes feed is ACCOUNT-WIDE, not folder-scoped, and Drive is a
 *     DAG rather than a tree. Both are handled in `changeRelativePath` below.
 *
 * SINGLE FILE on purpose: the QuickJS runtime is handed this file alone
 * (`tests_google_drive_write.rs` loads it with an empty module map), so a
 * sibling `import` would resolve to nothing at run time.
 */

var DRIVE = "https://www.googleapis.com/drive/v3";
var UPLOAD = "https://www.googleapis.com/upload/drive/v3";
var FOLDER_MIME = "application/vnd.google-apps.folder";

// Fields requested for every file so ExternalItem can be built without extra calls.
var FILE_FIELDS =
  "id,name,mimeType,size,parents,createdTime,modifiedTime,version," +
  "md5Checksum,webViewLink,webContentLink,trashed,shared,iconLink";

// Drive requires every chunk of a resumable upload EXCEPT THE LAST to be a
// multiple of 256 KiB; a non-multiple is rejected mid-transfer, after the bytes
// have already crossed the wire. 10 MiB is 40 such units.
//
// It is sent explicitly even though the engine's current fallback happens to be
// the same number: the 256 KiB rule is GOOGLE'S, the engine's default is the
// engine's, and an adapter that leans on someone else's default is one release
// away from having every non-final chunk rejected. The engine uses this value
// VERBATIM and rounds nothing, precisely so the provider's rule stays here.
var UPLOAD_CHUNK_SIZE = 40 * 256 * 1024;

// Drive answers EVERY non-final chunk of a resumable upload with `308 Resume
// Incomplete`. Declared for the same reason as the chunk size: 308 is a fact
// about Drive's protocol, and an engine default of `[]` (2xx only) would fail
// every multi-chunk upload on chunk one. On the FINAL chunk a 308 is still a
// hard failure — it means Drive does not consider the object written — and that
// judgement belongs to the engine, which makes it.
var UPLOAD_CONTINUE_STATUSES = [308];

function coded(message, code) {
  var e = new Error(message);
  e.code = code;
  return e;
}

// Throw the reserved error codes the engine dispatches on. Never swallow an
// auth failure into an empty result — that reads as "everything was deleted".
function raiseForStatus(resp, context, isWrite) {
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
  // A write-scope shortfall, which is the FIRST thing a newly writable mount
  // hits: the connector asks for a read scope, so every read succeeds and every
  // write 403s. Left as a plain Error this is transient, i.e. the same doomed
  // request re-sent on every drain forever, with the operator sent to reconnect
  // an account whose consent is not the problem. Terminal and named instead.
  if (status === 403 && isWrite) {
    throw coded(
      context + ": Google refused the write (403 " + (reason || "forbidden") +
        "). This is almost certainly a missing WRITE scope rather than a stale " +
        "token: add https://www.googleapis.com/auth/drive (or " +
        "https://www.googleapis.com/auth/drive.file for app-created files only) " +
        "to the Google connector's OAuth scopes and RECONNECT each account — " +
        "Google only issues a widened scope on fresh consent.",
      "config_error"
    );
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
    raiseForStatus(resp, opts.context || method + " " + url, opts.write);
  }
  return resp;
}

// One response header, whatever the host capitalized it as.
//
// The host builds this map from reqwest's own header names, which are
// lowercased — but the resumable session URL arrives in exactly one header and
// losing it means the bytes have nowhere to go, so the lookup does not bet on
// that staying true.
function headerValue(headers, name) {
  if (!headers) return null;
  if (typeof headers[name] === "string") return headers[name];
  var lower = String(name).toLowerCase();
  for (var k in headers) {
    if (String(k).toLowerCase() === lower && typeof headers[k] === "string") {
      return headers[k];
    }
  }
  return null;
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
    // use.
    //
    // Each is demanded only by the mount that would USE it, not by every mirror
    // mount — `write/plan.rs::resolve_mirror` asks for `can_create` only when
    // `write_config.create_node_types` is non-empty and for `can_delete` only
    // when the resolved delete policy actually pushes (a `detach` mount never
    // calls `delete`). So omitting one here does not make the whole mount
    // read-only; it silently removes exactly the operation it names from the
    // mounts configured to want it, which is the harder failure to see.
    can_create: true,
    can_update: true,
    can_delete: true,
    can_submit: false,

    // THE BYTE CHANNEL. Without it the engine sends metadata only and a
    // "mirrored" file arrives at Drive as a name with no content — which is
    // what this adapter did for as long as its upload path read
    // `params.content` as a string the engine has never sent.
    //
    // Declaring it also changes when a create is ATTEMPTED: the engine defers
    // any create whose node has no bytes yet (`write::content::content_pending`)
    // rather than minting an empty file the next walk would call synced.
    accepts_content: true,

    // What a local edit may push. Drive files are content plus one writable
    // piece of metadata worth mirroring — the name. The node property is
    // `title`, which is what the default mapper writes; the reverse mapper
    // turns it back into Drive's `name`. Everything else the mapper emits is
    // provider-computed (size, checksums, links, timestamps) and a PATCH
    // carrying it would be rejected or silently ignored.
    mutable_fields: ["title"],

    // A locally-created FOLDER becomes a real Drive folder (`opCreate`'s folder
    // branch, and the default mapper's `to_external` emits the folder mime type
    // on a create so the mapper — not the adapter — stays the authority on what
    // a node translates to). This flag is what makes the engine offer
    // raisin:Folder as a creatable type at all.
    //
    // KNOWN GAP, stated rather than papered over: the engine's create drain
    // defers every candidate whose node carries no file bytes while
    // `accepts_content` is declared, and a folder never carries any — so a
    // folder create is currently issued only by a mount that does not take
    // content. Nothing here throws when the engine does ask; the branch is real.
    // The deferral is `write/content.rs::content_pending`, and it is the same
    // for the ms-graph adapter.

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

// ---- the write receipt -----------------------------------------------------

/**
 * The `{ external_id, etag }` the engine stamps back onto the node.
 *
 * THE ETAG MUST BE THE ONE THE NEXT WALK COMPUTES for the post-write state. The
 * read path skips an item only when its etag matches the stored one, so a
 * receipt carrying anything else makes the run FOLLOWING this push mismatch its
 * own write, rebuild the node from remote and reseed `__pushed_state` — silently
 * reverting whatever was edited while the push was in flight. So it is derived
 * with `toExternalItem`'s formula (`version`, falling back to `modifiedTime`)
 * and never from a response header, which the walk never sees.
 *
 * A null etag is not merely imprecise: the engine falls back to the STALE
 * pre-write value, which is the same clobber one step later. Callers that can
 * end up here without one read the file back instead.
 */
function writeReceipt(body, fallbackId) {
  body = body && typeof body === "object" ? body : {};
  return {
    external_id: body.id || fallbackId || null,
    etag: body.version != null ? String(body.version) : body.modifiedTime || null,
  };
}

// ---- create ----------------------------------------------------------------

/**
 * The Drive metadata body for a create or an update.
 *
 * The MAPPER's payload is the base — it is the authorized translator between
 * node shape and provider shape, and a mount pointed at a custom mapper must be
 * able to decide the remote fields without forking this adapter.
 *
 * `is_folder` is stripped because it is the ENGINE's vocabulary, not Drive's:
 * Google rejects an unknown field in the resource body outright ("Invalid JSON
 * payload received. Unknown name"), so passing the mapper's own folder flag
 * through would 400 the very create it was meant to describe.
 */
function driveMetadata(payload, name, parent) {
  var metadata = {};
  if (payload && typeof payload === "object") {
    for (var k in payload) {
      if (k === "is_folder") continue;
      metadata[k] = payload[k];
    }
  }
  if (name) metadata.name = name;
  if (parent) metadata.parents = [parent];
  return metadata;
}

/**
 * The name to create under.
 *
 * The mapper's `payload.name` wins, `content.name` (the engine's own echo of the
 * node's file Resource) is the fallback, and the last segment of
 * `relative_path` is the last resort — that one exists because the engine always
 * sends a relative_path and a mapper that emits only metadata would otherwise
 * leave a create with no name at all.
 */
function targetName(params) {
  var payload = params.payload || {};
  var content = params.content || {};
  if (typeof payload.name === "string" && payload.name) return payload.name;
  if (typeof content.name === "string" && content.name) return content.name;
  if (typeof params.relative_path === "string" && params.relative_path) {
    var parts = params.relative_path.split("/");
    var last = parts[parts.length - 1];
    if (last) return last;
  }
  return null;
}

/**
 * WHICH folder a create files into.
 *
 * `parent_external_id` is the node's OWN parent folder and wins whenever the
 * engine could resolve one: a file authored under `Gründung/` belongs in that
 * folder, and creating it at the top of the mount instead is wrong in a way that
 * looks like success — the next walk then re-places the local node at the root
 * to match, so the mistake propagates back and reads as "sync moved my file".
 *
 * `parent_id` (the mount's own remote root) is the fallback, and the right
 * answer for a node sitting directly under the mount path or whose parent folder
 * has no provider id yet. `null` means My Drive's root, which is where Drive
 * files a create that names no parents.
 */
function createParent(mount, params) {
  if (typeof params.parent_external_id === "string" && params.parent_external_id) {
    return params.parent_external_id;
  }
  if (typeof params.parent_id === "string" && params.parent_id) return params.parent_id;
  var root = mount && mount.remote_root;
  return typeof root === "string" && root ? root : null;
}

/**
 * Folder, or file?
 *
 * The MAPPER is the authority: `mimeType` is Drive's own answer and `is_folder`
 * the engine's spelling of it, and either settles the question. An explicit
 * non-folder mime type also settles it the other way, which is what lets a
 * Google-native document (a Doc, a Sheet) be created — those have no bytes at
 * all and would otherwise be mistaken for folders by the fallback.
 *
 * The fallback — no bytes means a folder — is only trustworthy because this
 * adapter declares `accepts_content`: the engine then DEFERS a create whose
 * content has not arrived, so "no content here" means "this node has none",
 * not "not yet".
 */
function createKind(params) {
  var payload = params.payload || {};
  if (payload.mimeType === FOLDER_MIME || payload.is_folder === true) return "folder";
  if (typeof payload.mimeType === "string" && payload.mimeType) return "file";
  return params.content ? "file" : "folder";
}

/**
 * Create one object at the provider.
 *
 * `params` is what the write drain actually sends — `{ payload, parent_id,
 * parent_external_id, relative_path, content }` — and NOT the
 * `{ name, is_folder, mime_type, content-as-a-string }` this function used to
 * read. Every one of those keys was absent on every real call, so the name was
 * `undefined`, the folder branch never ran, and `parents` was built from a
 * `parent_id` that is only half the answer.
 *
 * NOTE ON COLLISIONS: Drive permits two siblings with the same name and has no
 * `conflictBehavior`, so a create never overwrites a stranger's file the way a
 * Graph `replace` would. The cost is the opposite failure — a create retried
 * after a receipt was lost leaves a duplicate — which is why the engine refuses
 * to adopt a node without an id rather than guessing one.
 */
function opCreate(credential, mount, params) {
  params = params || {};
  var name = targetName(params);
  if (!name) {
    throw coded(
      "create: no name — the mapper emitted no payload.name, the engine sent no " +
        "content.name, and relative_path was empty. Drive has no nameless file.",
      "config_error"
    );
  }
  var metadata = driveMetadata(params.payload, name, createParent(mount, params));

  if (createKind(params) === "folder") {
    metadata.mimeType = FOLDER_MIME;
    var folder = driveFetch(
      credential,
      "POST",
      DRIVE + "/files?fields=" + enc(FILE_FIELDS) + "&supportsAllDrives=true",
      {
        headers: { "Content-Type": "application/json" },
        body: metadata,
        context: "create(folder)",
        write: true,
      }
    );
    return requireId(writeReceipt(folder.body, null), "create", folder.status);
  }

  // No bytes: a metadata-only create, which is how a Google-native document is
  // made (its mimeType IS the whole request) and the only shape left for a mount
  // whose adapter was handed no content.
  if (!params.content) {
    var file = driveFetch(
      credential,
      "POST",
      DRIVE + "/files?fields=" + enc(FILE_FIELDS) + "&supportsAllDrives=true",
      {
        headers: { "Content-Type": "application/json" },
        body: metadata,
        context: "create(file)",
        write: true,
      }
    );
    return requireId(writeReceipt(file.body, null), "create", file.status);
  }

  return beginUpload(
    credential,
    "POST",
    UPLOAD + "/files?uploadType=resumable&fields=" + enc(FILE_FIELDS) +
      "&supportsAllDrives=true",
    metadata,
    "create:resumable"
  );
}

// The engine refuses to adopt a node without a real id, and it is right to: a
// fabricated one makes the node unmatchable and undeletable, and the next
// reconcile creates a SECOND copy at the provider. So an id-less 2xx is named
// here rather than passed on as a null.
function requireId(receipt, context, status) {
  if (!receipt.external_id) {
    throw coded(
      context + ": Google accepted the request (HTTP " + status + ") but returned no " +
        "file id, so the new file cannot be matched to its node",
      "transient"
    );
  }
  return receipt;
}

// ---- the byte channel ------------------------------------------------------

/**
 * Open a resumable upload session and hand the ENGINE the URL to stream to.
 *
 * Why every content write goes this way, small files included: `raisin.http.fetch`
 * sends raw bytes only as a whole `bodyBase64` body, so a multipart/related
 * envelope (JSON metadata part + binary part) cannot be assembled here at all —
 * concatenating base64 fragments is not base64. The alternatives were a
 * metadata POST followed by a `uploadType=media` PATCH, which leaves an empty
 * file at the provider and an unadoptable orphan whenever the second call fails,
 * or this: ONE call that either yields a session or fails having created
 * nothing.
 *
 * `headers` is deliberately ABSENT from the answer. The session URL carries its
 * own `upload_id` and is pre-authenticated; attaching our bearer token would put
 * a Google credential on a URL this adapter does not otherwise talk to, for no
 * benefit.
 *
 * The metadata travels IN the initiation body, so a "renamed and re-uploaded"
 * push is one request and cannot half-apply. The initiation URL's query string
 * (`fields`) is replayed on the session's final response, which is what makes
 * `version` — the etag the walk computes — available to `finalize_upload`.
 */
function beginUpload(credential, method, url, metadata, context) {
  var resp = driveFetch(credential, method, url, {
    headers: { "Content-Type": "application/json; charset=UTF-8" },
    body: metadata,
    context: context,
    write: true,
  });
  var session = headerValue(resp.headers, "Location");
  if (!session) {
    throw coded(
      context + ": Google opened no resumable session (HTTP " + resp.status +
        ", no Location header), so there is nowhere to send the bytes",
      "transient"
    );
  }
  return {
    upload: {
      url: session,
      method: "PUT",
      chunk_size: UPLOAD_CHUNK_SIZE,
      continue_statuses: UPLOAD_CONTINUE_STATUSES,
    },
  };
}

/**
 * The second half of an engine-streamed upload: `{ status, body, headers,
 * intent, item_id }`, where `body` is Drive's parsed answer to the LAST chunk.
 *
 * This call exists so provider-shaped parsing stays in the adapter — the engine
 * moved the bytes and must not also learn that a Drive file keeps its id in `id`
 * and its concurrency token in `version`.
 *
 * `headers` is read only as a last resort. Drive answers a completed session
 * with the file resource as JSON, so unlike S3's `PutObject` there is a body to
 * read; the header path is here because the engine now supplies it and a bodiless
 * 200 would otherwise stamp a null etag, which falls back to the STALE pre-write
 * value and lets the next walk overwrite this upload.
 */
function opFinalizeUpload(credential, mount, params) {
  params = params || {};
  var status = Number(params.status);
  var what = params.intent === "update" ? "update" : "create";
  if (!isFinite(status)) {
    throw coded(
      "finalize_upload: no HTTP status for the completed upload (" + what + ")",
      "config_error"
    );
  }
  // Non-2xx keeps the shared taxonomy, so a 401 at the end of an upload is still
  // auth_expired and a 429 is still rate_limited. A 308 cannot reach here — the
  // engine fails a non-2xx FINAL chunk itself — but if it ever did, this is
  // where it stops, because a 308 means Drive does not consider the file written.
  if (status < 200 || status >= 300) {
    raiseForStatus(
      { status: status, headers: params.headers || {}, body: params.body || {} },
      "finalize_upload",
      true
    );
  }

  var body = params.body && typeof params.body === "object" ? params.body : {};
  if (!body.id) {
    throw coded(
      "finalize_upload: the upload session reported success (HTTP " + status + ") for " +
        "this " + what + " but returned no file id, so the file cannot be matched to " +
        "its node",
      "transient"
    );
  }
  var receipt = writeReceipt(body, params.item_id || null);
  if (receipt.etag) return receipt;
  // A read-back rather than a null etag, for the reason `writeReceipt` states:
  // the engine would otherwise stamp the pre-write value and the next walk would
  // clobber the bytes this upload just stored. It goes through `opGet` so the
  // etag is byte-identical to the one the next walk computes.
  var item = opGet(credential, mount, { item_id: body.id });
  if (item) return { external_id: item.external_id, etag: item.etag };
  return receipt;
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

/**
 * Update one file: metadata, bytes, or both.
 *
 * `params` is `{ item_id, payload, fields, etag, content? }`. The payload is the
 * mount mapper's `to_external` output — already provider-shaped and already
 * narrowed to the mount's field allow-list — and it is the only source of
 * metadata: the `params.name` / `params.mime_type` this function used to merge
 * are not keys the write drain has ever sent, and reading them made the code
 * look like it supported a shape it never received.
 */
function opUpdate(credential, mount, params) {
  params = params || {};
  if (!params.item_id) {
    throw coded("update: params.item_id is required", "config_error");
  }
  // A vanished file SETTLES the node rather than failing it: the delta feed
  // reports the deletion and the engine removes the node on its own schedule.
  if (checkVersion(credential, params.item_id, params.etag, "update") === "gone") {
    return null;
  }

  var metadata = driveMetadata(params.payload, null, null);

  if (params.content) {
    // The metadata rides IN the session initiation body, so a push that both
    // renames and re-uploads is ONE request and cannot half-apply. An empty
    // metadata object is fine here — the bytes are the point of the call.
    return beginUpload(
      credential,
      "PATCH",
      UPLOAD + "/files/" + enc(params.item_id) + "?uploadType=resumable&fields=" +
        enc(FILE_FIELDS) + "&supportsAllDrives=true",
      metadata,
      "update:resumable"
    );
  }

  if (isEmptyObject(metadata)) {
    // An empty PATCH still bumps the file's `version`, which invalidates every
    // stored etag and makes the next delta re-deliver the file for no reason —
    // and on a mirror that is a revision per file per drain, forever.
    throw coded("update: refusing an empty PATCH body", "config_error");
  }

  var resp = driveFetch(
    credential,
    "PATCH",
    DRIVE + "/files/" + enc(params.item_id) + "?fields=" + enc(FILE_FIELDS) +
      "&supportsAllDrives=true",
    {
      headers: { "Content-Type": "application/json" },
      body: metadata,
      context: "update",
      write: true,
    }
  );
  return writeReceipt(resp.body, params.item_id);
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
        // A delete IS a write, and it is the write most likely to be the first
        // one a newly writable mount issues. Without this flag its 403 misses
        // the missing-write-scope branch in `raiseForStatus` and comes back a
        // plain Error, i.e. Transient — the same doomed request re-sent on every
        // drain forever, against a mount whose operator was never told which
        // scope is missing. Create and update already say so; delete did not.
        write: true,
      }
    );
    if (patched.status === 404) return { deleted: true, trashed: true };
    raiseForStatus(patched, "delete(trash)", true);
    return { deleted: true, trashed: true };
  }

  var resp = driveFetch(
    credential,
    "DELETE",
    DRIVE + "/files/" + enc(params.item_id) + "?supportsAllDrives=true",
    // `write: true` for the reason the trash branch states: a 403 here is a
    // missing write scope, not a transient fault.
    { context: "delete", rawStatusOk: true, write: true }
  );
  // Already-absent items delete idempotently.
  if (resp.status === 404) return { deleted: true };
  raiseForStatus(resp, "delete", true);
  return { deleted: true };
}

// ---- where a delta item lands, relative to the mount root ------------------
//
// ONLY the delta feed needs this. The full walk recurses folder by folder and
// the ENGINE accumulates the prefix as it descends (`full.rs`
// `resolve_item_path`: `{prefix}/{item.name}`, where the prefix is the parent
// folder's own resolved path). The changes feed has no recursion, so the path
// has to be reconstructed — and it has to come out IDENTICAL to the walk's, or
// the same file sits in one place after a backfill and another after a delta.
//
// That is what this replaces: `relative_path: item.name`, flat. A file two
// folders deep was delivered by the walk at `a/b/report.pdf` and by every delta
// at `report.pdf`, so the engine's remap MOVED the node on every disagreeing
// run — out of its folder on a delta, back into it on the next full reconcile,
// forever. Not cosmetic: a node move rewrites its path and everything that
// referenced the old one.
//
// TWO WAYS DRIVE DIFFERS FROM GRAPH, both handled here:
//
//  * The changes feed is ACCOUNT-WIDE. It reports every file the account can
//    see, not the mount's subtree, so the parent walk is also the SUBTREE
//    FILTER: an item whose ancestry never reaches the mount root returns null
//    and is dropped from the page. Nothing else keeps a stranger's file out of
//    the mount, and the engine joins `relative_path` to `mount_path` verbatim.
//
//  * Drive is a DAG, not a tree: a legacy file can have SEVERAL parents. The
//    rule is the FIRST parent (in Drive's own order) whose chain reaches the
//    mount root — deterministic, and the cheapest walk. A file with two parents
//    INSIDE one mount is genuinely ambiguous and the full walk is ambiguous
//    about it too (it lists the file under both folders and the materializer
//    keeps whichever it saw last), so no choice here can be "correct"; what
//    matters is that it is stable between runs. Drive has allowed only one
//    parent for files created since September 2020, so this is a legacy shape.
//
// Costs one `files.get` per ANCESTOR FOLDER not already seen, cached for the
// whole `get_changes` call — a page of siblings resolves its folder chain once.

var MAX_PARENT_DEPTH = 64;

function newPathCache() {
  return { meta: {}, rootId: undefined };
}

// One folder's `{id, name, parents}`, cached. `null` means "gone or not
// readable", which the caller treats as a chain that cannot be followed rather
// than as the root.
function fileMeta(credential, cache, id) {
  if (Object.prototype.hasOwnProperty.call(cache.meta, id)) return cache.meta[id];
  var resp = driveFetch(
    credential,
    "GET",
    DRIVE + "/files/" + enc(id) + "?fields=" + enc("id,name,parents") +
      "&supportsAllDrives=true",
    { context: "get_changes(parent)", rawStatusOk: true }
  );
  var meta = null;
  if (resp.status !== 404) {
    raiseForStatus(resp, "get_changes(parent)");
    meta = resp.body || null;
  }
  cache.meta[id] = meta;
  return meta;
}

// The folder id every path must terminate at.
//
// `remote_root` when the mount names one. Otherwise the mount is the whole of My
// Drive, and the alias "root" has to be resolved to a real id: `parents` arrays
// never contain the alias, so leaving it unresolved would make every chain walk
// past the top and every item look like it lives outside the mount.
function mountRootId(credential, mount, cache) {
  if (cache.rootId !== undefined) return cache.rootId;
  var configured = mount && mount.remote_root;
  if (typeof configured === "string" && configured && configured !== "root") {
    cache.rootId = configured;
    return cache.rootId;
  }
  var resp = driveFetch(credential, "GET", DRIVE + "/files/root?fields=id", {
    context: "get_changes(root)",
  });
  cache.rootId = (resp.body && resp.body.id) || null;
  return cache.rootId;
}

// The folder names between the mount root and this item, or null when the chain
// never reaches the root.
function chainToRoot(credential, cache, rootId, parents, depth) {
  if (!parents || !parents.length) return null;
  // A malformed or circular parent graph must not spin: bounded, and answered
  // with "outside the mount", which drops the item rather than materializing it
  // somewhere invented.
  if (depth >= MAX_PARENT_DEPTH) return null;
  for (var i = 0; i < parents.length; i++) {
    var pid = parents[i];
    if (pid === rootId) return [];
    var meta = fileMeta(credential, cache, pid);
    if (!meta || !meta.name) continue;
    var up = chainToRoot(credential, cache, rootId, meta.parents, depth + 1);
    if (up !== null) return up.concat([meta.name]);
  }
  return null;
}

/**
 * One changed file's path relative to the mount root, or null to SKIP it.
 *
 * Names are used VERBATIM, exactly as the walk uses `item.name`. Drive permits a
 * "/" inside a file name and neither path survives that intact — but they fail
 * identically, which is the property that matters: an adapter that sanitized
 * here and not in `list` would reintroduce the flip-flop this function exists to
 * remove.
 */
function changeRelativePath(credential, mount, cache, file) {
  var rootId = mountRootId(credential, mount, cache);
  if (!rootId) {
    // Refuse the page rather than emit paths we cannot place. A thrown plain
    // Error is transient: the cursor is not advanced and the changes are
    // re-delivered next run, whereas returning an empty page would advance the
    // token past changes nobody ever saw.
    throw new Error("get_changes: could not resolve the mount root folder id");
  }
  // Drive reports the mount's own folder like any other file. Emitting it would
  // create a folder node standing for the mount, inside itself.
  if (file.id === rootId) return null;
  var chain = chainToRoot(credential, cache, rootId, file.parents, 0);
  if (chain === null) return null;
  return chain.concat([file.name]).join("/");
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
      return opCreate(credential, mount, params);
    case "update":
      return opUpdate(credential, mount, params);
    case "delete":
      return opDelete(credential, params);
    // The engine streamed the bytes itself and is handing back Drive's answer to
    // the final chunk. All that is left is reading a file's id and `version` out
    // of it — provider-shaped parsing, which is why it comes back here rather
    // than being done in Rust.
    case "finalize_upload":
      return opFinalizeUpload(credential, mount, params);
    case "get_changes":
      return opGetChanges(credential, mount, params);
    default:
      throw new Error("Unsupported operation: " + operation);
  }
}
