// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The write operations, dispatched per resource: `update` (mail state +
//! calendar mirror), `create` and `delete` (calendar mirror), and the DRIVE
//! surface, which lives in `drive.js` because it carries bytes and forks on a
//! size limit nothing else has.
//!
//! Every status a write diagnoses differently from a read is claimed in ONE
//! place, `diagnoseWrite` in `write-common.js`, so no operation on any surface
//! can drift on what a 403 or a 412 means. The payload is always whatever the
//! mount's MAPPER produced — the adapter never re-derives the node-to-Graph
//! field mapping.

import { coded, enc, isEmptyObject } from "./common.js";
import { GRAPH, graphFetch } from "./http.js";
import { calendarId, outlookHeaders, principal, resourceOf } from "./mount.js";
import { driveCreate, driveDelete, driveUpdate } from "./drive.js";
import {
  ETAG_SHAPE,
  WRITE_STATUSES,
  diagnoseWrite,
  ifMatch,
  receiptOrReadBack,
  writeReceipt,
  writeScopeHint,
} from "./write-common.js";

// Re-exported so `write.js` stays the one name a caller has to know for "the
// write path", wherever the implementation actually sits.
export {
  ETAG_SHAPE,
  WRITE_STATUSES,
  diagnoseWrite,
  ifMatch,
  receiptOrReadBack,
  writeReceipt,
  writeScopeHint,
};

// ---- update (mail state writeback) ----------------------------------------

// The provider object one write addresses.
//
// MAILBOX-scoped for mail and CALENDAR-scoped for events, never folder-scoped:
// `{principal}/messages/{id}`, not `{principal}/mailFolders/{f}/messages/{id}`.
// A literal `/me` here would write the CONNECTED account while the mount reads a
// shared one, silently, with no error anywhere.
export function writeItemUrl(mount, resource, id) {
  if (resource === "calendar") {
    return GRAPH + principal(mount) + "/events/" + enc(id);
  }
  return GRAPH + principal(mount) + "/messages/" + enc(id);
}

export function opUpdate(credential, mount, params) {
  params = params || {};
  var resource = resourceOf(mount);
  // A driveItem update is a different request in every respect — it may carry
  // bytes, it forks on size, and its receipt can arrive one call later — so it
  // is routed out whole rather than threaded through the Outlook branches.
  if (resource === "files") return driveUpdate(credential, mount, params);
  if (resource !== "mail" && resource !== "calendar") {
    // Terminal on purpose: retrying sends the same unsupported request forever.
    throw coded(
      "update: only the mail, calendar and files resources are writable (this mount " +
        "is '" + resource + "')",
      "config_error"
    );
  }
  if (!params.item_id) {
    throw coded("update: params.item_id is required", "config_error");
  }
  if (isEmptyObject(params.payload)) {
    // An empty PATCH still bumps the message's change key, which invalidates
    // every stored etag and makes the next delta re-deliver the message for no
    // reason. The mapper already returns null rather than emit one; this is the
    // second guard.
    throw coded("update: refusing an empty PATCH body", "config_error");
  }

  var resp = graphFetch(credential, "PATCH", writeItemUrl(mount, resource, params.item_id), {
    headers: ifMatch(params.etag, mount),
    body: params.payload,
    context: "update",
    rawStatuses: WRITE_STATUSES,
  });

  // A 404 settles the node rather than failing it: the item moved and got a new
  // id, and the delta feed will re-import it under that id.
  if (diagnoseWrite(resp, "update", resource) === "gone") return null;

  // Graph answers a PATCH with 200 and the FULL updated object, so the receipt
  // is usually read straight off the response with the walk's own etag formula
  // and no extra request is spent; `receiptOrReadBack` only re-reads when the
  // response yields no etag (a bodiless 2xx from a proxy, or a future Graph
  // behavior change).
  return receiptOrReadBack(credential, mount, resp, params.item_id);
}

// ---- create (calendar mirror) ---------------------------------------------

