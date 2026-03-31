/**
 * Tag management operations
 */

import { RequestContext, RequestType } from './protocol';

export class Tags {
  private sendRequest: (payload: unknown, requestType: string) => Promise<unknown>;

  constructor(
    _context: RequestContext,
    sendRequest: (payload: unknown, requestType: string) => Promise<unknown>
  ) {
    this.sendRequest = sendRequest;
  }

  /**
   * Create a new tag
   */
  async create(name: string, revision: string, message?: string): Promise<unknown> {
    return this.sendRequest(
      {
        name,
        revision,
        message
      },
      RequestType.TagCreate
    );
  }

  /**
   * Get a tag by name
   */
  async get(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.TagGet
    );
  }

  /**
   * List all tags
   */
  async list(): Promise<unknown[]> {
    const result = await this.sendRequest(
      {},
      RequestType.TagList
    );
    return Array.isArray(result) ? result : [];
  }

  /**
   * Delete a tag
   */
  async delete(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.TagDelete
    );
  }
}
