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

// An instant is only recoverable when the string carries Z or ±HH:MM.
export function utcOf(s) {
  if (typeof s !== "string" || !s) return null;
  if (!/(Z|[+-]\d{2}:?\d{2})$/.test(s)) return null;
  var d = new Date(s);
  if (isNaN(d.getTime())) return null;
  return d.toISOString().replace(/\.\d+Z$/, "Z");
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
  return WINDOWS_TO_IANA[name] || null;
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
