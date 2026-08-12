// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The write operations: `update` (mail state + calendar mirror), `create` and
//! `delete` (calendar mirror).
//!
//! Every status a write diagnoses differently from a read is claimed in ONE
//! place, `diagnoseWrite`, so the three operations cannot drift on what a 403 or
//! a 412 means. The payload is always whatever the mount's MAPPER produced — the
//! adapter never re-derives the node-to-Graph field mapping.

import { coded, enc, isEmptyObject } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { calendarId, outlookHeaders, principal, resourceOf } from "./mount.js";
import { toExternalItem } from "./items.js";
import { opGet } from "./read.js";

// ---- update (mail state writeback) ----------------------------------------

// An etag TOKEN looks like `W/"CQAAABYAAAA..."` or `"abc"`. A stored `__etag`
// does NOT always: `toExternalItem` falls back to `lastModifiedDateTime` when
// Graph sent no `@odata.etag`, so a perfectly healthy mail node can carry an ISO
// timestamp there. Sending that as If-Match is a 400 from Graph, and 400 is
// TERMINAL (config_error) — one such message would mark the whole mount
// misconfigured. So the header is sent only when the stored value has the shape
// of an etag; otherwise the write is last-write-wins, which is what it already
// was before the value was stored.
export var ETAG_SHAPE = /^(W\/)?"/;


//   { item_id, payload, fields, etag }
// `payload` is forwarded VERBATIM. The adapter must not rebuild it from
// `fields`: `fields` are NODE property names and the mapper is the only
// authorized translator between the two vocabularies.
//
// MAILBOX-scoped, exactly like `opGet` — `{principal}/messages/{id}`, never
// `{principal}/mailFolders/{f}/messages/{id}`. A literal `/me` here would patch
// the CONNECTED account's mailbox while the mount reads a shared one, silently,
// with no error anywhere.
// Which OAuth scope a write on this surface needs. Named in every 403 message
// because the diagnosis is the whole value of that branch: 403 on a write is
// almost never a stale token, and sending the operator to reconnect the account
// with unchanged consent costs more than the failure did.
export function writeScopeHint(resource) {
  if (resource === "calendar") {
    return "Calendars.ReadWrite (Calendars.ReadWrite.Shared for a shared calendar)";
  }
  return "Mail.ReadWrite (and Mail.ReadWrite.Shared for a shared mailbox)";
}

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

// Every status a WRITE diagnoses differently from a read, in one place.
//
// Returns "gone" when the object no longer exists and the caller must decide
// what that means (settled, for an update; a completed delete, for a delete);
// throws for everything terminal; falls through to the shared read mapping for
// 401/429/5xx, which is correct for a write too.
export function diagnoseWrite(resp, context, resource) {
  var status = resp.status;
  var body = resp.body || {};
  var err = (body && body.error) || {};
  var graphCode = err.code || "";
  var graphMsg = err.message || "";

  // The object changed remotely since we read it. Resolved by the mount's
  // conflict policy, and NEVER a retry: the retry sends the same stale If-Match
  // and fails identically.
  //
  // The message text is load-bearing: `AdapterError::classify` scans for
  // auth_expired, rate_limited, cursor_invalid, config_error and THEN conflict,
  // so a conflict message containing any earlier token is misclassified.
  if (status === 412 || status === 409 || graphCode === "ErrorIrresolvableConflict") {
    throw coded(
      context + ": the item changed in Microsoft 365 since it was read (HTTP " + status + ")",
      "conflict"
    );
  }

  // GONE, not misconfigured. Graph message ids are NOT stable — a message that
  // moves folders gets a NEW id unless requests carry `Prefer:
  // IdType="ImmutableId"`, which this adapter does not send. A read tolerates
  // that (the delta re-reports the item under its new id); a write against the
  // stale stored id 404s. Left to `raiseForStatus` that is a config_error, i.e.
  // one moved message would permanently mark a healthy mount misconfigured.
  if (status === 404 || graphCode === "ErrorItemNotFound") {
    return "gone";
  }

  // A write-scope shortfall, which is the FIRST thing a new writable mount hits:
  // the connector requests read scopes only, so every read succeeds and every
  // write 403s.
  if (status === 403) {
    throw coded(
      context + ": Microsoft Graph refused the write (403 " + (graphCode || "Forbidden") +
        "). This is almost certainly a missing WRITE scope, not a stale token: add " +
        writeScopeHint(resource) + " to the Microsoft 365 connector's OAuth scopes in " +
        "the console and RECONNECT each account — Microsoft only issues a new scope " +
        "on fresh consent.",
      "config_error"
    );
  }

  // A 400 is TWO different faults wearing one status code, and coding them
  // alike broke the whole drain over one bad item.
  //
  // `config_error` is terminal FOR THE MOUNT: the engine stops the drain and
  // badges it `misconfigured`. That is right when the request could never
  // succeed as written — an unknown property, a resource this mount may not
  // patch — and exactly wrong for a rejection of ONE item's payload. There,
  // every candidate ordered after it was skipped, deterministically, on every
  // run: one malformed event head-of-line blocked the rest of the calendar
  // indefinitely, while `first_error` carried Graph's message with no node id
  // to attribute it to.
  //
  // The codes below name the ITEM, so they must not be mount-fatal. They carry
  // no reserved code, which the engine classifies as transient: the drain
  // continues past them and the item is counted as failed. That does mean a
  // permanently malformed item is re-attempted once per drain — bounded, and a
  // far better trade than blocking every edit behind it.
  if (status === 400) {
    var itemFault =
      graphCode === "ErrorInvalidPropertyValue" ||
      graphCode === "ErrorInvalidIdMalformed" ||
      graphCode === "ErrorInvalidRecipients" ||
      graphCode === "ErrorMessageSizeExceeded" ||
      (typeof graphCode === "string" && graphCode.indexOf("ErrorOccurrence") === 0);
    if (itemFault) {
      throw new Error(
        context + ": Microsoft Graph rejected THIS ITEM (400 " + graphCode + "): " +
          (graphMsg || "no message") + ". Other items in this mount are unaffected."
      );
    }
    throw coded(
      context + ": " + (graphMsg || "Microsoft Graph rejected the request (400)"),
      "config_error"
    );
  }

  // Anything else (401, 429, 5xx, ...) keeps the shared read mapping.
  raiseForStatus(resp, context);
  return null;
}

