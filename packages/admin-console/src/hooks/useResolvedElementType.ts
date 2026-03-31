import { useState, useEffect, useRef } from 'react'
import { elementTypesApi, type ResolvedElementType } from '../api/elementtypes'

// Module-level cache to avoid redundant fetches when multiple elements share a type
const elementTypeCache = new Map<string, ResolvedElementType>()

/**
 * Hook to fetch and cache resolved element types.
 * Uses a module-level cache so multiple components sharing the same
 * element type don't trigger redundant API calls.
 */
export function useResolvedElementType(
  repo: string | undefined,
  branch: string | undefined,
  name: string | null | undefined
): { data: ResolvedElementType | null; loading: boolean; error: Error | null } {
  const [data, setData] = useState<ResolvedElementType | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<Error | null>(null)
  const fetchIdRef = useRef(0)

  useEffect(() => {
    if (!name || !repo || !branch) {
      setData(null)
      setLoading(false)
      setError(null)
      return
    }

    const cacheKey = `${repo}:${branch}:${name}`
    const cached = elementTypeCache.get(cacheKey)
    if (cached) {
      setData(cached)
      setLoading(false)
      setError(null)
      return
    }

    const fetchId = ++fetchIdRef.current

    async function fetchElementType() {
      setLoading(true)
      setError(null)
      try {
        if (!name || !repo || !branch) return
        const resolved = await elementTypesApi.getResolved(repo, branch, name)
        elementTypeCache.set(cacheKey, resolved)
        if (fetchId === fetchIdRef.current) {
          setData(resolved)
        }
      } catch (err) {
        if (fetchId === fetchIdRef.current) {
          setError(err as Error)
          setData(null)
        }
      } finally {
        if (fetchId === fetchIdRef.current) {
          setLoading(false)
        }
      }
    }

    fetchElementType()
  }, [repo, branch, name])

  return { data, loading, error }
}

/** Invalidate the element type cache (e.g. after schema changes) */
export function clearElementTypeCache() {
  elementTypeCache.clear()
}
