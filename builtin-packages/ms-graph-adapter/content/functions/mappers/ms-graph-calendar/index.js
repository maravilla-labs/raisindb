/**
 * Microsoft 365 calendar mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 * Bidirectional dispatch (adapter contract §6.0), same shape as the mail mapper:
 *
 *   to_node             { external_item, mount }               -> { node_type, name?, properties } | null
 *   to_external         { node, mount, fields?, intent? }      -> { payload } | null
 *   mapper_capabilities { mount }                              -> { to_external: true }
 *
 * An absent operation means to_node, so the engine's read path is unchanged.
 *
 * `to_external` emits GRAPH-shaped event JSON, which the adapter forwards
 * verbatim — the same division the mail mapper follows. The read direction is
 * the asymmetric one: there the adapter pre-digests Graph into a neutral
 * `metadata` bag. Both directions still have exactly ONE translator per hop, so
 * a custom mapper can reshape nodes freely without the adapter disagreeing.
 *
 * SHAPE: raisin:Event v2, a provider-NEUTRAL model the google-calendar mapper
 * emits identically, so a consumer never branches on provider. Attendees are
 * objects (never "Name <addr>" strings), organizer is split into
 * organizer_email/organizer_name, `status` is the RFC 5545 EVENT status only
 * (free/busy goes to show_as, RSVP to my_response), recurrence is an array of
 * RFC 5545 content lines (never a JSON blob), and a cancelled event is
 * MATERIALIZED with status="cancelled" rather than skipped.
 *
 * TIME: BOTH forms are kept, and they answer different questions. The instant
 * (start_utc/end_utc) is what a client converts to the viewer's own zone — that
 * is the only way an event in another zone renders at the right hour, and the
 * only way one that crosses midnight there is drawn on two days. The wall clock
 * plus `timezone` is what a FUTURE event actually promises: "09:00 Zurich" must
 * stay 09:00 Zurich even if a government moves a DST transition before the date,
 * which an instant alone cannot survive. raisin:Event carries both for exactly
 * this reason, so neither is derived away.
 *
 * start_utc/end_utc are written when the instant is RECOVERABLE — the value
 * carries its own Z/offset, or its zone is UTC (which is what Graph returns
 * here, since the adapter deliberately sends no `Prefer: outlook.timezone`).
 * For a named zone like Europe/Berlin they stay null: `Intl` does not exist in
 * this runtime (see time.js), and a null is honest where a naive value in an
 * ordered UTC column is silently wrong. start_local/end_local are always
 * written, unchanged.
 *
 * `name` stays the Graph item id so distinct events never collide on a path.
 * The engine stamps the reserved __ properties on top of what is returned; they
 * are declared by the raisin:VirtualNode mixin, not here.
 *
 * LAYOUT. This file owns the dispatch and the two field-by-field translations
 * (`toNode`, `toExternal`). The two mappings that have to be EXACT INVERSES of
 * each other live one per file — time.js (Graph's naive wall clock + Windows
 * zone <-> UTC/local/IANA) and recurrence.js (patternedRecurrence <-> RRULE) —
 * because when both directions of one relationship sat at opposite ends of this
 * file they drifted, and a rule taught to only one side reschedules a real
 * series on the next save.
 */

import { dateOf, graphTime, ianaZone, localOf, utcOf } from "./time.js";
import { graphRecurrence, recurrenceLines } from "./recurrence.js";

export function handler(input) {
  switch (input && input.operation) {
    case "to_external":
      return toExternal(input.node, input.mount, input.fields, input.intent);
    case "mapper_capabilities":
      return { to_external: true };
    case "to_node":
    default:
      return toNode(input);
  }
}

// ---- people ---------------------------------------------------------------

var RSVP = {
  none: "needs_action",
  notResponded: "needs_action",
  accepted: "accepted",
  declined: "declined",
  tentativelyAccepted: "tentative",
  organizer: "organizer",
};

