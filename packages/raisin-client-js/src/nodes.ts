/**
 * Node operations interface
 */

import {
  Node,
  PropertyValue,
  NodeCreatePayload,
  NodeUpdatePayload,
  NodeDeletePayload,
  NodeGetPayload,
  NodeQueryPayload,
  RelationAddPayload,
  RelationRemovePayload,
  RelationsGetPayload,
  NodeRelationships,
} from './protocol';

/**
 * Options for creating a node
 */
export interface NodeCreateOptions {
  /** Node type */
  type: string;
  /** Node path */
  path: string;
  /** Node properties */
  properties?: Record<string, PropertyValue>;
  /** Node content (optional) */
  content?: unknown;
}

/**
 * Options for updating a node
 */
export interface NodeUpdateOptions {
  /** Properties to update */
  properties?: Record<string, PropertyValue>;
  /** Content to update (optional) */
  content?: unknown;
}

/**
 * Options for querying nodes
 */
export interface NodeQueryOptions {
  /** Query object */
  query: unknown;
  /** Maximum number of results */
  limit?: number;
  /** Offset for pagination */
  offset?: number;
}

/**
 * Node operations interface
 */
export class NodeOperations {
  private sendRequest: (payload: unknown, requestType?: string) => Promise<unknown>;
  private workspace?: string;

  constructor(
    sendRequest: (payload: unknown, requestType?: string) => Promise<unknown>,
    workspace?: string
  ) {
    this.sendRequest = sendRequest;
    this.workspace = workspace;
  }

  /**
   * Create a new node
   *
   * @param options - Node creation options
   * @returns Created node
   */
  async create(options: NodeCreateOptions): Promise<Node> {
    const payload: NodeCreatePayload = {
      node_type: options.type,
      path: options.path,
      properties: options.properties ?? {},
      content: options.content,
    };

    const result = await this.sendRequest(payload);
    return result as Node;
  }

  /**
   * Update an existing node
   *
   * @param id - Node ID
   * @param options - Node update options
   * @returns Updated node
   */
  async update(id: string, options: NodeUpdateOptions): Promise<Node> {
    const payload: NodeUpdatePayload = {
      node_id: id,
      properties: options.properties ?? {},
      content: options.content,
    };

    const result = await this.sendRequest(payload);
    return result as Node;
  }

  /**
   * Delete a node
   *
   * @param id - Node ID
   * @returns true if deleted successfully
   */
  async delete(id: string): Promise<boolean> {
    const payload: NodeDeletePayload = {
      node_id: id,
    };

    await this.sendRequest(payload);
    return true;
  }

  /**
   * Get a node by ID
   *
   * @param id - Node ID
   * @returns Node or null if not found
   */
  async get(id: string): Promise<Node | null> {
    const payload: NodeGetPayload = {
      node_id: id,
    };

    try {
      const result = await this.sendRequest(payload);
      return result as Node;
    } catch (error) {
      // If node not found, return null
      if (error instanceof Error && error.message.includes('not found')) {
        return null;
      }
      throw error;
    }
  }

  /**
   * Query nodes
   *
   * @param options - Query options
   * @returns Array of nodes
   */
  async query(options: NodeQueryOptions): Promise<Node[]> {
    const payload: NodeQueryPayload = {
      query: options.query,
      limit: options.limit,
      offset: options.offset,
    };

    const result = await this.sendRequest(payload);
    return result as Node[];
  }

  /**
   * Get a node by path
   *
   * @param path - Node path
   * @returns Node or null if not found
   */
  async getByPath(path: string): Promise<Node | null> {
    try {
      const result = await this.query({
        query: { path },
        limit: 1,
      });
      return result.length > 0 ? result[0] : null;
    } catch (error) {
      return null;
    }
  }

  /**
   * Query nodes by property
   *
   * @param propertyName - Property name
   * @param propertyValue - Property value
   * @param limit - Maximum number of results
   * @returns Array of nodes
   */
  async queryByProperty(
    propertyName: string,
    propertyValue: PropertyValue,
    limit?: number
  ): Promise<Node[]> {
    return this.query({
      query: {
        properties: {
          [propertyName]: propertyValue,
        },
      },
      limit,
    });
  }

  /**
   * Query nodes by type
   *
   * @param nodeType - Node type
   * @param limit - Maximum number of results
   * @returns Array of nodes
   */
  async queryByType(nodeType: string, limit?: number): Promise<Node[]> {
    return this.query({
      query: { node_type: nodeType },
      limit,
    });
  }

  /**
   * Get children of a node
   *
   * @param parentId - Parent node ID
   * @param limit - Maximum number of results
   * @returns Array of child nodes
   */
  async getChildren(parentId: string, limit?: number): Promise<Node[]> {
    return this.query({
      query: { parent: parentId },
      limit,
    });
  }

