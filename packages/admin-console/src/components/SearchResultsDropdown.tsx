import { useNavigate } from 'react-router-dom'
import { File, Folder, FileText, Database, Sparkles, Search } from 'lucide-react'
import type { SearchResultItem } from '../api/search'

interface SearchResultsDropdownProps {
  results: SearchResultItem[]
  isLoading: boolean
  query: string
  repo: string
  branch: string
  onClose: () => void
}

// Group results by workspace
function groupByWorkspace(results: SearchResultItem[]) {
  const grouped = new Map<string, SearchResultItem[]>()

  for (const result of results) {
    if (!grouped.has(result.workspace_id)) {
      grouped.set(result.workspace_id, [])
    }
    grouped.get(result.workspace_id)!.push(result)
  }

  return grouped
}

// Get icon for node type
function getNodeTypeIcon(nodeType: string) {
  if (nodeType.includes('Folder')) return <Folder className="w-4 h-4" />
  if (nodeType.includes('Page')) return <FileText className="w-4 h-4" />
  if (nodeType.includes('Document')) return <File className="w-4 h-4" />
  return <Database className="w-4 h-4" />
}

export default function SearchResultsDropdown({
  results,
  isLoading,
  query,
  repo,
  branch,
  onClose,
}: SearchResultsDropdownProps) {
  const navigate = useNavigate()

  const handleResultClick = (result: SearchResultItem) => {
    // Navigate to the content explorer for this workspace and node
    navigate(
      `/repos/${repo}/${branch}/${result.workspace_id}/content${result.path}`
    )
    onClose()
  }

  if (isLoading) {
    return (
      <div className="absolute z-50 w-full mt-2 p-6 bg-zinc-800/95 backdrop-blur-sm border border-zinc-700 rounded-lg shadow-xl">
        <div className="flex items-center justify-center text-zinc-400">
          <div className="animate-pulse">Searching...</div>
        </div>
      </div>
    )
  }

  if (!results.length) {
    return (
      <div className="absolute z-50 w-full mt-2 p-6 bg-zinc-800/95 backdrop-blur-sm border border-zinc-700 rounded-lg shadow-xl">
        <div className="text-center text-zinc-400">
          <p className="text-sm">No results found for "{query}"</p>
          <p className="text-xs mt-2 text-zinc-500">
            Try different keywords or check your search syntax
          </p>
        </div>
      </div>
    )
  }

  const groupedResults = groupByWorkspace(results)

  return (
    <div className="absolute z-50 w-full mt-2 max-h-[500px] overflow-y-auto bg-zinc-800/95 backdrop-blur-sm border border-zinc-700 rounded-lg shadow-xl">
      <div className="p-2">
        <div className="text-xs text-zinc-500 px-3 py-2">
          {results.length} result{results.length !== 1 ? 's' : ''} found
        </div>

        {Array.from(groupedResults.entries()).map(([workspaceId, workspaceResults]) => (
          <div key={workspaceId} className="mb-3 last:mb-0">
            {/* Workspace header */}
            <div className="px-3 py-1.5 mb-1">
              <div className="flex items-center gap-2">
                <div className="px-2 py-0.5 bg-primary-500/20 border border-primary-500/30 rounded text-xs font-semibold text-primary-300">
                  {workspaceId}
                </div>
                <span className="text-xs text-zinc-500">
                  {workspaceResults.length} result{workspaceResults.length !== 1 ? 's' : ''}
                </span>
              </div>
            </div>

            {/* Results for this workspace */}
            <div className="space-y-0.5">
              {workspaceResults.map((result) => (
                <button
                  key={`${result.workspace_id}-${result.node_id}`}
                  onClick={() => handleResultClick(result)}
                  className="w-full px-3 py-2.5 text-left hover:bg-zinc-700/50 rounded-lg transition-colors group"
                >
                  <div className="flex items-start gap-3">
                    {/* Icon */}
                    <div className="mt-0.5 text-zinc-400 group-hover:text-primary-400 transition-colors">
                      {getNodeTypeIcon(result.node_type)}
                    </div>

                    {/* Content */}
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2 mb-1">
                        <span className="font-medium text-white group-hover:text-primary-300 transition-colors truncate">
                          {result.name}
                        </span>
                        <span className="text-xs px-1.5 py-0.5 bg-zinc-700/50 rounded text-zinc-400 whitespace-nowrap">
                          {result.node_type}
                        </span>
                      </div>

                      <div className="flex items-center gap-2 text-xs text-zinc-500">
                        <span className="truncate">{result.path}</span>

                        {/* Hybrid search badges */}
                        {result.fulltext_rank && result.vector_distance !== undefined && (
                          <>
                            <span className="px-1.5 py-0.5 bg-blue-500/10 text-blue-400 rounded whitespace-nowrap flex items-center gap-1">
                              <Search className="w-3 h-3" />
                              #{result.fulltext_rank}
                            </span>
                            <span className="px-1.5 py-0.5 bg-primary-500/10 text-primary-400 rounded whitespace-nowrap flex items-center gap-1">
                              <Sparkles className="w-3 h-3" />
                              {result.vector_distance.toFixed(3)}
                            </span>
                          </>
                        )}

                        {/* Single mode badges */}
                        {result.fulltext_rank && result.vector_distance === undefined && (
                          <span className="px-1.5 py-0.5 bg-blue-500/10 text-blue-400 rounded whitespace-nowrap flex items-center gap-1">
                            <Search className="w-3 h-3" />
                            #{result.fulltext_rank}
                          </span>
                        )}

                        {result.vector_distance !== undefined && !result.fulltext_rank && (
                          <span className="px-1.5 py-0.5 bg-primary-500/10 text-primary-400 rounded whitespace-nowrap flex items-center gap-1">
                            <Sparkles className="w-3 h-3" />
                            {result.vector_distance.toFixed(3)}
                          </span>
                        )}

                        {/* Legacy score badge (for backward compatibility) */}
                        {result.score > 0 && !result.fulltext_rank && result.vector_distance === undefined && (
                          <span className="px-1.5 py-0.5 bg-amber-500/10 text-amber-400 rounded whitespace-nowrap">
                            {(result.score * 100).toFixed(0)}% match
                          </span>
                        )}
                      </div>
                    </div>
                  </div>
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
