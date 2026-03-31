/**
 * NodeType management operations
 */

import { RequestContext, RequestType } from './protocol';

export class NodeTypes {
  private sendRequest: (payload: unknown, requestType: string) => Promise<unknown>;

  constructor(
    _context: RequestContext,
    sendRequest: (payload: unknown, requestType: string) => Promise<unknown>
  ) {
    this.sendRequest = sendRequest;
  }

  /**
   * Create a new NodeType
   */
  async create(name: string, nodeType: Record<string, unknown>): Promise<unknown> {
    return this.sendRequest(
      {
        name,
        node_type: nodeType
      },
      RequestType.NodeTypeCreate
    );
  }

  /**
   * Get a NodeType by name
   */
  async get(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.NodeTypeGet
    );
  }

  /**
   * List all NodeTypes
   */
  async list(publishedOnly = false): Promise<unknown[]> {
    const result = await this.sendRequest(
      {
        published_only: publishedOnly
      },
      RequestType.NodeTypeList
    );
    return Array.isArray(result) ? result : [];
  }

  /**
   * Update a NodeType
   */
  async update(name: string, nodeType: Record<string, unknown>): Promise<unknown> {
    return this.sendRequest(
      {
        name,
        node_type: nodeType
      },
      RequestType.NodeTypeUpdate
    );
  }

  /**
   * Delete a NodeType
   */
  async delete(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.NodeTypeDelete
    );
  }

  /**
   * Publish a NodeType
   */
  async publish(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.NodeTypePublish
    );
  }

  /**
   * Unpublish a NodeType
   */
  async unpublish(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.NodeTypeUnpublish
    );
  }

  /**
   * Validate a node against its NodeType
   */
  async validate(node: Record<string, unknown>): Promise<unknown> {
    return this.sendRequest(
      {
        node
      },
      RequestType.NodeTypeValidate
    );
  }

  /**
   * Get resolved NodeType with full inheritance applied
   */
  async getResolved(name: string): Promise<unknown> {
    return this.sendRequest(
      {
        name
      },
      RequestType.NodeTypeGetResolved
    );
  }
}
