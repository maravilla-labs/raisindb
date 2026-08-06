// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The read operations: `list`, `get` and `get_content`.

import { CAL, calFetch, enc, raiseForStatus } from "./http.js";
import { calendarId, syncWindow } from "./mount.js";
import { toExternalItem } from "./items.js";

export function opList(credential, mount, params) {
  var calId = calendarId(mount);
  var win = syncWindow(mount);
  var pageSize =
    params.limit && params.limit > 0 ? Math.min(params.limit, 2500) : 250;
  var url =
    CAL +
    "/calendars/" +
    enc(calId) +
    // No singleEvents: masters and exceptions, never expanded occurrences.
    // orderBy=startTime goes with it — Google only accepts that ordering when
    // singleEvents is true, so leaving it in is a hard 400 on every list.
    // showDeleted, to match the delta feed. A CANCELLED INSTANCE of a recurring
    // series is only ever reported as a deleted record, and it is the sole
    // evidence that the occurrence does not happen: the expander suppresses a
    // generated occurrence only when an exception node exists at that slot
    // (`calendar_expand/rebuild.rs`). Without it the full walk cannot see the
    // cancellation, classifies the exception node as "not seen upstream" and
    // prunes it — after which the expander regenerates the meeting that was
    // called off. The two phases then oscillate, ghost and no ghost, depending
    // on which ran last.
    "/events?showDeleted=true&timeMin=" +
    enc(win.timeMin) +
    "&timeMax=" +
    enc(win.timeMax) +
    "&maxResults=" +
    pageSize;
  if (params.cursor) url += "&pageToken=" + enc(params.cursor);

  var resp = calFetch(credential, "GET", url, { context: "list" });
  var body = resp.body || {};
  var events = body.items || [];
  var items = [];
  for (var i = 0; i < events.length; i++) {
    var ev = events[i];
    // `showDeleted` also returns genuinely deleted single events and deleted
    // masters. Those must NOT be materialized — a full reconcile deletes what it
    // does not list, which is exactly the right treatment for them, and
    // returning them as items would resurrect every deleted event in the window
    // as a node with status: cancelled.
    //
    // An instance cancellation is the opposite case and is told apart by
    // `recurringEventId`: there is no node to leave alone, there is one to
    // CREATE, because the exception node IS the suppression record.
    if (ev && ev.status === "cancelled" && !ev.recurringEventId) continue;
    items.push(toExternalItem(ev, calId, mount));
  }
  return { items: items, next_cursor: body.nextPageToken || null };
}

export function opGet(credential, mount, params) {
  var calId = calendarId(mount);
  // Events are keyed by id; relative_path is that same id, so path resolves to
  // an item_id lookup either way.
  var eventId = params.item_id || params.path;
  if (!eventId) return null;
  eventId = String(eventId).replace(/^\/+/, "");
  var url =
    CAL + "/calendars/" + enc(calId) + "/events/" + enc(eventId);
  var resp = calFetch(credential, "GET", url, {
    context: "get",
    rawStatusOk: true,
  });
  if (resp.status === 404 || resp.status === 410) return null;
  raiseForStatus(resp, "get");
  var ev = resp.body;
  if (!ev || ev.status === "cancelled") return null;
  return toExternalItem(ev, calId, mount);
}

// Events carry no binary payload; content sync returns the event resource as a
// JSON document so opt-in content mounts still receive something meaningful.
export function opGetContent(credential, mount, params) {
  var calId = calendarId(mount);
  var eventId = String(params.item_id || "").replace(/^\/+/, "");
  var url = CAL + "/calendars/" + enc(calId) + "/events/" + enc(eventId);
  var resp = calFetch(credential, "GET", url, {
    context: "get_content",
    rawStatusOk: true,
  });
  if (resp.status === 404 || resp.status === 410) return null;
  raiseForStatus(resp, "get_content");
  return { content: JSON.stringify(resp.body), mime_type: "application/json" };
}
