// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The BYTE half of a drive write: how big is too big, the inline payload, the
//! upload session, and `finalize_upload`.
//!
//! Split from `drive.js` so that file stays about addressing driveItems while
//! this one stays about moving bytes. Every limit in here is Microsoft's rather
//! than ours, which is also why they are named constants: a number inlined at a
//! call site is a number nobody revisits when the provider moves it.

import { coded } from "./common.js";
import { graphFetch, raiseForStatus } from "./http.js";
import { resourceOf } from "./mount.js";
import { WRITE_STATUSES, diagnoseWrite, ifMatch, writeReceipt } from "./write-common.js";

// Graph's name-collision instruction: a query parameter on the simple PUT, a
// property in the session body. Spelled literally rather than percent-encoded —
// `@` is a legal query character and this is the form Microsoft documents.
export var CONFLICT_KEY = "@microsoft.graph.conflictBehavior";

// Microsoft's ceiling for `PUT .../content`. Above it Graph answers 413, which
// is not in WRITE_STATUSES and would surface as an unclassified transient — so
// the whole point of this constant is that we never send that request.
//
// It sits BELOW the engine's 8 MiB inline ceiling on purpose: the engine can
// hand us bytes Graph will not take in one shot, so the fork is decided from
// `content.size`, never from whether base64 arrived.
export var SIMPLE_PUT_MAX = 4 * 1024 * 1024;

// The chunk the engine streams. Microsoft requires every fragment except the
// last to be a multiple of 320 KiB; 3.2 MiB is ten units and stays comfortably
// inside a default request timeout on a slow link. A non-multiple is rejected
// mid-transfer, after the bytes have already been sent.
export var UPLOAD_CHUNK_SIZE = 10 * 320 * 1024;

// ---- the size fork --------------------------------------------------------

// Exact byte count, from the engine's stated size or — failing that — from the
// base64 itself (4 characters carry 3 bytes, less the padding).
export function contentSize(content) {
  var stated = content ? Number(content.size) : NaN;
  if (isFinite(stated) && stated >= 0) return stated;
  var b64 = content && content.content_base64;
  if (typeof b64 !== "string") return null;
  var pad = 0;
  if (b64.charAt(b64.length - 1) === "=") pad = b64.charAt(b64.length - 2) === "=" ? 2 : 1;
  return Math.floor(b64.length / 4) * 3 - pad;
}

// Whether this content can go up in ONE request.
//
// Requires BOTH: bytes we actually hold, and a size Graph will accept. An
// unknown size takes the session path — guessing "small" costs a 413 that no
// retry can turn into a success, while guessing "large" costs one round trip.
export function inlineable(content) {
  if (!content || content.inline !== true) return false;
  var size = contentSize(content);
  return size !== null && size <= SIMPLE_PUT_MAX;
}

// The bytes, or a stated failure. `inline: true` with no `content_base64` is a
// contract break on the engine's side, and the reason it is fatal rather than
// tolerated is what would otherwise happen: an empty PUT stores a ZERO-BYTE
// file at the provider, Graph answers 201 with a real id, and the engine adopts
// the node as successfully mirrored. A file that reports success and holds
// nothing is worse than a file that failed.
export function inlineBytes(content, context) {
  var b64 = content.content_base64;
  if (typeof b64 !== "string") {
    throw coded(
      context + ": the engine marked this content inline (size " + contentSize(content) +
        ") but sent no content_base64 — refusing to upload an empty file under a name " +
        "that would then report success",
      "config_error"
    );
  }
  return b64;
}

// ---- upload session -------------------------------------------------------

// Ask Graph for a session and hand the ENGINE the URL to stream to.
//
// `headers` is deliberately ABSENT from the answer. The session URL is
// pre-authenticated and lives on a *.sharepoint.com host outside this adapter's
// `allowed_urls`; attaching our bearer token would hand a Graph credential to a
// host we do not otherwise talk to, for no benefit — the fragment PUTs take no
// Authorization header.
export function beginUpload(credential, mount, url, name, behavior, etag, context) {
  var item = {};
  item[CONFLICT_KEY] = behavior;
  if (name) item.name = name;

  // An ordinary JSON POST to Graph — no new capability, no new host. The etag is
  // checked HERE and only here: once the session exists the engine streams
  // fragments to a URL that carries no If-Match, so a concurrent remote edit
  // arriving mid-transfer is not detectable. Stated rather than pretended.
  var resp = graphFetch(credential, "POST", url, {
    headers: ifMatch(etag, mount),
    body: { item: item },
    context: context,
    rawStatuses: WRITE_STATUSES,
  });
  if (diagnoseWrite(resp, context, "files") === "gone") {
    throw coded(
      context + ": the target does not exist in this drive. On a create that is the " +
        "parent folder (the mount's remote_root, or the parent node's external id); " +
        "on an update it is the item itself.",
      "config_error"
    );
  }
  var uploadUrl = resp.body && resp.body.uploadUrl;
  if (!uploadUrl) {
    throw coded(
      context + ": Microsoft Graph created no upload session (HTTP " + resp.status +
        ") — there is nowhere to send the bytes",
      "transient"
    );
  }
  return { upload: { url: uploadUrl, method: "PUT", chunk_size: UPLOAD_CHUNK_SIZE } };
}

// ---- finalize_upload ------------------------------------------------------

// The second half of an engine-streamed upload: `{ status, body, intent,
// item_id? }`, where `body` is the provider's parsed response to the LAST
// fragment.
//
// This call exists so provider-shaped parsing stays in the adapter. The engine
// moved the bytes; it must not also learn that a Microsoft driveItem keeps its
// id in `id` and its etag in `@odata.etag`.
export function opFinalizeUpload(mount, params) {
  var resource = resourceOf(mount);
  if (resource !== "files") {
    throw coded(
      "finalize_upload: only the files resource streams uploads (this mount is '" +
        resource + "')",
      "config_error"
    );
  }
  var status = Number(params.status);
  var body = params.body || {};
  var what = params.intent === "update" ? "update" : "create";
  if (!isFinite(status)) {
    throw coded(
      "finalize_upload: no HTTP status for the completed upload (" + what + ")",
      "config_error"
    );
  }

  // Non-2xx keeps the shared taxonomy, so a 401 mid-upload is still auth_expired
  // and a 429 still carries its Retry-After.
  if (status < 200 || status >= 300) {
    raiseForStatus({ status: status, headers: {}, body: body }, "finalize_upload");
  }

  // 202 is Graph saying "send the next fragment". Reaching finalize on one means
  // the engine stopped streaming early; treating it as done would adopt a node
  // for a file that is still a partial upload at the provider.
  if (status !== 200 && status !== 201) {
    throw coded(
      "finalize_upload: HTTP " + status + " means the upload session is not finished " +
        "(202 = Graph is waiting for the next fragment), so there is no completed file " +
        "to adopt",
      "config_error"
    );
  }

  // FAIL LOUDLY on a missing id. Without one the engine cannot match the file to
  // its node: on a create it adopts nothing, and on an update it would stamp a
  // null etag, which falls back to the STALE pre-write value and lets the next
  // walk overwrite the push. An upload that reports success without an id is the
  // exact failure this whole path exists to prevent.
  if (!body.id) {
    throw coded(
      "finalize_upload: the upload session reported success (HTTP " + status + ") for " +
        "this " + what + " but returned no driveItem id, so the file cannot be matched " +
        "to its node",
      "transient"
    );
  }
  return writeReceipt({ body: body }, null);
}
