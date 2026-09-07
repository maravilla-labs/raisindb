/**
 * NodeType management operations
 */

import { RequestContext, RequestType } from './protocol';

export class NodeTypes {
  private context: RequestContext;
  private sendRequest: (
    payload: unknown,
    requestType: string,
    contextOverride?: RequestContext
  ) => Promise<unknown>;

  constructor(
    context: RequestContext,
    sendRequest: (
      payload: unknown,
      requestType: string,
      contextOverride?: RequestContext
    ) => Promise<unknown>
  ) {
    this.context = context;
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
   * Validate a node against its NodeType.
   *
   * The server validates in the context of a workspace (workspace-level
   * allow-lists and unique checks), so one is required. Pass it here, or
   * create the database with a workspace already in its context.
   */
  async validate(node: Record<string, unknown>, workspace?: string): Promise<unknown> {
    const ws = workspace ?? this.context.workspace;
    if (!ws) {
      throw new Error(
        'nodeTypes().validate() requires a workspace: pass it as the second argument'
      );
    }
    return this.sendRequest(
      {
        node
      },
      RequestType.NodeTypeValidate,
      { ...this.context, workspace: ws }
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
