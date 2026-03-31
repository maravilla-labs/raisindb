/**
 * Flow Helper Utilities
 *
 * Functions for traversing and manipulating the flow tree.
 */

import type {
  FlowNode,
  FlowContainer,
  FlowDefinition,
  InsertPosition,
} from '../types';
import { isFlowContainer } from '../types';

/** Result of finding a node with its parent context */
export interface FindNodeResult {
  /** The found node */
  node: FlowNode;
  /** Parent container (null if at root) */
  parent: FlowContainer | null;
  /** Index within parent's children or root nodes */
  index: number;
  /** Full path of parent IDs from root */
  path: string[];
}

/**
 * Find a node by ID in the flow tree
 */
export function findNodeById(
  nodes: FlowNode[],
  nodeId: string,
  parent: FlowContainer | null = null,
  path: string[] = []
): FindNodeResult | null {
  for (let i = 0; i < nodes.length; i++) {
    const node = nodes[i];
    if (node.id === nodeId) {
      return { node, parent, index: i, path };
    }
    if (isFlowContainer(node)) {
      const result = findNodeById(
        node.children,
        nodeId,
        node,
        [...path, node.id]
      );
      if (result) return result;
    }
  }
  return null;
}

/**
 * Find a node and its parent in the flow definition
 */
export function findNodeAndParent(
  flow: FlowDefinition,
  nodeId: string
): FindNodeResult | null {
  return findNodeById(flow.nodes, nodeId);
}

/**
 * Get all ancestor IDs for a node
 */
export function getAncestorIds(
  flow: FlowDefinition,
  nodeId: string
): string[] {
  const result = findNodeAndParent(flow, nodeId);
  return result?.path ?? [];
}

/**
 * Check if a node is an ancestor of another node
 */
export function isAncestorOf(
  flow: FlowDefinition,
  ancestorId: string,
  descendantId: string
): boolean {
  const result = findNodeAndParent(flow, descendantId);
  return result?.path.includes(ancestorId) ?? false;
}

/**
 * Remove a node from the flow tree by ID
 */
export function removeNodeById(
  nodes: FlowNode[],
  nodeId: string
): boolean {
  for (let i = 0; i < nodes.length; i++) {
    if (nodes[i].id === nodeId) {
      nodes.splice(i, 1);
      return true;
    }
    const node = nodes[i];
    if (isFlowContainer(node)) {
      if (removeNodeById(node.children, nodeId)) {
        return true;
      }
    }
  }
  return false;
}

/**
 * Insert a node relative to a target node
 */
export function insertNode(
  nodes: FlowNode[],
  newNode: FlowNode,
  targetId: string,
  position: InsertPosition
): boolean {
  const normalizedPosition =
    position === 'left'
      ? 'before'
      : position === 'right'
      ? 'after'
      : position;

  for (let i = 0; i < nodes.length; i++) {
    const node = nodes[i];

    if (node.id === targetId) {
      if (normalizedPosition === 'inside' && isFlowContainer(node)) {
        node.children.push(newNode);
        return true;
      } else if (normalizedPosition === 'before') {
        nodes.splice(i, 0, newNode);
        return true;
      } else if (normalizedPosition === 'after') {
        nodes.splice(i + 1, 0, newNode);
        return true;
      }
    }

    if (isFlowContainer(node)) {
      if (insertNode(node.children, newNode, targetId, position)) {
        return true;
      }
    }
  }
  return false;
}

/**
 * Deep clone a flow definition to ensure immutability
 */
export function cloneFlow(flow: FlowDefinition): FlowDefinition {
  return JSON.parse(JSON.stringify(flow));
}

/**
 * Deep clone a flow node
 */
export function cloneNode<T extends FlowNode>(node: T): T {
  return JSON.parse(JSON.stringify(node));
}

/**
 * Count total nodes in the flow tree
 */
export function countNodes(nodes: FlowNode[]): number {
  let count = nodes.length;
  for (const node of nodes) {
    if (isFlowContainer(node)) {
      count += countNodes(node.children);
    }
  }
  return count;
}

/**
 * Get all node IDs in the flow tree
 */
export function getAllNodeIds(nodes: FlowNode[]): string[] {
  const ids: string[] = [];
  for (const node of nodes) {
    ids.push(node.id);
    if (isFlowContainer(node)) {
      ids.push(...getAllNodeIds(node.children));
    }
  }
  return ids;
}

/**
 * Create an empty flow definition
 */
export function createEmptyFlow(): FlowDefinition {
  return {
    version: 1,
    error_strategy: 'fail_fast',
    nodes: [],
  };
}
