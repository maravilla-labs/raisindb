// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The read operations: `list`, `get` and `get_content`, plus the series
//! expansion `list` needs to emit one node per series rather than one per
//! occurrence.

import { coded, enc } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { INSTANCE_PAGE, calendarId, driveBase, driveContainer, eventSelect, mailFolderId, mailSelect, pageSize, principal, resourceOf, windowBounds } from "./mount.js";
import { enrichAttachments } from "./mail.js";
import { toExternalItem } from "./items.js";
import { calendarChanges } from "./changes.js";

export function opList(credential, mount, params) {
  var resource = resourceOf(mount);
  var url;
  if (params.cursor) {
    url = params.cursor;
  } else if (resource === "calendar") {
    url =
      GRAPH + principal(mount) + "/calendars/" + enc(calendarId(mount)) +
      "/events?$top=" + pageSize(params) + "&$select=" + enc(eventSelect(mount));
  } else if (resource === "files") {
    // The engine's full walk recurses folders EXPLICITLY: each `list` call
    // names the folder whose children it wants via `params.folder_id` (null
    // for the mount root). Ignoring it — as this branch used to — re-listed
    // the mount ROOT for every queued subfolder: nested content was never
    // enumerated at all, the same root-level folders were rediscovered and
    // re-queued on every pop, and the walk never converged. A flat drive
    // masked it completely, because the first pop IS the root.
    var container = params.folder_id
      ? driveBase(mount) + "/items/" + enc(params.folder_id)
      : driveContainer(mount);
    url = GRAPH + container + "/children?$top=" + pageSize(params);
  } else {
    url =
      GRAPH + principal(mount) + "/mailFolders/" + enc(mailFolderId(mount)) +
      "/messages?$top=" + pageSize(params) + "&$select=" + enc(mailSelect(mount));
  }
  var resp = graphFetch(credential, "GET", url, { context: "list" });
  var body = resp.body || {};
  var values = body.value || [];
  var items = values.map(function (v) {
    return toExternalItem(v, resource, mount);
  });
  if (resource === "calendar") {
    items = items.concat(seriesExceptions(credential, mount, values));
  } else if (resource === "mail") {
    enrichAttachments(credential, mount, items);
  }
  return { items: items, next_cursor: body["@odata.nextLink"] || null };
}

// The full walk's half of "an exception is its own node".
//
// `/events` returns singleInstance and seriesMaster entities only — an
// EXCEPTION (a single occurrence that was moved, renamed or cancelled) is not
// among them. The delta feed does see exceptions. If only the delta emitted
// them, every full reconcile would find those nodes absent from the listing and
// DELETE them, so an exception node would flap in and out of existence on
// alternating runs. Both paths must agree about what an item is.
//
// So each series master in the page is expanded once through `/instances` and
// only its `type === "exception"` instances are kept; plain occurrences stay
// collapsed into the master, exactly as the delta path collapses them. A
// non-recurring calendar makes zero extra requests — the cost is one request
// per SERIES, not per event.
export function seriesExceptions(credential, mount, values) {
  var out = [];
  var win = windowBounds(mount);
  for (var i = 0; i < values.length; i++) {
    var v = values[i];
    if (!v || v.type !== "seriesMaster" || !v.id) continue;
    var url =
      GRAPH + principal(mount) + "/events/" + enc(v.id) + "/instances" +
      "?startDateTime=" + enc(win.start) + "&endDateTime=" + enc(win.end) +
      "&$top=" + INSTANCE_PAGE + "&$select=" + enc(eventSelect(mount));
    var resp = graphFetch(credential, "GET", url, {
      context: "list:series_instances",
      rawStatusOk: true,
    });
    // A master that vanished between the page and this call is not an error:
    // the next run reconciles it. Anything else is reported normally.
    if (resp.status === 404) continue;
    raiseForStatus(resp, "list:series_instances");
    var instances = (resp.body && resp.body.value) || [];
    for (var j = 0; j < instances.length; j++) {
      if (instances[j] && instances[j].type === "exception") {
        out.push(toExternalItem(instances[j], "calendar", mount));
      }
    }
    cancelledOccurrences(credential, mount, v.id, out);
  }
  return out;
}

// How many cancelled instances of ONE series are recovered per walk.
export var MAX_CANCELLED_PER_SERIES = 50;

