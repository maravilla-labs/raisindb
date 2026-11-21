import { describe, expect, it } from 'vitest';
import { ConversationManager } from './conversations';
import type { AuthManager } from './auth';

const fakeAuth = { getAccessToken: () => null } as unknown as AuthManager;

function manager(rows: Record<string, unknown>[]) {
  const queries: string[] = [];
  const executeSql = async (sql: string) => {
    queries.push(sql);
    // The message query returns the rows; the child-node query none.
    return { rows: sql.includes("node_type = 'raisin:Message'") ? rows : [], columns: [], row_count: 0, execution_time_ms: 0 } as never;
  };
  const mgr = new ConversationManager('http://x', 'repo', fakeAuth, {}, executeSql as never);
  return { mgr, queries };
}

describe('getMessages and tool steps', () => {
  it('returns ai_tool_call messages with messageType, toolCallId and data, in order', async () => {
    const { mgr, queries } = manager([
      { id: 'm1', path: '/c/m1', name: 'm1', created_at: '2026-01-01T00:00:00Z', properties: { role: 'user', message_type: 'chat', body: { content: 'hi' } } },
      {
        id: 'm2', path: '/c/m2', name: 'm2', created_at: '2026-01-01T00:00:01Z',
        properties: {
          role: 'assistant', message_type: 'ai_tool_call', body: { content: 'page.list_agenda' },
          data: { tool_call_id: 'c1', tool_name: 'page.list_agenda', arguments: { day: 'fri' }, status: 'completed', result_summary: '3 items', run_id: 'r1' },
        },
      },
    ]);
    const messages = await mgr.getMessages('/c');
    expect(messages.map((m) => m.messageType)).toEqual(['chat', 'ai_tool_call']);
    const step = messages[1];
    expect(step.toolCallId).toBe('c1');
    expect(step.data).toMatchObject({ tool_name: 'page.list_agenda', status: 'completed', result_summary: '3 items' });
    expect(step.role).toBe('assistant');
    // Still hidden: raw tool results and thoughts.
    const sql = queries.find((q) => q.includes("node_type = 'raisin:Message'"))!;
    expect(sql).not.toContain("'ai_tool_call'");
    expect(sql).toContain("'ai_tool_result'");
    expect(sql).toContain("'ai_thought'");
  });

  it('orders and timestamps by properties.created_at when present, else the system column', async () => {
    const { mgr } = manager([
      // Stored last (a backfill), but it happened between the other two.
      { id: 'late', path: '/c/late', name: 'late', created_at: '2026-02-01T00:00:00Z', properties: { role: 'assistant', message_type: 'ai_tool_call', created_at: '2026-01-01T00:00:01Z', body: { content: 't' }, data: { tool_call_id: 'c1' } } },
      { id: 'a', path: '/c/a', name: 'a', created_at: '2026-01-01T00:00:00Z', properties: { role: 'user', message_type: 'chat', body: { content: 'hi' } } },
      { id: 'b', path: '/c/b', name: 'b', created_at: '2026-01-01T00:00:02Z', properties: { role: 'assistant', message_type: 'chat', body: { content: 'done' } } },
      { id: 'bad', path: '/c/bad', name: 'bad', created_at: '2026-01-01T00:00:03Z', properties: { role: 'assistant', message_type: 'chat', created_at: 'not a date', body: { content: 'x' } } },
    ]);
    const messages = await mgr.getMessages('/c');
    expect(messages.map((m) => m.id)).toEqual(['a', 'late', 'b', 'bad']);
    expect(messages[1].timestamp).toBe('2026-01-01T00:00:01Z');
    expect(messages[3].timestamp).toBe('2026-01-01T00:00:03Z');
  });
});