  /**
   * Get children by path
   *
   * @param parentPath - Parent node path
   * @param limit - Maximum number of results
   * @returns Array of child nodes
   */
  async getChildrenByPath(parentPath: string, limit?: number): Promise<Node[]> {
    const parent = await this.getByPath(parentPath);
    if (!parent) {
      return [];
    }
    return this.getChildren(parent.id, limit);
  }

  // ========================================================================
  // Tree Operations
  // ========================================================================

  /**
   * List children of a parent node
   *
   * @param parentPath - Path of the parent node
   * @returns Array of child nodes
   */
  async listChildren(parentPath: string): Promise<Node[]> {
    const payload = {
      parent_path: parentPath,
    };

    const result = await this.sendRequest(payload, 'node_list_children');
    return result as Node[];
  }

  /**
   * Get a node tree starting from a root node
   *
   * @param rootPath - Path of the root node
   * @param maxDepth - Maximum depth to traverse (optional)
   * @returns Root node with nested children
   */
  async getTree(rootPath: string, maxDepth?: number): Promise<Node> {
    const payload: { root_path: string; max_depth?: number } = {
      root_path: rootPath,
    };

    if (maxDepth !== undefined) {
      payload.max_depth = maxDepth;
    }

    const result = await this.sendRequest(payload, 'node_get_tree');
    return result as Node;
  }

  /**
   * Get a flattened node tree starting from a root node
   *
   * @param rootPath - Path of the root node
   * @param maxDepth - Maximum depth to traverse (optional)
   * @returns Array of nodes in tree order
   */
  async getTreeFlat(rootPath: string, maxDepth?: number): Promise<Node[]> {
    const payload: { root_path: string; max_depth?: number } = {
      root_path: rootPath,
    };

    if (maxDepth !== undefined) {
      payload.max_depth = maxDepth;
    }

    const result = await this.sendRequest(payload, 'node_get_tree_flat');
    return result as Node[];
  }

  // ========================================================================
  // Node Manipulation Operations
  // ========================================================================

  /**
   * Move a node to a new parent
   *
   * @param fromPath - Source node path
   * @param toParentPath - Destination parent path
   * @returns Moved node
   */
  async move(fromPath: string, toParentPath: string): Promise<Node> {
    const payload = {
      from_path: fromPath,
      to_parent_path: toParentPath,
    };

    const result = await this.sendRequest(payload, 'node_move');
    return result as Node;
  }

  /**
   * Rename a node
   *
   * @param nodePath - Node path
   * @param newName - New name for the node
   * @returns Renamed node
   */
  async rename(nodePath: string, newName: string): Promise<Node> {
    const payload = {
      node_path: nodePath,
      new_name: newName,
    };

    const result = await this.sendRequest(payload, 'node_rename');
    return result as Node;
  }

  /**
   * Copy a node to a new parent (shallow copy)
   *
   * @param fromPath - Source node path
   * @param toParentPath - Destination parent path
   * @param newName - New name for the copied node (optional)
   * @returns Copied node
   */
  async copy(fromPath: string, toParentPath: string, newName?: string): Promise<Node> {
    const payload: { from_path: string; to_parent_path: string; new_name?: string; deep: boolean } = {
      from_path: fromPath,
      to_parent_path: toParentPath,
      deep: false,
    };

    if (newName !== undefined) {
      payload.new_name = newName;
    }

    const result = await this.sendRequest(payload, 'node_copy');
    return result as Node;
  }

  /**
   * Copy a node tree to a new parent (deep copy with all children)
   *
   * @param fromPath - Source node path
   * @param toParentPath - Destination parent path
   * @param newName - New name for the copied node (optional)
   * @returns Copied node tree
   */
  async copyTree(fromPath: string, toParentPath: string, newName?: string): Promise<Node> {
    const payload: { from_path: string; to_parent_path: string; new_name?: string; deep: boolean } = {
      from_path: fromPath,
      to_parent_path: toParentPath,
      deep: true,
    };

    if (newName !== undefined) {
      payload.new_name = newName;
    }

    const result = await this.sendRequest(payload, 'node_copy_tree');
    return result as Node;
  }

  /**
   * Reorder a node by setting a new order key
   *
   * @param nodePath - Node path
   * @param orderKey - New order key (base62-encoded fractional index)
   * @returns Reordered node
   */
  async reorder(nodePath: string, orderKey: string): Promise<Node> {
    const payload = {
      node_path: nodePath,
      order_key: orderKey,
    };

    const result = await this.sendRequest(payload, 'node_reorder');
    return result as Node;
  }

