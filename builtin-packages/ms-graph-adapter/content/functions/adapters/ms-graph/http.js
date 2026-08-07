// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The Graph HTTP hop: the base URL, one `fetch` wrapper, and the ONE place a
//! status code becomes a classified `AdapterError`.
//!
//! Every request in this adapter goes through `graphFetch`, so the retry/park
//! semantics of a 429 or a 503 are decided once rather than per operation.

import { coded } from "./common.js";

export var GRAPH = "https://graph.microsoft.com/v1.0";

export function raiseForStatus(resp, context) {
  var status = resp.status;
  if (status >= 200 && status < 300) return;
  var body = resp.body || {};
  if (status === 401 || status === 403) {
    throw coded("Microsoft Graph rejected the access token", "auth_expired");
  }
  if (status === 429) {
    throw coded("Microsoft Graph rate limit exceeded", "rate_limited");
  }
  var msg =
    (body && body.error && body.error.message) ||
    "Microsoft Graph request failed (" + status + ")";

  // 400 and 404 mean Graph rejected the REQUEST, not the moment: a malformed or
  // unknown folder/calendar/drive id, or a resource this mailbox does not have.
  // Retrying sends the identical request and gets the identical rejection, so
  // these are reported as config errors — the engine records the diagnosis,
  // stops retrying and marks the mount misconfigured, instead of hammering
  // Graph on every scheduler tick.
  //
  // An EXPIRED DELTA CURSOR. Graph answers 410 Gone, and also reports it as
  // `syncStateNotFound` / `resyncRequired` — sometimes with a 400, which is why
  // this is checked BEFORE the 400/404 branch below. The documented recovery is
  // "start over with a full sync", so it is reported as `cursor_invalid`: the
  // engine drops the stored cursor and full-reconciles in the same run.
  //
  // This is normal operation, not a fault. Until it had its own code it fell
  // through to a plain Error → `Transient`, so the job retried the same rejected
  // cursor three times per tick forever; the failure counter that accumulated
  // then gated the backfill fast path too, so a production mailbox could neither
  // delta-sync nor finish its import. Graph had moved to sync generation 51
  // while our stored token still said generation 1.
  var graphCode = (body && body.error && body.error.code) || "";
  var isStaleCursor =
    status === 410 ||
    graphCode === "syncStateNotFound" ||
    graphCode === "resyncRequired" ||
    /sync state.*not found|resync required/i.test(msg);
  if (isStaleCursor) {
    throw coded(context + ": " + msg, "cursor_invalid");
  }

  // Deliberately NARROW. Other 4xx codes must stay retryable:
  //   408 request timeout      — a blip
  //   409 conflict             — resolved by the next sync's fresh read
  // (401/403 and 429 are already handled above as auth_expired / rate_limited;
  //  410/resync is handled just above as cursor_invalid.)
  if (status === 400 || status === 404) {
    throw coded(context + ": " + msg, "config_error");
  }

  // SERVICE BUSY. Graph answers 503 (and 504) when it is shedding load, and it
  // means exactly what 429 means for our purposes: come back later, the request
  // itself was fine.
  //
  // Left as a plain Error these became `Transient`, which the engine retries
  // promptly — so a throttled tenant got hammered rather than backed off. On a
  // large calendar that is self-sustaining: the walk is slow because Graph is
  // throttling, the retries make it slower, the run outlives the job watchdog
  // and is killed before it can finalize, and the mount sits at `syncing`
  // forever while never finishing an import. Reported as `rate_limited` so the
  // engine requeues with backoff, which is the only response that lets the
  // walk converge.
  if (status === 503 || status === 504) {
    throw coded(context + ": Microsoft Graph is busy (" + status + ")", "rate_limited");
  }

  throw new Error(context + ": " + msg);
}

// Whether the CALLER asked to handle this status itself instead of letting
// `raiseForStatus` map it.
//
// `rawStatusOk` is the historical 404-only form and stays exactly as it was.
// `rawStatuses` is the general form, added for `update`: a PATCH's statuses do
// not mean what the same statuses mean on a GET (412 is a real conflict, 404 is
// "the message moved and got a new id", 403 is a missing WRITE scope), and
// `raiseForStatus` is shared by every read in this file — widening it there
// would change read behaviour to serve one writer.
export function statusIsRaw(opts, status) {
  if (opts.rawStatusOk && status === 404) return true;
  var list = opts.rawStatuses;
  if (list) {
    for (var i = 0; i < list.length; i++) {
      if (list[i] === status) return true;
    }
  }
  return false;
}

// Single authorized request. `raisin.http.fetch` is synchronous and returns
// { status, headers, body }.
export function graphFetch(credential, method, url, opts) {
  opts = opts || {};
  // The engine passes `credential: null` when no account is selected. Without
  // this guard that surfaced as an opaque
  // "cannot read property 'access_token' of null" TypeError from deep inside
  // the adapter. Plain Error on purpose: a coded "auth_expired" would be
  // rewritten by the host into "credential is expired or was rejected", which
  // is the wrong diagnosis for "no account connected".
  if (!credential || !credential.access_token) {
    throw new Error(
      "no account credential — connect a Microsoft account and select it for this connector or mount"
    );
  }
  var headers = { Authorization: "Bearer " + credential.access_token };
  if (opts.headers) {
    for (var k in opts.headers) headers[k] = opts.headers[k];
  }
  var request = { method: method, headers: headers };
  if (opts.body !== undefined) request.body = opts.body;
  var resp = raisin.http.fetch(url, request);
  if (!statusIsRaw(opts, resp.status)) {
    raiseForStatus(resp, opts.context || method + " " + url);
  }
  return resp;
}

