import { describe, expect, it, vi } from 'vitest';
import {
  McpClient,
  type McpFrameStream,
  type McpJsonRpcFrame,
  type McpTransport,
} from './mcp';

/** Build a frame stream from a fixed list of frames (for subscribe tests). */
function frameStream(frames: McpJsonRpcFrame[]): McpFrameStream {
  return {
    close: vi.fn(),
    async *[Symbol.asyncIterator]() {
      for (const frame of frames) {
        yield frame;
      }
    },
  };
}

describe('McpClient.listTools', () => {
  it('calls tools/list and returns the tools array', async () => {
    const rpc = vi.fn(async () => ({
      tools: [{ name: 'order_card', description: 'Render an order' }],
    }));
    const transport: McpTransport = { rpc, subscribe: vi.fn() };
    const client = new McpClient(transport);

    const { tools } = await client.listTools();

    expect(rpc).toHaveBeenCalledWith('tools/list');
    expect(tools).toHaveLength(1);
    expect(tools[0].name).toBe('order_card');
  });

  it('defaults to an empty tools array when the result omits it', async () => {
    const transport: McpTransport = { rpc: vi.fn(async () => ({})), subscribe: vi.fn() };
    const client = new McpClient(transport);

    const { tools } = await client.listTools();

    expect(tools).toEqual([]);
  });
});

describe('McpClient.callTool', () => {
  it('calls tools/call with name + arguments and returns the result', async () => {
    const rpc = vi.fn(async () => ({
      content: [{ type: 'text', text: 'ok' }],
      isError: false,
      structuredContent: { orderId: '42' },
    }));
    const transport: McpTransport = { rpc, subscribe: vi.fn() };
    const client = new McpClient(transport);

    const result = await client.callTool('order_card', { orderId: '42' });

    expect(rpc).toHaveBeenCalledWith('tools/call', {
      name: 'order_card',
      arguments: { orderId: '42' },
    });
    expect(result.structuredContent).toEqual({ orderId: '42' });
    expect(result.isError).toBe(false);
  });

  it('defaults arguments to an empty object', async () => {
    const rpc = vi.fn(async () => ({ content: [] }));
    const transport: McpTransport = { rpc, subscribe: vi.fn() };
    const client = new McpClient(transport);

    await client.callTool('ping');

    expect(rpc).toHaveBeenCalledWith('tools/call', { name: 'ping', arguments: {} });
  });
});

describe('McpClient.readResource', () => {
  it('calls resources/read with the uri and returns contents', async () => {
    const rpc = vi.fn(async () => ({
      contents: [{ uri: 'raisin://content/home', text: '{"title":"Home"}' }],
    }));
    const transport: McpTransport = { rpc, subscribe: vi.fn() };
    const client = new McpClient(transport);

    const { contents } = await client.readResource('raisin://content/home');

    expect(rpc).toHaveBeenCalledWith('resources/read', {
      uri: 'raisin://content/home',
    });
    expect(contents[0].uri).toBe('raisin://content/home');
  });
});

describe('McpClient.subscribeResource', () => {
  it('subscribes and yields only resource-updated notifications', async () => {
    const uri = 'raisin://content/home';
    const subscribe = vi.fn(
      (): McpFrameStream =>
        frameStream([
          // Initial ack — should NOT be yielded.
          { jsonrpc: '2.0', id: 1, result: { subscribed: true, uri } },
          {
            jsonrpc: '2.0',
            method: 'notifications/resources/updated',
            params: { uri },
          },
          // A frame with no uri — should be skipped.
          {
            jsonrpc: '2.0',
            method: 'notifications/resources/updated',
            params: {},
          },
        ]),
    );
    const transport: McpTransport = { rpc: vi.fn(), subscribe };
    const client = new McpClient(transport);

    const updates = [];
    for await (const update of client.subscribeResource(uri)) {
      updates.push(update);
    }

    expect(subscribe).toHaveBeenCalledWith('resources/subscribe', { uri });
    expect(updates).toEqual([{ uri }]);
  });

  it('exposes close() that tears down the underlying stream', () => {
    const stream = frameStream([]);
    const transport: McpTransport = {
      rpc: vi.fn(),
      subscribe: vi.fn(() => stream),
    };
    const client = new McpClient(transport);

    const sub = client.subscribeResource('raisin://content/home');
    sub.close();

    expect(stream.close).toHaveBeenCalledTimes(1);
  });
});
