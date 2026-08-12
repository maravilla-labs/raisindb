/**
 * Microsoft 365 mail mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 * Bidirectional (adapter contract §6.0): it dispatches on input.operation and
 * both directions live here, so node shape and its inverse have one author.
 *
 *   to_node             { external_item, mount }  -> { node_type, name?, properties } | null
 *   to_external         { node, mount, fields? }  -> { payload, external_id? } | null
 *   mapper_capabilities { mount }                 -> { to_external: true }
 *
 * An absent operation means to_node, so the engine's read path is unchanged.
 *
 * Messages map to `raisin:Mail`, a global nodetype shared with the IMAP/Gmail
 * connectors. It declares the indexing that makes mail queryable: Fulltext on
 * subject/body_html/body_text/snippet, Property on from_address,
 * conversation_id, message_id, date, unread, folder — so
 * `GROUP BY conversation_id` and sender filters are index-backed rather than
 * full scans.
 *
 * The MESSAGE is not raisin:Asset: it is metadata plus a body that is only
 * inlined when the mount sets `sync_config.include_body`. Its ATTACHMENTS are,
 * as `raisin:Asset` child nodes (see `attachmentChildren`) — metadata at sync,
 * bytes on demand through the adapter's get_content.
 *
 * `name` stays the Graph item id (external_item.name) so distinct messages never
 * collide on a path; the human-readable subject lives in the `subject` property.
 *
 * The engine stamps the reserved __virtual/__mount_id/__external_id/__etag/
 * __synced_at properties on top of whatever is returned, so they are not set here.
 */

function handler(input) {
  switch (input && input.operation) {
    case "to_external":
      return toExternal(input.node, input.mount, input.fields);
    // Probed once per sync run. Without it the mount is reported read-only,
    // which is what every mapper that has not been taught to_external wants.
    case "mapper_capabilities":
      return { to_external: true };
    // Absent operation === "to_node": the engine sent "to_node" long before any
    // mapper switched on it, and a mapper must keep working either way.
    case "to_node":
    default:
      return toNode(input);
  }
}

function toNode(input) {
  var item = input.external_item;
  if (!item || !item.external_id) return null;

  var meta = item.metadata || {};
  var mount = input.mount || {};
  var sync = mount.sync_config || {};

  var props = {
    subject: meta.subject || "(no subject)",
    from: meta.from || null,
    from_address: meta.from_address || null,
    to: meta.to || null,
    cc: meta.cc || null,
    // Only ever populated on a copy the account itself sent, and that is the
    // point: without them a synced Sent Items copy cannot seed a reply-all
    // without silently dropping recipients.
    bcc: meta.bcc || null,
    reply_to: meta.reply_to || null,
    date: meta.date || item.modified_at || null,
    // The unambiguous halves of `date`. "no received_at" is how an outgoing
    // copy is recognised without branching on folder name.
    received_at: meta.received_at || null,
    sent_at: meta.sent_at || null,
    is_draft: meta.is_draft === true,
    snippet: meta.snippet || null,
    unread: meta.unread === true,
    // The Graph-truth ALIAS of `unread` (is_read === !unread; it does NOT
    // invert against Graph's isRead). It exists because "is_read" is the
    // natural snake_case of Graph's own field and callers kept writing it —
    // and an edit to a property the engine does not carry in its
    // __pushed_state baseline dies silently at divergence detection, with no
    // error anywhere. Emitting BOTH spellings here keeps the baseline complete
    // so an edit to either one diverges and nominates the node.
    is_read: meta.unread !== true,
    has_attachments: meta.has_attachments === true,
    size: item.size_bytes != null ? item.size_bytes : null,
    importance: meta.importance || null,
    // Outlook categories land in the same column as Gmail labels: one concept,
    // one array, so a consumer never branches on provider to read a label.
    labels: Array.isArray(meta.labels) ? meta.labels : null,
    conversation_id: meta.conversation_id || null,
    // RFC 5322 Message-ID — stable across folders and providers, so it is what
    // identifies the same mail seen through both an Inbox and a Sent Items
    // mount. The Graph item id is per-folder and cannot do that.
    message_id: meta.internet_message_id || null,
    // Which mailbox this copy came from, so one query can span several mail
    // mounts and still tell them apart. Mirrors the adapter's remote_root
    // default of "inbox".
    folder: mount.remote_root || sync.remote_root || "inbox",
    web_url: item.web_url || meta.web_url || null,
    provider: "ms-graph",
    mime_type: "message/rfc822",
    // Provider-specific passthrough is preserved verbatim.
    provider_metadata: meta,
  };

  // `body` + `body_type` REPLACED by an explicit pair, split at the source: one
  // column whose meaning depended on a sibling column could not be searched,
  // rendered or replied to without branching.
  //
  // Each is set only when the adapter actually returned it. Writing "" for an
  // absent body would blank a previously synced body and, worse, change the node
  // on every run — defeating the etag skip-write that stops a re-sync from
  // re-firing every downstream trigger.
  if (typeof meta.body_html === "string") props.body_html = meta.body_html;
  if (typeof meta.body_text === "string") props.body_text = meta.body_text;

  // Attachments become raisin:Asset CHILD NODES, one per attachment, not an
  // `attachments` array property: the Drive adapter already maps provider blobs
  // to raisin:Asset, and a second parallel blob path is this codebase's most
  // expensive recurring bug class.
  //
  // METADATA ONLY. No `file` property is written here — its absence is what the
  // engine's on-demand `get_content` fetch keys off, and a placeholder Resource
  // pointing at nothing would read to every consumer as "the bytes are there".
  //
  // Present only when the mount set `sync_config.include_attachments`; absent
  // means "not synced", which is why a child list is never emitted as empty.
  var children = attachmentChildren(meta.attachments);

  var out = {
    node_type: "raisin:Mail",
    // id, not subject — path stability is owned by the adapter's external_id.
    name: item.name,
    properties: props,
  };
  if (children) out.children = children;
  return out;
}

