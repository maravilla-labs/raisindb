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
 * TIME: start_utc/end_utc are written only when the provider value carries a
 * zone designator. Graph returns a NAIVE local datetime plus a separate
 * timeZone, and converting that needs a tz database this mapper does not have —
 * so start_utc stays null until the adapter sends `Prefer:
 * outlook.timezone="UTC"`. A null is honest; a naive value in an ordered UTC
 * column is silently wrong. start_local/end_local are always written.
 *
 * `name` stays the Graph item id so distinct events never collide on a path.
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

// An instant is only recoverable when the string carries Z or ±HH:MM.
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

// Graph reports Windows zone names; raisin:Event.timezone is IANA. A name that
// cannot be mapped becomes null rather than a wrong zone. The full CLDR
// windowsZones table belongs in the adapter; this covers the common ones.
var WINDOWS_TO_IANA = {
  "UTC": "UTC",
  "GMT Standard Time": "Europe/London",
  "W. Europe Standard Time": "Europe/Berlin",
  "Central Europe Standard Time": "Europe/Budapest",
  "Romance Standard Time": "Europe/Paris",
  "Central European Standard Time": "Europe/Warsaw",
  "GTB Standard Time": "Europe/Bucharest",
  "FLE Standard Time": "Europe/Kiev",
  "Israel Standard Time": "Asia/Jerusalem",
  "Russian Standard Time": "Europe/Moscow",
  "Arab Standard Time": "Asia/Riyadh",
  "India Standard Time": "Asia/Kolkata",
  "China Standard Time": "Asia/Shanghai",
  "Singapore Standard Time": "Asia/Singapore",
  "Tokyo Standard Time": "Asia/Tokyo",
  "AUS Eastern Standard Time": "Australia/Sydney",
  "New Zealand Standard Time": "Pacific/Auckland",
  "Eastern Standard Time": "America/New_York",
  "Central Standard Time": "America/Chicago",
  "Mountain Standard Time": "America/Denver",
  "Pacific Standard Time": "America/Los_Angeles",
  "Hawaiian Standard Time": "Pacific/Honolulu",
  "E. South America Standard Time": "America/Sao_Paulo",
  "South Africa Standard Time": "Africa/Johannesburg",
};

