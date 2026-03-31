import { useState, useEffect, useRef, useCallback } from 'react'
import { Search, Loader, X, Sparkles } from 'lucide-react'
import { searchApi, type SearchResultItem } from '../api/search'
import { embeddingsApi } from '../api/embeddings'
import SearchResultsDropdown from './SearchResultsDropdown'

interface GlobalSearchBarProps {
  repo: string
  branch: string
}

export default function GlobalSearchBar({ repo, branch }: GlobalSearchBarProps) {
  const [query, setQuery] = useState('')
  const [isOpen, setIsOpen] = useState(false)
  const [results, setResults] = useState<SearchResultItem[]>([])
  const [isLoading, setIsLoading] = useState(false)
  const [showHints, setShowHints] = useState(false)
  const [hasEmbeddings, setHasEmbeddings] = useState(false)
  const [includeVectorSearch, setIncludeVectorSearch] = useState(false)

  const inputRef = useRef<HTMLInputElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const debounceTimerRef = useRef<number | null>(null)

  // Check if tenant has embeddings configured
  useEffect(() => {
    const checkEmbeddings = async () => {
      try {
        const config = await embeddingsApi.getConfig('default') // TODO: get tenant from context
        setHasEmbeddings(config.enabled && config.has_api_key)
      } catch (error) {
        console.error('Failed to check embeddings config:', error)
        setHasEmbeddings(false)
      }
    }
    checkEmbeddings()
  }, [])

  // Debounced search function
  const performSearch = useCallback(async (searchQuery: string) => {
    if (!searchQuery.trim()) {
      setResults([])
      setIsLoading(false)
      return
    }

    setIsLoading(true)
    try {
      if (hasEmbeddings && includeVectorSearch) {
        // Use hybrid search when vector search is enabled
        const response = await searchApi.hybridSearch(repo, {
          q: searchQuery,
          strategy: 'hybrid',
          limit: 20,
          branch,
          workspace: 'staff', // TODO: Get from route/context
        })
        setResults(response.results)
      } else {
        // Use traditional fulltext search
        const searchResults = await searchApi.search(repo, branch, {
          query: searchQuery,
          limit: 20,
        })
        setResults(searchResults)
      }
    } catch (error) {
      console.error('Search failed:', error)
      setResults([])
    } finally {
      setIsLoading(false)
    }
  }, [repo, branch, hasEmbeddings, includeVectorSearch])

  // Re-run search when vector search mode changes (if there's already a query)
  // Use debounce to avoid immediate re-search
  useEffect(() => {
    if (query.trim() && debounceTimerRef.current === null) {
      // Only re-search if there's no active typing (debounce timer cleared)
      debounceTimerRef.current = setTimeout(() => {
        performSearch(query)
        debounceTimerRef.current = null
      }, 600) // Wait 600ms before re-searching on checkbox change
    }
  }, [includeVectorSearch]) // Only when checkbox changes

  // Handle input change with debouncing
  const handleInputChange = (value: string) => {
    setQuery(value)
    setIsOpen(true)

    // Clear existing timer
    if (debounceTimerRef.current) {
      clearTimeout(debounceTimerRef.current)
    }

    // Set new timer
    if (value.trim()) {
      debounceTimerRef.current = setTimeout(() => {
        performSearch(value)
        debounceTimerRef.current = null
      }, 600) // 600ms debounce (increased from 300ms)
    } else {
      setResults([])
      setIsLoading(false)
    }
  }

  // Close dropdown when clicking outside
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setIsOpen(false)
      }
    }

    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [])

  // Keyboard shortcuts
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Cmd/Ctrl + K to focus search
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault()
        inputRef.current?.focus()
        setIsOpen(true)
      }
      // Escape to close and blur
      if (e.key === 'Escape') {
        setIsOpen(false)
        inputRef.current?.blur()
      }
    }

    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [])

  const clearSearch = () => {
    setQuery('')
    setResults([])
    setIsOpen(false)
    inputRef.current?.focus()
  }

  return (
    <div ref={containerRef} className="relative w-full max-w-md">
      <div className="relative">
        {includeVectorSearch ? (
          <Sparkles className="absolute left-3 top-1/2 transform -translate-y-1/2 w-4 h-4 text-primary-400 animate-pulse" />
        ) : (
          <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-4 h-4 text-zinc-400" />
        )}

        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => handleInputChange(e.target.value)}
          onFocus={() => {
            setIsOpen(true)
            setShowHints(true)
          }}
          onBlur={() => setTimeout(() => setShowHints(false), 200)}
          onKeyDown={(e) => {
            // Prevent Enter from submitting or doing anything unexpected
            // Users can click on results to navigate
            if (e.key === 'Enter') {
              e.preventDefault()
              e.stopPropagation()
            }
          }}
          placeholder={includeVectorSearch ? "AI semantic search... (⌘K)" : "Search nodes... (⌘K)"}
          className={`w-full pl-10 pr-10 py-2 bg-zinc-800/50 border rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:ring-2 focus:ring-primary-500 transition-all ${
            includeVectorSearch
              ? 'border-primary-500/50 focus:border-primary-500'
              : 'border-zinc-700 focus:border-primary-500'
          }`}
        />

        {isLoading && (
          <Loader className="absolute right-10 top-1/2 transform -translate-y-1/2 w-4 h-4 text-primary-400 animate-spin" />
        )}

        {query && !isLoading && (
          <button
            onClick={clearSearch}
            className="absolute right-3 top-1/2 transform -translate-y-1/2 text-zinc-400 hover:text-white transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        )}
      </div>

      {/* Search hints (when focused and empty) */}
      {isOpen && showHints && !query && (
        <div className="absolute z-50 w-full mt-2 p-4 bg-zinc-800/95 backdrop-blur-sm border border-zinc-700 rounded-lg shadow-xl">
          {/* Vector Search Toggle (only if embeddings configured) */}
          {hasEmbeddings && (
            <div className="mb-4 pb-3 border-b border-zinc-700">
              <label className="flex items-center gap-2 cursor-pointer group">
                <input
                  type="checkbox"
                  checked={includeVectorSearch}
                  onChange={(e) => setIncludeVectorSearch(e.target.checked)}
                  className="w-4 h-4 rounded border-zinc-600 bg-zinc-700 text-primary-500 focus:ring-2 focus:ring-primary-500 focus:ring-offset-0 transition-all"
                />
                <Sparkles className={`w-4 h-4 transition-colors ${includeVectorSearch ? 'text-primary-400' : 'text-zinc-500 group-hover:text-primary-400'}`} />
                <span className="text-sm text-zinc-300 group-hover:text-white transition-colors">
                  Include AI semantic search
                </span>
              </label>
              <p className="text-xs text-zinc-500 mt-1 ml-6">
                {includeVectorSearch
                  ? '✓ Combining fulltext + vector similarity with RRF'
                  : 'Enable to find semantically similar content'}
              </p>
            </div>
          )}

          <div className="text-xs text-zinc-400 space-y-2">
            <p className="font-semibold text-zinc-300">Search features:</p>
            <ul className="space-y-1 ml-2">
              <li>✨ <span className="text-primary-300">Auto fuzzy</span> - Automatically finds similar words (e.g., "hallo" finds "halo")</li>
              <li><code className="px-1 bg-zinc-700/50 rounded">hal*</code> - Wildcard (finds "halo", "hall", "halloween")</li>
              <li><code className="px-1 bg-zinc-700/50 rounded">h?lo</code> - Single char wildcard (finds "halo", "hilo")</li>
              <li><code className="px-1 bg-zinc-700/50 rounded">"exact phrase"</code> - Phrase search</li>
              <li><code className="px-1 bg-zinc-700/50 rounded">cat AND dog</code> - Boolean operators (AND, OR, NOT)</li>
            </ul>
            <p className="text-zinc-500 text-[10px] mt-2 italic">
              💡 Wildcards disable fuzzy matching. Use one or the other.
            </p>
          </div>
        </div>
      )}

      {/* Search results dropdown */}
      {isOpen && query && (
        <SearchResultsDropdown
          results={results}
          isLoading={isLoading}
          query={query}
          repo={repo}
          branch={branch}
          onClose={() => setIsOpen(false)}
        />
      )}
    </div>
  )
}
