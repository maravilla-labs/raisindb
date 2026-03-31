/**
 * useTypePickerTree - Hook for building and filtering the namespace tree
 *
 * Converts a flat list of types into a hierarchical tree structure
 * grouped by namespace, with search filtering support.
 */

import { useMemo } from 'react'
import type { PickableType, TypeTreeNode } from './types'

/**
 * Parse a type name into namespace segments and type name
 * e.g., "news:component:Hero" -> { namespace: ["news", "component"], typeName: "Hero" }
 */
function parseNamespace(name: string): { namespace: string[]; typeName: string } {
  const parts = name.split(':')
  if (parts.length === 1) {
    // No namespace (e.g., "Article")
    return { namespace: [], typeName: parts[0] }
  }
  // Multi-level namespace (e.g., "news:component:Hero" -> ["news", "component"], "Hero")
  return {
    namespace: parts.slice(0, -1),
    typeName: parts[parts.length - 1],
  }
}

/**
 * Count all item nodes in a tree node (recursively)
 */
export function countItems(node: TypeTreeNode): number {
  if (node.type === 'item') return 1
  if (!node.children) return 0
  return node.children.reduce((sum, child) => sum + countItems(child), 0)
}

/**
 * Sort tree nodes: namespaces first (alphabetically), then items (alphabetically)
 */
function sortNodes(nodes: TypeTreeNode[]): TypeTreeNode[] {
  return [...nodes]
    .sort((a, b) => {
      // Namespaces first
      if (a.type !== b.type) {
        return a.type === 'namespace' ? -1 : 1
      }
      // Then alphabetically by name
      return a.name.localeCompare(b.name)
    })
    .map((node) => ({
      ...node,
      children: node.children ? sortNodes(node.children) : undefined,
    }))
}

/**
 * Build a tree structure from a flat list of types
 */
function buildTree(items: PickableType[]): TypeTreeNode[] {
  // Map to track namespace nodes by their full path
  const namespaceMap = new Map<string, TypeTreeNode>()
  // Root level nodes (items without namespace and top-level namespaces)
  const rootNodes: TypeTreeNode[] = []

  for (const item of items) {
    const { namespace, typeName } = parseNamespace(item.name)

    if (namespace.length === 0) {
      // Root-level item (no namespace)
      rootNodes.push({
        id: `item-${item.name}`,
        type: 'item',
        name: typeName,
        fullPath: item.name,
        depth: 0,
        item,
      })
    } else {
      // Ensure all namespace levels exist
      let currentPath = ''

      for (let i = 0; i < namespace.length; i++) {
        const segment = namespace[i]
        const parentPath = currentPath
        currentPath = currentPath ? `${currentPath}:${segment}` : segment

        if (!namespaceMap.has(currentPath)) {
          const namespaceNode: TypeTreeNode = {
            id: `ns-${currentPath}`,
            type: 'namespace',
            name: segment,
            fullPath: currentPath,
            depth: i,
            children: [],
          }
          namespaceMap.set(currentPath, namespaceNode)

          // Add to parent or root
          if (parentPath) {
            const parent = namespaceMap.get(parentPath)
            parent?.children?.push(namespaceNode)
          } else {
            rootNodes.push(namespaceNode)
          }
        }
      }

      // Add the item to its namespace
      const parentNamespace = namespace.join(':')
      const parentNode = namespaceMap.get(parentNamespace)
      if (parentNode?.children) {
        parentNode.children.push({
          id: `item-${item.name}`,
          type: 'item',
          name: typeName,
          fullPath: item.name,
          depth: namespace.length,
          item,
        })
      }
    }
  }

  return sortNodes(rootNodes)
}

/**
 * Filter tree by search query, returning filtered tree and paths to expand
 */
function filterTree(
  tree: TypeTreeNode[],
  query: string
): { filtered: TypeTreeNode[]; expandedPaths: Set<string> } {
  const lowerQuery = query.toLowerCase().trim()
  const expandedPaths = new Set<string>()

  function filterNode(node: TypeTreeNode): TypeTreeNode | null {
    if (node.type === 'item') {
      // Check if item matches the query
      const nameMatches = node.name.toLowerCase().includes(lowerQuery)
      const fullPathMatches = node.fullPath.toLowerCase().includes(lowerQuery)
      const descMatches = node.item?.description?.toLowerCase().includes(lowerQuery)

      return nameMatches || fullPathMatches || descMatches ? node : null
    }

    // Namespace node - filter children recursively
    const filteredChildren = node.children
      ?.map((child) => filterNode(child))
      .filter((child): child is TypeTreeNode => child !== null)

    if (filteredChildren && filteredChildren.length > 0) {
      // This namespace has matching descendants - mark it for expansion
      expandedPaths.add(node.fullPath)
      return { ...node, children: filteredChildren }
    }

    // Check if namespace name itself matches
    if (node.name.toLowerCase().includes(lowerQuery)) {
      expandedPaths.add(node.fullPath)
      return node // Return with all children
    }

    return null
  }

  const filtered = tree
    .map((node) => filterNode(node))
    .filter((node): node is TypeTreeNode => node !== null)

  return { filtered, expandedPaths }
}

/**
 * Flatten tree to get all item paths in order (for keyboard navigation)
 */
export function flattenTreePaths(
  tree: TypeTreeNode[],
  expandedPaths: Set<string>
): string[] {
  const paths: string[] = []

  function traverse(nodes: TypeTreeNode[]) {
    for (const node of nodes) {
      if (node.type === 'item') {
        paths.push(node.fullPath)
      } else if (node.children && expandedPaths.has(node.fullPath)) {
        traverse(node.children)
      }
    }
  }

  traverse(tree)
  return paths
}

/**
 * Hook for managing the type picker tree
 */
export function useTypePickerTree(items: PickableType[], searchQuery: string) {
  // Build the full tree
  const tree = useMemo(() => buildTree(items), [items])

  // Filter tree based on search query
  const { filtered, searchExpandedPaths } = useMemo(() => {
    if (!searchQuery.trim()) {
      return { filtered: tree, searchExpandedPaths: new Set<string>() }
    }
    const result = filterTree(tree, searchQuery)
    return { filtered: result.filtered, searchExpandedPaths: result.expandedPaths }
  }, [tree, searchQuery])

  return {
    tree,
    filteredTree: filtered,
    searchExpandedPaths,
  }
}
