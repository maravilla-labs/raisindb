import { describe, expect, it, vi } from 'vitest';
import { FunctionsApi } from './functions-api';

describe('FunctionsApi.invoke', () => {
  it('clamps wait timeout to default transport timeout when no request override is provided', async () => {
    const sendRequest = vi.fn(async (_payload, _type, _context, _requestOptions) => ({
      execution_id: 'exec-1',
      job_id: 'job-1',
      status: 'scheduled',
      completed: false,
    }));

    const api = new FunctionsApi(
      'demo',
      { tenant_id: 'default', repository: 'demo' },
      sendRequest,
    );

    await api.invoke(
      'plan-approval-handler',
      { action: 'approve' },
      { waitForResult: true, waitTimeoutMs: 45000 },
    );

    expect(sendRequest).toHaveBeenCalledTimes(1);
    expect(sendRequest.mock.calls[0][0]).toMatchObject({
      wait_for_completion: true,
      wait_timeout_ms: 30000,
    });
    expect(sendRequest.mock.calls[0][3]).toBeUndefined();
  });

  it('honors request timeout override and clamps wait timeout to it', async () => {
    const sendRequest = vi.fn(async (_payload, _type, _context, _requestOptions) => ({
      execution_id: 'exec-2',
      job_id: 'job-2',
      status: 'scheduled',
      completed: false,
    }));

    const api = new FunctionsApi(
      'demo',
      { tenant_id: 'default', repository: 'demo' },
      sendRequest,
    );

    await api.invoke(
      'plan-approval-handler',
      { action: 'approve' },
      { waitForResult: true, waitTimeoutMs: 120000, requestTimeoutMs: 60000 },
    );

    expect(sendRequest).toHaveBeenCalledTimes(1);
    expect(sendRequest.mock.calls[0][0]).toMatchObject({
      wait_for_completion: true,
      wait_timeout_ms: 60000,
    });
    expect(sendRequest.mock.calls[0][3]).toEqual({ timeoutMs: 60000 });
  });
});
