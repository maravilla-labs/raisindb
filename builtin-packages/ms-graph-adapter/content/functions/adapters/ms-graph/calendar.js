// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Calendar READ mapping: Graph's `patternedRecurrence` to RFC 5545 content
//! lines, plus the `metadata` bag one event becomes.
//!
//! The RFC 5545 direction lives here rather than in the mapper on purpose: a
//! custom mapper must not have to understand patternedRecurrence. The INVERSE
//! (RFC 5545 back to a Graph pattern) lives in the ms-graph-calendar mapper,
//! because a write's payload is the mapper's output — see `graphUntil` and its
//! counterpart `untilToEndDate`, which must stay exact inverses.

import { includeBody } from "./mount.js";

// ---- recurrence: patternedRecurrence -> RFC 5545 content lines -------------
//
// The canonical shape of `raisin:Event.recurrence` is an ARRAY of RFC 5545
// content lines — which is Google Calendar's native wire format. Graph is the
// side that has to convert, so it converts HERE rather than in the mapper: a
// custom mapper must not have to understand patternedRecurrence, and the two
// providers must hand the engine the same thing. Before this, the adapter
// carried `JSON.stringify(v.recurrence)`, so one column held a Graph JSON blob
// for one provider and nothing at all for the other (Google's masters were
// never fetched), neither of which was the RRULE the nodetype documents.
//
// Graph's own defaults are honoured rather than emitted: INTERVAL=1, WKST=MO
// and an open-ended range are all RFC 5545 defaults, so writing them would only
// make two equivalent rules compare unequal.

export var GRAPH_DAY = {
  sunday: "SU",
  monday: "MO",
  tuesday: "TU",
  wednesday: "WE",
  thursday: "TH",
  friday: "FR",
  saturday: "SA",
};

// relativeMonthly / relativeYearly index -> BYSETPOS. "last" is -1, which is
// why this cannot be a plain 1..5 mapping.
export var GRAPH_INDEX = { first: 1, second: 2, third: 3, fourth: 4, last: -1 };

export var GRAPH_FREQ = {
  daily: "DAILY",
  weekly: "WEEKLY",
  absoluteMonthly: "MONTHLY",
  relativeMonthly: "MONTHLY",
  absoluteYearly: "YEARLY",
  relativeYearly: "YEARLY",
};

export function graphDays(list) {
  var out = [];
  for (var i = 0; i < (list || []).length; i++) {
    var d = GRAPH_DAY[String(list[i]).toLowerCase()];
    if (d && out.indexOf(d) === -1) out.push(d);
  }
  return out;
}

// "2026-12-31" (Graph range dates are date-only) -> a UTC UNTIL instant.
//
// THE TRAP: both ends are inclusive, so "the end of that day" is the right
// DATE — but RFC 5545 compares UNTIL against the INSTANTS the rule expands to,
// and Graph's endDate is a date in the RANGE'S OWN ZONE, not in UTC. Rendering
// it as `T235959Z` therefore drops the final occurrence of every series whose
// local time is late enough to fall on the next UTC day: 19:00 America/New_York
// on 2026-12-31 is 2027-01-01T00:00:00Z, which is past `20261231T235959Z`, so
// `rrule` silently omits it. Every US-evening and Pacific-afternoon series lost
// its last instance, with the master still showing the correct end date.
//
// This sandbox has no tz database (see the TIME note below — it is the same
// reason `start_utc` is derived in the engine and not here), so the exact
// instant of local midnight cannot be computed. What CAN be computed is a
// cutoff that lies strictly between the last occurrence and the one that would
// follow it: take the series' own local time-of-day on `endDate`, read as UTC,
// and add 12 hours. The last occurrence is at that local time minus the zone
// offset, so the cutoff covers every offset down to UTC-12 — and the next
// occurrence is at least a day later, so nothing extra is admitted for any zone
// east of UTC up to +12.
//
// The residue, stated rather than hidden: a zone at UTC+12 or beyond (Fiji,
// Auckland in summer, Kiritimati) can admit ONE occurrence past the end of a
// DAILY series. That is the wrong direction to err in only for a handful of
// zones, against losing an occurrence for all of the Americas, and it is fixed
// properly the day this adapter can resolve an IANA offset.
export var UNTIL_PAD_MS = 12 * 3600000;

export function pad2(n) {
  return (n < 10 ? "0" : "") + n;
}

export function graphUntil(endDate, localTimeOfDay) {
  var m = typeof endDate === "string" ? /^(\d{4})-(\d{2})-(\d{2})/.exec(endDate) : null;
  if (!m) return null;
  var t =
    typeof localTimeOfDay === "string"
      ? /T(\d{2}):(\d{2}):(\d{2})/.exec(localTimeOfDay)
      : null;
  // No usable local start (an all-day series carries a bare date): end of day
  // is then the whole of the truth there is, and an all-day occurrence starts
  // at local midnight, so the same 12-hour cover applies from 00:00:00.
  var hh = t ? Number(t[1]) : 0;
  var mm = t ? Number(t[2]) : 0;
  var ss = t ? Number(t[3]) : 0;
  var at = new Date(
    Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]), hh, mm, ss) + UNTIL_PAD_MS
  );
  return (
    String(at.getUTCFullYear()) +
    pad2(at.getUTCMonth() + 1) +
    pad2(at.getUTCDate()) +
    "T" +
    pad2(at.getUTCHours()) +
    pad2(at.getUTCMinutes()) +
    pad2(at.getUTCSeconds()) +
    "Z"
  );
}

