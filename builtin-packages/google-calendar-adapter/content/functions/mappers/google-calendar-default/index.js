/**
 * Default Google Calendar mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 * Bidirectional dispatch (adapter contract §6.0), same shape as the mail mapper:
 *
 *   to_node             { external_item, mount }          -> { node_type, name?, properties } | null
 *   to_external         { node, mount, fields?, intent? } -> { payload } | null
 *   mapper_capabilities { mount }                         -> { to_external: true }
 *
 * An absent operation means to_node, so the engine's read path is unchanged.
 *
 * `to_external` emits GOOGLE-shaped event JSON, which the adapter forwards
 * verbatim — the same division the ms-graph mappers follow. The read direction
 * is the asymmetric one: there the adapter pre-digests the provider into a
 * neutral `metadata` bag. Both directions still have exactly ONE translator per
 * hop, so a custom mapper can reshape nodes without the adapter disagreeing.
 *
 * SHAPE: raisin:Event v2, a provider-NEUTRAL model the ms-graph-calendar mapper
 * emits identically, so a consumer never branches on provider. Three former
 * divergences are gone: attendees are objects with the same four keys (Graph
 * used "Name <addr>" strings), an absent attendee list is null on BOTH sides,
 * and a cancelled event is now MATERIALIZED with status="cancelled" instead of
 * being dropped here while Graph kept it.
 *
 * TIME: Google's dateTime carries its own UTC offset, so start_utc IS
 * recoverable here — unlike Graph, which returns a naive local time. All-day
 * events use the bare date with an exclusive end, matching RFC 5545 DTEND.
 *
 * `name` stays the Google event id so distinct events never collide on a path.
 * The engine stamps the reserved __ properties on top of what is returned; they
 * are declared by the raisin:VirtualNode mixin, not here.
 */

