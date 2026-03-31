import { useState, useEffect, useCallback } from 'react'
import { revisionsApi, Revision } from '../api/revisions'

export function useRevisions(repo: string, branch?: string) {
  const [revisions, setRevisions] = useState<Revision[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<Error | null>(null)

  useEffect(() => {
    loadRevisions()
  }, [repo, branch])

  async function loadRevisions() {
    try {
      setLoading(true)
      setError(null)
      const data = await revisionsApi.list(repo, 50, 0, false, branch)
      setRevisions(data)
    } catch (err) {
      setError(err instanceof Error ? err : new Error('Failed to load revisions'))
    } finally {
      setLoading(false)
    }
  }

  const getRevision = useCallback(async (revisionNumber: string) => {
    return revisionsApi.get(repo, revisionNumber)
  }, [repo])

  const compareRevisions = useCallback(async (from: string, to: string) => {
    return revisionsApi.compare(repo, from, to)
  }, [repo])

  const getChanges = useCallback(async (revisionNumber: string) => {
    return revisionsApi.getChanges(repo, revisionNumber)
  }, [repo])

  return {
    revisions,
    loading,
    error,
    loadRevisions,
    getRevision,
    compareRevisions,
    getChanges,
  }
}
