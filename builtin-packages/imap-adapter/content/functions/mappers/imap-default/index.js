/**
 * Default IMAP mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 *   input  = { external_item: ExternalItem, mount: { mount_id, mount_path, sync_config } }
 *   return = { node_type, name?, properties, children? } | null
 *
 * Mapping:
 *   - mailboxes -> raisin:Folder
 *   - messages  -> raisin:Mail
 *
 * MESSAGES MAP TO raisin:Mail, not raisin:Node. They did not until now, while
 * the ms-graph mapper did — and the shared nodetype's own comment claimed both
 * providers targeted it. The consequence was not cosmetic: `raisin:Mail`
 * declares the Fulltext and Property indexing that makes mail queryable, so
 * every IMAP-synced message was a `raisin:Node` carrying mail-shaped properties
 * that no mail query could find. A `GROUP BY conversation_id` or a sender filter
 * silently covered only the Graph half of a mailbox.
 *
 * Property NAMES follow the global nodetype, not the IMAP wire shape, for the
 * same reason: the point of one global type is that a consumer reads `subject`
 * and `from_address` without knowing which connector produced the row.
 *
 * `metadata.from` / `metadata.to` are already-formatted address strings from the
 * native raisin.imap binding (e.g. "Ada <ada@example.org>"), not arrays.
 *
 * The engine stamps the reserved __virtual/__mount_id/__external_id/__etag/
 * __synced_at properties on top of whatever is returned, so they are not set here.
 */

function handler(input) {
  var item = input.external_item;
  if (!item || !item.external_id) return null;

  if (item.is_folder) {
    return {
      node_type: "raisin:Folder",
      name: item.name,
      properties: {
        title: item.name,
        icon: "folder",
      },
    };
  }

  var meta = item.metadata || {};
  var mount = input.mount || {};
  var headers = meta.headers || {};
  var subject = meta.subject || item.name || "(no subject)";
  var flags = normalizeFlags(meta.flags);

  var properties = {
    subject: subject,
    from: meta.from || null,
    // Bare address, no display name. `from` carries whatever name the sender
    // currently uses, which changes over time and makes it useless as a
    // GROUP BY key — which is exactly what the nodetype indexes this for.
    from_address: bareAddress(meta.from),
    to: meta.to || null,
    cc: header(headers, "cc"),
    bcc: header(headers, "bcc"),
    reply_to: header(headers, "reply-to"),
    date: meta.date || item.modified_at || null,
    // IMAP has no notion of an outgoing copy at the protocol level, so the one
    // timestamp it gives is the message's own Date header. It is the received
    // time for anything in an incoming folder; `sent_at` is left to a header.
    received_at: meta.date || null,
    sent_at: header(headers, "date"),
    snippet: meta.snippet || null,
    message_id: meta.message_id || null,
    // The header pair is what reconstructs a thread across providers, and IMAP
    // is the provider most likely to have nothing else: `thread_id` is a Gmail
    // extension (X-GM-THRID) that plain RFC 3501 servers do not supply.
    in_reply_to: header(headers, "in-reply-to"),
    references: splitReferences(header(headers, "references")),
    thread_id: meta.thread_id || null,
    unread: meta.unread === true,
    flags: flags,
    is_draft: flags.indexOf("draft") !== -1,
    has_attachments: Array.isArray(meta.attachments) && meta.attachments.length > 0,
    size: item.size_bytes != null ? item.size_bytes : null,
    // folder_path FIRST, and only it is new. A tree mount spans N mailboxes, so
    // mount.remote_root — a mount-level constant — would label every message in
    // the tree with the mount root. It is empty for a message in the root
    // mailbox and absent in folder mode, both of which fall through to exactly
    // the value this line produced before, so no existing mount re-writes.
    folder: meta.folder_path || mount.remote_root || meta.mailbox || null,
    provider: "imap",
    mime_type: item.mime_type || "message/rfc822",
    // Provider-specific passthrough is preserved verbatim.
    provider_metadata: meta,
  };

  // Only when the adapter actually returned one. Writing "" for an absent body
  // would blank a previously synced body and change the node on every run,
  // defeating the etag skip-write that stops a re-sync re-firing every
  // downstream trigger.
  if (typeof meta.body_html === "string") properties.body_html = meta.body_html;
  if (typeof meta.body_text === "string") properties.body_text = meta.body_text;

  var out = {
    node_type: "raisin:Mail",
    name: subject,
    properties: properties,
  };
  var children = attachmentChildren(meta.attachments);
  if (children) out.children = children;
  return out;
}

/**
 * One raisin:Asset child per attachment, metadata only.
 *
 * Not an `attachments` array property: the Drive adapter already maps provider
 * blobs to raisin:Asset, and a second parallel blob path is this codebase's most
 * expensive recurring bug class. No `file` is written — its absence is what the
 * engine's on-demand `get_content` fetch keys off.
 *
 * `external_id` is the MIME part number for IMAP, which is unique only within
 * the message ("2" on nearly every message that has an attachment); the engine
 * namespaces it under the message's own external id so two messages cannot
 * collide.
 *
 * Returns null rather than [] when nothing was reported, so "attachments were
 * not synced" stays distinguishable from "this message has none" — an empty
 * array would tell the engine to reconcile away every attachment node it had.
 */
function attachmentChildren(list) {
  if (!Array.isArray(list) || !list.length) return null;
  var out = [];
  for (var i = 0; i < list.length; i++) {
    var a = list[i];
    var id = a && (a.part || a.external_id || a.id);
    if (!id) continue;
    out.push({
      name: a.name || a.filename || String(id),
      node_type: "raisin:Asset",
      external_id: String(id),
      properties: {
        title: a.name || a.filename || String(id),
        file_type: a.mime_type || a.content_type || null,
        file_size: a.size != null ? a.size : null,
        inline: a.inline === true || a.disposition === "inline",
        // Angle brackets stripped: `body_html` references it as `cid:<value>`.
        content_id: a.content_id ? String(a.content_id).replace(/^<|>$/g, "") : null,
      },
    });
  }
  return out.length ? out : null;
}

/** Case-insensitive header lookup; the binding's key casing is not guaranteed. */
function header(headers, name) {
  if (!headers) return null;
  for (var k in headers) {
    if (Object.prototype.hasOwnProperty.call(headers, k) && k.toLowerCase() === name) {
      var v = headers[k];
      if (Array.isArray(v)) v = v.join(", ");
      return v ? String(v) : null;
    }
  }
  return null;
}

/** "Ada <ada@example.org>" -> "ada@example.org"; a bare address passes through. */
function bareAddress(formatted) {
  if (!formatted) return null;
  var m = String(formatted).match(/<([^>]+)>/);
  return m ? m[1] : String(formatted).trim() || null;
}

/**
 * RFC 5322 References, oldest first, as an Array — not the raw space-joined
 * header. The nodetype declares an Array because a consumer walking an ancestry
 * should not have to re-parse a header it was handed.
 */
function splitReferences(raw) {
  if (!raw) return null;
  var parts = String(raw).split(/\s+/).filter(function (s) {
    return s.length > 0;
  });
  return parts.length ? parts : null;
}

/**
 * IMAP system flags, backslashes stripped and lowercased ("\\Seen" -> "seen").
 *
 * Normalized here rather than passed through so the column means the same thing
 * whichever server produced it; the raw form stays in `provider_metadata`.
 */
function normalizeFlags(flags) {
  var input = flags || [];
  var out = [];
  for (var i = 0; i < input.length; i++) {
    var f = String(input[i]).replace(/\\/g, "").toLowerCase();
    if (f) out.push(f);
  }
  return out;
}
