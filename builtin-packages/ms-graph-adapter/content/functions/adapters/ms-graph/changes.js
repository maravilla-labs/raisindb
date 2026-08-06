// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The incremental feed: the first delta URL, `get_changes`, and the collapse
//! that keeps a recurring series ONE node instead of one per occurrence.

import { enc } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { calendarSupportsDelta, driveContainer, eventSelect, mailFolderId, mailSelect, principal, resourceOf, windowBounds } from "./mount.js";
import { enrichAttachments } from "./mail.js";
import { toExternalItem } from "./items.js";
import { seriesExceptions } from "./read.js";

// Build the FIRST delta URL (no since_token yet). Subsequent calls reuse the
// engine-persisted token verbatim — it is a full @odata.nextLink/deltaLink.
//
// `baselineOnly` asks Graph for a delta link WITHOUT enumerating. This is the
// difference between "import everything from the beginning" and "tell me what
// changes from now on", and getting it wrong is not subtle:
//
// A delta query with no token performs an INITIAL FULL ENUMERATION. Graph
// returns every item in the folder, paged, and only emits @odata.deltaLink on
// the final page. The engine stores whatever comes back as the delta token, so
// page 1 of an enumeration becomes the "baseline" — and every later delta run
// walks that enumeration a page at a time, re-reading items the full walk had
// already imported (`0 written / 600 skipped`, run after run) while genuinely
// new items sit unreachable behind it. On a large mailbox that never converges.
//
// `$deltatoken=latest` (drive: `token=latest`) returns the delta link straight
// away with an empty page. The engine calls this ONLY after a full walk has
// materialized everything, which is exactly when "from now on" is correct.
export function initialDeltaUrl(mount, resource, baselineOnly) {
  if (resource === "calendar") {
    var win = windowBounds(mount);
    // Mailbox-level, NOT `/calendars/{id}/calendarView/delta` — that route is
    // not part of v1.0. `calendarSupportsDelta` is what guarantees we only get
    // here for the primary calendar, so addressing the mailbox is correct.
    return (
      GRAPH + principal(mount) + "/calendarView/delta?startDateTime=" + enc(win.start) +
      "&endDateTime=" + enc(win.end) +
      "&$select=" + enc(eventSelect(mount)) +
      (baselineOnly ? "&$deltatoken=latest" : "")
    );
  }
  if (resource === "files") {
    // Drive spells it differently: `token=latest`, no `$`.
    return GRAPH + driveContainer(mount) + "/delta" +
      (baselineOnly ? "?token=latest" : "");
  }
  return (
    GRAPH + principal(mount) + "/mailFolders/" + enc(mailFolderId(mount)) +
    "/messages/delta?$select=" + enc(mailSelect(mount)) +
    (baselineOnly ? "&$deltatoken=latest" : "")
  );
}

export function opGetChanges(credential, mount, params) {
  var resource = resourceOf(mount);
  var token = params.since_token;
  // Only meaningful when there is no token yet — a stored token already IS a
  // resume point and must be used verbatim.
  var baselineOnly = !token && params.baseline_only === true;
  var url = token || initialDeltaUrl(mount, resource, baselineOnly);
  var resp = graphFetch(credential, "GET", url, { context: "get_changes" });
  var body = resp.body || {};
  var values = body.value || [];
  var items =
    resource === "calendar"
      ? calendarChanges(credential, mount, values)
      : values.map(function (v) {
          if (v["@removed"]) {
            return { type: "deleted", item: { external_id: v.id }, relative_path: v.id };
          }
          var item = toExternalItem(v, resource, mount);
          return { type: "updated", item: item, relative_path: item.external_id };
        });
  if (resource === "mail") {
    enrichAttachments(
      credential,
      mount,
      items.filter(function (c) { return c.type === "updated"; })
           .map(function (c) { return c.item; })
    );
  }
  // Durable, resumable cursor. While paging Graph returns @odata.nextLink; the
  // final page returns @odata.deltaLink. NEVER null: when nothing is new the
  // deltaLink round-trips, and we defensively echo the prior token/url otherwise.
  var next = body["@odata.nextLink"] || body["@odata.deltaLink"] || token || url;
  return { items: items, next_token: next };
}

