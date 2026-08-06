// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Reading the MOUNT: which calendar it addresses, the sync window, and the
//! body opt-in.

import { DAY_MS } from "./http.js";

export function calendarId(mount) {
  return (mount && mount.remote_root) || "primary";
}

export function syncWindow(mount) {
  var cfg = (mount && mount.sync_config) || {};
  var w = cfg.window || {};
  var daysAhead = w.days_ahead != null ? Number(w.days_ahead) : 90;
  var daysBack = w.days_back != null ? Number(w.days_back) : 7;
  var now = Date.now();
  return {
    timeMin: new Date(now - daysBack * DAY_MS).toISOString(),
    timeMax: new Date(now + daysAhead * DAY_MS).toISOString(),
  };
}

// Normalize a Calendar {dateTime|date} to an ISO string; report all-day and the
// IANA zone Google sends alongside. The zone used to be dropped, which left the
// wall-clock a human agreed to unrecoverable and made recurrence expansion
// ambiguous — "every Tuesday 09:00 Europe/Zurich" is a different instant in
// winter and in summer.
export function whenOf(slot) {
  if (!slot) return { value: null, allDay: false, tz: null };
  var tz = slot.timeZone || null;
  if (slot.dateTime) return { value: slot.dateTime, allDay: false, tz: tz };
  if (slot.date) return { value: slot.date, allDay: true, tz: tz };
  return { value: null, allDay: false, tz: tz };
}

// Whether this mount wants the event description inline (sync_config.include_body).
// Off by default, mirroring the ms-graph mail/calendar opt-in: a description is
// an arbitrarily long HTML document, and a String property's full text lands in
// a property-index key on every revision.
export function includeBody(mount) {
  var cfg = (mount && (mount.sync_config || mount.config)) || {};
  var v = cfg.include_body;
  return v === true || v === "true";
}

