/**
 * The two things every part of this adapter needs, and neither of the transports
 * owns.
 *
 * Shared rather than duplicated because `coded` is how EVERY path in this
 * package reaches the engine's dispatch codes (auth_expired, rate_limited,
 * config_error, conflict): a second spelling of it in the send or push module
 * would be a second place for those reserved strings to drift, and a code the
 * engine does not recognise is silently classified transient.
 */

export function coded(message, code) {
  var e = new Error(message);
  e.code = code;
  return e;
}