function ianaZone(name) {
  if (typeof name !== "string" || !name) return null;
  if (name.indexOf("/") !== -1) return name; // already IANA
  return WINDOWS_TO_IANA[name] || null;
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

// ---- recurrence -----------------------------------------------------------

var DAY = {
  sunday: "SU",
  monday: "MO",
  tuesday: "TU",
  wednesday: "WE",
  thursday: "TH",
  friday: "FR",
  saturday: "SA",
};
var FREQ = {
  daily: "DAILY",
  weekly: "WEEKLY",
  absoluteMonthly: "MONTHLY",
  relativeMonthly: "MONTHLY",
  absoluteYearly: "YEARLY",
  relativeYearly: "YEARLY",
};
var SETPOS = { first: 1, second: 2, third: 3, fourth: 4, last: -1 };

// RFC 5545 content lines.
//
// The ms-graph adapter now converts `patternedRecurrence` itself and hands over
// an ARRAY of lines — the same shape Google Calendar puts on the wire natively,
// which is the whole point of the convergence. That array is passed through
// untouched.
//
// The patternedRecurrence branch below is a LEGACY fallback for an adapter that
// has not been updated (a custom one, or a package still on the old build). It
// duplicates logic that now lives in the adapter and should be deleted once no
// shipped adapter emits the old shape.
function recurrenceLines(raw) {
  if (Array.isArray(raw)) {
    var lines = [];
    for (var k = 0; k < raw.length; k++) {
      if (typeof raw[k] === "string" && raw[k]) lines.push(raw[k]);
    }
    return lines.length ? lines : null;
  }
  var r = raw;
  if (typeof r === "string") {
    try {
      r = JSON.parse(r);
    } catch (e) {
      return null;
    }
  }
  if (!r || !r.pattern) return null;
  var p = r.pattern;
  var freq = FREQ[p.type];
  if (!freq) return null;

  var parts = ["FREQ=" + freq];
  if (p.interval && p.interval !== 1) parts.push("INTERVAL=" + p.interval);

  var days = [];
  for (var i = 0; i < (p.daysOfWeek || []).length; i++) {
    var d = DAY[p.daysOfWeek[i]];
    if (d) days.push(d);
  }
  if (days.length) parts.push("BYDAY=" + days.join(","));
  if (p.dayOfMonth) parts.push("BYMONTHDAY=" + p.dayOfMonth);
  if (p.month) parts.push("BYMONTH=" + p.month);
  if (SETPOS[p.index] !== undefined && days.length) parts.push("BYSETPOS=" + SETPOS[p.index]);

  var range = r.range || {};
  if (range.type === "numbered" && range.numberOfOccurrences) {
    parts.push("COUNT=" + range.numberOfOccurrences);
  } else if (range.type === "endDate" && dateOf(range.endDate)) {
    parts.push("UNTIL=" + dateOf(range.endDate).replace(/-/g, "") + "T235959Z");
  }
  return ["RRULE:" + parts.join(";")];
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

    start_utc: allDay ? (dateOf(meta.start) ? dateOf(meta.start) + "T00:00:00Z" : null) : utcOf(meta.start),
    end_utc: allDay ? (dateOf(meta.end) ? dateOf(meta.end) + "T00:00:00Z" : null) : utcOf(meta.end),
    start_local: localOf(meta.start, allDay),
    end_local: localOf(meta.end, allDay),
    timezone: ianaZone(meta.start_tz || meta.timezone),
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

var GRAPH_DAY_NAME = {
  SU: "sunday",
  MO: "monday",
  TU: "tuesday",
  WE: "wednesday",
  TH: "thursday",
  FR: "friday",
  SA: "saturday",
};
var DOW_BY_INDEX = [
  "sunday",
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
];
var GRAPH_INDEX_NAME = { 1: "first", 2: "second", 3: "third", 4: "fourth", "-1": "last" };
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

// "2026-08-11T09:00:00Z" / "2026-08-11T09:00:00" / "2026-08-11" -> the naive
// local datetime Graph wants beside an explicit `timeZone`.
function naive(s, allDay) {
  var m = typeof s === "string" ? /^(\d{4}-\d{2}-\d{2})(?:[T ](\d{2}:\d{2}(?::\d{2})?))?/.exec(s) : null;
  if (!m) return null;
  if (allDay) return m[1] + "T00:00:00";
  var t = m[2] || "00:00:00";
  return m[1] + (t.length === 5 ? t + ":00" : t).replace(/^/, "T");
}

// One end of the event as a Graph dateTimeTimeZone, or null when the node does
// not carry enough to name an instant.
//
// The zone question is the whole of it. `start_local` is a NAIVE wall clock and
// `timezone` may legitimately be null (`toNode` writes null rather than guess a
// zone it cannot map). Pairing a naive local with `timeZone: "UTC"` would move
// every event in a non-UTC calendar by its offset — a silent, plausible-looking
// corruption of real meetings. So a naive local without a zone falls back to the
// UTC value, and when there is no UTC value either this returns null and the
// caller declines rather than guesses.
function graphTime(local, utc, tz, allDay) {
  if (tz && local) return { dateTime: naive(local, allDay), timeZone: tz };
  if (utc) return { dateTime: naive(utc, allDay), timeZone: "UTC" };
  if (allDay && local) return { dateTime: naive(local, true), timeZone: tz || "UTC" };
  return null;
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

// Date arithmetic on a naive "YYYY-MM-DD", zone-free by construction.
function dateParts(s) {
  var m = typeof s === "string" ? /^(\d{4})-(\d{2})-(\d{2})/.exec(s) : null;
  return m ? { y: Number(m[1]), m: Number(m[2]), d: Number(m[3]) } : null;
}

// RFC 5545 `UNTIL=20261231T210000Z` -> Graph's `endDate: "2026-12-31"`.
//
// THIS IS THE EXACT INVERSE OF `graphUntil` IN THE ADAPTER, and the two must
// stay inverses. Graph's endDate is a date in the series' OWN zone while UNTIL
// is an instant, so the adapter pads by 12 hours to cover every offset; reading
// it back therefore has to subtract the same 12 hours before taking the date, or
// an evening series round-trips one day later every time it is written — adding
// an occurrence per save, forever. Subtracting also does the right thing for a
// foreign RRULE that wrote a naive `T235959Z`: 12h back is 11:59 on the same
// day.
var UNTIL_PAD_MS = 12 * 3600000;

function untilToEndDate(until) {
  var m = /^(\d{4})(\d{2})(\d{2})(?:T(\d{2})(\d{2})(\d{2})Z?)?$/.exec(until || "");
  if (!m) return null;
  var at = new Date(
    Date.UTC(
      Number(m[1]),
      Number(m[2]) - 1,
      Number(m[3]),
      Number(m[4] || 0),
      Number(m[5] || 0),
      Number(m[6] || 0)
    ) - UNTIL_PAD_MS
  );
  if (isNaN(at.getTime())) return null;
  return (
    String(at.getUTCFullYear()) +
    "-" +
    (at.getUTCMonth() + 1 < 10 ? "0" : "") +
    (at.getUTCMonth() + 1) +
    "-" +
    (at.getUTCDate() < 10 ? "0" : "") +
    at.getUTCDate()
  );
}

function parseRrule(lines) {
  if (!Array.isArray(lines)) return null;
  for (var i = 0; i < lines.length; i++) {
    var line = typeof lines[i] === "string" ? lines[i] : "";
    if (line.indexOf("RRULE") !== 0) continue;
    var body = line.slice(line.indexOf(":") + 1);
    var out = {};
    var parts = body.split(";");
    for (var k = 0; k < parts.length; k++) {
      var eq = parts[k].indexOf("=");
      if (eq === -1) continue;
      out[parts[k].slice(0, eq).toUpperCase()] = parts[k].slice(eq + 1);
    }
    return out.FREQ ? out : null;
  }
  return null;
}

function bydayNames(byday) {
  var out = [];
  var list = (byday || "").split(",");
  for (var i = 0; i < list.length; i++) {
    // Strip an ordinal prefix (`2FR`, `-1SU`): Graph carries that as `index`,
    // and BYSETPOS is where this mapper reads it from.
    var code = list[i].replace(/^[+-]?\d+/, "").toUpperCase();
    var name = GRAPH_DAY_NAME[code];
    if (name && out.indexOf(name) === -1) out.push(name);
  }
  return out;
}

// RFC 5545 -> Graph patternedRecurrence.
//
// Graph's pattern vocabulary is strictly narrower than RRULE's, so a rule it
// cannot express returns null and the caller drops `recurrence` from the payload
// rather than sending an approximation — a "close enough" pattern would silently
// reschedule a real series. EXDATE/RDATE lines are likewise not carried: Graph
// has no such field on a master and models the same thing as exception events,
// which are their own nodes.
function graphRecurrence(lines, startDate, tz) {
  var r = parseRrule(lines);
  if (!r || !startDate) return null;
  var d = dateParts(startDate);
  if (!d) return null;
  var freq = String(r.FREQ).toUpperCase();
  var interval = Number(r.INTERVAL || 1) || 1;
  var days = bydayNames(r.BYDAY);
  var setpos = r.BYSETPOS !== undefined ? GRAPH_INDEX_NAME[String(Number(r.BYSETPOS))] : null;
  var monthDay = r.BYMONTHDAY ? Number(String(r.BYMONTHDAY).split(",")[0]) : null;
  var month = r.BYMONTH ? Number(String(r.BYMONTH).split(",")[0]) : null;
  // Weekday of the series start, needed when a WEEKLY rule omits BYDAY. Naive
  // date read as UTC: a calendar weekday does not depend on the zone.
  var startDow = DOW_BY_INDEX[new Date(Date.UTC(d.y, d.m - 1, d.d)).getUTCDay()];

  var pattern = null;
  if (freq === "DAILY") {
    pattern = { type: "daily", interval: interval };
  } else if (freq === "WEEKLY") {
    pattern = {
      type: "weekly",
      interval: interval,
      daysOfWeek: days.length ? days : [startDow],
    };
    var wkst = GRAPH_DAY_NAME[String(r.WKST || "").toUpperCase()];
    if (wkst) pattern.firstDayOfWeek = wkst;
  } else if (freq === "MONTHLY") {
    if (days.length && setpos) {
      pattern = { type: "relativeMonthly", interval: interval, daysOfWeek: days, index: setpos };
    } else {
      pattern = { type: "absoluteMonthly", interval: interval, dayOfMonth: monthDay || d.d };
    }
  } else if (freq === "YEARLY") {
    if (days.length && setpos) {
      pattern = {
        type: "relativeYearly",
        interval: interval,
        daysOfWeek: days,
        index: setpos,
        month: month || d.m,
      };
    } else {
      pattern = {
        type: "absoluteYearly",
        interval: interval,
        dayOfMonth: monthDay || d.d,
        month: month || d.m,
      };
    }
  } else {
    // HOURLY / MINUTELY / SECONDLY have no Graph equivalent at all.
    return null;
  }

  var range = { type: "noEnd", startDate: startDate };
  if (r.COUNT) {
    range = {
      type: "numbered",
      startDate: startDate,
      numberOfOccurrences: Number(r.COUNT),
    };
  } else if (r.UNTIL) {
    var endDate = untilToEndDate(r.UNTIL);
    if (endDate) range = { type: "endDate", startDate: startDate, endDate: endDate };
  }
  if (tz) range.recurrenceTimeZone = tz;

  return { pattern: pattern, range: range };
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
      // whose Windows timezone is missing from `WINDOWS_TO_IANA` moved the time
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
