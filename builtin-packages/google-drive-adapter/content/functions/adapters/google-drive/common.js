/**
 * The three helpers with no provider in them at all.
 *
 * Shared because every module in this adapter reaches the engine's dispatch
 * codes through `coded` — a second spelling of it would be a second place for
 * the reserved strings (auth_expired, rate_limited, config_error, conflict) to
 * drift, and a code the engine does not recognise is silently classified
 * transient.
 */

// The engine sees a thrown Error only as its MESSAGE STRING (the QuickJS host
// surfaces nothing else), so anything the engine must act on has to survive in
// the text. `code` is matched there, and so is `retry_after=<seconds>`, which
// `parse_retry_after` (adapter.rs) reads back and caps at an hour: when Google
// states how long to wait, guessing an exponential backoff instead is strictly
// worse — too short re-hammers a throttled account, which is how throttling
// becomes self-sustaining, and too long stalls a mount that was told it could
// resume in 20 seconds. Same encoding as the ms-graph adapter, deliberately:
// the WIRE FORMAT is what the engine shares, since no module crosses a package
// boundary.
export function coded(message, code, retryAfterSeconds) {
  var text = message;
  var n = Number(retryAfterSeconds);
  if (isFinite(n) && n > 0) {
    text = text + " (retry_after=" + Math.ceil(n) + ")";
  }
  var e = new Error(text);
  e.code = code;
  if (isFinite(n) && n > 0) e.retry_after = Math.ceil(n);
  return e;
}

export function enc(v) {
  return encodeURIComponent(v);
}

export function isEmptyObject(v) {
  if (!v || typeof v !== "object") return true;
  for (var k in v) return false;
  return true;
}
