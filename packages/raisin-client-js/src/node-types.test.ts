import { describe, expect, it } from 'vitest';
import { NodeTypes } from './node-types';
import type { RequestContext } from './protocol';

function harness(context: RequestContext) {
  const calls: Array<{ payload: unknown; type: string; ctx?: RequestContext }> = [];
  const send = async (payload: unknown, type: string, ctx?: RequestContext) => {
    calls.push({ payload, type, ctx });
    return { valid: true, errors: [] };
  };
  return { api: new NodeTypes(context, send), calls };
}

const BASE: RequestContext = { tenant_id: 'default', repository: 'repo', branch: 'main' };

describe('NodeTypes.validate', () => {
  it('sends the workspace in the request context', async () => {
    const { api, calls } = harness(BASE);
    await api.validate({ name: 'x', node_type: 'blog:Article', properties: {} }, 'blog');
    expect(calls).toHaveLength(1);
    expect(calls[0].type).toBe('node_type_validate');
    expect(calls[0].ctx).toEqual({ ...BASE, workspace: 'blog' });
    expect(calls[0].payload).toEqual({
      node: { name: 'x', node_type: 'blog:Article', properties: {} },
    });
  });

  it('falls back to the workspace already in the context', async () => {
    const { api, calls } = harness({ ...BASE, workspace: 'pages' });
    await api.validate({ name: 'x', node_type: 'blog:Article' });
    expect(calls[0].ctx?.workspace).toBe('pages');
  });

  it('fails locally when no workspace is known', async () => {
    const { api, calls } = harness(BASE);
    await expect(api.validate({ name: 'x', node_type: 'blog:Article' })).rejects.toThrow(
      /requires a workspace/
    );
    expect(calls).toHaveLength(0);
  });
});
