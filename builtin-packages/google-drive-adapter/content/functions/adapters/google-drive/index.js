/**
 * Google Drive virtual-node adapter.
 *
 * Implements the frozen adapter contract (docs/reference/virtual-node-adapters.md)
 * over the Google Drive v3 REST API using the synchronous `raisin.http.fetch`
 * binding. The sync engine invokes this function directly, decrypts the account
 * credential just before the call, and materializes returned items into nodes.
 *
 * Entrypoint: handler(input) — exactly one argument.
 *   input = { operation, params, credential, mount }
 *
 * Token lifecycle is owned entirely by the engine: `credential.access_token` is
 * a current, decrypted token; there is NO refresh_token and no refresh logic
 * here. If a token is rejected, throw `auth_expired` and let the engine handle
 * the reconnect/refresh cycle.
 *
 * WRITES are the full mirror set — create, update, delete — plus BYTES. Two
 * facts about Google shape that half and are worth knowing before reading it:
 *
 *   * `raisin.http.fetch` can send raw bytes (`bodyBase64`) but cannot assemble
 *     a multipart/related body around them, so every content write goes through
 *     a RESUMABLE UPLOAD SESSION: this adapter negotiates the session (an
 *     ordinary JSON call) and hands the ENGINE the URL to stream to, and the
 *     engine calls back with `finalize_upload`. That is also why the multipart
 *     path that used to live here is gone — it stringified `params.content`,
 *     which the engine sends as an OBJECT, so it could only ever have uploaded
 *     the literal text "[object Object]".
 *   * Drive's changes feed is ACCOUNT-WIDE, not folder-scoped, and Drive is a
 *     DAG rather than a tree. Both are handled in `changeRelativePath` below.
 *
 * THIS FILE IS DISPATCH ONLY. Everything else lives in the sibling modules the
 * other adapters in this repo already use — http, items, capabilities, read,
 * write-common, write, drive-upload, paths, changes — which the engine's module
 * loader resolves from the function node's own file map (`load_sibling_files`),
 * exactly as it does for ms-graph and google-calendar. Anything that hands this
 * file to a runtime must hand it the siblings too.
 */

import { opCapabilities } from "./capabilities.js";
import { opGet, opGetContent, opList } from "./read.js";
import { opCreate, opDelete, opUpdate } from "./write.js";
import { opFinalizeUpload } from "./drive-upload.js";
import { opGetChanges } from "./changes.js";

export function handler(input) {
  var operation = input.operation;
  var params = input.params || {};
  var credential = input.credential;
  var mount = input.mount || {};

  switch (operation) {
    case "capabilities":
      return opCapabilities();
    case "list":
      return opList(credential, mount, params);
    case "get":
      return opGet(credential, mount, params);
    case "get_content":
      return opGetContent(credential, params);
    case "create":
      return opCreate(credential, mount, params);
    case "update":
      return opUpdate(credential, mount, params);
    case "delete":
      return opDelete(credential, params);
    // The engine streamed the bytes itself and is handing back Drive's answer to
    // the final chunk. All that is left is reading a file's id and `version` out
    // of it — provider-shaped parsing, which is why it comes back here rather
    // than being done in Rust.
    case "finalize_upload":
      return opFinalizeUpload(credential, mount, params);
    case "get_changes":
      return opGetChanges(credential, mount, params);
    default:
      throw new Error("Unsupported operation: " + operation);
  }
}
