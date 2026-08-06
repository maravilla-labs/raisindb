// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! One Calendar event resource to one `ExternalItem`.
//!
//! Everything here is already on the wire — this adapter sends no `fields=`
//! projection — so the only question is what survives into `metadata`.

import { includeBody, whenOf } from "./mount.js";

// Google's video-conference link. `hangoutLink` is the legacy convenience field;
// conferenceData is the current one and is present when hangoutLink is not.
export function conferenceUrl(ev) {
  if (ev.hangoutLink) return ev.hangoutLink;
  var points = (ev.conferenceData && ev.conferenceData.entryPoints) || [];
  for (var i = 0; i < points.length; i++) {
    if (points[i] && points[i].entryPointType === "video" && points[i].uri) {
      return points[i].uri;
    }
  }
  return null;
}

// Build a normalized ExternalItem from a Calendar event resource. Events are
// leaves (never folders); name and relative_path are the stable event id so the
// engine's upsert key is deterministic regardless of a mutable summary.
//
// Everything below is already on the wire — this adapter sends no `fields=`
// projection, so the whole event resource arrives and the only question is what
// survives into metadata. `recurringEventId`, `originalStartTime`, `iCalUID`,
// `transparency`, `start.timeZone` and the conference link were all present in
// every response and discarded here.
export function toExternalItem(ev, calId, mount) {
  var start = whenOf(ev.start);
  var end = whenOf(ev.end);
  var meta = {
    summary: ev.summary || null,
    ical_uid: ev.iCalUID || null,
    status: ev.status || null,
    location: ev.location || null,
    htmlLink: ev.htmlLink || null,
    // The RAW organizer object, not a bare email: the mapper needs `self` to
    // decide my_response, and `displayName` to fill organizer_name.
    organizer: ev.organizer || null,
    attendees: ev.attendees || null,
    // Already an array of RFC 5545 content lines — the canonical shape. Only a
    // master carries it, which is why singleEvents had to go.
    recurrence: ev.recurrence || null,
    // Present on an instance only; names the master it belongs to.
    recurring_event_id: ev.recurringEventId || null,
    original_start: ev.originalStartTime || null,
    start: start.value,
    end: end.value,
    start_timezone: start.tz,
    end_timezone: end.tz,
    all_day: start.allDay,
    transparency: ev.transparency || null,
    online_meeting_url: conferenceUrl(ev),
    calendar_id: calId,
  };
  // Absent key, not an empty string: writing "" for an event with no
  // description would blank a previously synced one and change the node on
  // every run, defeating the etag skip-write.
  if (includeBody(mount) && typeof ev.description === "string") {
    meta.description = ev.description;
  }
  return {
    external_id: ev.id,
    name: ev.id,
    relative_path: ev.id,
    mime_type: null,
    size_bytes: null,
    is_folder: false,
    parent_id: null,
    created_at: ev.created || null,
    modified_at: ev.updated || null,
    // Google's per-event etag is a stable change token — lets the engine's
    // skip-write suppress needless revisions when nothing changed.
    etag: ev.etag || ev.updated || null,
    web_url: ev.htmlLink || null,
    download_url: null,
    metadata: meta,
  };
}
