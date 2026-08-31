// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The incremental feed: syncToken baseline and delta.

import { CAL, calFetch, coded, enc, raiseForStatus } from "./http.js";
import { calendarId, syncWindow } from "./mount.js";
import { toExternalItem } from "./items.js";

export function opGetChanges(credential, mount, params) {
  var calId = calendarId(mount);
  var token = params.since_token;

  // Baseline: no prior token. Page a windowed list to the end purely to harvest
  // a nextSyncToken; the engine has already reconciled the initial state, so we
  // report zero changes and hand back the token to drive future deltas.
  if (!token) {
    var win = syncWindow(mount);
    var base =
      CAL +
      "/calendars/" +
      enc(calId) +
      // PARAMETER IDENTITY. Google documents the sync contract as "each list
      // request should use the same set of query parameters, including the
      // initial request", and names the only exemptions in the events.list
      // reference: iCalUID, orderBy, privateExtendedProperty, q,
      // sharedExtendedProperty, timeMin, timeMax, updatedMin. `showDeleted` is
      // NOT one of them, so it has to be here too — the delta below sends it,
      // and the same reference says a syncToken request may not set it to false
      // at all.
      //
      // This was the singleEvents trap a second time (see the 400 branch below):
      // a baseline minted WITHOUT showDeleted and a delta sent WITH it are two
      // different sync identities, so the delta either silently drops the
      // cancellations this whole file exists to carry, or is rejected with a
      // 400 — which the branch below turns into `cursor_invalid`, whereupon the
      // next run mints another mismatched baseline and the mount never gets an
      // incremental feed at all.
      "/events?showDeleted=true&maxResults=2500" +
      "&timeMin=" +
      enc(win.timeMin) +
      "&timeMax=" +
      enc(win.timeMax);
    var syncToken = null;
    var pageToken = null;
    // Walk to the last page; nextSyncToken only appears on the final page.
    for (var guard = 0; guard < 50; guard++) {
      var u = base + (pageToken ? "&pageToken=" + enc(pageToken) : "");
      var r = calFetch(credential, "GET", u, { context: "get_changes(base)" });
      var b = r.body || {};
      if (b.nextSyncToken) {
        syncToken = b.nextSyncToken;
        break;
      }
      if (b.nextPageToken) {
        pageToken = b.nextPageToken;
        continue;
      }
      break;
    }
    return { items: [], next_token: syncToken || "" };
  }

  // Incremental: syncToken sync cannot be combined with timeMin/timeMax/orderBy.
  var url =
    CAL +
    "/calendars/" +
    enc(calId) +
    "/events?showDeleted=true&maxResults=2500" +
    "&syncToken=" +
    enc(token);
  if (params.cursor) url += "&pageToken=" + enc(params.cursor);

  var resp = calFetch(credential, "GET", url, {
    context: "get_changes",
    rawStatusOk: true,
  });
  // 410 GONE → the syncToken expired. `cursor_invalid` is what tells the engine
  // to drop the stored token and full-reconcile IN THE SAME RUN. This used to
  // throw a plain Error, which the engine reads as transient, so it retried the
  // identical rejected token every tick forever — `raiseForStatus` had the right
  // mapping and `rawStatusOk` skipped past it.
  if (resp.status === 410) {
    throw coded(
      "get_changes: Google Calendar syncToken expired (410 GONE); full resync required",
      "cursor_invalid"
    );
  }
  // A syncToken is only valid for a request with the SAME parameters that
  // produced it, and dropping `singleEvents=true` changed them. Every token
  // minted by the previous build is therefore rejected exactly once, with a 400,
  // and reporting that as `cursor_invalid` is what turns a one-time breaking
  // change into a single automatic full reconcile instead of a stuck mount.
  // (A 400 here can have no other cause: the URL is otherwise constant and the
  // token is the only caller-supplied part of it.)
  if (resp.status === 400) {
    throw coded(
      "get_changes: Google Calendar rejected the syncToken (400); it was minted " +
        "with different list parameters — discarding it and full-reconciling",
      "cursor_invalid"
    );
  }
  raiseForStatus(resp, "get_changes");

  var body = resp.body || {};
  var events = body.items || [];
  var items = events.map(function (ev) {
    // A cancelled record with a `recurringEventId` is a cancelled OCCURRENCE of
    // a series, not a deleted event. Reporting it as a delete removes nothing
    // (no node exists for an unmodified occurrence — it is projected from the
    // master) and, worse, discards the only evidence there is: the expander
    // suppresses a generated occurrence solely on the existence of an exception
    // node at that slot. So it is materialized like any other exception,
    // carrying status: cancelled, and the projection loses the ghost.
    if (ev.status === "cancelled" && !ev.recurringEventId) {
      return {
        type: "deleted",
        item: { external_id: ev.id },
        relative_path: ev.id,
      };
    }
    var item = toExternalItem(ev, calId, mount);
    return { type: "updated", item: item, relative_path: item.relative_path };
  });
  // next_token is NEVER null: prefer a fresh nextSyncToken, fall back to the
  // page token while paging, else echo the caller's token so the cursor holds.
  // `has_more` tells the engine explicitly whether this is a mid-enumeration
  // page (nextPageToken: keep paging now) or a caught-up cursor (stop; resume
  // next run) — token identity is not a reliable signal across providers.
  var next = body.nextSyncToken || body.nextPageToken || token;
  return { items: items, next_token: next, has_more: Boolean(body.nextPageToken) };
}