var FREE_BUSY = {
  free: "free",
  tentative: "tentative",
  busy: "busy",
  oof: "out_of_office",
  workingElsewhere: "working_elsewhere",
};

// The adapter flattens attendees to "Name <addr>" today; a widened adapter may
// hand over the raw Graph objects instead. Accept both, emit one shape.
function person(a) {
  if (!a) return null;
  if (typeof a === "string") {
    var m = /^\s*(.*?)\s*<([^>]+)>\s*$/.exec(a);
    if (m) return { email: m[2], name: m[1] || null };
    return a.indexOf("@") !== -1 ? { email: a, name: null } : { email: null, name: a };
  }
  var e = a.emailAddress || a;
  return { email: e.address || e.email || null, name: e.name || e.displayName || null };
}

function attendeeOf(a) {
  var p = person(a);
  if (!p) return null;
  var type = "required";
  var response = null;
  if (a && typeof a === "object") {
    if (a.type === "optional" || a.type === "resource") type = a.type;
    var raw = a.status && a.status.response;
    response = RSVP[raw] || null;
  }
  return { email: p.email, name: p.name, type: type, response: response };
}

// ---- to_node --------------------------------------------------------------

function toNode(input) {
  var item = input.external_item;
  if (!item || !item.external_id) return null;

  var meta = item.metadata || {};
  var mount = input.mount || {};
  var allDay = meta.all_day === true;

  // Graph folds isCancelled, responseStatus and showAs into one `status` string,
  // so it has to be unpicked into the three columns they actually belong to.
  var cancelled = meta.is_cancelled === true || meta.status === "cancelled";
  var showAs = FREE_BUSY[meta.show_as] || FREE_BUSY[meta.status] || null;
  var myResponse = RSVP[meta.my_response] || RSVP[meta.status] || null;

  var attendees = null;
  if (Array.isArray(meta.attendees) && meta.attendees.length) {
    attendees = [];
    for (var i = 0; i < meta.attendees.length; i++) {
      var a = attendeeOf(meta.attendees[i]);
      if (a) attendees.push(a);
    }
    if (!attendees.length) attendees = null;
  }

  var organizer = person(meta.organizer) || { email: null, name: null };
  var startTz = meta.start_tz || meta.timezone || null;
  var recurrence = recurrenceLines(meta.recurrence);

  // Graph's own discriminator when the adapter forwards it; otherwise the
  // presence of a pattern is the only evidence available.
  var kind = meta.event_type;
  var recurrenceType =
    kind === "seriesMaster"
      ? "series_master"
      : kind === "exception"
        ? "exception"
        : kind === "occurrence"
          ? "occurrence"
          : recurrence
            ? "series_master"
            : "single";

  var props = {
    title: meta.subject || "(untitled event)",
    ical_uid: meta.ical_uid || null,
    // The adapter's own default is "calendar"; reading mount.remote_root alone
    // left calendar_id null on every default-calendar mount.
    calendar_id: mount.remote_root || "calendar",

    // The zone travels WITH the value: Graph puts the wall clock and its zone in
    // two separate fields, and utcOf needs both to decide whether the instant is
    // recoverable. `end_tz` falls back to `start_tz` because Graph omits it on
    // some projections, and an event's two ends are always in one zone.
    start_utc: allDay ? (dateOf(meta.start) ? dateOf(meta.start) + "T00:00:00Z" : null) : utcOf(meta.start, startTz),
    end_utc: allDay ? (dateOf(meta.end) ? dateOf(meta.end) + "T00:00:00Z" : null) : utcOf(meta.end, meta.end_tz || startTz),
    start_local: localOf(meta.start, allDay),
    end_local: localOf(meta.end, allDay),
    timezone: ianaZone(startTz),
    all_day: allDay,

    recurrence_type: recurrenceType,
    recurrence: recurrence,
    series_master_external_id: meta.series_master_id || null,
    original_start_utc: utcOf(meta.original_start),
    original_start_local: localOf(meta.original_start, allDay),

    status: cancelled ? "cancelled" : "confirmed",
    show_as: showAs,
    my_response: myResponse,
    organizer_email: organizer.email,
    organizer_name: organizer.name,
    attendees: attendees,

    location: meta.location || null,
    location_geo: meta.location_geo || null,
    online_meeting_url: meta.online_meeting_url || null,
    url: item.web_url || meta.webLink || null,
  };

  // Only when the mount opted into bodies AND the adapter returned one. Writing
  // "" for an absent body would blank a previously synced description and change
  // the node on every run, defeating the etag skip-write.
  if (typeof meta.body === "string") {
    if (meta.body_type === "html") props.description_html = meta.body;
    else props.description_text = meta.body;
  }

  return { node_type: "raisin:Event", name: item.name, properties: props };
}