// ONE NODE PER SERIES, not one per occurrence.
//
// The two calendar paths disagreed about what an item IS. The full walk reads
// `/events`, which returns single instances and SERIES MASTERS — one item per
// series, carrying `recurrence`. The delta path reads `/calendarView/delta`,
// which returns OCCURRENCES AND EXCEPTIONS expanded across the window — one
// item per instance, each with its own id and no `recurrence`. Since a node is
// keyed on the Graph id, a weekly meeting became ~5 nodes and a daily standup
// ~26, all siblings of the series-master node the full walk had already created
// for the same meeting, with nothing relating them.
//
// calendarView/delta is the only delta a v1.0 calendar has, so the fix is to
// collapse its output rather than abandon it: an unmodified OCCURRENCE is
// reported as an update of its `seriesMasterId`, deduped within the page.
//
// An EXCEPTION is NOT collapsed. It is a real divergence from the rule — a
// single occurrence moved, renamed or cancelled — and folding it into the
// master produced an "update" whose properties were byte-identical, so
// rescheduling one occurrence of a weekly meeting changed nothing observable in
// the data. It is emitted as its own item, carrying `seriesMasterId` and
// `originalStart`, which is what lets a consumer subtract that slot from the
// expanded series. Its master is emitted alongside it, so the node holding the
// recurrence rule exists before anything points at it. The full walk emits the
// same set (see `seriesExceptions`), or a reconcile would delete them.
//
// Two consequences worth stating:
//  * A single recurring series changing produces ONE update no matter how many
//    of its occurrences moved, unless those occurrences are exceptions.
//  * The master is fetched only when the page did not already contain it, so
//    the common case (a series edited as a whole) costs no extra request.
export function calendarChanges(credential, mount, values) {
  var out = [];
  var emitted = {};
  var i;

  function emit(v) {
    if (!v || !v.id || emitted[v.id]) return;
    emitted[v.id] = true;
    var item = toExternalItem(v, "calendar", mount);
    out.push({ type: "updated", item: item, relative_path: item.external_id });
  }

  // Series masters present in this page, so an occurrence of one of them needs
  // no extra fetch.
  var mastersInPage = {};
  for (i = 0; i < values.length; i++) {
    if (!values[i]["@removed"] && values[i].type === "seriesMaster") {
      mastersInPage[values[i].id] = values[i];
    }
  }

  for (i = 0; i < values.length; i++) {
    var v = values[i];

    if (v["@removed"]) {
      // A removal from calendarView is NOT necessarily a deletion. Microsoft
      // documents that within a date-bound view, `@removed` also covers events
      // that merely moved OUTSIDE the window — so treating every one as a delete
      // silently destroyed events an operator had only rescheduled. We cannot
      // tell the two apart from the delta payload, and deleting real content is
      // far worse than keeping a stale node, so only a removal we can attribute
      // to a whole series or a standalone event is acted on.
      //
      // A removed OCCURRENCE says nothing about its series: the series is still
      // there, and the next full walk reconciles anything genuinely gone.
      if (v.seriesMasterId) continue;
      out.push({ type: "deleted", item: { external_id: v.id }, relative_path: v.id });
      continue;
    }

    if (v.type === "occurrence" || v.type === "exception") {
      var masterId = v.seriesMasterId;
      if (!masterId) {
        // Shouldn't happen, but an occurrence with no master is better carried
        // through as itself than dropped.
        emit(v);
        continue;
      }
      // The master carries the recurrence rule, so it is emitted whether or not
      // the instance is. Skipped only when this page already emitted it.
      if (!emitted[masterId]) {
        var master = mastersInPage[masterId] || fetchEvent(credential, mount, masterId);
        // A master we cannot read (deleted between pages, or no access) is
        // skipped rather than materialized from the occurrence, which would
        // reintroduce exactly the per-occurrence nodes this exists to prevent.
        if (master) emit(master);
      }
      // The exception itself is the override; a plain occurrence adds nothing
      // the rule does not already say.
      if (v.type === "exception") emit(v);
      continue;
    }

    emit(v);
  }
  return out;
}

// Read one event by id, or null when it is gone. Used to resolve an occurrence
// back to its series master when the delta page did not include it.
export function fetchEvent(credential, mount, eventId) {
  var url = GRAPH + principal(mount) + "/events/" + enc(eventId) +
    "?$select=" + enc(eventSelect(mount));
  var resp = graphFetch(credential, "GET", url, {
    context: "get_changes:series_master",
    rawStatusOk: true,
  });
  if (resp.status === 404) return null;
  raiseForStatus(resp, "get_changes:series_master");
  return resp.body || null;
}
