/**
 * circle-sync — bring a circle's group conversation in line with its roster.
 *
 * Group chat delivers to whoever is in the circle when a message is posted, so a new member would only
 * see the conversation with the next post, and someone who left would keep a copy that looks live. Call
 * this after changing MEMBER_OF / MODERATES edges (an application's roster sync, an admin):
 *
 *   - every member and moderator gets their copy of `grp-<circle id>` (created empty when missing,
 *     `membership: member`);
 *   - every copy whose owner is no longer in the circle is marked `membership: left` (kept as history,
 *     nothing more is delivered to it).
 *
 * Input: { circle: 'circle:<node id>' | '/circles/<name>' }. Runs as the system; call it from trusted
 * code (a function, a flow, a server), never expose it to members.
 * Output: { conversation_id, members, created, left }.
 */

import { ensureChatsFolder, createIdempotent, sqlRows } from '../handlers/shared/parties.js';
import { resolveCircle, rosterFor, conversationIdOf } from '../handlers/shared/circles.js';

const AC = 'raisin:access_control';

export async function circle_sync(input) {
  const ref = input?.circle ?? input?.flow_input?.circle;
  const circle = await resolveCircle(typeof ref === 'string' ? ref : '');
  if (!circle) throw new Error('unknown_circle');

  const conversationId = conversationIdOf(circle);
  const roster = await rosterFor(circle, null);
  const now = new Date().toISOString();
  const homes = new Set();
  let created = 0;

  for (const person of roster.everyone) {
    homes.add(person.path);
    const chats = await ensureChatsFolder(person.workspace, person.path);
    const path = `${chats}/${conversationId}`;
    const existing = await raisin.nodes.get(person.workspace, path);
    const shape = {
      conversation_type: 'group',
      conversation_id: conversationId,
      subject: circle.name,
      circle_id: circle.id,
      circle_type: circle.type,
      participants: [`circle:${circle.id}`],
      member_count: roster.memberCount,
      stream_channel: `chat:${conversationId}`,
      membership: 'member',
    };
    if (!existing) {
      if (await createIdempotent(person.workspace, chats, {
        slug: conversationId, name: conversationId, node_type: 'raisin:Conversation',
        properties: { ...shape, unread_count: 0, updated_at: now },
      })) created += 1;
    } else {
      await raisin.nodes.update(person.workspace, path, { properties: { ...(existing.properties ?? {}), ...shape } });
    }
  }

  // Copies of people who are no longer in the circle.
  const copies = await sqlRows(
    `SELECT path, properties FROM '${AC}'
     WHERE node_type = 'raisin:Conversation' AND properties->>'conversation_id' = $1`,
    [conversationId],
  );
  let left = 0;
  for (const copy of copies) {
    const owner = String(copy.path).split('/inbox/')[0];
    if (homes.has(owner) || copy.properties?.membership === 'left') continue;
    await raisin.nodes.update(AC, copy.path, {
      properties: { ...(copy.properties ?? {}), membership: 'left', left_at: now },
    });
    left += 1;
  }

  return { conversation_id: conversationId, members: roster.everyone.length, created, left };
}
