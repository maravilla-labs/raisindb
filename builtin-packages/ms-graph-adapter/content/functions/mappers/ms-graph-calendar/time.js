/**
 * Time and zone translation for the Microsoft 365 calendar mapper — BOTH
 * directions, deliberately in one file.
 *
 * Graph speaks a naive wall clock plus a Windows zone name; raisin:Event speaks
 * a UTC instant, a naive local, and an IANA zone. Reading (`utcOf`, `localOf`,
 * `ianaZone`) and writing (`naive`, `graphTime`) are two halves of the SAME
 * mapping, and they only stay inverses if they are edited together — splitting
 * them across files is how one side learns a zone the other does not and an
 * event round-trips to a different hour.
 *
 * Pure: no I/O, no globals. Imported by index.js and recurrence.js.
 */

// Zone names that mean UTC EXACTLY and never observe DST, so a naive wall clock
// carried beside one is already the UTC wall clock and the instant is exact
// arithmetic rather than a lookup.
//
// The list is short on purpose. It must never grow a name whose offset varies:
// Windows "GMT Standard Time" is Europe/London, which is UTC only in winter —
// including it would move every British summer meeting by an hour. Membership
// here is a claim that the offset is zero on EVERY date, forever.
var UTC_ZONES = {
  "UTC": true,
  "Etc/UTC": true,
  "Etc/GMT": true,
  "Etc/GMT+0": true,
  "Etc/GMT-0": true,
  "Etc/Zulu": true,
  "Etc/Universal": true,
  "Universal": true,
  "Zulu": true,
  "GMT": true, // the IANA zone, fixed at +00:00 — NOT "GMT Standard Time"
};

// Naive "YYYY-MM-DDTHH:MM[:SS][.fraction]" -> the same wall clock stamped Z.
// The fraction is dropped rather than rounded: Graph sends seven digits and
// raisin:Event.*_utc is a second-resolution column, so keeping it would only
// make the value differ from every other row for no gain.
function utcOfNaive(s) {
  var m = /^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}(?::\d{2})?)/.exec(s);
  if (!m) return null;
  var t = m[2].length === 5 ? m[2] + ":00" : m[2];
  var d = new Date(m[1] + "T" + t + "Z");
  if (isNaN(d.getTime())) return null;
  return d.toISOString().replace(/\.\d+Z$/, "Z");
}

// The UTC instant, or null when it is not RECOVERABLE — never a guess. A wrong
// value in an ordered instant column is far worse than an absent one, because
// nothing downstream can tell it is wrong.
//
// Two things make it recoverable:
//   1. the string carries its own designator (Z or ±HH:MM), or
//   2. `tz` is a zone that is UTC on every date (see UTC_ZONES) — then the
//      naive wall clock Graph sent IS the UTC wall clock.
//
// (2) is not a special case, it is the common one: the adapter deliberately
// does not send `Prefer: outlook.timezone`, and Graph's default for these reads
// is UTC, so production events arrive as {dateTime: "…T21:30:00.0000000",
// timeZone: "UTC"} — unambiguous, and previously thrown away because the zone
// sat in a SEPARATE field from the wall clock.
//
// WHY NOT EVERY NAMED ZONE. Converting "W. Europe Standard Time" needs a tz
// database, and this code runs in QuickJS where `Intl` is UNDEFINED (verified:
// `typeof Intl === "undefined"`, and `Date.prototype.toLocaleString` silently
// IGNORES its `timeZone` option — asking for Asia/Tokyo, UTC and the default
// all return the host's local rendering, so it fails without erroring). The
// only alternative is a hand-rolled DST table that goes stale the next time a
// government moves a transition. So a named zone keeps writing null, and the
// wall clock plus `timezone` remains the truth for those events.
export function utcOf(s, tz) {
  if (typeof s !== "string" || !s) return null;
  if (/(Z|[+-]\d{2}:?\d{2})$/.test(s)) {
    var d = new Date(s);
    if (isNaN(d.getTime())) return null;
    return d.toISOString().replace(/\.\d+Z$/, "Z");
  }
  // `=== true`, not truthiness: these are BARE objects, so `UTC_ZONES["constructor"]`
  // (or "toString", "valueOf", …) reaches Object.prototype and is truthy. That
  // would stamp a Z on a zone we have proved nothing about — the exact opposite
  // of what this table means — for any provider that ever sends such a name.
  if (typeof tz !== "string" || UTC_ZONES[tz] !== true) return null;
  return utcOfNaive(s);
}

export function dateOf(s) {
  var m = typeof s === "string" ? /^(\d{4}-\d{2}-\d{2})/.exec(s) : null;
  return m ? m[1] : null;
}

// Wall clock with no offset: "YYYY-MM-DDTHH:MM:SS", or the bare date.
export function localOf(s, allDay) {
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

export function ianaZone(name) {
  if (typeof name !== "string" || !name) return null;
  if (name.indexOf("/") !== -1) return name; // already IANA
  // Same prototype trap as UTC_ZONES: an unmapped name must become null, and
  // WINDOWS_TO_IANA["constructor"] is a FUNCTION, which would land in a string
  // column and break every consumer that reads it as a zone.
  var mapped = WINDOWS_TO_IANA[name];
  return typeof mapped === "string" ? mapped : null;
}

// "2026-08-11T09:00:00Z" / "2026-08-11T09:00:00" / "2026-08-11" -> the naive
// local datetime Graph wants beside an explicit `timeZone`.
export function naive(s, allDay) {
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
export function graphTime(local, utc, tz, allDay) {
  if (tz && local) return { dateTime: naive(local, allDay), timeZone: tz };
  if (utc) return { dateTime: naive(utc, allDay), timeZone: "UTC" };
  if (allDay && local) return { dateTime: naive(local, true), timeZone: tz || "UTC" };
  return null;
}
