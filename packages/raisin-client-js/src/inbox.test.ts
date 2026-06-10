import { describe, expect, it, vi } from 'vitest';
import { InboxApi } from './inbox';
import type { AuthManager } from './auth';

const fakeAuth = {
  getAccessToken: () => 'test-token',
} as unknown as AuthManager;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('InboxApi', () => {
  it('lists tasks with status filter and auth header', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ assignee: '/users/alice', count: 1, tasks: [{ id: 't1' }] }),
    );
    const inbox = new InboxApi('http://localhost:8081', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    const result = await inbox.listTasks({ status: 'pending' });

    expect(result.count).toBe(1);
    expect(result.tasks[0].id).toBe('t1');
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('http://localhost:8081/api/inbox/demo?status=pending');
    expect(init.method).toBe('GET');
    expect((init.headers as Record<string, string>)['Authorization']).toBe(
      'Bearer test-token',
    );
  });

  it('lists tasks without filters', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ assignee: '/users/alice', count: 0, tasks: [] }),
    );
    const inbox = new InboxApi('http://localhost:8081/', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    await inbox.listTasks();
    const [url] = fetchMock.mock.calls[0] as unknown as [string];
    expect(url).toBe('http://localhost:8081/api/inbox/demo');
  });

  it('completes a task with the response payload', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({
        task_id: 't1',
        task_path: '/users/alice/inbox/t1',
        status: 'completed',
        flow: { instance_id: 'inst-1', job_id: 'job-1' },
      }),
    );
    const inbox = new InboxApi('http://localhost:8081', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    const result = await inbox.completeTask('t1', {
      action: 'approve',
      comment: 'ok',
    });

    expect(result.status).toBe('completed');
    expect(result.flow?.instance_id).toBe('inst-1');
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('http://localhost:8081/api/inbox/demo/tasks/t1/complete');
    expect(init.method).toBe('POST');
    expect(JSON.parse(init.body as string)).toEqual({
      response: { action: 'approve', comment: 'ok' },
    });
  });

  it('gets a task by id with URL encoding', async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ id: 'a/b' }));
    const inbox = new InboxApi('http://localhost:8081', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    await inbox.getTask('a/b');
    const [url] = fetchMock.mock.calls[0] as unknown as [string];
    expect(url).toBe('http://localhost:8081/api/inbox/demo/tasks/a%2Fb');
  });

  it('throws a classified error on HTTP failure', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ message: 'Task is assigned to another principal' }, 403),
    );
    const inbox = new InboxApi('http://localhost:8081', 'demo', fakeAuth, {
      fetch: fetchMock as unknown as typeof fetch,
    });

    await expect(inbox.completeTask('t1', {})).rejects.toThrow(
      /assigned to another principal/,
    );
  });
});