// POST one new event from the payload the MAPPER produced.
//
// `params` is `{ payload, parent_id }`, where `parent_id` is the mount's
// `remote_root` — the calendar this mount is anchored to. A `files` create is
// routed to `drive.js`, which also receives `content`.
//
// MAIL is still refused, and that is not an oversight: a locally-authored mail
// node is not a message, it is an INTENT to send one, and that is what `submit`
// and the outbox mount are for. POSTing it to /messages would create a silent
// draft nobody sends.
export function opCreate(credential, mount, params) {
  params = params || {};
  var resource = resourceOf(mount);
  if (resource === "files") return driveCreate(credential, mount, params);
  if (resource !== "calendar") {
    throw coded(
      "create: only the calendar and files resources can create objects (this mount " +
        "is '" + resource + "'). A new mail is a `submit` command on an outbox mount, " +
        "not a create.",
      "config_error"
    );
  }
  if (isEmptyObject(params.payload)) {
    throw coded("create: refusing an empty event body", "config_error");
  }

  // `parent_id` is the engine's word for the mount's remote root; falling back
  // to `calendarId(mount)` keeps a default-calendar mount (which has no
  // remote_root at all) working rather than 400ing on an empty id segment.
  var calId =
    typeof params.parent_id === "string" && params.parent_id
      ? params.parent_id
      : calendarId(mount);
  var url = GRAPH + principal(mount) + "/calendars/" + enc(calId) + "/events";

  var resp = graphFetch(credential, "POST", url, {
    headers: outlookHeaders(mount, { "Content-Type": "application/json" }),
    body: params.payload,
    context: "create",
    rawStatuses: WRITE_STATUSES,
  });

  // A 404 on a CREATE is the calendar, not the event — there is no event yet to
  // be gone. Terminal and named, because the recovery ("point remote_root at a
  // calendar that exists") is an operator action, and returning null here would
  // make the engine believe the create succeeded with no id.
  if (diagnoseWrite(resp, "create", resource) === "gone") {
    throw coded(
      "create: calendar '" + calId + "' does not exist or is not accessible to this " +
        "account. Check the mount's remote_root.",
      "config_error"
    );
  }

  // Graph answers 201 with the created event. The id is the whole point of the
  // call — without it the engine cannot adopt the node, and says so loudly — so
  // an absent one is diagnosed here rather than passed on as a null.
  var created = resp.body || {};
  if (!created.id) {
    throw coded(
      "create: Microsoft Graph accepted the event (HTTP " + resp.status +
        ") but returned no id, so the new event cannot be matched to its node",
      "transient"
    );
  }
  return writeReceipt(resp, null);
}

// ---- delete (calendar mirror) ---------------------------------------------

// `params` is `{ item_id, policy, etag }`. The POLICY is the interesting half:
// the engine resolved it from the mount's `delete_policy` and this adapter's
// declared default, and `detach` never arrives here at all (it means "push
// nothing").
//
// Graph's DELETE /events/{id} is a SOFT delete — the event lands in the
// mailbox's Deleted Items and a human can recover it — and Graph offers no
// permanent-delete for an event at all. So `trash` is served exactly and `purge`
// is REFUSED rather than quietly served as a soft delete: an operator who typed
// the one policy the engine will never default to is asking for irreversibility,
// and answering "done" to a request we did not perform is the failure that
// matters here.
export function opDelete(credential, mount, params) {
  params = params || {};
  var resource = resourceOf(mount);
  // Graph's drive DELETE is also a recycle-bin move, so `drive.js` refuses
  // `purge` for the same reason this branch does.
  if (resource === "files") return driveDelete(credential, mount, params);
  if (resource !== "calendar") {
    throw coded(
      "delete: only the calendar and files resources can delete objects (this mount " +
        "is '" + resource + "')",
      "config_error"
    );
  }
  if (!params.item_id) {
    throw coded("delete: params.item_id is required", "config_error");
  }
  if (params.policy === "purge") {
    throw coded(
      "delete: Microsoft Graph has no permanent delete for an event — DELETE moves it " +
        "to Deleted Items, where it is recoverable. Set the mount's delete_policy to " +
        "'trash' (which is what this provider can actually do) or to 'detach'.",
      "config_error"
    );
  }

  var resp = graphFetch(credential, "DELETE", writeItemUrl(mount, resource, params.item_id), {
    headers: ifMatch(params.etag, mount),
    context: "delete",
    rawStatuses: WRITE_STATUSES,
  });

  // Already gone is SUCCESS, not a failure — a delete is the one operation whose
  // desired end state a 404 already satisfies, and treating it as an error would
  // leave the engine retrying forever against an id that can never come back.
  // That is why the "gone" answer is not branched on: both outcomes are done.
  diagnoseWrite(resp, "delete", resource);
  return { external_id: params.item_id, deleted: true };
}
