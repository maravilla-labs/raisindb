/**
 * Where a delta item lands, relative to the mount root.
 *
 * The counterpart of `ms-graph/paths.js`, and deliberately NOT the same
 * implementation: Graph puts a `parentReference.path` on every item, so that
 * adapter decodes a string, while Drive's changes feed carries only parent IDS
 * and Drive is a DAG — so this one WALKS the parent chain, caches it, and uses
 * the walk as the subtree filter too. Same file name, same job, visibly
 * different mechanism.
 */

import { enc } from "./common.js";
import { DRIVE, driveFetch, raiseForStatus } from "./http.js";

//
// ONLY the delta feed needs this. The full walk recurses folder by folder and
// the ENGINE accumulates the prefix as it descends (`full.rs`
// `resolve_item_path`: `{prefix}/{item.name}`, where the prefix is the parent
// folder's own resolved path). The changes feed has no recursion, so the path
// has to be reconstructed — and it has to come out IDENTICAL to the walk's, or
// the same file sits in one place after a backfill and another after a delta.
//
// That is what this replaces: `relative_path: item.name`, flat. A file two
// folders deep was delivered by the walk at `a/b/report.pdf` and by every delta
// at `report.pdf`, so the engine's remap MOVED the node on every disagreeing
// run — out of its folder on a delta, back into it on the next full reconcile,
// forever. Not cosmetic: a node move rewrites its path and everything that
// referenced the old one.
//
// TWO WAYS DRIVE DIFFERS FROM GRAPH, both handled here:
//
//  * The changes feed is ACCOUNT-WIDE. It reports every file the account can
//    see, not the mount's subtree, so the parent walk is also the SUBTREE
//    FILTER: an item whose ancestry never reaches the mount root returns null
//    and is dropped from the page. Nothing else keeps a stranger's file out of
//    the mount, and the engine joins `relative_path` to `mount_path` verbatim.
//
//  * Drive is a DAG, not a tree: a legacy file can have SEVERAL parents. The
//    rule is the FIRST parent (in Drive's own order) whose chain reaches the
//    mount root — deterministic, and the cheapest walk. A file with two parents
//    INSIDE one mount is genuinely ambiguous and the full walk is ambiguous
//    about it too (it lists the file under both folders and the materializer
//    keeps whichever it saw last), so no choice here can be "correct"; what
//    matters is that it is stable between runs. Drive has allowed only one
//    parent for files created since September 2020, so this is a legacy shape.
//
// Costs one `files.get` per ANCESTOR FOLDER not already seen, cached for the
// whole `get_changes` call — a page of siblings resolves its folder chain once.

var MAX_PARENT_DEPTH = 64;

export function newPathCache() {
  return { meta: {}, rootId: undefined };
}

// One folder's `{id, name, parents}`, cached. `null` means "gone or not
// readable", which the caller treats as a chain that cannot be followed rather
// than as the root.
function fileMeta(credential, cache, id) {
  if (Object.prototype.hasOwnProperty.call(cache.meta, id)) return cache.meta[id];
  var resp = driveFetch(
    credential,
    "GET",
    DRIVE + "/files/" + enc(id) + "?fields=" + enc("id,name,parents") +
      "&supportsAllDrives=true",
    { context: "get_changes(parent)", rawStatusOk: true }
  );
  var meta = null;
  if (resp.status !== 404) {
    raiseForStatus(resp, "get_changes(parent)");
    meta = resp.body || null;
  }
  cache.meta[id] = meta;
  return meta;
}

// The folder id every path must terminate at.
//
// `remote_root` when the mount names one. Otherwise the mount is the whole of My
// Drive, and the alias "root" has to be resolved to a real id: `parents` arrays
// never contain the alias, so leaving it unresolved would make every chain walk
// past the top and every item look like it lives outside the mount.
function mountRootId(credential, mount, cache) {
  if (cache.rootId !== undefined) return cache.rootId;
  var configured = mount && mount.remote_root;
  if (typeof configured === "string" && configured && configured !== "root") {
    cache.rootId = configured;
    return cache.rootId;
  }
  var resp = driveFetch(credential, "GET", DRIVE + "/files/root?fields=id", {
    context: "get_changes(root)",
  });
  cache.rootId = (resp.body && resp.body.id) || null;
  return cache.rootId;
}

// The folder names between the mount root and this item, or null when the chain
// never reaches the root.
function chainToRoot(credential, cache, rootId, parents, depth) {
  if (!parents || !parents.length) return null;
  // A malformed or circular parent graph must not spin: bounded, and answered
  // with "outside the mount", which drops the item rather than materializing it
  // somewhere invented.
  if (depth >= MAX_PARENT_DEPTH) return null;
  for (var i = 0; i < parents.length; i++) {
    var pid = parents[i];
    if (pid === rootId) return [];
    var meta = fileMeta(credential, cache, pid);
    if (!meta || !meta.name) continue;
    var up = chainToRoot(credential, cache, rootId, meta.parents, depth + 1);
    if (up !== null) return up.concat([meta.name]);
  }
  return null;
}

/**
 * One changed file's path relative to the mount root, or null to SKIP it.
 *
 * Names are used VERBATIM, exactly as the walk uses `item.name`. Drive permits a
 * "/" inside a file name and neither path survives that intact — but they fail
 * identically, which is the property that matters: an adapter that sanitized
 * here and not in `list` would reintroduce the flip-flop this function exists to
 * remove.
 */
export function changeRelativePath(credential, mount, cache, file) {
  var rootId = mountRootId(credential, mount, cache);
  if (!rootId) {
    // Refuse the page rather than emit paths we cannot place. A thrown plain
    // Error is transient: the cursor is not advanced and the changes are
    // re-delivered next run, whereas returning an empty page would advance the
    // token past changes nobody ever saw.
    throw new Error("get_changes: could not resolve the mount root folder id");
  }
  // Drive reports the mount's own folder like any other file. Emitting it would
  // create a folder node standing for the mount, inside itself.
  if (file.id === rootId) return null;
  var chain = chainToRoot(credential, cache, rootId, file.parents, 0);
  if (chain === null) return null;
  return chain.concat([file.name]).join("/");
}