/**
 * One raisin:Asset child per attachment.
 *
 * `external_id` is the provider's attachment id, which is unique only WITHIN the
 * message — the engine namespaces it under the message's own external id, so
 * two messages carrying attachment "1" do not collide.
 *
 * `name` is that id TOO, not the filename, because name is what the child's node
 * PATH is built from and a filename is not unique. One message carrying two
 * `scan.pdf` attachments resolved both children onto one path; the engine kept
 * whichever came last, dropped the other with a warning that counted as neither
 * a write nor a failure, and then alternated the surviving node between the two
 * on every subsequent run. The mail node itself already follows this rule —
 * "external_id / name / relative_path are ALWAYS the Graph item id, never the
 * subject/title/filename" — and the filename keeps its home in `title`, where it
 * is displayable and searchable without being load-bearing.
 *
 * Returns null (not []) when the adapter reported nothing, so "attachments were
 * not synced" and "this message has none" stay distinguishable: an empty array
 * would tell the engine to reconcile away every attachment node it had.
 */
function attachmentChildren(list) {
  if (!Array.isArray(list) || !list.length) return null;
  var out = [];
  for (var i = 0; i < list.length; i++) {
    var a = list[i];
    if (!a || !a.external_id) continue;
    out.push({
      name: a.external_id,
      node_type: "raisin:Asset",
      external_id: a.external_id,
      properties: {
        title: a.name || a.external_id,
        file_type: a.mime_type || null,
        file_size: a.size != null ? a.size : null,
        inline: a.inline === true,
        // Angle brackets stripped: `body_html` references it as `cid:<value>`.
        content_id: a.content_id ? String(a.content_id).replace(/^<|>$/g, "") : null,
      },
    });
  }
  return out.length ? out : null;
}

/**
 * A mail message is immutable content with mutable STATE, so this mount is
 * `state_only`: only the properties named below ever push, and `fields` (when
 * the engine supplies it) narrows that further to the ones that actually
 * changed. Anything outside the allow-list is dropped rather than guessed at —
 * sending a whole message object where a patch was meant is how a sync
 * overwrites a body it was never asked to touch.
 *
 * `unread` inverts: Graph's property is `isRead`. `is_read` is its Graph-truth
 * alias (declared in the adapter's capabilities too) and does NOT invert.
 */
var WRITABLE_FIELDS = ["unread", "is_read"];

function toExternal(node, mount, fields) {
  if (!node) return null;
  var props = node.properties || {};
  var wanted = fields && fields.length ? fields : WRITABLE_FIELDS;

  // `fields` is the DIVERGED-ONLY subset, not "everything writable": the
  // engine names exactly the properties whose value moved off the
  // __pushed_state baseline. Any single member must therefore produce a full,
  // valid PATCH on its own — gating a payload key on two fields arriving
  // together is how a one-field divergence becomes a null return, a Skipped
  // push, and a node re-nominated forever with no request ever going out.
  //
  // `unread` and `is_read` are two spellings of ONE Graph property, so they
  // are resolved to a single `isRead` rather than looped over independently.
  // `unread` is the canonical raisin:Mail column and wins when both diverged
  // and disagree — they only disagree when a caller wrote one spelling over
  // stale sync state in the other, and trusting the canonical column is the
  // deterministic answer. Absent is not the same as false either way: a node
  // that never carried the flag must not be pushed as "read", hence the
  // typeof gates.
  var payload = {};
  var emitted = 0;
  if (wanted.indexOf("unread") !== -1 && typeof props.unread === "boolean") {
    payload.isRead = props.unread !== true;
    emitted++;
  } else if (wanted.indexOf("is_read") !== -1 && typeof props.is_read === "boolean") {
    payload.isRead = props.is_read === true;
    emitted++;
  }

  // Nothing writable in this request: say "not writable" rather than issuing an
  // empty PATCH that touches the message's change key for no reason.
  if (!emitted) return null;

  var out = { payload: payload };
  if (props.__external_id) out.external_id = props.__external_id;
  return out;
}