// ---- to_external ----------------------------------------------------------
//
// The inverse of `toNode`, for a `mirror` calendar mount. Graph shape out; the
// adapter PATCHes / POSTs it verbatim.
//
// THE FIELD LIST IS AN ALLOW-LIST, NOT A SUGGESTION. `fields` is the engine's
// intersection of the mount's `mutable_fields` with the adapter's, and emitting
// a key outside it is how a mount configured to push titles quietly starts
// overwriting attendee lists. The one deliberate exception is the TIME GROUP —
// see `emitTime`.
//
// Everything Graph computes or owns is absent by construction: `organizer`,
// `iCalUId`, `webLink`, `onlineMeeting`, `isCancelled`, `responseStatus`. They
// are read-only at the provider, so sending them is a 400 at best and a silent
// no-op at worst, and none of them is a thing a local edit should mean.
// Cancelling is a DELETE (see the mount's delete_policy); RSVP is a `submit`
// command on a separate mount, deliberately, because it notifies the organizer.

var SHOW_AS_OUT = {
  free: "free",
  tentative: "tentative",
  busy: "busy",
  out_of_office: "oof",
  working_elsewhere: "workingElsewhere",
};

// A time field is never pushed alone.
//
// Graph validates start against end on the SAME request, so PATCHing a start
// that now falls after the stored end is rejected outright — and a start whose
// timeZone is omitted is interpreted in whatever zone Graph last stored, which
// silently moves the meeting. So the whole group travels whenever any member of
// it is allowed, which is the one place this mapper deliberately emits keys the
// caller did not name. It is safe in the direction that matters: every key in
// the group is written from THIS node's own properties, so the extra keys carry
// the values Graph already has unless the user changed them too.
var TIME_FIELDS = ["start_local", "start_utc", "end_local", "end_utc", "timezone", "all_day"];

function allowed(fields, name) {
  // No list at all means "the whole object" — a create.
  if (!fields || !fields.length) return true;
  return fields.indexOf(name) !== -1;
}

function anyAllowed(fields, names) {
  for (var i = 0; i < names.length; i++) {
    if (allowed(fields, names[i])) return true;
  }
  return false;
}

function str(v) {
  return typeof v === "string" && v ? v : null;
}

function attendeeOut(a) {
  if (!a) return null;
  var email = str(a.email);
  if (!email) return null;
  var type = a.type === "optional" || a.type === "resource" ? a.type : "required";
  return {
    emailAddress: { address: email, name: str(a.name) || undefined },
    type: type,
  };
}