export function recurrenceLines(recurrence, localStart) {
  if (!recurrence || !recurrence.pattern) return null;
  var p = recurrence.pattern;
  var freq = GRAPH_FREQ[p.type];
  if (!freq) return null;

  var parts = ["FREQ=" + freq];
  if (p.interval && Number(p.interval) !== 1) parts.push("INTERVAL=" + p.interval);

  var days = graphDays(p.daysOfWeek);

  // absoluteYearly/absoluteMonthly carry a day-of-month; the relative forms
  // carry an index + weekdays instead. A pattern never carries both.
  if (freq === "YEARLY" && p.month) parts.push("BYMONTH=" + p.month);
  if (days.length) parts.push("BYDAY=" + days.join(","));
  if (p.dayOfMonth) parts.push("BYMONTHDAY=" + p.dayOfMonth);
  if (days.length && GRAPH_INDEX[p.index] !== undefined) {
    parts.push("BYSETPOS=" + GRAPH_INDEX[p.index]);
  }
  // Only weekly recurrence is week-boundary sensitive, and only a non-default
  // first day changes any expansion.
  if (freq === "WEEKLY" && p.firstDayOfWeek) {
    var wkst = GRAPH_DAY[String(p.firstDayOfWeek).toLowerCase()];
    if (wkst && wkst !== "MO") parts.push("WKST=" + wkst);
  }

  var range = recurrence.range || {};
  if (range.type === "numbered" && range.numberOfOccurrences) {
    parts.push("COUNT=" + range.numberOfOccurrences);
  } else if (range.type === "endDate") {
    var until = graphUntil(range.endDate, localStart);
    if (until) parts.push("UNTIL=" + until);
  }
  // range.type === "noEnd" (and an absent range) emit nothing: unbounded is the
  // RFC 5545 default.

  return ["RRULE:" + parts.join(";")];
}

// Graph's location.coordinates -> a GeoJSON Point, which is what
// PropertyValue::Geometry expects. Spatial indexing is by VALUE TYPE with no
// opt-in, so emitting this makes ST_DWITHIN over a calendar work with no other
// machinery. Google Calendar has no coordinates at all, so this is Graph-only.
export function locationGeo(loc) {
  var c = loc && loc.coordinates;
  if (!c) return null;
  var lat = Number(c.latitude);
  var lon = Number(c.longitude);
  if (!isFinite(lat) || !isFinite(lon)) return null;
  return { type: "Point", coordinates: [lon, lat] };
}

export function onlineMeetingUrl(v) {
  if (v.onlineMeeting && v.onlineMeeting.joinUrl) return v.onlineMeeting.joinUrl;
  return v.onlineMeetingUrl || null;
}

// TIME, and what is deliberately NOT done here.
//
// Graph returns a NAIVE local datetime plus a separate `timeZone`; the instant
// is not recoverable without a tz database, which this sandbox does not have.
// The obvious fix — `Prefer: outlook.timezone="UTC"` — is NOT sent, and that is
// a decision rather than an omission: under it Graph returns ONLY the UTC form,
// so the wall-clock a human agreed to is lost, and an all-day event's midnight
// converts to the previous day in every zone east of UTC, silently moving the
// DATE. So the naive local time and its zone are both carried, faithfully, and
// the UTC instant is derived later where a real tz database exists.
//
// `start_tz` was already captured here before this stage and then dropped by
// the mapper. It is now load-bearing: "every Tuesday 09:00 Europe/Zurich" is
// 07:00Z in winter and 08:00Z in summer, so a recurrence without its zone
// cannot be expanded correctly.
export function calendarMeta(v, mount) {
  var meta = {
    subject: v.subject || null,
    ical_uid: v.iCalUId || null,

    start: v.start ? v.start.dateTime : null,
    start_tz: v.start ? v.start.timeZone : null,
    end: v.end ? v.end.dateTime : null,
    end_tz: v.end ? v.end.timeZone : null,
    all_day: v.isAllDay === true,

    location: v.location ? v.location.displayName || null : null,
    location_geo: locationGeo(v.location),

    // The RAW Graph attendee objects, not "Name <addr>" strings: flattening
    // them discarded `status.response` and `type` — the two things that make an
    // attendee list answerable — and put a string in a column where the Google
    // adapter put an object.
    attendees: v.attendees || null,
    organizer: v.organizer || null,

    recurrence: recurrenceLines(v.recurrence, v.start ? v.start.dateTime : null),
    // Graph's own discriminators. Both were already in $select and used
    // internally to fold occurrences into series, then dropped — so a node
    // carried no record of whether it was a series, and an exception (one moved
    // occurrence) was indistinguishable from the master it belongs to.
    event_type: v.type || null,
    series_master_id: v.seriesMasterId || null,
    original_start: v.originalStart || null,

    // Three separate concepts that used to share one `status` string.
    is_cancelled: v.isCancelled === true,
    show_as: v.showAs || null,
    my_response: (v.responseStatus && v.responseStatus.response) || null,

    online_meeting_url: onlineMeetingUrl(v),
    webLink: v.webLink || null,
  };
  // Present only when the mount opted in, exactly as mail does: an absent key
  // tells the mapper to leave the property unset rather than write an empty
  // string over a previously synced description.
  if (includeBody(mount) && v.body && typeof v.body.content === "string") {
    meta.body = v.body.content;
    meta.body_type = v.body.contentType === "html" ? "html" : "text";
  }
  return meta;
}
