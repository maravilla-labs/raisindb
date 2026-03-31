import { useState, useEffect } from 'react'
import { nodeTypesApi, type ResolvedNodeType } from '../api/nodetypes'

/**
 * Hook to fetch and cache node type definitions
 */
export function useNodeType(
  repo: string | undefined,
  branch: string | undefined,
  nodeTypeName: string | undefined,
  workspace?: string
) {
  const [nodeType, setNodeType] = useState<ResolvedNodeType | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<Error | null>(null)

  useEffect(() => {
    if (!nodeTypeName || !repo || !branch) {
      setNodeType(null)
      setLoading(false)
      setError(null)
      return
    }

    let cancelled = false

    async function fetchNodeType() {
      setLoading(true)
      setError(null)

      try {
        // TypeScript needs this check even though we checked above
        if (!nodeTypeName || !repo || !branch) return

        const resolved = await nodeTypesApi.getResolved(repo, branch, nodeTypeName, workspace)
        if (!cancelled) {
          setNodeType(resolved)
        }
      } catch (err) {
        if (!cancelled) {
          setError(err as Error)
          setNodeType(null)
        }
      } finally {
        if (!cancelled) {
          setLoading(false)
        }
      }
    }

    fetchNodeType()

    return () => {
      cancelled = true
    }
  }, [repo, branch, workspace, nodeTypeName])

  return { nodeType, loading, error }
}

/**
 * Hook to validate node properties against its type schema
 */
export function useNodeValidation(
  nodeType: ResolvedNodeType | null,
  properties: Record<string, any>
) {
  const [errors, setErrors] = useState<Record<string, string>>({})

  useEffect(() => {
    if (!nodeType || !nodeType.resolved_properties) {
      setErrors({})
      return
    }

    const newErrors: Record<string, string> = {}

    // Validate each property defined in the schema
    nodeType.resolved_properties.forEach((propSchema: any) => {
      const propName = propSchema.name
      const value = properties[propName]

      // Check required fields
      if (propSchema.required && (value === undefined || value === null || value === '')) {
        newErrors[propName] = `${propName} is required`
        return
      }

      // Type validation
      if (value !== undefined && value !== null) {
        switch (propSchema.type?.toLowerCase()) {
          case 'string':
            if (typeof value !== 'string') {
              newErrors[propName] = `${propName} must be a string`
            } else {
              if (propSchema.minLength && value.length < propSchema.minLength) {
                newErrors[propName] = `${propName} must be at least ${propSchema.minLength} characters`
              }
              if (propSchema.maxLength && value.length > propSchema.maxLength) {
                newErrors[propName] = `${propName} must be at most ${propSchema.maxLength} characters`
              }
              if (propSchema.pattern && !new RegExp(propSchema.pattern).test(value)) {
                newErrors[propName] = `${propName} has invalid format`
              }
            }
            break

          case 'number':
          case 'integer':
            if (typeof value !== 'number' || isNaN(value)) {
              newErrors[propName] = `${propName} must be a number`
            } else {
              if (propSchema.minimum !== undefined && value < propSchema.minimum) {
                newErrors[propName] = `${propName} must be at least ${propSchema.minimum}`
              }
              if (propSchema.maximum !== undefined && value > propSchema.maximum) {
                newErrors[propName] = `${propName} must be at most ${propSchema.maximum}`
              }
              if (propSchema.type === 'integer' && !Number.isInteger(value)) {
                newErrors[propName] = `${propName} must be an integer`
              }
            }
            break

          case 'boolean':
            if (typeof value !== 'boolean') {
              newErrors[propName] = `${propName} must be a boolean`
            }
            break

          case 'array':
            if (!Array.isArray(value)) {
              newErrors[propName] = `${propName} must be an array`
            } else {
              if (propSchema.minItems && value.length < propSchema.minItems) {
                newErrors[propName] = `${propName} must have at least ${propSchema.minItems} items`
              }
              if (propSchema.maxItems && value.length > propSchema.maxItems) {
                newErrors[propName] = `${propName} must have at most ${propSchema.maxItems} items`
              }
            }
            break

          case 'object':
            if (typeof value !== 'object' || value === null || Array.isArray(value)) {
              newErrors[propName] = `${propName} must be an object`
            }
            break
        }

        // Check enum values
        if (propSchema.enum && !propSchema.enum.includes(value)) {
          newErrors[propName] = `${propName} must be one of: ${propSchema.enum.join(', ')}`
        }
      }
    })

    setErrors(newErrors)
  }, [nodeType, properties])

  return {
    errors,
    isValid: Object.keys(errors).length === 0,
    validate: () => Object.keys(errors).length === 0
  }
}

// Cache for node types to avoid repeated fetches
const nodeTypeCache = new Map<string, ResolvedNodeType>()

/**
 * Hook to manage node type cache
 */
export function useNodeTypeCache(repo: string, branch: string) {
  const prefetch = async (nodeTypeNames: string[]) => {
    const promises = nodeTypeNames
      .filter(name => !nodeTypeCache.has(`${repo}:${branch}:${name}`))
      .map(async name => {
        try {
          const resolved = await nodeTypesApi.getResolved(repo, branch, name)
          nodeTypeCache.set(`${repo}:${branch}:${name}`, resolved)
        } catch (error) {
          console.error(`Failed to prefetch node type ${name}:`, error)
        }
      })
    await Promise.all(promises)
  }

  const getCached = (nodeTypeName: string): ResolvedNodeType | undefined => {
    return nodeTypeCache.get(`${repo}:${branch}:${nodeTypeName}`)
  }

  const clearCache = () => {
    nodeTypeCache.clear()
  }

  return { prefetch, getCached, clearCache }
}
