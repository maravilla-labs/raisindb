/**
 * The WRITE operations: create, update and delete, plus the read-then-compare
 * that stands in for the conditional request Drive does not have.
 *
 * The byte half lives in `drive-upload.js`; the receipt rules in
 * `write-common.js`.
 */

import { coded, enc, isEmptyObject } from "./common.js";
import { DRIVE, UPLOAD, driveFetch, raiseForStatus } from "./http.js";
import { FILE_FIELDS, FOLDER_MIME } from "./items.js";
import { requireId, writeReceipt } from "./write-common.js";
import { beginUpload } from "./drive-upload.js";

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
export function opCreate(credential, mount, params) {
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
export function opUpdate(credential, mount, params) {
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
export function opDelete(credential, params) {
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
