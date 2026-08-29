/**
 * The BYTE channel: opening a resumable upload session, and reading Drive's
 * answer to the last chunk the engine streamed.
 *
 * Separate from `write.js` because the bytes never pass through this adapter at
 * all — it negotiates a session, hands the ENGINE a URL, and is called back with
 * `finalize_upload`. Everything about Drive's resumable protocol (the 256 KiB
 * chunk rule, the 308) is claimed here and nowhere else.
 */

import { coded } from "./common.js";
import { driveFetch, headerValue, raiseForStatus } from "./http.js";
import { writeReceipt } from "./write-common.js";
import { opGet } from "./read.js";

// Drive requires every chunk of a resumable upload EXCEPT THE LAST to be a
// multiple of 256 KiB; a non-multiple is rejected mid-transfer, after the bytes
// have already crossed the wire. 10 MiB is 40 such units.
//
// It is sent explicitly even though the engine's current fallback happens to be
// the same number: the 256 KiB rule is GOOGLE'S, the engine's default is the
// engine's, and an adapter that leans on someone else's default is one release
// away from having every non-final chunk rejected. The engine uses this value
// VERBATIM and rounds nothing, precisely so the provider's rule stays here.
export var UPLOAD_CHUNK_SIZE = 40 * 256 * 1024;

// Drive answers EVERY non-final chunk of a resumable upload with `308 Resume
// Incomplete`. Declared for the same reason as the chunk size: 308 is a fact
// about Drive's protocol, and an engine default of `[]` (2xx only) would fail
// every multi-chunk upload on chunk one. On the FINAL chunk a 308 is still a
// hard failure — it means Drive does not consider the object written — and that
// judgement belongs to the engine, which makes it.
export var UPLOAD_CONTINUE_STATUSES = [308];

/**
 * Open a resumable upload session and hand the ENGINE the URL to stream to.
 *
 * Why every content write goes this way, small files included: `raisin.http.fetch`
 * sends raw bytes only as a whole `bodyBase64` body, so a multipart/related
 * envelope (JSON metadata part + binary part) cannot be assembled here at all —
 * concatenating base64 fragments is not base64. The alternatives were a
 * metadata POST followed by a `uploadType=media` PATCH, which leaves an empty
 * file at the provider and an unadoptable orphan whenever the second call fails,
 * or this: ONE call that either yields a session or fails having created
 * nothing.
 *
 * `headers` is deliberately ABSENT from the answer. The session URL carries its
 * own `upload_id` and is pre-authenticated; attaching our bearer token would put
 * a Google credential on a URL this adapter does not otherwise talk to, for no
 * benefit.
 *
 * The metadata travels IN the initiation body, so a "renamed and re-uploaded"
 * push is one request and cannot half-apply. The initiation URL's query string
 * (`fields`) is replayed on the session's final response, which is what makes
 * `version` — the etag the walk computes — available to `finalize_upload`.
 */
export function beginUpload(credential, method, url, metadata, context) {
  var resp = driveFetch(credential, method, url, {
    headers: { "Content-Type": "application/json; charset=UTF-8" },
    body: metadata,
    context: context,
    write: true,
  });
  var session = headerValue(resp.headers, "Location");
  if (!session) {
    throw coded(
      context + ": Google opened no resumable session (HTTP " + resp.status +
        ", no Location header), so there is nowhere to send the bytes",
      "transient"
    );
  }
  return {
    upload: {
      url: session,
      method: "PUT",
      chunk_size: UPLOAD_CHUNK_SIZE,
      continue_statuses: UPLOAD_CONTINUE_STATUSES,
    },
  };
}

/**
 * The second half of an engine-streamed upload: `{ status, body, headers,
 * intent, item_id }`, where `body` is Drive's parsed answer to the LAST chunk.
 *
 * This call exists so provider-shaped parsing stays in the adapter — the engine
 * moved the bytes and must not also learn that a Drive file keeps its id in `id`
 * and its concurrency token in `version`.
 *
 * `headers` is read only as a last resort. Drive answers a completed session
 * with the file resource as JSON, so unlike S3's `PutObject` there is a body to
 * read; the header path is here because the engine now supplies it and a bodiless
 * 200 would otherwise stamp a null etag, which falls back to the STALE pre-write
 * value and lets the next walk overwrite this upload.
 */
export function opFinalizeUpload(credential, mount, params) {
  params = params || {};
  var status = Number(params.status);
  var what = params.intent === "update" ? "update" : "create";
  if (!isFinite(status)) {
    throw coded(
      "finalize_upload: no HTTP status for the completed upload (" + what + ")",
      "config_error"
    );
  }
  // Non-2xx keeps the shared taxonomy, so a 401 at the end of an upload is still
  // auth_expired and a 429 is still rate_limited. A 308 cannot reach here — the
  // engine fails a non-2xx FINAL chunk itself — but if it ever did, this is
  // where it stops, because a 308 means Drive does not consider the file written.
  if (status < 200 || status >= 300) {
    raiseForStatus(
      { status: status, headers: params.headers || {}, body: params.body || {} },
      "finalize_upload",
      true
    );
  }

  var body = params.body && typeof params.body === "object" ? params.body : {};
  if (!body.id) {
    throw coded(
      "finalize_upload: the upload session reported success (HTTP " + status + ") for " +
        "this " + what + " but returned no file id, so the file cannot be matched to " +
        "its node",
      "transient"
    );
  }
  var receipt = writeReceipt(body, params.item_id || null);
  if (receipt.etag) return receipt;
  // A read-back rather than a null etag, for the reason `writeReceipt` states:
  // the engine would otherwise stamp the pre-write value and the next walk would
  // clobber the bytes this upload just stored. It goes through `opGet` so the
  // etag is byte-identical to the one the next walk computes.
  var item = opGet(credential, mount, { item_id: body.id });
  if (item) return { external_id: item.external_id, etag: item.etag };
  return receipt;
}