function toExternal(node, mount, fields, intent) {
  if (!node) return null;
  var props = node.properties || {};
  var kind = props.recurrence_type;

  // A derived occurrence is a PROJECTION of its series master, regenerated from
  // the rule. Pushing one would mint a standalone meeting at the provider that
  // the next rebuild disowns, so it is refused in both directions and for every
  // intent. (The engine refuses these by path as well; this is the second gate,
  // because a mount whose projection path differs would slip past the first.)
  if (kind === "occurrence") return null;

  // An EXCEPTION is updatable and NOT creatable, and the difference is a fact
  // about Graph rather than a policy choice: an exception has its own event id
  // once it exists, so PATCHing it is ordinary — but Graph mints one only by
  // diverging an occurrence of an existing series. POSTing /events for one
  // creates a standalone event that the series still overlaps, i.e. a duplicate
  // meeting in the user's calendar. `intent` is the only thing that separates
  // the two cases; `fields` cannot, because a mirror update and a create both
  // arrive with an empty list.
  if (kind === "exception" && intent === "create") return null;

  var allDay = props.all_day === true;
  var tz = str(props.timezone);
  var payload = {};
  var emitted = 0;

  if (allowed(fields, "title") && typeof props.title === "string") {
    payload.subject = props.title;
    emitted++;
  }

  // html wins when both are allowed and both present: it is the richer of the
  // two and Graph stores ONE body, so sending text would discard formatting the
  // user can see.
  if (allowed(fields, "description_html") && typeof props.description_html === "string") {
    payload.body = { contentType: "html", content: props.description_html };
    emitted++;
  } else if (allowed(fields, "description_text") && typeof props.description_text === "string") {
    payload.body = { contentType: "text", content: props.description_text };
    emitted++;
  }

  if (anyAllowed(fields, TIME_FIELDS)) {
    var start = graphTime(props.start_local, props.start_utc, tz, allDay);
    var end = graphTime(props.end_local, props.end_utc, tz, allDay);
    if (start && end) {
      payload.start = start;
      payload.end = end;
      payload.isAllDay = allDay;
      emitted++;
    } else {
      // DECLINE THE WHOLE PUSH — on update as well as on create.
      //
      // On create this was always right: an event with no resolvable start is
      // not a thing Graph can store, and a POST that omits it is a 400 the
      // engine would retry forever.
      //
      // On UPDATE it used to fall through, dropping the time from the PATCH
      // while the rest of the payload went out. The engine then baselined every
      // field it had ASKED to send — `start_local` included — because
      // `__pushed_state` is stamped from the node's own values and never checked
      // against what the payload actually carried. So rescheduling an event
      // whose Windows timezone is missing from time.js's `WINDOWS_TO_IANA` moved the time
      // in RaisinDB, never moved it in Outlook, and recorded it as pushed. The
      // two then diverge permanently and invisibly: nothing re-nominates the
      // node, so it says 14:00 while Outlook says 10:00, forever.
      //
      // Declining parks the intent instead: attributable, visible, and fixing
      // the zone mapping still pushes it.
      return null;
    }
  }

  if (allowed(fields, "location")) {
    var loc = str(props.location);
    if (loc) {
      payload.location = { displayName: loc };
      emitted++;
    }
  }

  if (allowed(fields, "attendees") && Array.isArray(props.attendees)) {
    var list = [];
    for (var i = 0; i < props.attendees.length; i++) {
      var a = attendeeOut(props.attendees[i]);
      if (a) list.push(a);
    }
    // An empty array is meaningful — it clears the attendee list — so it is
    // emitted, unlike an absent property.
    payload.attendees = list;
    emitted++;
  }

  if (allowed(fields, "show_as") && SHOW_AS_OUT[props.show_as]) {
    payload.showAs = SHOW_AS_OUT[props.show_as];
    emitted++;
  }

  if (allowed(fields, "recurrence")) {
    var startDate = dateOf(props.start_local) || dateOf(props.start_utc);
    var rec = graphRecurrence(props.recurrence, startDate, tz);
    if (rec) {
      payload.recurrence = rec;
      emitted++;
    }
  }

  // Nothing to say. Null rather than `{}`: an empty PATCH still bumps the
  // event's change key at Graph, which invalidates every stored __etag and makes
  // the next delta re-deliver the whole series for no reason.
  if (!emitted) return null;

  return { payload: payload };
}