// A CANCELLED single occurrence, which nothing else in this adapter can see.
//
// Graph removes a cancelled instance from `/instances` entirely — it is not
// returned as a cancelled exception — and the delta feed reports it only as an
// `@removed` entry carrying a `seriesMasterId`, which `calendarChanges` refuses
// to act on (correctly: a removal from a bounded calendarView also covers an
// event that merely moved out of the window, so treating one as a delete
// destroys rescheduled events). The result was that a cancellation produced NO
// item at all, in either path.
//
// That is not a cosmetic gap. The expander suppresses a projected occurrence
// only when an exception node exists at that slot (`calendar_expand/rebuild.rs`
// builds the suppression set from `recurrence_type: exception` +
// `original_start_utc`). With no node, it regenerates the meeting on every
// rebuild — a cancelled standup showing up every week, forever, with no error
// anywhere.
//
// `cancelledOccurrences` on the series master is the documented way to see
// them. It is read through its own request, and every failure degrades to the
// previous behaviour rather than failing the walk: a Graph that does not answer
// it must not be able to take the whole calendar listing down.
export function cancelledOccurrences(credential, mount, masterId, out) {
  var probe = graphFetch(
    credential,
    "GET",
    GRAPH + principal(mount) + "/events/" + enc(masterId) +
      "?$select=id,cancelledOccurrences",
    { context: "list:cancelled_occurrences", rawStatusOk: true }
  );
  if (probe.status < 200 || probe.status >= 300) return;
  var ids = (probe.body && probe.body.cancelledOccurrences) || [];
  if (!ids.length) return;

  var limit = Math.min(ids.length, MAX_CANCELLED_PER_SERIES);
  for (var k = 0; k < limit; k++) {
    // The occurrence id is `OID.{master}.{date}` — a DATE, not the instant the
    // suppression is keyed by. Only the occurrence resource itself carries
    // `originalStart`, which Graph states in UTC, so it is fetched rather than
    // derived: this sandbox has no tz database and a wrong instant suppresses
    // the wrong slot, which is worse than suppressing none.
    var one = graphFetch(
      credential,
      "GET",
      GRAPH + principal(mount) + "/events/" + enc(ids[k]) +
        "?$select=" + enc(eventSelect(mount)),
      { context: "list:cancelled_occurrence", rawStatusOk: true }
    );
    if (one.status < 200 || one.status >= 300) continue;
    var ev = one.body;
    if (!ev || !ev.id || !ev.originalStart) continue;
    // Stated as an EXCEPTION, whatever Graph calls it. A cancelled instance is
    // an override of one slot of the series, which is exactly what an exception
    // is; typed as an `occurrence` it would instead look like one of the
    // expander's own projection nodes and be overwritten by the next rebuild.
    ev.type = "exception";
    ev.isCancelled = true;
    if (!ev.seriesMasterId) ev.seriesMasterId = masterId;
    out.push(toExternalItem(ev, "calendar", mount));
  }
}

export function opGet(credential, mount, params) {
  var resource = resourceOf(mount);
  if (!params.item_id) return null;
  var url;
  if (resource === "calendar") {
    url =
      GRAPH + principal(mount) + "/events/" + enc(params.item_id) +
      "?$select=" + enc(eventSelect(mount));
  } else if (resource === "files") {
    url = GRAPH + driveBase(mount) + "/items/" + enc(params.item_id);
  } else {
    url =
      GRAPH + principal(mount) + "/messages/" + enc(params.item_id) +
      "?$select=" + enc(mailSelect(mount));
  }
  var resp = graphFetch(credential, "GET", url, { context: "get", rawStatusOk: true });
  if (resp.status === 404) return null;
  raiseForStatus(resp, "get");
  return toExternalItem(resp.body, resource, mount);
}

// Message/event body (or file bytes) on demand. Not called during ordinary
// link-only sync. For files, Graph's /content 302-redirects to a per-item
// download host that the adapter network policy does NOT allow-list, so opt-in
// file content sync may be blocked — link via metadata.download_url instead.
export function opGetContent(credential, mount, params) {
  var resource = resourceOf(mount);
  if (resource === "files") {
    // REFUSED, not attempted. Two independent reasons this path cannot return
    // correct bytes today, and both fail silently rather than loudly:
    // `raisin.http.fetch` decodes every response as TEXT, so any binary file
    // (image, PDF, zip) is corrupted before the adapter even sees it — the
    // exact failure the mail-attachment branch below documents; and Graph's
    // `/content` 302-redirects to a per-item download host outside the
    // adapter's network allow-list. Storing corrupted bytes that read as
    // "fetched" is strictly worse than saying no; consumers should fetch via
    // the item's `download_url` metadata (re-read it via `get` first — the
    // pre-authenticated URL expires within about an hour).
    throw coded(
      "get_content: drive file content sync is not supported by this adapter yet " +
        "(binary-safe fetch is not available; use the download_url from a fresh `get`)",
      "config_error"
    );
  }
  // A mail ATTACHMENT: `parent_item_id` is the message, `item_id` the
  // attachment. Graph has no route that addresses an attachment on its own, so
  // the engine sends both halves of the namespaced `__external_id`.
  //
  // Returned as `content_base64`, never `content`: a JS string cannot hold
  // arbitrary bytes, and a PDF round-tripped through one comes back corrupted
  // with no error anywhere. `contentBytes` is already base64 on the wire, so
  // this is a pass-through, not an encode.
  if (resource === "mail" && params.parent_item_id) {
    var att = graphFetch(
      credential,
      "GET",
      GRAPH + principal(mount) + "/messages/" + enc(params.parent_item_id) +
        "/attachments/" + enc(params.item_id),
      { context: "get_content(attachment)" }
    ).body;
    if (!att || typeof att.contentBytes !== "string") {
      // A referenceAttachment / itemAttachment carries no bytes at all. Saying
      // so is better than storing an empty file that reads as "fetched".
      throw coded(
        "get_content: attachment " + params.item_id + " has no downloadable content " +
          "(type " + (att && att["@odata.type"]) + ")",
        "config_error"
      );
    }
    return {
      content_base64: att.contentBytes,
      mime_type: att.contentType || "application/octet-stream",
    };
  }

  var base =
    resource === "calendar"
      ? GRAPH + principal(mount) + "/events/" + enc(params.item_id)
      : GRAPH + principal(mount) + "/messages/" + enc(params.item_id);
  var resp2 = graphFetch(credential, "GET", base + "?$select=body", {
    context: "get_content",
  });
  var b = resp2.body && resp2.body.body;
  var mime = b && b.contentType === "html" ? "text/html" : "text/plain";
  return { content: b ? b.content || "" : "", mime_type: mime };
}
