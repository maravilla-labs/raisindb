import { useParams, useSearchParams } from 'react-router-dom'

/**
 * Hook to extract repository context from URL parameters
 * with sensible defaults and revision support
 */
export function useRepositoryContext() {
  const { repo, branch, workspace } = useParams<{
    repo: string
    branch?: string
    workspace?: string
  }>()
  
  const [searchParams] = useSearchParams()
  const revision = searchParams.get('rev') ? parseInt(searchParams.get('rev')!, 10) : null

  return {
    repo: repo!,
    branch: branch || 'main',
    workspace: workspace || 'main',
    revision,
    isTimeTravelMode: revision !== null,
  }
}
