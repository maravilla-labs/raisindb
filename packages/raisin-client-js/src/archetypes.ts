/**
 * Archetype management operations
 */

import { RequestContext, RequestType } from './protocol';

export class Archetypes {
  private sendRequest: (payload: unknown, requestType: string) => Promise<unknown>;

  constructor(
    _context: RequestContext,
    sendRequest: (payload: unknown, requestType: string) => Promise<unknown>
  ) {
    this.sendRequest = sendRequest;
  }

  /**
   * Create a new Archetype
   */
  async create(name: string, archetype: Record<string, unknown>): Promise<unknown> {
    return this.sendRequest(
      {
        name,
        archetype
      },
      RequestType.ArchetypeCreate
    );
  }

  /**
   * Get an Archetype by name
   */
  async get(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.ArchetypeGet
    );
  }

  /**
   * List all Archetypes
   */
  async list(publishedOnly = false): Promise<unknown[]> {
    const result = await this.sendRequest(
      {
        published_only: publishedOnly
      },
      RequestType.ArchetypeList
    );
    return Array.isArray(result) ? result : [];
  }

  /**
   * Update an Archetype
   */
  async update(name: string, archetype: Record<string, unknown>): Promise<unknown> {
    return this.sendRequest(
      {
        name,
        archetype
      },
      RequestType.ArchetypeUpdate
    );
  }

  /**
   * Delete an Archetype
   */
  async delete(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.ArchetypeDelete
    );
  }

  /**
   * Publish an Archetype
   */
  async publish(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.ArchetypePublish
    );
  }

  /**
   * Unpublish an Archetype
   */
  async unpublish(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.ArchetypeUnpublish
    );
  }
}
