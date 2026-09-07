import { describe, expect, it, vi } from 'vitest';
import { NodeOperations } from './nodes';

/**
 * The WebSocket server understands a fixed vocabulary per request type
 * (crates/raisin-transport-ws/src/handlers/nodes/query.rs, tree.rs and
 * protocol/payloads_node.rs). These tests pin the SDK to that vocabulary:
 *
 * - `node_query` filters on `query.type` / `query.parent`; anything else lists
 *   every node, which is how `getByPath` used to return the wrong node.
 * - `node_query_by_path` takes `{ path }` and answers a single node or null.
 * - `node_query_by_property` takes `{ query: { <prop>: <value> } }`.
 * - `node_get_tree` / `node_get_tree_flat` take `parent_path`, not `root_path`.
 */
describe('NodeOperations wire payloads', () => {
  function ops() {
    const calls: Array<{ payload: unknown; type?: string }> = [];
    const sendRequest = vi.fn(async (payload: unknown, type?: string) => {
      calls.push({ payload, type });
      if (type === 'node_query_by_path') {
        return (payload as { path: string }).path === '/missing' ? null : { id: 'n1', path: (payload as { path: string }).path };
      }
      return [];
    });
    return { nodes: new NodeOperations(sendRequest), calls };
  }

  it('getByPath uses node_query_by_path and returns null for a miss', async () => {
    const { nodes, calls } = ops();
    const hit = await nodes.getByPath('/articles/hello');
    expect(hit).toEqual({ id: 'n1', path: '/articles/hello' });
    expect(calls[0]).toEqual({ payload: { path: '/articles/hello' }, type: 'node_query_by_path' });

    expect(await nodes.getByPath('/missing')).toBeNull();
  });

  it('queryByType sends query.type through node_query', async () => {
    const { nodes, calls } = ops();
    await nodes.queryByType('raisin:Page', 10);
    expect(calls[0].payload).toEqual({ query: { type: 'raisin:Page' }, limit: 10, offset: undefined });
    expect(calls[0].type).toBeUndefined(); // inferred as node_query by the workspace client
  });

  it('queryByProperty sends the bare key/value through node_query_by_property', async () => {
    const { nodes, calls } = ops();
    await nodes.queryByProperty('status', 'published', 5);
    expect(calls[0]).toEqual({
      payload: { query: { status: 'published' }, limit: 5 },
      type: 'node_query_by_property',
    });
  });

  it('getTree and getTreeFlat send parent_path', async () => {
    const { nodes, calls } = ops();
    await nodes.getTree('/menu', 2);
    await nodes.getTreeFlat('/menu');
    expect(calls[0]).toEqual({ payload: { parent_path: '/menu', max_depth: 2 }, type: 'node_get_tree' });
    expect(calls[1]).toEqual({ payload: { parent_path: '/menu' }, type: 'node_get_tree_flat' });
  });

  it('getChildrenByPath resolves the parent id by path, then queries by parent', async () => {
    const { nodes, calls } = ops();
    await nodes.getChildrenByPath('/articles', 3);
    expect(calls[0].type).toBe('node_query_by_path');
    expect(calls[1].payload).toEqual({ query: { parent: 'n1' }, limit: 3, offset: undefined });
  });
});
