/**
 * The receipt every write hands back, and the rule that an id-less success is
 * not a success.
 *
 * Shared by `write.js` and `drive-upload.js` — the metadata writes and the byte
 * channel — so the two cannot drift on how the etag the engine stamps is
 * derived. That is the whole point of the file: a receipt carrying anything but
 * the etag the NEXT walk computes silently reverts the write one run later.
 */

import { coded } from "./common.js";

/**
 * The `{ external_id, etag }` the engine stamps back onto the node.
 *
 * THE ETAG MUST BE THE ONE THE NEXT WALK COMPUTES for the post-write state. The
 * read path skips an item only when its etag matches the stored one, so a
 * receipt carrying anything else makes the run FOLLOWING this push mismatch its
 * own write, rebuild the node from remote and reseed `__pushed_state` — silently
 * reverting whatever was edited while the push was in flight. So it is derived
 * with `toExternalItem`'s formula (`version`, falling back to `modifiedTime`)
 * and never from a response header, which the walk never sees.
 *
 * A null etag is not merely imprecise: the engine falls back to the STALE
 * pre-write value, which is the same clobber one step later. Callers that can
 * end up here without one read the file back instead.
 */
export function writeReceipt(body, fallbackId) {
  body = body && typeof body === "object" ? body : {};
  return {
    external_id: body.id || fallbackId || null,
    etag: body.version != null ? String(body.version) : body.modifiedTime || null,
  };
}

// The engine refuses to adopt a node without a real id, and it is right to: a
// fabricated one makes the node unmatchable and undeletable, and the next
// reconcile creates a SECOND copy at the provider. So an id-less 2xx is named
// here rather than passed on as a null.
export function requireId(receipt, context, status) {
  if (!receipt.external_id) {
    throw coded(
      context + ": Google accepted the request (HTTP " + status + ") but returned no " +
        "file id, so the new file cannot be matched to its node",
      "transient"
    );
  }
  return receipt;
}
