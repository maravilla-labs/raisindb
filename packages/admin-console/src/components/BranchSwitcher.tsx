import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { useNavigate } from 'react-router-dom'
import { GitBranch, Tag, Check, ChevronDown, Search, Lock } from 'lucide-react'
import { branchesApi, tagsApi, type Branch, type Tag as TagType } from '../api/branches'
import { useRepositoryContext } from '../hooks/useRepositoryContext'

interface BranchSwitcherProps {
  /** Optional className for styling */
  className?: string
  /** Show in compact mode (for mobile/tight spaces) */
  compact?: boolean
  /** Optional custom navigation handler. If not provided, defaults to content navigation. */
  onBranchSelect?: (branchName: string, isTag: boolean) => void
}

export default function BranchSwitcher({ 
  className = '', 
  compact = false,
  onBranchSelect: customNavigate
}: BranchSwitcherProps) {
  const navigate = useNavigate()
  const { repo, branch: currentBranch, workspace } = useRepositoryContext()
  const [isOpen, setIsOpen] = useState(false)
  const [branches, setBranches] = useState<Branch[]>([])
  const [tags, setTags] = useState<TagType[]>([])
  const [searchTerm, setSearchTerm] = useState('')
  const [loading, setLoading] = useState(false)
  const [buttonRect, setButtonRect] = useState<DOMRect | null>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)
  const [activeTab, setActiveTab] = useState<'branches' | 'tags'>('branches')

  // Preload branches and tags when component mounts or workspace changes
  useEffect(() => {
    if (repo) {
      loadBranchesAndTags()
    }
  }, [repo, workspace])

  // Close dropdown when clicking outside
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false)
      }
    }
    if (isOpen) {
      // Update button position when dropdown opens
      if (buttonRef.current) {
        setButtonRect(buttonRef.current.getBoundingClientRect())
      }
      document.addEventListener('mousedown', handleClickOutside)
      return () => document.removeEventListener('mousedown', handleClickOutside)
    }
  }, [isOpen])

  // Refresh data when dropdown opens (in case it's stale)
  useEffect(() => {
    if (isOpen && repo) {
      loadBranchesAndTags()
    }
  }, [isOpen])

  async function loadBranchesAndTags() {
    setLoading(true)
    try {
      const [branchesData, tagsData] = await Promise.all([
        branchesApi.list(repo),
        tagsApi.list(repo)
      ])
      setBranches(branchesData)
      setTags(tagsData)
    } catch (error) {
      console.error('Failed to load branches/tags:', error)
    } finally {
      setLoading(false)
    }
  }

  function handleBranchSelect(branchName: string) {
    if (customNavigate) {
      // Use custom navigation handler if provided
      customNavigate(branchName, false)
    } else {
      // Default: Navigate to content view while preserving workspace
      navigate(`/${repo}/content/${branchName}/${workspace}`)
    }
    setIsOpen(false)
    setSearchTerm('')
  }

  function handleTagSelect(tagName: string) {
    if (customNavigate) {
      // Use custom navigation handler if provided
      customNavigate(tagName, true)
    } else {
      // Default: Navigate to content view (read-only tag)
      navigate(`/${repo}/content/${tagName}/${workspace}`)
    }
    setIsOpen(false)
    setSearchTerm('')
  }

  // Check if current branch is actually a tag
  const isTag = tags.some(t => t.name === currentBranch)

  // Filter branches/tags by search term
  const filteredBranches = branches.filter(b =>
    b.name.toLowerCase().includes(searchTerm.toLowerCase())
  )
  const filteredTags = tags.filter(t =>
    t.name.toLowerCase().includes(searchTerm.toLowerCase())
  )

  return (
    <div className={`relative ${className}`}>
      {/* Trigger Button */}
      <button
        ref={buttonRef}
        onClick={() => setIsOpen(!isOpen)}
        className={`
          flex items-center gap-2 px-3 py-1.5 
          bg-black/30 hover:bg-black/40 
          border border-white/20 hover:border-white/30 
          rounded-lg text-white transition-colors
          ${compact ? 'text-sm' : ''}
        `}
      >
        {isTag ? (
          <>
            <Tag className="w-4 h-4 text-amber-400" />
            <span className="font-medium">{currentBranch}</span>
            <Lock className="w-3 h-3 text-amber-400" />
          </>
        ) : (
          <>
            <GitBranch className="w-4 h-4 text-primary-400" />
            <span className="font-medium">{currentBranch}</span>
          </>
        )}
        <ChevronDown className={`w-4 h-4 text-gray-400 transition-transform ${isOpen ? 'rotate-180' : ''}`} />
      </button>

      {/* Dropdown Menu */}
      {isOpen && buttonRect && createPortal(
        <div 
          ref={dropdownRef}
          className="fixed w-80 bg-zinc-900 border border-white/20 rounded-lg shadow-2xl overflow-hidden"
          style={{
            top: `${buttonRect.bottom + 8}px`,
            left: `${buttonRect.left}px`,
            zIndex: 9999
          }}
        >
          {/* Tabs */}
          <div className="flex border-b border-white/10">
            <button
              onClick={() => setActiveTab('branches')}
              className={`
                flex-1 flex items-center justify-center gap-2 px-4 py-3 text-sm font-medium transition-colors
                ${activeTab === 'branches' 
                  ? 'text-primary-400 bg-primary-500/10 border-b-2 border-primary-400' 
                  : 'text-gray-400 hover:text-white hover:bg-white/5'
                }
              `}
            >
              <GitBranch className="w-4 h-4" />
              Branches ({branches.length})
            </button>
            <button
              onClick={() => setActiveTab('tags')}
              className={`
                flex-1 flex items-center justify-center gap-2 px-4 py-3 text-sm font-medium transition-colors
                ${activeTab === 'tags' 
                  ? 'text-amber-400 bg-amber-500/10 border-b-2 border-amber-400' 
                  : 'text-gray-400 hover:text-white hover:bg-white/5'
                }
              `}
            >
              <Tag className="w-4 h-4" />
              Tags ({tags.length})
            </button>
          </div>

          {/* Search */}
          <div className="p-3 border-b border-white/10">
            <div className="relative">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400" />
              <input
                type="text"
                placeholder={`Search ${activeTab}...`}
                value={searchTerm}
                onChange={(e) => setSearchTerm(e.target.value)}
                className="w-full pl-10 pr-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
                autoFocus
              />
            </div>
          </div>

          {/* List */}
          <div className="max-h-80 overflow-y-auto">
            {loading ? (
              <div className="p-8 text-center text-gray-400">
                <div className="animate-spin w-6 h-6 border-2 border-primary-400 border-t-transparent rounded-full mx-auto mb-2" />
                Loading...
              </div>
            ) : activeTab === 'branches' ? (
              filteredBranches.length > 0 ? (
                <div className="py-2">
                  {filteredBranches.map((branch) => (
                    <button
                      key={branch.name}
                      onClick={() => handleBranchSelect(branch.name)}
                      className="w-full flex items-center gap-3 px-4 py-2 hover:bg-white/5 transition-colors text-left"
                    >
                      <GitBranch className="w-4 h-4 text-primary-400 flex-shrink-0" />
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-white text-sm font-medium truncate">
                            {branch.name}
                          </span>
                          {branch.name === currentBranch && !isTag && (
                            <Check className="w-4 h-4 text-green-400 flex-shrink-0" />
                          )}
                        </div>
                      </div>
                      <div className="text-xs text-gray-500">
                        r{branch.head}
                      </div>
                    </button>
                  ))}
                </div>
              ) : (
                <div className="p-8 text-center text-gray-400">
                  {searchTerm ? 'No branches match your search' : 'No branches found'}
                </div>
              )
            ) : (
              filteredTags.length > 0 ? (
                <div className="py-2">
                  {filteredTags.map((tag) => (
                    <button
                      key={tag.name}
                      onClick={() => handleTagSelect(tag.name)}
                      className="w-full flex items-center gap-3 px-4 py-2 hover:bg-white/5 transition-colors text-left"
                    >
                      <Tag className="w-4 h-4 text-amber-400 flex-shrink-0" />
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-white text-sm font-medium truncate">
                            {tag.name}
                          </span>
                          {tag.name === currentBranch && isTag && (
                            <Check className="w-4 h-4 text-green-400 flex-shrink-0" />
                          )}
                        </div>
                        {tag.message && (
                          <p className="text-xs text-gray-400 truncate">{tag.message}</p>
                        )}
                      </div>
                      <div className="flex items-center gap-2">
                        <Lock className="w-3 h-3 text-amber-400" />
                        <span className="text-xs text-gray-500">r{tag.revision}</span>
                      </div>
                    </button>
                  ))}
                </div>
              ) : (
                <div className="p-8 text-center text-gray-400">
                  {searchTerm ? 'No tags match your search' : 'No tags found'}
                </div>
              )
            )}
          </div>

          {/* Footer hint */}
          {isTag && (
            <div className="px-4 py-2 bg-amber-500/10 border-t border-amber-500/20 text-xs text-amber-300 flex items-center gap-2">
              <Lock className="w-3 h-3" />
              You are viewing a read-only tag. Switch to a branch to make changes.
            </div>
          )}
        </div>,
        document.body
      )}
    </div>
  )
}
