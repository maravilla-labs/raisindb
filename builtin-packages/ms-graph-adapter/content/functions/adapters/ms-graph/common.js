// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Shared primitives: coded errors, URL encoding, and the empty-object test.
//!
//! No Graph knowledge and no I/O — anything here is safe for every other module
//! to import, which is what keeps the module graph a DAG.


// The engine sees a thrown Error only as its MESSAGE STRING (the QuickJS host
// surfaces nothing else), so anything the engine must act on has to survive in
// the text. `code` is matched there, and so is `retry_after=<seconds>`: when a
// provider states how long to wait, guessing an exponential backoff instead is
// strictly worse — too short re-hammers a throttled tenant, too long stalls a
// mount that was told it could resume in 20 seconds.
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

// Throw the reserved error codes the engine dispatches on. Never swallow an auth
// failure into an empty result — that reads to the engine as "everything was
// deleted". A plain Error (no `code`) is treated as transient and retried.

export function isEmptyObject(v) {
  if (!v || typeof v !== "object") return true;
  for (var k in v) {
    if (Object.prototype.hasOwnProperty.call(v, k)) return false;
  }
  return true;
}

// PATCH one message with the payload the MAPPER produced.
//
// `params` is what the engine's writeback sends:

export function enc(v) {
  return encodeURIComponent(v);
}

// A SharePoint site id is the COMPOSITE form `hostname,siteGuid,webGuid`, and
// Microsoft documents it with literal commas. A comma is a sub-delim that RFC
// 3986 permits unescaped in a path segment, and Graph's routing is happier with
// the documented spelling than with %2C — so encode everything (a site id is
// still operator-supplied input that must not smuggle a `/` or `?` into the
// path) and then put the commas back.
export function encSiteId(v) {
  return encodeURIComponent(v).split("%2C").join(",");
}
