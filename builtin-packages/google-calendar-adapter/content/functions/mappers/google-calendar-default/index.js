/**
 * Default Google Calendar mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 *   input  = { external_item: ExternalItem, mount: { mount_id, mount_path, sync_config } }
 *   return = { node_type, name?, properties } | null
 *
 * Mapping:
 *   - cancelled events  -> null (skipped)
 *   - every other event -> raisin:Event
 *
 * The adapter carries the raw event fields through ExternalItem.metadata; this
 * mapper hoists them into the typed raisin:Event properties (start, end,
 * all_day, location, attendees, organizer, recurrence, status, url,
 * calendar_id) with title = the event summary.
 *
 * The engine stamps the reserved __virtual/__mount_id/__external_id/__etag/
 * __synced_at properties on top of whatever is returned, so they are not set here.
 */

function handler(input) {
  var item = input.external_item;
  if (!item || !item.external_id) return null;

  var meta = item.metadata || {};

  // Cancelled events are deletions in Calendar's model — skip materializing them.
  if (meta.status === "cancelled") return null;

  // recurrence arrives as an array of RRULE/RDATE strings; the typed property is
  // a single String, so join non-empty rules and null out when there are none.
  var recurrence = null;
  if (Array.isArray(meta.recurrence) && meta.recurrence.length) {
    recurrence = meta.recurrence.join("\n");
  } else if (typeof meta.recurrence === "string" && meta.recurrence.length) {
    recurrence = meta.recurrence;
  }

  var title = meta.summary || item.name || "(untitled event)";

  // raisin:Event is a STRICT node type: only its declared properties (plus the
  // engine-owned __ reserved ones the materializer stamps) may be written, or the
  // synced write is rejected. Every value below maps onto a declared column — no
  // free-form provider passthrough blob is emitted.
  var properties = {
    title: title,
    start: meta.start || null,
    end: meta.end || null,
    all_day: meta.all_day === true,
    location: meta.location || null,
    attendees: Array.isArray(meta.attendees) ? meta.attendees : null,
    organizer: meta.organizer || null,
    recurrence: recurrence,
    status: meta.status || null,
    url: meta.htmlLink || item.web_url || null,
    calendar_id: meta.calendar_id || null,
  };

  return {
    node_type: "raisin:Event",
    name: item.name,
    properties: properties,
  };
}