  /**
   * Move a child node before a reference sibling
   *
   * @param parentPath - Parent node path
   * @param childPath - Child node path to move
   * @param referencePath - Reference sibling path to position before
   * @returns Moved node
   */
  async moveChildBefore(parentPath: string, childPath: string, referencePath: string): Promise<Node> {
    const payload = {
      parent_path: parentPath,
      child_path: childPath,
      reference_path: referencePath,
    };

    const result = await this.sendRequest(payload, 'node_move_child_before');
    return result as Node;
  }

  /**
   * Move a child node after a reference sibling
   *
   * @param parentPath - Parent node path
   * @param childPath - Child node path to move
   * @param referencePath - Reference sibling path to position after
   * @returns Moved node
   */
  async moveChildAfter(parentPath: string, childPath: string, referencePath: string): Promise<Node> {
    const payload = {
      parent_path: parentPath,
      child_path: childPath,
      reference_path: referencePath,
    };

    const result = await this.sendRequest(payload, 'node_move_child_after');
    return result as Node;
  }

  // ========================================================================
  // Relationship Operations
  // ========================================================================

  /**
   * Add a relationship between two nodes
   *
   * @param nodePath - Source node path
   * @param relationType - Type of relationship
   * @param targetNodePath - Target node path
   * @param weight - Optional relationship weight (legacy signature)
   * @returns True if the relation exists after the call
   */
  async addRelation(
    nodePath: string,
    relationType: string,
    targetNodePath: string,
    weightOrOptions?: number | RelationAddOptions
  ): Promise<boolean> {
    const options =
      typeof weightOrOptions === 'number'
        ? { weight: weightOrOptions }
        : weightOrOptions;

    const payload: RelationAddPayload = {
      source_path: nodePath,
      target_workspace: this.resolveTargetWorkspace(options),
      target_path: targetNodePath,
      relation_type: relationType,
    };

    if (options?.weight !== undefined) {
      payload.weight = options.weight;
    }

    const result = await this.sendRequest(payload, 'relation_add');
    return this.parseSuccess(result);
  }

  /**
   * Remove a relationship between two nodes
   *
   * @param nodePath - Source node path
   * @param targetPathOrRelationType - Target node path (preferred) or legacy relation type
   * @param targetPathOrOptions - Target path for the legacy signature or mutation options
   * @param options - Relationship options when using the legacy signature
   * @returns True if the relation was removed or no longer exists
   */
  async removeRelation(
    nodePath: string,
    targetPathOrRelationType: string,
    targetPathOrOptions?: string | RelationRemoveOptions,
    options?: RelationRemoveOptions
  ): Promise<boolean> {
    let targetPath: string;
    let resolvedOptions: RelationRemoveOptions | undefined;

    if (typeof targetPathOrOptions === 'string') {
      // Legacy signature: removeRelation(source, relationType, targetPath)
      targetPath = targetPathOrOptions;
      resolvedOptions = options;
    } else {
      // Preferred signature: removeRelation(source, targetPath, options?)
      targetPath = targetPathOrRelationType;
      resolvedOptions = targetPathOrOptions;
    }

    const payload: RelationRemovePayload = {
      source_path: nodePath,
      target_workspace: this.resolveTargetWorkspace(resolvedOptions),
      target_path: targetPath,
    };

    const result = await this.sendRequest(payload, 'relation_remove');
    return this.parseSuccess(result);
  }

  /**
   * Get all incoming and outgoing relationships for a node
   *
   * @param nodePath - Node path
   * @returns Relationship summary including incoming/outgoing edges
   */
  async getRelationships(nodePath: string): Promise<NodeRelationships> {
    const payload: RelationsGetPayload = {
      node_path: nodePath,
    };

    const result = await this.sendRequest(payload, 'relations_get');
    return result as NodeRelationships;
  }

  /**
   * Determine which workspace to target for relationship operations
   */
  private resolveTargetWorkspace(options?: RelationTargetOptions): string {
    const workspace = options?.targetWorkspace ?? this.workspace;
    if (!workspace) {
      throw new Error(
        'targetWorkspace is required when workspace context is not set'
      );
    }
    return workspace;
  }

  /**
   * Normalize success responses from relationship mutations
   */
  private parseSuccess(result: unknown): boolean {
    if (typeof result === 'boolean') {
      return result;
    }
    if (
      result &&
      typeof result === 'object' &&
      'success' in result &&
      typeof (result as { success?: unknown }).success !== 'undefined'
    ) {
      return Boolean((result as { success?: boolean }).success);
    }
    return true;
  }
}

interface RelationTargetOptions {
  targetWorkspace?: string;
}

interface RelationAddOptions extends RelationTargetOptions {
  weight?: number;
}

type RelationRemoveOptions = RelationTargetOptions;
