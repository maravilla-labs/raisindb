/**
 * NodeBuilder - Ergonomic builder pattern for creating RaisinDB nodes
 *
 * @example
 * ```typescript
 * const node = NodeBuilder.create()
 *   .nodeType("Page")
 *   .name("My Page")
 *   .path("/content/my-page")
 *   .property("title", "Welcome")
 *   .property("published", true)
 *   .build();
 * ```
 */

import { Node, PropertyValue, RelationRef } from './protocol';

export class NodeBuilder {
  private node: Partial<Node> = {
    properties: {},
    children: [],
    relations: [],
    version: 1,
  };

  /**
   * Create a new NodeBuilder instance
   */
  static create(): NodeBuilder {
    return new NodeBuilder();
  }

  /**
   * Set the node ID
   */
  id(id: string): this {
    this.node.id = id;
    return this;
  }

  /**
   * Set the node name
   */
  name(name: string): this {
    this.node.name = name;
    return this;
  }

  /**
   * Set the node path
   */
  path(path: string): this {
    this.node.path = path;
    return this;
  }

  /**
   * Set the node type
   */
  nodeType(nodeType: string): this {
    this.node.node_type = nodeType;
    return this;
  }

  /**
   * Set the archetype
   */
  archetype(archetype: string): this {
    this.node.archetype = archetype;
    return this;
  }

  /**
   * Set a property value
   */
  property(key: string, value: PropertyValue): this {
    if (!this.node.properties) {
      this.node.properties = {};
    }
    this.node.properties[key] = value;
    return this;
  }

  /**
   * Set multiple properties at once
   */
  properties(properties: Record<string, PropertyValue>): this {
    this.node.properties = properties;
    return this;
  }

  /**
   * Add a child node ID
   */
  child(childId: string): this {
    if (!this.node.children) {
      this.node.children = [];
    }
    this.node.children.push(childId);
    return this;
  }

  /**
   * Set the children array
   */
  children(children: string[]): this {
    this.node.children = children;
    return this;
  }

  /**
   * Set the order key
   */
  orderKey(orderKey: string): this {
    this.node.order_key = orderKey;
    return this;
  }

  /**
   * Set the parent name
   */
  parent(parent: string): this {
    this.node.parent = parent;
    return this;
  }

  /**
   * Set the version
   */
  version(version: number): this {
    this.node.version = version;
    return this;
  }

  /**
   * Set the workspace
   */
  workspace(workspace: string): this {
    this.node.workspace = workspace;
    return this;
  }

  /**
   * Set the tenant ID
   */
  tenantId(tenantId: string): this {
    this.node.tenant_id = tenantId;
    return this;
  }

  /**
   * Add a relation to another node
   */
  relation(relation: RelationRef): this {
    if (!this.node.relations) {
      this.node.relations = [];
    }
    this.node.relations.push(relation);
    return this;
  }

  /**
   * Add a relation using individual parameters
   */
  addRelation(
    target: string,
    workspace: string,
    relationType: string,
    targetNodeType?: string,
    weight?: number
  ): this {
    return this.relation({
      target,
      workspace,
      relation_type: relationType,
      target_node_type: targetNodeType || '',
      weight,
    });
  }

  /**
   * Set the relations array
   */
  relations(relations: RelationRef[]): this {
    this.node.relations = relations;
    return this;
  }

  /**
   * Build and return the Node object
   * @throws {Error} if required fields are missing
   */
  build(): Node {
    // Validate required fields
    if (!this.node.id) {
      throw new Error('Node ID is required');
    }
    if (!this.node.name) {
      throw new Error('Node name is required');
    }
    if (!this.node.path) {
      throw new Error('Node path is required');
    }
    if (!this.node.node_type) {
      throw new Error('Node type is required');
    }

    // Ensure required array fields are initialized
    if (!this.node.children) this.node.children = [];
    if (!this.node.relations) this.node.relations = [];
    if (!this.node.properties) this.node.properties = {};
    if (!this.node.order_key) this.node.order_key = 'a';
    if (typeof this.node.version !== 'number') this.node.version = 1;

    return this.node as Node;
  }
}

/**
 * Helper functions for working with Node objects
 */
export class NodeHelpers {
  /**
   * Get a property value from a node
   */
  static getProperty<T = PropertyValue>(node: Node, key: string): T | undefined {
    return node.properties[key] as T | undefined;
  }

  /**
   * Set a property value on a node (mutating)
   */
  static setProperty(node: Node, key: string, value: PropertyValue): void {
    node.properties[key] = value;
  }

  /**
   * Get the parent path derived from the node's path
   */
  static getParentPath(node: Node): string | null {
    if (!node.path || node.path === '/') {
      return null;
    }
    const lastSlash = node.path.lastIndexOf('/');
    if (lastSlash <= 0) {
      return '/';
    }
    return node.path.substring(0, lastSlash);
  }

  /**
   * Check if the node is published
   */
  static isPublished(node: Node): boolean {
    return node.published_at != null && node.published_at.length > 0;
  }

  /**
   * Check if the node has children
   */
  static hasChildren(node: Node): boolean {
    return node.has_children ?? (node.children?.length ?? 0) > 0;
  }

  /**
   * Add a relation to a node (mutating)
   */
  static addRelation(
    node: Node,
    target: string,
    workspace: string,
    relationType: string,
    targetNodeType?: string,
    weight?: number
  ): void {
    if (!node.relations) {
      node.relations = [];
    }
    node.relations.push({
      target,
      workspace,
      relation_type: relationType,
      target_node_type: targetNodeType || '',
      weight,
    });
  }

  /**
   * Get relations of a specific type
   */
  static getRelationsByType(node: Node, relationType: string): RelationRef[] {
    return (node.relations || []).filter(r => r.relation_type === relationType);
  }

  /**
   * Get the name from a path (last segment)
   */
  static getNameFromPath(path: string): string {
    const parts = path.split('/').filter(p => p.length > 0);
    return parts[parts.length - 1] || '';
  }

  /**
   * Create a minimal node for testing/mocking
   */
  static createMock(overrides?: Partial<Node>): Node {
    return {
      id: 'mock-id',
      name: 'Mock Node',
      path: '/mock',
      node_type: 'MockType',
      properties: {},
      children: [],
      relations: [],
      order_key: 'a',
      version: 1,
      ...overrides,
    };
  }
}