export var WRITE_STATUSES = [400, 403, 404, 409, 412];

// The etag header, sent only when the stored value has the shape of one.
export function ifMatch(etag, mount) {
  var headers = { "Content-Type": "application/json" };
  if (typeof etag === "string" && ETAG_SHAPE.test(etag)) {
    headers["If-Match"] = etag;
  }
  // The write MUST address the same id space the read paths listed with, or
  // every PATCH against an immutable id 404s (and vice versa). `mount` is
  // optional only so an older caller keeps compiling; always pass it.
  return mount ? outlookHeaders(mount, headers) : headers;
}

// The `{external_id, etag}` receipt the engine stamps back.
//
// THE ETAG MUST BE THE ONE THE NEXT WALK/DELTA COMPUTES for the post-write
// state. The engine's read path skips an item only when its etag matches the
// stored one; a receipt that stamps anything else makes the run FOLLOWING this
// push mismatch its own write, rebuild the node wholesale and reseed
// __pushed_state from remote — silently reverting any edit made while the run
// was in flight, because the read path has no local-wins branch. (This is the
// exact clobber the Hue adapter shipped and had to fix.)
//
// The read paths derive an item's etag as
// `@odata.etag || eTag || lastModifiedDateTime` (items.js `toExternalItem`) —
// and mail items DO land on the ISO fallback in practice — so the receipt
// reads Graph's write response with the SAME formula. The response HEADER etag
// is deliberately not used: the walk never sees headers, so a header-only
// value is a guaranteed mismatch. A null etag (bodiless response) falls back
// at the engine to the STALE pre-write stored etag; `opUpdate` avoids that by
// reading the item back instead.
export function writeReceipt(resp, fallbackId) {
  var body = resp.body || {};
  return {
    external_id: body.id || fallbackId || null,
    etag: body["@odata.etag"] || body.eTag || body.lastModifiedDateTime || null,
  };
}

export function opUpdate(credential, mount, params) {
  params = params || {};
  var resource = resourceOf(mount);
  if (resource !== "mail" && resource !== "calendar") {
    // Terminal on purpose: retrying sends the same unsupported request forever.
    // `files` on this adapter is read-only — a Graph drive write is an upload
    // session, not a PATCH, and declaring it here without implementing it is how
    // a mount resolves to a mode that throws at drain time.
    throw coded(
      "update: only the mail and calendar resources are writable (this mount is '" +
        resource + "')",
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
  // can usually be read straight off the response with the walk's own etag
  // formula — no extra request spent. When the response yields NO etag by that
  // formula (a bodiless 2xx from a proxy or a future Graph behavior change),
  // the receipt comes from a read-after-write through `opGet` instead: it
  // builds the item through the same $select / `toExternalItem` path the walk
  // uses, so the stamped etag is byte-identical to what the next run computes.
  // Returning a null etag here would fall back at the engine to the STALE
  // pre-write etag, and the next walk would clobber the very state this PATCH
  // just pushed.
  var receipt = writeReceipt(resp, params.item_id);
  if (!receipt.etag) {
    var item = opGet(credential, mount, { item_id: receipt.external_id || params.item_id });
    // A read-back 404 means the item changed ids between the PATCH and the
    // read; the delta feed re-imports it under the new id, and the bodiless
    // receipt is the best remaining answer.
    if (item) return { external_id: item.external_id, etag: item.etag };
  }
  return receipt;
}

// ---- create (calendar mirror) ---------------------------------------------

// POST one new event from the payload the MAPPER produced.
//
// `params` is `{ payload, parent_id }`, where `parent_id` is the mount's
// `remote_root` — the calendar this mount is anchored to. Calendar ONLY, and the
// refusal for the other two resources is not an oversight:
//
//   * mail — a locally-authored mail node is not a message, it is an INTENT to
//     send one, and that is what `submit` and the outbox mount are for. POSTing
//     it to /messages would create a silent draft nobody sends.
//   * files — a Graph drive create is an upload session for the BYTES, not a
//     JSON POST, and a file node whose content lives in the binary store has no
//     bytes here to send.
export function opCreate(credential, mount, params) {
  params = params || {};
  var resource = resourceOf(mount);
  if (resource !== "calendar") {
    throw coded(
      "create: only the calendar resource can create objects (this mount is '" +
        resource + "'). A new mail is a `submit` command on an outbox mount, not a create.",
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
  if (resource !== "calendar") {
    throw coded(
      "delete: only the calendar resource can delete objects (this mount is '" +
        resource + "')",
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
