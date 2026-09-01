// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! What every write shares, whatever surface it addresses: the etag header, the
//! receipt, and the ONE place a status code becomes a write-shaped diagnosis.
//!
//! Extracted from `write.js` when the drive surface arrived, so the Outlook
//! writes (`write.js`) and the drive writes (`drive.js`) can both use it without
//! importing each other — a cycle the QuickJS module loader has no reason to
//! survive. Nothing here knows which surface it is serving; everything that does
//! stays in the caller. It may import `read.js` (for the receipt read-back)
//! because nothing on the read side imports back: read/changes/mount reach only
//! common, http, items and mail.

import { coded } from "./common.js";
import { raiseForStatus } from "./http.js";
import { outlookHeaders } from "./mount.js";
import { opGet } from "./read.js";
import { etagFolderPath, providerEtag, withFolderPath } from "./items.js";

// An etag TOKEN looks like `W/"CQAAABYAAAA..."` or `"abc"`. A stored `__etag`
// does NOT always: `toExternalItem` falls back to `lastModifiedDateTime` when
// Graph sent no `@odata.etag`, so a perfectly healthy mail node can carry an ISO
// timestamp there. Sending that as If-Match is a 400 from Graph, and 400 is
// TERMINAL (config_error) — one such message would mark the whole mount
// misconfigured. So the header is sent only when the stored value has the shape
// of an etag; otherwise the write is last-write-wins, which is what it already
// was before the value was stored.
export var ETAG_SHAPE = /^(W\/)?"/;

// Which OAuth scope a write on this surface needs. Named in every 403 message
// because the diagnosis is the whole value of that branch: 403 on a write is
// almost never a stale token, and sending the operator to reconnect the account
// with unchanged consent costs more than the failure did.
export function writeScopeHint(resource) {
  if (resource === "calendar") {
    return "Calendars.ReadWrite (Calendars.ReadWrite.Shared for a shared calendar)";
  }
  if (resource === "files") {
    return "Files.ReadWrite (Files.ReadWrite.All for another user's OneDrive, " +
      "Sites.ReadWrite.All for a SharePoint document library)";
  }
  return "Mail.ReadWrite (and Mail.ReadWrite.Shared for a shared mailbox)";
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
  if (status === 404 || graphCode === "ErrorItemNotFound" || graphCode === "itemNotFound") {
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
      // The drive spellings of "this ITEM is wrong", not "this mount is wrong":
      // a name that collides, and a file the tenant's malware/DLP scan refused.
      // Both are permanent for the ONE item and irrelevant to every other item
      // in the drive. Graph's broad `invalidRequest` is deliberately NOT here:
      // it covers genuinely mount-fatal mistakes too, and reading one of those
      // as transient retries a request that can never succeed, forever.
      graphCode === "nameAlreadyExists" ||
      graphCode === "malwareDetected" ||
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
//
// `providerEtag` FIRST, and the order is the whole point. A tree-mode mail
// node's stored etag is the provider's with `|p=<folder chain>` appended, and
// that composed string still passes ETAG_SHAPE — it starts `W/"` like any
// other. So the shape test alone happily sent Graph a change key with a folder
// path on the end, which Graph rejects as `The change key is invalid.`: a 400,
// classified terminal, so the edit could never be pushed at all.
//
// Stripping is a no-op for every other surface — drive and calendar never
// compose — so this is done once here rather than at each of the five callers,
// where the one that was forgotten would fail exactly this way again.
export function ifMatch(etag, mount) {
  var headers = { "Content-Type": "application/json" };
  var bare = providerEtag(etag);
  if (typeof bare === "string" && ETAG_SHAPE.test(bare)) {
    headers["If-Match"] = bare;
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
// at the engine to the STALE pre-write stored etag; the callers avoid that by
// reading the item back instead.
export function writeReceipt(resp, fallbackId) {
  var body = resp.body || {};
  return {
    external_id: body.id || fallbackId || null,
    etag: body["@odata.etag"] || body.eTag || body.lastModifiedDateTime || null,
  };
}

// The receipt WITH the read-after-write, which is what every caller actually
// wants: `writeReceipt` alone can answer a null etag, and a null etag falls back
// at the engine to the STALE pre-write value — the next walk then mismatches its
// own write, rebuilds the node from remote and reverts whatever was edited while
// the run was in flight.
//
// `opGet` builds the item through the same $select / `toExternalItem` path the
// walk uses, so the stamped etag is byte-identical to what the next run
// computes. A read-back that finds nothing means the item changed ids between
// the write and the read; the delta feed re-imports it under the new id, and the
// bodiless receipt is the best remaining answer.
//
// `priorEtag` IS THE STORED ETAG THIS WRITE WAS ISSUED AGAINST, and passing it
// is what keeps a tree-mode mail write from reverting itself.
//
// Neither source of a receipt can produce a composed etag on its own: Graph's
// PATCH response carries the bare `@odata.etag`, and the `opGet` read-back goes
// through `toExternalItem` with no `folderPath`. So a tree mount stamped a BARE
// etag while its next walk computed `provider|p=chain` — a guaranteed mismatch,
// every time, which is precisely the "run following this push rebuilds the node
// and reseeds __pushed_state from remote" clobber described above. It reverts
// edits silently instead of failing, so nothing in the logs names it.
//
// The chain is read off `priorEtag` rather than resolved from the provider: a
// mail update is `state_only` (isRead, flag, categories) and cannot move the
// message, so the folder it was in before the PATCH is the folder it is in
// after — and that costs no request. A bare `priorEtag` (folder mode, calendar,
// drive) yields a null path and `withFolderPath` returns the etag untouched.
export function receiptOrReadBack(credential, mount, resp, itemId, priorEtag) {
  var path = etagFolderPath(priorEtag);
  var receipt = writeReceipt(resp, itemId);
  if (!receipt.etag) {
    var item = opGet(credential, mount, { item_id: receipt.external_id || itemId });
    if (item) receipt = { external_id: item.external_id, etag: item.etag };
  }
  return { external_id: receipt.external_id, etag: withFolderPath(receipt.etag, path) };
}
