/**
 * Recurrence translation for the Microsoft 365 calendar mapper — BOTH
 * directions, deliberately in one file.
 *
 * Graph's `patternedRecurrence` and RFC 5545's RRULE are ONE relationship, and
 * this mapper has to walk it in both directions: `recurrenceLines` reads Graph
 * into RRULE lines for raisin:Event.recurrence, `graphRecurrence` translates the
 * lines back into a pattern Graph will accept. When those two tables lived at
 * opposite ends of a 699-line index.js they drifted — a weekday, an index name
 * or a range type taught to one side and not the other is a series that
 * reschedules itself on the next save, and nothing in a one-way test sees it.
 * Here, a change to one direction is visibly a change beside the other.
 *
 * Pure: no I/O, no globals.
 */

import { dateOf } from "./time.js";

// ---- Graph -> RFC 5545 ----------------------------------------------------

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
export function recurrenceLines(raw) {
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

// ---- RFC 5545 -> Graph ----------------------------------------------------
//
// The inverse tables of DAY / SETPOS above. They are kept as their own maps
// rather than derived, because Graph's read and write vocabularies are not
// quite mirrors (`relativeYearly` reads as YEARLY but is written from BYDAY +
// BYSETPOS + BYMONTH), but they sit here so a weekday added to one is added to
// the other in the same edit.

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
export function graphRecurrence(lines, startDate, tz) {
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
