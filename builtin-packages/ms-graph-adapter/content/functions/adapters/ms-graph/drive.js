// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The OneDrive / SharePoint WRITE path: create, update and delete a driveItem.
//!
//! Separate from `write.js` because a drive write is not shaped like an Outlook
//! one. It carries BYTES, and the byte channel forks on a limit MICROSOFT sets
//! (4 MiB for a simple PUT) rather than on anything the engine or the node
//! knows. Everything common to all three surfaces — the etag header, the
//! receipt, the status taxonomy — lives in `write-common.js` and is shared, so
//! the two write paths cannot drift on what a 403 or a 412 means. The byte half
//! is `drive-upload.js`.
//!
//! THE TWO ANSWERS. A create/update either completes here and returns
//! `{ external_id, etag }`, or it returns `{ upload: { url, chunk_size } }` and
//! the ENGINE streams the bytes in Rust, then calls back with `finalize_upload`.
//! The second shape exists because base64 crosses the QuickJS boundary through
//! JSON.stringify/JSON.parse — three full copies against a 64 MiB function
//! budget — which puts the realistic inline ceiling around 10-15 MB and makes it
//! useless for media. It is the write-side mirror of the read path's
//! `fetch_url`, for the same reason.

import { coded, enc, isEmptyObject } from "./common.js";
import { GRAPH, graphFetch } from "./http.js";
import { driveBase, driveContainer } from "./mount.js";
import { WRITE_STATUSES, diagnoseWrite, ifMatch, writeReceipt } from "./write-common.js";
import { CONFLICT_KEY, beginUpload, inlineBytes, inlineable } from "./drive-upload.js";
import { opGet } from "./read.js";

// ---- addressing -----------------------------------------------------------

function itemUrl(mount, id) {
  return GRAPH + driveBase(mount) + "/items/" + enc(id);
}

// A NOT-YET-EXISTING child, addressed by parent + name: Graph's `:/name:` path
// syntax, which is the only way to name a file that has no id yet.
//
// `parent_id` absent means the mount root, and `driveContainer` already knows
// whether that is `/root` or `/items/{remote_root}` — reimplementing the choice
// here is how the write path and the walk end up rooted at different folders.
// WHICH container a create files into.
//
// `parent_external_id` is the node's OWN parent folder and wins whenever the
// engine could resolve one: a file uploaded into `Gründung` belongs in that
// folder, and creating it at the top of the library instead is wrong in a way
// that looks like success — the walk then re-places the local node at the root
// to match, so the mistake propagates back and reads as "sync moved my file".
//
// `parent_id` (the mount's remote root) is the fallback, and the right answer
// for a node sitting directly under the mount path or whose parent folder does
// not exist at the provider yet.
function createParent(params) {
  return params.parent_external_id || params.parent_id || null;
}

function newChildUrl(mount, parentId, name, suffix) {
  var container = parentId
    ? driveBase(mount) + "/items/" + enc(parentId)
    : driveContainer(mount);
  return GRAPH + container + ":/" + enc(name) + ":" + suffix;
}

// The file name to write under.
//
// The MAPPER's `name` wins: it is the authorized translator between node shape
// and provider shape, and a mount pointed at a custom mapper must be able to
// decide the remote filename without forking the adapter. `content.name` is the
// engine's own echo of the node, and serves as the fallback for a mapper that
// emits no name at all.
function targetName(params) {
  var payload = params.payload || {};
  var content = params.content || {};
  if (typeof payload.name === "string" && payload.name) return payload.name;
  if (typeof content.name === "string" && content.name) return content.name;
  return null;
}

function conflictBehavior(payload, fallback) {
  var v = payload && payload[CONFLICT_KEY];
  return typeof v === "string" && v ? v : fallback;
}

// The rename a payload asks for, if any.
function renameIn(payload) {
  var v = payload && payload.name;
  return typeof v === "string" && v ? v : null;
}

// ---- create ---------------------------------------------------------------

// `params` is `{ payload, parent_id, content }`.
export function driveCreate(credential, mount, params) {
  var content = params.content || null;
  if (!content) {
    // A create with no bytes is a FOLDER create, and this adapter does not
    // implement one (`can_create_folders` stays false). Refusing is the honest
    // answer: POSTing a folder facet here would let a mirror mount push its own
    // tree at the provider while `capabilities` says it cannot.
    throw coded(
      "create: a drive create needs the file's bytes, and none arrived. Creating a " +
        "FOLDER is not implemented (can_create_folders is false) — create the folder " +
        "in OneDrive/SharePoint and let the walk import it.",
      "config_error"
    );
  }
  var name = targetName(params);
  if (!name) {
    throw coded(
      "create: no file name — neither the mapper's payload.name nor content.name was " +
        "set, and a driveItem cannot be created without one",
      "config_error"
    );
  }

  // `rename` rather than `replace`, because the file already at that name may
  // not be ours: a mirror create is a locally-born node meeting a drive full of
  // documents this mount never imported, and `replace` would destroy a
  // stranger's file and report success. Graph answers with the real — possibly
  // renamed — item, the engine adopts THAT id, and the next walk reconciles the
  // node's name to it. The mapper can override per item.
  var behavior = conflictBehavior(params.payload, "rename");

  if (!inlineable(content)) {
    return beginUpload(
      credential, mount,
      newChildUrl(mount, createParent(params), name, "/createUploadSession"),
      name, behavior, null, "create:createUploadSession"
    );
  }

  var url =
    newChildUrl(mount, createParent(params), name, "/content") +
    "?" + CONFLICT_KEY + "=" + enc(behavior);
  var resp = graphFetch(credential, "PUT", url, {
    // The file's OWN media type, not application/json: this request body IS the
    // document, and `bodyBase64` is what makes the host send bytes rather than
    // the base64 text of them.
    headers: { "Content-Type": content.mime_type || "application/octet-stream" },
    bodyBase64: inlineBytes(content, "create"),
    context: "create",
    rawStatuses: WRITE_STATUSES,
  });
  if (diagnoseWrite(resp, "create", "files") === "gone") {
    throw coded(
      "create: the parent folder does not exist or is not accessible to this account. " +
        "Check the mount's remote_root and the parent node's external id.",
      "config_error"
    );
  }
  // The engine refuses to adopt a node without a real id, and it is right to: a
  // fabricated one makes the node unmatchable and undeletable, and the next
  // reconcile creates a SECOND copy at the provider.
  if (!(resp.body || {}).id) {
    throw coded(
      "create: Microsoft Graph accepted the upload (HTTP " + resp.status +
        ") but returned no id, so the new file cannot be matched to its node",
      "transient"
    );
  }
  return writeReceipt(resp, null);
}

