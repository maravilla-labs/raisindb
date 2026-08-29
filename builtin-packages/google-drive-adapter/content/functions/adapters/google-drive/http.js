/**
 * ONE authorized request to Drive, and the ONE place a status code becomes an
 * engine dispatch code.
 *
 * Shared by every operation module. It is the only file that may decide what a
 * 401, a 403 or a 429 means: a second mapping would let a read and a write
 * disagree about whether an account needs reconnecting, which is the drift this
 * package pays for most often.
 */

import { coded } from "./common.js";

export var DRIVE = "https://www.googleapis.com/drive/v3";
export var UPLOAD = "https://www.googleapis.com/upload/drive/v3";

/**
 * Seconds Google asked us to wait, from `Retry-After`.
 *
 * Drive sends it on 429 and on the 403 usage-limit reasons, in delta-seconds.
 * Reading it is the difference between backing off as instructed and guessing:
 * the engine's exponential backoff has no idea whether the account is 20 seconds
 * or 20 minutes from recovery, and being wrong in the SHORT direction against
 * Google is the mistake this project has already paid for.
 *
 * Header casing is not guaranteed across hosts, so match case-insensitively. An
 * HTTP-date form (legal, rare from Google) is ignored rather than mis-parsed — a
 * wrong number is worse than none.
 */
function retryAfterSeconds(resp) {
  var headers = resp && resp.headers;
  if (!headers) return null;
  for (var k in headers) {
    if (String(k).toLowerCase() !== "retry-after") continue;
    var n = Number(headers[k]);
    if (isFinite(n) && n > 0) return Math.ceil(n);
    return null;
  }
  return null;
}

// Throw the reserved error codes the engine dispatches on. Never swallow an
// auth failure into an empty result — that reads as "everything was deleted".
export function raiseForStatus(resp, context, isWrite) {
  var status = resp.status;
  if (status >= 200 && status < 300) return;

  var body = resp.body || {};
  var reason = "";
  try {
    if (body && body.error && body.error.errors && body.error.errors.length) {
      reason = body.error.errors[0].reason || "";
    }
  } catch (_) {
    reason = "";
  }

  if (status === 401) {
    throw coded("Google Drive rejected the access token", "auth_expired");
  }
  if (status === 429) {
    throw coded("Google Drive rate limit exceeded", "rate_limited", retryAfterSeconds(resp));
  }
  if (
    status === 403 &&
    (reason === "rateLimitExceeded" ||
      reason === "userRateLimitExceeded" ||
      reason === "dailyLimitExceeded")
  ) {
    throw coded("Google Drive usage limit exceeded", "rate_limited", retryAfterSeconds(resp));
  }
  // A write-scope shortfall, which is the FIRST thing a newly writable mount
  // hits: the connector asks for a read scope, so every read succeeds and every
  // write 403s. Left as a plain Error this is transient, i.e. the same doomed
  // request re-sent on every drain forever, with the operator sent to reconnect
  // an account whose consent is not the problem. Terminal and named instead.
  if (status === 403 && isWrite) {
    throw coded(
      context + ": Google refused the write (403 " + (reason || "forbidden") +
        "). This is almost certainly a missing WRITE scope rather than a stale " +
        "token: add https://www.googleapis.com/auth/drive (or " +
        "https://www.googleapis.com/auth/drive.file for app-created files only) " +
        "to the Google connector's OAuth scopes and RECONNECT each account — " +
        "Google only issues a widened scope on fresh consent.",
      "config_error"
    );
  }
  var msg =
    (body && body.error && body.error.message) ||
    "Google Drive request failed (" + status + ")";
  throw new Error(context + ": " + msg);
}

// Single authorized request. `raisin.http.fetch` is synchronous and returns
// { status, headers, body }.
export function driveFetch(credential, method, url, opts) {
  opts = opts || {};
  // The engine passes `credential: null` when no account is selected; guard so
  // that surfaces as a readable error rather than a TypeError. Plain Error on
  // purpose — a coded "auth_expired" would be rewritten by the host into
  // "credential is expired or was rejected", the wrong diagnosis here.
  if (!credential || !credential.access_token) {
    throw new Error(
      "no account credential — connect a Google account and select it for this connector or mount"
    );
  }
  var headers = { Authorization: "Bearer " + credential.access_token };
  if (opts.headers) {
    for (var k in opts.headers) headers[k] = opts.headers[k];
  }
  var request = { method: method, headers: headers };
  if (opts.body !== undefined) request.body = opts.body;
  var resp = raisin.http.fetch(url, request);
  if (!opts.rawStatusOk || (resp.status !== 404 && resp.status !== 412)) {
    raiseForStatus(resp, opts.context || method + " " + url, opts.write);
  }
  return resp;
}

// One response header, whatever the host capitalized it as.
//
// The host builds this map from reqwest's own header names, which are
// lowercased — but the resumable session URL arrives in exactly one header and
// losing it means the bytes have nowhere to go, so the lookup does not bet on
// that staying true.
export function headerValue(headers, name) {
  if (!headers) return null;
  if (typeof headers[name] === "string") return headers[name];
  var lower = String(name).toLowerCase();
  for (var k in headers) {
    if (String(k).toLowerCase() === lower && typeof headers[k] === "string") {
      return headers[k];
    }
  }
  return null;
}
