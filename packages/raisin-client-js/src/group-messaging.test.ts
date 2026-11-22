import { describe, expect, it, vi } from 'vitest';
import { ConversationManager } from './conversations';
import type { AuthManager } from './auth';
import type { SqlResult } from './protocol';

const fakeAuth = { getAccessToken: () => 'test-token' } as unknown as AuthManager;

/** executeSql answering the current user and one conversation, recording every insert. */
function world(conversation: Record<string, unknown>) {
  const inserts: Array<{ path: string; properties: Record<string, unknown> }> = [];
  const executeSql = vi.fn(async (sql: string, params?: unknown[]): Promise<SqlResult> => {
    if (sql.includes('RAISIN_CURRENT_USER()')) {
      return { columns: ['home', 'user_id'], rows: [{ home: '/users/internal/u_ana', user_id: 'u-ana' }], row_count: 1 };
    }
    if (sql.includes('SELECT properties FROM')) {
      return { columns: ['properties'], rows: [{ properties: conversation }], row_count: 1 };
    }
    if (sql.includes('INSERT INTO')) {
      inserts.push({ path: params?.[0] as string, properties: JSON.parse(params?.[1] as string) });
      return { columns: ['id'], rows: [{ id: 'm1' }], row_count: 1 };
    }
    return { columns: [], rows: [], row_count: 0 };
  });
  const manager = new ConversationManager('http://localhost:8081', 'demo', fakeAuth, {}, executeSql);
  return { manager, inserts };
}

const file = {
  ref: { 'raisin:ref': '/messages/grp-c1/photo.jpg', 'raisin:workspace': 'assets' },
  name: 'photo.jpg',
  mime_type: 'image/jpeg',
};

describe('group conversations', () => {
  it('a message in a group conversation is addressed to the circle, files in the body', async () => {
    const { manager, inserts } = world({
      conversation_type: 'group',
      conversation_id: 'grp-c1',
      participants: ['circle:c1'],
    });
    await manager.createUserMessage('/users/internal/u_ana/inbox/chats/grp-c1', 'Who brings snacks?', { files: [file] });
    expect(inserts).toHaveLength(1);
    const { path, properties } = inserts[0];
    expect(path.startsWith('/users/internal/u_ana/outbox/')).toBe(true);
    expect(properties.recipient_id).toBe('circle:c1');
    expect(properties.conversation_id).toBe('grp-c1');
    expect(properties.body).toMatchObject({ message_text: 'Who brings snacks?', thread_id: 'grp-c1', attachments: [file] });
  });

  it('files never go into the body of a message to an agent', async () => {
    const { manager, inserts } = world({ conversation_id: 'c2', participants: ['u-ana', 'agent:support'] });
    await manager.createUserMessage('/users/internal/u_ana/inbox/chats/c2', 'hi', { files: [file] });
    expect((inserts[0].properties.body as Record<string, unknown>).attachments).toBeUndefined();
  });
});

describe('assignTask', () => {
  it('writes a task_assignment into the caller\'s own outbox', async () => {
    const { manager, inserts } = world({});
    const out = await manager.assignTask({
      assignee: 'circle:c1',
      task_type: 'bring_item',
      title: 'Bring toilet paper on Saturday',
      completion: 'any',
      clientId: 'abc',
    });
    expect(out.messagePath).toBe('/users/internal/u_ana/outbox/task-abc');
    const { properties } = inserts[0];
    expect(properties.message_type).toBe('task_assignment');
    expect(properties.status).toBe('pending');
    expect(properties.body).toEqual({
      assignee: 'circle:c1',
      task_type: 'bring_item',
      title: 'Bring toilet paper on Saturday',
      completion: 'any',
    });
    expect(properties.sender_path, 'the engine takes the sender from the outbox').toBeUndefined();
    expect(properties.sender_id, 'required by raisin:Message, checked against the outbox').toBe('u-ana');
    expect(properties.recipient_id).toBe('circle:c1');
  });

  it('refuses a task without an assignee or title', async () => {
    const { manager } = world({});
    await expect(manager.assignTask({ assignee: '', title: 'x' })).rejects.toThrow(/assignee/);
    await expect(manager.assignTask({ assignee: 'u_x', title: ' ' })).rejects.toThrow(/title/);
  });
});
