/**
 * ElementType management operations
 */

import { RequestContext, RequestType } from './protocol';

export class ElementTypes {
  private sendRequest: (payload: unknown, requestType: string) => Promise<unknown>;

  constructor(
    _context: RequestContext,
    sendRequest: (payload: unknown, requestType: string) => Promise<unknown>
  ) {
    this.sendRequest = sendRequest;
  }

  /**
   * Create a new ElementType
   */
  async create(name: string, elementType: Record<string, unknown>): Promise<unknown> {
    return this.sendRequest(
      {
        name,
        element_type: elementType
      },
      RequestType.ElementTypeCreate
    );
  }

  /**
   * Get an ElementType by name
   */
  async get(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.ElementTypeGet
    );
  }

  /**
   * List all ElementTypes
   */
  async list(publishedOnly = false): Promise<unknown[]> {
    const result = await this.sendRequest(
      {
        published_only: publishedOnly
      },
      RequestType.ElementTypeList
    );
    return Array.isArray(result) ? result : [];
  }

  /**
   * Update an ElementType
   */
  async update(name: string, elementType: Record<string, unknown>): Promise<unknown> {
    return this.sendRequest(
      {
        name,
        element_type: elementType
      },
      RequestType.ElementTypeUpdate
    );
  }

  /**
   * Delete an ElementType
   */
  async delete(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.ElementTypeDelete
    );
  }

  /**
   * Publish an ElementType
   */
  async publish(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.ElementTypePublish
    );
  }

  /**
   * Unpublish an ElementType
   */
  async unpublish(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.ElementTypeUnpublish
    );
  }
}
