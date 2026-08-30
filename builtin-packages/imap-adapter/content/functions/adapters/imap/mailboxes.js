/**
 * Mailbox identity and mailbox -> path, in ONE place.
 *
 * The whole point of this module is that the full walk (`opList`) and the
 * incremental delta (`opGetChanges`) call the SAME two functions to decide what
 * a mailbox is called and where it sits. When those two disagree by so much as
 * a character, the engine relocates every node under the disagreeing folder on
 * every run — the divergence that bit google-drive and ms-graph before it could
 * bite us.
 *
 * THE DELIMITER IS PER MAILBOX, NOT PER SERVER. RFC 3501 returns a hierarchy
 * delimiter (or NIL) in each LIST response, and RFC 2342 lets two namespaces on
 * ONE server use different ones. The previous code guessed "/" and then "."
 * (index.js `mailboxParent`) and returned null for everything else, which
 * flattened the whole tree on any server using something else.
 *
 * The Rust binding does not surface the delimiter today: client.rs
 * `list_mailboxes_inner` reads `name.delimiter()` to compute the leaf and then
 * throws it away, so MailboxInfo carries only { name, path, flags }. Adding
 * `delimiter` to MailboxInfo is a binding change and is NOT in this package.
 * Until it lands we DERIVE the delimiter exactly rather than guessing: the
 * binding already split the leaf off the path with the real delimiter, so the
 * one character sitting between the parent and the leaf in `path` IS that
 * delimiter. `mailboxDelimiter` prefers `mbox.delimiter` the moment the binding
 * starts sending it, so this file needs no edit on that day.
 */

/**
 * The delimiter this mailbox's own path uses, or null for a top-level/flat one.
 *
 * Prefers what the server said (once the binding forwards it); otherwise reads
 * back the character the binding itself used to split `name` off `path`.
 */
export function mailboxDelimiter(mbox) {
  if (!mbox) return null;
  if (typeof mbox.delimiter === "string" && mbox.delimiter.length) return mbox.delimiter;
  var path = mbox.path || "";
  var name = mbox.name || "";
  if (!path || !name || path === name) return null;
  if (path.length <= name.length) return null;
  if (path.slice(path.length - name.length) !== name) return null;
  var d = path.charAt(path.length - name.length - 1);
  return d.length ? d : null;
}

/** Parent mailbox PATH, or null when the mailbox is top-level. */
export function mailboxParentPath(mbox) {
  var d = mailboxDelimiter(mbox);
  if (!d) return null;
  var path = mbox.path || "";
  var name = mbox.name || "";
  var cut = path.length - name.length - d.length;
  if (cut <= 0) return null;
  return path.slice(0, cut);
}

/**
 * ONE sanitizer, used by both the walk and the delta.
 *
 * A "/" inside a mailbox NAME (perfectly legal on a "."-delimited server) would
 * otherwise introduce a path level the other side does not know about, so the
 * same message would resolve to two different paths and be relocated on every
 * run.
 */
export function segment(name) {
  var s = String(name == null ? "" : name)
    // Control characters first: a name carrying one yields a path segment no
    // store or console can render, and either side could normalise it away and
    // then disagree with the other about the path.
    .replace(/[\u0000-\u001f\u007f]/g, "")
    .replace(/\//g, "-")
    .trim();
  return s.length ? s : "_";
}

/**
 * The segment chain from just BELOW `rootPath` down to `path`, as an array.
 *
 * Returns [] for the root itself and null when `path` is not under the root —
 * null means SKIP, never "place it at the root", because placing an unrelated
 * mailbox at the mount root is how a mount silently swallows a folder nobody
 * asked it to sync.
 */
export function mailboxChain(path, delimiter, rootPath) {
  var p = String(path || "");
  var root = String(rootPath || "");
  if (!p) return null;
  if (!root) {
    // No root configured: the whole account is the tree.
    return delimiter ? p.split(delimiter).map(segment) : [segment(p)];
  }
  if (p === root) return [];
  // FALLBACK, and it is not cosmetic. `mailboxDelimiter` can only read the
  // delimiter back when the leaf is a suffix of the path, which a binding is not
  // obliged to guarantee (Gmail's "[Gmail]/All Mail" is the shape that broke it:
  // a display name that is not the path's tail). Returning null there dropped
  // the mailbox as "not under the root" — and with it the skip that keeps All
  // Mail out, so its CHILDREN were then synced while it was not. The root prefix
  // names the delimiter just as exactly: whatever character separates the root
  // from the rest of this path IS it.
  var d = delimiter;
  if (!d && p.length > root.length && p.slice(0, root.length) === root) {
    d = p.charAt(root.length);
  }
  if (!d) return null;
  var delimiter_ = d;
  var prefix = root + delimiter_;
  if (p.length <= prefix.length || p.slice(0, prefix.length) !== prefix) return null;
  return p.slice(prefix.length).split(delimiter_).map(segment);
}

function hasAttr(flags, attr) {
  var f = flags || [];
  var want = String(attr).replace(/\\/g, "").toLowerCase();
  for (var i = 0; i < f.length; i++) {
    if (String(f[i]).replace(/\\/g, "").toLowerCase() === want) return true;
  }
  return false;
}

/**
 * Can this mailbox hold messages at all? \Noselect / \NonExistent mailboxes are
 * pure hierarchy nodes; SELECTing one is a protocol error, so a tree mount that
 * tried would spend a whole login per poll to fail.
 */
export function selectable(flags) {
  return !hasAttr(flags, "Noselect") && !hasAttr(flags, "NonExistent");
}

/**
 * Mailboxes a tree mount must not descend into.
 *
 * \All is the load-bearing one: Gmail's "[Gmail]/All Mail" re-lists EVERY
 * message in the account, so a naive tree mount imports the entire mailbox a
 * second time under a second path. \Trash and \Junk are excluded because a tree
 * mount is for the mail someone works, and both are high-volume noise.
 *
 * Keyed on the RFC 6154 SPECIAL-USE attributes the binding already puts in
 * MailboxInfo.flags — never on the names, which Gmail localises ("[Gmail]/Alle
 * Nachrichten"), so a name-based rule silently stops working per account.
 * A server that does not advertise SPECIAL-USE reports none of these, and its
 * Trash/Junk are then synced like any other mailbox; `sync_config.exclude_patterns`
 * is the operator's answer there, and it works because the mailbox chain IS the
 * relative path in tree mode.
 */
export function skipByAttribute(flags) {
  return hasAttr(flags, "All") || hasAttr(flags, "Trash") || hasAttr(flags, "Junk");
}
