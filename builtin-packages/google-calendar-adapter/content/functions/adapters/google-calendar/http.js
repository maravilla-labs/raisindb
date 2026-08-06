// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The Calendar HTTP hop: the base URL, one `fetch` wrapper, and the ONE
//! place a status code becomes a classified `AdapterError`.
//!
//! Every request goes through `calFetch`, so what a 410 or a 429 means is
//! decided once rather than per operation.


export var CAL = "https://www.googleapis.com/calendar/v3";
export var DAY_MS = 86400000;
// Google web_hook channels live at most ~7 days; request the full window and let
// the engine's renewal job rotate before expiry.
export var WATCH_TTL_SECONDS = 604800;

export function coded(message, code) {
  var e = new Error(message);
  e.code = code;
  return e;
}

// Throw the reserved error codes the engine dispatches on. Never swallow an
// auth failure into an empty result — that reads as "everything was deleted".
export function raiseForStatus(resp, context) {
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
    throw coded("Google Calendar rejected the access token", "auth_expired");
  }
  if (status === 429) {
    throw coded("Google Calendar rate limit exceeded", "rate_limited");
  }
  if (status === 403) {
    if (
      reason === "rateLimitExceeded" ||
      reason === "userRateLimitExceeded" ||
      reason === "dailyLimitExceeded"
    ) {
      throw coded("Google Calendar usage limit exceeded", "rate_limited");
    }
    // Other 403s are authorization failures (revoked grant, insufficient scope).
    throw coded("Google Calendar denied the request", "auth_expired");
  }
  var msg =
    (body && body.error && body.error.message) ||
    "Google Calendar request failed (" + status + ")";

  // An EXPIRED syncToken. Google answers 410 GONE (reason `fullSyncRequired`)
  // and documents the recovery as "discard the token and do a full sync", which
  // is exactly what `cursor_invalid` asks the engine to do — in the same run,
  // rather than relying on some later full pass that may never be scheduled.
  //
  // This used to be a plain Error → `Transient`, so the job retried the same
  // rejected token three times per tick and the failure counter it accumulated
  // gated the mount's backfill fast path as well. Same defect the ms-graph
  // adapter had; fixed in both rather than left to drift.
  if (status === 410 || reason === "fullSyncRequired") {
    throw coded(context + ": " + msg, "cursor_invalid");
  }
  throw new Error(context + ": " + msg);
}

// Single authorized request. `raisin.http.fetch` is synchronous and returns
// { status, headers, body }.
export function calFetch(credential, method, url, opts) {
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
  // Callers that inspect specific statuses (404 on get, 410 on sync) opt out of
  // the automatic raise for exactly those codes.
  if (!opts.rawStatusOk) {
    raiseForStatus(resp, opts.context || method + " " + url);
  }
  return resp;
}

export function enc(v) {
  return encodeURIComponent(v);
}