// ---- update ---------------------------------------------------------------

// `params` is `{ item_id, payload, fields, etag, content? }`. Three shapes:
// metadata only (a rename — a plain PATCH), metadata + small bytes, and bytes
// too large to inline.
export function driveUpdate(credential, mount, params) {
  if (!params.item_id) {
    throw coded("update: params.item_id is required", "config_error");
  }
  var content = params.content || null;
  if (!content) return metadataUpdate(credential, mount, params);

  var behavior = conflictBehavior(params.payload, "replace");

  if (!inlineable(content)) {
    // The session body carries the rename too, so a "renamed and edited" push is
    // ONE request rather than two and cannot half-apply.
    return beginUpload(
      credential, mount,
      itemUrl(mount, params.item_id) + "/createUploadSession",
      renameIn(params.payload), behavior, params.etag, "update:createUploadSession"
    );
  }

  // A content PUT addresses the item by id and therefore cannot rename it, so a
  // payload that names one is applied first. Dropping it silently is the failure
  // this ordering prevents: the engine baselines `__pushed_state` from the
  // node's own values, not from what the request actually carried, so a dropped
  // rename is recorded as pushed and the two names then diverge permanently.
  if (renameIn(params.payload)) metadataUpdate(credential, mount, params);

  var resp = graphFetch(credential, "PUT", itemUrl(mount, params.item_id) + "/content", {
    headers: { "Content-Type": content.mime_type || "application/octet-stream" },
    bodyBase64: inlineBytes(content, "update"),
    context: "update:content",
    rawStatuses: WRITE_STATUSES,
  });
  // Gone SETTLES the node rather than failing it: the file was deleted at the
  // provider and the next walk prunes the node.
  if (diagnoseWrite(resp, "update:content", "files") === "gone") return null;
  return receiptFor(credential, mount, resp, params.item_id);
}

function metadataUpdate(credential, mount, params) {
  if (isEmptyObject(params.payload)) {
    // An empty PATCH still bumps the item's eTag, which invalidates the stored
    // one and makes the next delta re-deliver the file for no reason.
    throw coded("update: refusing an empty PATCH body", "config_error");
  }
  var resp = graphFetch(credential, "PATCH", itemUrl(mount, params.item_id), {
    headers: ifMatch(params.etag, mount),
    body: params.payload,
    context: "update",
    rawStatuses: WRITE_STATUSES,
  });
  if (diagnoseWrite(resp, "update", "files") === "gone") return null;
  return receiptFor(credential, mount, resp, params.item_id);
}

// The receipt, with the read-after-write the mail path uses for the same reason:
// a null etag falls back at the engine to the STALE pre-write value, and the
// next walk then rebuilds the node from remote — reverting whatever was edited
// while the run was in flight.
function receiptFor(credential, mount, resp, itemId) {
  var receipt = writeReceipt(resp, itemId);
  if (!receipt.etag) {
    var item = opGet(credential, mount, { item_id: receipt.external_id || itemId });
    if (item) return { external_id: item.external_id, etag: item.etag };
  }
  return receipt;
}

// ---- delete ---------------------------------------------------------------

// Graph's `DELETE /items/{id}` moves the item to the drive's RECYCLE BIN, where
// a user restores it — so `trash` is served exactly, and `purge` is refused.
//
// Graph v1.0 exposes no permanent delete for a driveItem: there is no endpoint
// that empties the recycle bin for one item. Serving `purge` as a soft delete
// would answer "irreversibly destroyed" to an operator who typed the one policy
// no layer ever defaults to — and who typed it precisely because they wanted
// irreversibility. Refusing is the only honest answer, and it is the same call
// the calendar branch makes against the same limitation.
export function driveDelete(credential, mount, params) {
  if (!params.item_id) {
    throw coded("delete: params.item_id is required", "config_error");
  }
  if (params.policy === "purge") {
    throw coded(
      "delete: Microsoft Graph v1.0 has no permanent delete for a drive item — DELETE " +
        "moves it to the recycle bin, where it stays restorable. Set the mount's " +
        "delete_policy to 'trash' (which is what this provider can actually do) or to " +
        "'detach'.",
      "config_error"
    );
  }

  var resp = graphFetch(credential, "DELETE", itemUrl(mount, params.item_id), {
    headers: ifMatch(params.etag, mount),
    context: "delete",
    rawStatuses: WRITE_STATUSES,
  });
  // Already gone is SUCCESS: a delete is the one operation whose desired end
  // state a 404 already satisfies.
  diagnoseWrite(resp, "delete", "files");
  return { external_id: params.item_id, deleted: true };
}