function handler(input) {
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

// ---- time -----------------------------------------------------------------

function utcOf(s) {
  if (typeof s !== "string" || !s) return null;
  if (!/(Z|[+-]\d{2}:?\d{2})$/.test(s)) return null;
  var d = new Date(s);
  if (isNaN(d.getTime())) return null;
  return d.toISOString().replace(/\.\d+Z$/, "Z");
}

function dateOf(s) {
  var m = typeof s === "string" ? /^(\d{4}-\d{2}-\d{2})/.exec(s) : null;
  return m ? m[1] : null;
}

// Wall clock with no offset: "YYYY-MM-DDTHH:MM:SS", or the bare date.
function localOf(s, allDay) {
  var m = typeof s === "string" ? /^(\d{4}-\d{2}-\d{2})(?:[T ](\d{2}:\d{2}:\d{2}))?/.exec(s) : null;
  if (!m) return null;
  if (allDay) return m[1];
  return m[2] ? m[1] + "T" + m[2] : m[1];
}

function instantOf(value, allDay) {
  if (allDay) {
    var d = dateOf(value);
    return d ? d + "T00:00:00Z" : null;
  }
  return utcOf(value);
}

// ---- people ---------------------------------------------------------------

var RSVP = {
  needsAction: "needs_action",
  needs_action: "needs_action",
  accepted: "accepted",
  declined: "declined",
  tentative: "tentative",
  organizer: "organizer",
};

// The adapter reduces organizer to a bare email today; a widened adapter may
// forward the raw object. Accept both, emit one shape.
function person(o) {
  if (!o) return null;
  if (typeof o === "string") {
    return o.indexOf("@") !== -1 ? { email: o, name: null } : { email: null, name: o };
  }
  return { email: o.email || null, name: o.displayName || o.name || null };
}

function attendeeOf(a) {
  var p = person(a);
  if (!p) return null;
  var type = "required";
  if (a && typeof a === "object") {
    if (a.resource === true) type = "resource";
    else if (a.optional === true) type = "optional";
  }
  var response = a && typeof a === "object" ? RSVP[a.responseStatus] || null : null;
  return { email: p.email, name: p.name, type: type, response: response };
}

// Google marks the mounted principal's own row with `self: true`; that row's
// responseStatus IS my_response. An organizing principal has no useful RSVP, so
// `organizer` wins.
function myResponse(rawAttendees, organizer) {
  if (organizer && typeof organizer === "object" && organizer.self === true) return "organizer";
  if (!Array.isArray(rawAttendees)) return null;
  for (var i = 0; i < rawAttendees.length; i++) {
    var a = rawAttendees[i];
    if (a && typeof a === "object" && a.self === true) {
      if (a.organizer === true) return "organizer";
      return RSVP[a.responseStatus] || "needs_action";
    }
  }
  return null;
}

// ---- to_node --------------------------------------------------------------

var TRANSPARENCY = { transparent: "free", opaque: "busy" };
var STATUS = { confirmed: "confirmed", tentative: "tentative", cancelled: "cancelled" };

function toNode(input) {
  var item = input.external_item;
  if (!item || !item.external_id) return null;

  var meta = item.metadata || {};
  var allDay = meta.all_day === true;

  // Google's `recurrence` is ALREADY an array of RFC 5545 content lines, which
  // is exactly what the column holds — no conversion, no join. It only arrives
  // once the adapter stops expanding with singleEvents=true.
  var recurrence = null;
  if (Array.isArray(meta.recurrence) && meta.recurrence.length) {
    recurrence = meta.recurrence.slice();
  } else if (typeof meta.recurrence === "string" && meta.recurrence.length) {
    recurrence = meta.recurrence.split("\n");
  }

  var rawAttendees = Array.isArray(meta.attendees) ? meta.attendees : null;
  var attendees = null;
  if (rawAttendees && rawAttendees.length) {
    attendees = [];
    for (var i = 0; i < rawAttendees.length; i++) {
      var a = attendeeOf(rawAttendees[i]);
      if (a) attendees.push(a);
    }
    if (!attendees.length) attendees = null;
  }

  var organizer = person(meta.organizer) || { email: null, name: null };

  // recurring_event_id names the master, so its presence is what separates an
  // instance from a plain single event; original_start distinguishes a moved
  // occurrence (an exception) from an untouched one.
  var masterId = meta.recurring_event_id || meta.recurringEventId || null;
  var originalStart = meta.original_start || meta.originalStartTime || null;
  if (originalStart && typeof originalStart === "object") {
    originalStart = originalStart.dateTime || originalStart.date || null;
  }
  var recurrenceType = masterId
    ? originalStart
      ? "exception"
      : "occurrence"
    : recurrence
      ? "series_master"
      : "single";

  var props = {
    title: meta.summary || item.name || "(untitled event)",
    ical_uid: meta.ical_uid || meta.iCalUID || null,
    // The adapter always resolves this (defaulting to "primary"), so unlike the
    // Graph side it was never null.
    calendar_id: meta.calendar_id || null,

    start_utc: instantOf(meta.start, allDay),
    end_utc: instantOf(meta.end, allDay),
    start_local: localOf(meta.start, allDay),
    end_local: localOf(meta.end, allDay),
    // Google sends an IANA name alongside each slot; it is passed through
    // verbatim once the adapter carries it.
    timezone: meta.start_timezone || meta.timezone || null,
    all_day: allDay,

    recurrence_type: recurrenceType,
    recurrence: recurrence,
    series_master_external_id: masterId,
    original_start_utc: instantOf(originalStart, allDay),
    original_start_local: localOf(originalStart, allDay),

    // Google's status is natively the RFC 5545 event status, so it maps
    // one-to-one; free/busy and RSVP live in their own columns.
    status: STATUS[meta.status] || "confirmed",
    show_as: TRANSPARENCY[meta.transparency] || null,
    my_response: myResponse(rawAttendees, meta.organizer),
    organizer_email: organizer.email,
    organizer_name: organizer.name,
    attendees: attendees,

    location: meta.location || null,
    // Google Calendar's location is free text with no coordinates at all, so
    // this column is Graph-only. Stated rather than left implicit.
    location_geo: null,
    online_meeting_url: meta.hangout_link || meta.online_meeting_url || null,
    url: meta.htmlLink || item.web_url || null,
  };

  // Only when the mount opted into bodies AND the adapter returned one. Writing
  // "" for an absent description would blank a previously synced one and change
  // the node on every run, defeating the etag skip-write.
  if (typeof meta.description === "string") {
    props.description_html = meta.description;
  }

  return { node_type: "raisin:Event", name: item.name, properties: props };
}

// ---- to_external ----------------------------------------------------------
//
// The inverse of `toNode`, for a `mirror` calendar mount. Google shape out; the
// adapter PATCHes / POSTs it verbatim.
//
// THE FIELD LIST IS AN ALLOW-LIST, NOT A SUGGESTION. `fields` is the engine's
// intersection of the mount's `mutable_fields` with the adapter's, and emitting
// a key outside it is how a mount configured to push titles quietly starts
// overwriting attendee lists. The one deliberate exception is the TIME GROUP —
// see `emitTime` below.
//
// RECURRENCE COSTS NOTHING HERE, and that is the whole reason RFC 5545 is the
// canonical shape. Google's `recurrence` IS an array of RFC 5545 content lines,
// so the column travels verbatim in both directions — no pattern vocabulary to
// translate, no UNTIL arithmetic, nothing to get wrong. The ms-graph mapper has
// ~150 lines converting the same column to `patternedRecurrence` and back.
//
// Everything Google computes or owns is absent by construction: `iCalUID`,
// `htmlLink`, `hangoutLink`/`conferenceData`, `organizer`, `created`/`updated`,
// and the caller's own `responseStatus`. They are read-only at the provider, so
// sending them is a 400 at best and a silent no-op at worst. The two that look
// writable are the dangerous ones: cancelling is a DELETE under the mount's
// delete_policy, and an RSVP notifies the organizer and must not hide behind a
// property edit.

var TRANSPARENCY_OUT = { free: "transparent", busy: "opaque" };

// A time field is never pushed alone.
//
// Google validates start against end on the SAME request, so a start that now
// falls after the stored end is rejected outright — and an all-day event flips
// the representation of BOTH ends at once (`date` instead of `dateTime`), so
// pushing one without the other produces a resource Google rejects. The whole
// group therefore travels whenever any member of it is allowed, which is the one
// place this mapper emits keys the caller did not name. Safe in the direction
// that matters: every key is written from THIS node's own properties, so the
// extra ones carry what Google already has unless the user changed them too.
var TIME_FIELDS_OUT = [
  "start_local",
  "start_utc",
  "end_local",
  "end_utc",
  "timezone",
  "all_day",
];

function allowedOut(fields, name) {
  // No list at all means "the whole object" — a create.
  if (!fields || !fields.length) return true;
  return fields.indexOf(name) !== -1;
}

function anyAllowedOut(fields, names) {
  for (var i = 0; i < names.length; i++) {
    if (allowedOut(fields, names[i])) return true;
  }
  return false;
}

function strOut(v) {
  return typeof v === "string" && v ? v : null;
}

// One end of the event as a Google EventDateTime, or null when the node does not
// carry enough to name an instant.
//
// The zone question is the whole of it. `start_local` is a NAIVE wall clock, and
// Google reads a `dateTime` with no offset in the request's `timeZone` — so a
// naive local paired with no zone would be interpreted in the CALENDAR's zone
// and silently move the event for anyone whose series is in a different one.
// The UTC value is preferred precisely because it is unambiguous; the zone still
// travels beside it, because Google expands the recurrence rule server-side in
// that zone and a series with no zone expands in the calendar's.
function googleTime(local, utc, tz, allDay) {
  if (allDay) {
    var d = dateOf(local) || dateOf(utc);
    return d ? { date: d } : null;
  }
  if (utc) {
    var slot = { dateTime: utc };
    if (tz) slot.timeZone = tz;
    return slot;
  }
  if (local && tz) return { dateTime: local, timeZone: tz };
  return null;
}

function attendeeOut(a) {
  if (!a) return null;
  var email = strOut(a.email);
  if (!email) return null;
  var out = { email: email };
  var name = strOut(a.name);
  if (name) out.displayName = name;
  if (a.type === "optional") out.optional = true;
  if (a.type === "resource") out.resource = true;
  return out;
}

function toExternal(node, mount, fields, intent) {
  if (!node) return null;
  var props = node.properties || {};
  var kind = props.recurrence_type;

  // A derived occurrence is a PROJECTION of its series master, regenerated from
  // the rule. Pushing one would mint a standalone meeting at the provider that
  // the next rebuild disowns, so it is refused in both directions and for every
  // intent. (The engine also refuses these by path; this is the second gate,
  // because a mount whose projection path differs would slip past the first.)
  if (kind === "occurrence") return null;

  // An EXCEPTION is updatable and NOT creatable, and the difference is a fact
  // about the provider rather than a policy choice: an exception has its own
  // event id once it exists, so PATCHing it is ordinary — but Google mints one
  // only by patching an INSTANCE of a live series. POSTing /events for one
  // creates a standalone event the series still overlaps, i.e. a duplicate
  // meeting in the user's calendar. `intent` is the only thing that separates
  // the two cases; `fields` cannot, because a mirror update and a create both
  // arrive with an empty list.
  if (kind === "exception" && intent === "create") return null;

  var allDay = props.all_day === true;
  var tz = strOut(props.timezone);
  var payload = {};
  var emitted = 0;

  if (allowedOut(fields, "title") && typeof props.title === "string") {
    payload.summary = props.title;
    emitted++;
  }

  // Google stores ONE description field and renders a small HTML subset in it,
  // so html wins when both are present — sending the text would discard
  // formatting the user can see.
  if (allowedOut(fields, "description_html") && typeof props.description_html === "string") {
    payload.description = props.description_html;
    emitted++;
  } else if (
    allowedOut(fields, "description_text") &&
    typeof props.description_text === "string"
  ) {
    payload.description = props.description_text;
    emitted++;
  }

  if (anyAllowedOut(fields, TIME_FIELDS_OUT)) {
    var start = googleTime(props.start_local, props.start_utc, tz, allDay);
    var end = googleTime(props.end_local, props.end_utc, tz, allDay);
    if (start && end) {
      payload.start = start;
      payload.end = end;
      emitted++;
    } else if (intent === "create") {
      // An event with no resolvable start is not a thing Google can store, and a
      // POST that omits it is a 400 the engine would count as a per-item failure
      // and retry forever. Declining is the honest answer: nothing is sent,
      // nothing is stamped, and fixing the node still creates it.
      return null;
    }
  }

  if (allowedOut(fields, "location")) {
    var loc = strOut(props.location);
    if (loc) {
      payload.location = loc;
      emitted++;
    }
  }

  if (allowedOut(fields, "attendees") && Array.isArray(props.attendees)) {
    var list = [];
    for (var i = 0; i < props.attendees.length; i++) {
      var a = attendeeOut(props.attendees[i]);
      if (a) list.push(a);
    }
    // An empty array is meaningful — it clears the invitee list — so it is
    // emitted, unlike an absent property.
    payload.attendees = list;
    emitted++;
  }

  // Google's transparency has exactly two values. `out_of_office` and
  // `working_elsewhere` are Graph concepts with no Google equivalent, and
  // flattening them onto `opaque` would silently rewrite the user's choice on
  // every push — so an unmappable value emits nothing at all.
  if (allowedOut(fields, "show_as") && TRANSPARENCY_OUT[props.show_as]) {
    payload.transparency = TRANSPARENCY_OUT[props.show_as];
    emitted++;
  }

  // Verbatim, both directions. See the note at the top of this section.
  if (allowedOut(fields, "recurrence") && Array.isArray(props.recurrence)) {
    var lines = [];
    for (var k = 0; k < props.recurrence.length; k++) {
      if (typeof props.recurrence[k] === "string" && props.recurrence[k]) {
        lines.push(props.recurrence[k]);
      }
    }
    if (lines.length) {
      payload.recurrence = lines;
      emitted++;
    }
  }

  // Nothing to say. Null rather than `{}`: an empty PATCH still bumps the
  // event's etag, which invalidates every stored one and makes the next delta
  // re-deliver the event for no reason.
  if (!emitted) return null;

  return { payload: payload };
}
