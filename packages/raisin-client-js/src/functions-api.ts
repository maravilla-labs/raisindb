/**
 * FunctionsApi - Transport-agnostic function invocation.
 *
 * Invoke server-side functions through WebSocket or HTTP.
 * Functions can be queued as background jobs (async) or executed inline (sync).
 */

import type { RequestContext } from './protocol';
import { RequestType } from './protocol';

type SendRequestFn = (
  payload: unknown,
  requestType: string,
  contextOverride?: RequestContext,
  requestOptions?: { timeoutMs?: number },
) => Promise<unknown>;

export interface FunctionInvokeResponse {
  execution_id: string;
  job_id: string;
  status?: string;
  completed?: boolean;
  timed_out?: boolean;
  waited?: boolean;
  result?: unknown;
  error?: string;
  duration_ms?: number;
  logs?: string[];
}

export interface FunctionInvokeSyncResponse {
  execution_id: string;
  result?: unknown;
  error?: string;
  duration_ms?: number;
  logs?: string[];
}

export interface FunctionInvokeOptions {
  waitForResult?: boolean;
  waitTimeoutMs?: number;
  requestTimeoutMs?: number;
}

/**
 * FunctionsApi for WebSocket transport.
 *
 * Sends function invocation requests over the existing WS connection.
 */
export class FunctionsApi {
  private context: RequestContext;
  private sendRequest: SendRequestFn;

  constructor(
    repository: string,
    context: RequestContext,
    sendRequest: SendRequestFn,
  ) {
    this.context = { ...context, repository };
    this.sendRequest = sendRequest;
  }

  /**
   * Invoke a server-side function by name (async).
   *
   * Queues a `FunctionExecution` job and returns the execution and job IDs
   * immediately. The function runs asynchronously on the server.
   *
   * @param functionName - Name of the function to invoke
   * @param input - Input data passed to the function
   * @returns Execution ID and job ID for tracking
   */
  async invoke(
    functionName: string,
    input?: Record<string, unknown>,
    options?: FunctionInvokeOptions,
  ): Promise<FunctionInvokeResponse> {
    const effectiveRequestTimeoutMs = options?.requestTimeoutMs;
    let effectiveWaitTimeoutMs = options?.waitTimeoutMs;
    if (options?.waitForResult) {
      const transportTimeoutBudget = effectiveRequestTimeoutMs ?? 30000;
      effectiveWaitTimeoutMs = Math.min(
        effectiveWaitTimeoutMs ?? transportTimeoutBudget,
        transportTimeoutBudget,
      );
    }

    return (await this.sendRequest(
      {
        function_name: functionName,
        input: input ?? {},
        wait_for_completion: options?.waitForResult ?? false,
        wait_timeout_ms: effectiveWaitTimeoutMs,
      },
      RequestType.FunctionInvoke,
      this.context,
      effectiveRequestTimeoutMs != null ? { timeoutMs: effectiveRequestTimeoutMs } : undefined,
    )) as FunctionInvokeResponse;
  }

  /**
   * Invoke a server-side function synchronously.
   *
   * Executes the function inline on the server and returns the result
   * directly. Bypasses the job queue for immediate execution.
   *
   * @param functionName - Name of the function to invoke
   * @param input - Input data passed to the function
   * @returns Execution result including output, logs, and duration
   */
  async invokeSync(
    functionName: string,
    input?: Record<string, unknown>,
  ): Promise<FunctionInvokeSyncResponse> {
    return (await this.sendRequest(
      { function_name: functionName, input: input ?? {} },
      RequestType.FunctionInvokeSync,
      this.context,
    )) as FunctionInvokeSyncResponse;
  }
}

/**
 * HTTP-backed FunctionsApi for SSR / RaisinHttpClient.
 *
 * Calls `POST /api/functions/{repo}/{name}/invoke` via the HTTP client.
 */
export class HttpFunctionsApi {
  constructor(
    private repository: string,
    private invokeFn: (
      repository: string,
      functionName: string,
      input?: Record<string, unknown>,
      options?: FunctionInvokeOptions,
    ) => Promise<FunctionInvokeResponse>,
    private invokeSyncFn: (
      repository: string,
      functionName: string,
      input?: Record<string, unknown>,
    ) => Promise<FunctionInvokeSyncResponse>,
  ) {}

  /**
   * Invoke a server-side function by name (async).
   *
   * Calls the HTTP invoke endpoint and returns the execution and job IDs.
   *
   * @param functionName - Name of the function to invoke
   * @param input - Input data passed to the function
   * @returns Execution ID and job ID for tracking
   */
  async invoke(
    functionName: string,
    input?: Record<string, unknown>,
    options?: FunctionInvokeOptions,
  ): Promise<FunctionInvokeResponse> {
    return this.invokeFn(this.repository, functionName, input, options);
  }

  /**
   * Invoke a server-side function synchronously.
   *
   * Calls the HTTP invoke endpoint with `sync: true` and returns the
   * result directly.
   *
   * @param functionName - Name of the function to invoke
   * @param input - Input data passed to the function
   * @returns Execution result including output, logs, and duration
   */
  async invokeSync(
    functionName: string,
    input?: Record<string, unknown>,
  ): Promise<FunctionInvokeSyncResponse> {
    return this.invokeSyncFn(this.repository, functionName, input);
  }
}
