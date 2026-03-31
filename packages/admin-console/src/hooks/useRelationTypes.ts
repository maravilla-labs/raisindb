import { useState, useEffect } from 'react'
import { nodesApi, type Node } from '../api/nodes'

const WORKSPACE = 'raisin:access_control'
const BASE_PATH = '/relation-types'

export interface RelationType {
  id: string
  name: string
  relation_name: string
  title: string
  description?: string
  category?: string
  inverse_relation_name?: string
  bidirectional?: boolean
  icon?: string
  color?: string
}

function nodeToRelationType(node: Node): RelationType {
  return {
    id: node.id,
    name: node.name,
    relation_name: node.properties?.relation_name as string,
    title: node.properties?.title as string,
    description: node.properties?.description as string | undefined,
    category: node.properties?.category as string | undefined,
    inverse_relation_name: node.properties?.inverse_relation_name as string | undefined,
    bidirectional: node.properties?.bidirectional as boolean | undefined,
    icon: node.properties?.icon as string | undefined,
    color: node.properties?.color as string | undefined,
  }
}

// Cache for relation types per repo/branch
const relationTypesCache = new Map<string, RelationType[]>()

/**
 * Hook to fetch relation types from raisin:access_control/relation-types/
 * Returns both the full RelationType objects and a simple list of relation_name strings
 * for use in autocomplete.
 */
export function useRelationTypes(repo: string | undefined, branch: string | undefined) {
  const [relationTypes, setRelationTypes] = useState<RelationType[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<Error | null>(null)

  useEffect(() => {
    if (!repo || !branch) {
      setRelationTypes([])
      setLoading(false)
      setError(null)
      return
    }

    const cacheKey = `${repo}:${branch}`

    // Check cache first
    if (relationTypesCache.has(cacheKey)) {
      setRelationTypes(relationTypesCache.get(cacheKey)!)
      setLoading(false)
      return
    }

    let cancelled = false

    async function fetchRelationTypes() {
      setLoading(true)
      setError(null)

      try {
        if (!repo || !branch) return

        const nodes = await nodesApi.listChildrenAtHead(repo, branch, WORKSPACE, BASE_PATH)
        const relationTypeNodes = nodes.filter(n => n.node_type === 'raisin:RelationType')
        const types = relationTypeNodes.map(nodeToRelationType)

        if (!cancelled) {
          relationTypesCache.set(cacheKey, types)
          setRelationTypes(types)
        }
      } catch (err) {
        if (!cancelled) {
          setError(err as Error)
          setRelationTypes([])
        }
      } finally {
        if (!cancelled) {
          setLoading(false)
        }
      }
    }

    fetchRelationTypes()

    return () => {
      cancelled = true
    }
  }, [repo, branch])

  // Extract just the relation names for autocomplete
  const relationNames = relationTypes.map(rt => rt.relation_name).filter(Boolean)

  return {
    relationTypes,
    relationNames,
    loading,
    error,
    refresh: () => {
      if (repo && branch) {
        const cacheKey = `${repo}:${branch}`
        relationTypesCache.delete(cacheKey)
        // Trigger re-fetch by updating state
        setRelationTypes([])
      }
    }
  }
}

/**
 * Clear the relation types cache for a specific repo/branch or all
 */
export function clearRelationTypesCache(repo?: string, branch?: string) {
  if (repo && branch) {
    relationTypesCache.delete(`${repo}:${branch}`)
  } else {
    relationTypesCache.clear()
  }
}
