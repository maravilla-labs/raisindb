import { useState, useEffect } from 'react'
import { nodesApi, Node } from '../api/nodes'

export function useNode(
  repo: string,
  branch: string,
  workspace: string,
  path: string,
  revision: string | null = null
) {
  const [node, setNode] = useState<Node | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<Error | null>(null)
  
  useEffect(() => {
    if (!repo || !branch || !workspace || !path) {
      setNode(null)
      return
    }
    
    setLoading(true)
    setError(null)
    
    const loadNode = async () => {
      try {
        const data = revision !== null
          ? await nodesApi.getAtRevision(repo, branch, workspace, path, revision)
          : await nodesApi.getAtHead(repo, branch, workspace, path)
        setNode(data)
      } catch (err) {
        setError(err as Error)
        setNode(null)
      } finally {
        setLoading(false)
      }
    }
    
    loadNode()
  }, [repo, branch, workspace, path, revision])
  
  return { node, loading, error }
}
