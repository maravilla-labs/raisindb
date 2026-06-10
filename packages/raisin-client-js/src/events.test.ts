import { describe, expect, it, vi } from 'vitest';
import { EventHandler } from './events';
import { RequestType, type EventMessage, type SubscribePayload } from './protocol';

describe('EventHandler.restoreSubscriptions', () => {
  it('throws on failure, keeps failed entries, and retries without duplicating restored ones', async () => {
    let counter = 0;
    let failPathA = false;
    const sendRequest = vi.fn(async (payload: unknown, type: RequestType) => {
      if (type === RequestType.Subscribe) {
        const filters = (payload as SubscribePayload).filters;
        if (failPathA && filters.path === '/a') {
          throw new Error('subscribe failed');
        }
        counter++;
        return { subscription_id: `sub-${counter}` };
      }
      return {};
    });

    const handler = new EventHandler(sendRequest as never);
    try {
      const callbackA = vi.fn();
      const callbackB = vi.fn();
      await handler.subscribe({ path: '/a' }, callbackA);
      await handler.subscribe({ path: '/b' }, callbackB);
      expect(sendRequest).toHaveBeenCalledTimes(2);

      // Reconnect: /a fails to restore, /b succeeds
      failPathA = true;
      await expect(handler.restoreSubscriptions()).rejects.toThrow(/Failed to restore 1/);

      // /b got a new server id and still routes events
      const bSubscriptionId = `sub-${counter}`;
      handler.handleEvent({
        event_id: 'e1',
        subscription_id: bSubscriptionId,
        event_type: 'node:created',
        payload: { kind: 'Created' },
        timestamp: 't',
      } as EventMessage);
      expect(callbackB).toHaveBeenCalledTimes(1);

      // Retry only re-subscribes the failed entry (no duplicate for /b)
      failPathA = false;
      const callsBefore = sendRequest.mock.calls.length;
      await handler.restoreSubscriptions({ retry: true });
      const retryCalls = sendRequest.mock.calls.slice(callsBefore);
      expect(retryCalls).toHaveLength(1);
      expect((retryCalls[0][0] as SubscribePayload).filters.path).toBe('/a');

      // /a now routes events under its new server id
      const aSubscriptionId = `sub-${counter}`;
      handler.handleEvent({
        event_id: 'e2',
        subscription_id: aSubscriptionId,
        event_type: 'node:created',
        payload: { kind: 'Created' },
        timestamp: 't',
      } as EventMessage);
      expect(callbackA).toHaveBeenCalledTimes(1);
    } finally {
      handler.destroy();
    }
  });
});
