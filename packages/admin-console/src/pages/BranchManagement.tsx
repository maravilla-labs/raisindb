import { useState, useEffect } from 'react'
import { useParams } from 'react-router-dom'
import { GitBranch, Tag, Plus, Trash2, Lock, Shield, Calendar, User, Hash, ArrowUp, ArrowDown, Check, Clock, MessageSquare, GitMerge } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import Tabs, { type Tab } from '../components/Tabs'
import CreateBranchDialog from '../components/CreateBranchDialog'
import CreateTagDialog from '../components/CreateTagDialog'
import ConfirmDialog from '../components/ConfirmDialog'
import { useToast, ToastContainer } from '../components/Toast'
import { branchesApi, tagsApi, type Branch, type Tag as TagType, type BranchDivergence } from '../api/branches'
import { revisionsApi, type Revision } from '../api/revisions'

const tabs: Tab[] = [
  { id: 'branches', label: 'Branches', icon: GitBranch },
  { id: 'tags', label: 'Tags', icon: Tag },
]

export default function BranchManagement() {
  const { repo } = useParams<{ repo: string }>()
  const [activeTab, setActiveTab] = useState('branches')
  const [branches, setBranches] = useState<Branch[]>([])
  const [tags, setTags] = useState<TagType[]>([])
  const [loading, setLoading] = useState(true)
  const [showCreateBranch, setShowCreateBranch] = useState(false)
  const [showCreateTag, setShowCreateTag] = useState(false)
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false)
  const [itemToDelete, setItemToDelete] = useState<{ type: 'branch' | 'tag'; name: string } | null>(null)
  const { toasts, error: showError, closeToast } = useToast()

  // Divergence and last activity data for branches
  const [divergenceMap, setDivergenceMap] = useState<Record<string, BranchDivergence>>({})
  const [lastActivityMap, setLastActivityMap] = useState<Record<string, Revision>>({})
  const [loadingExtras, setLoadingExtras] = useState(false)

  useEffect(() => {
    if (repo) {
      loadData()
    }
  }, [repo])

  async function loadData() {
    setLoading(true)
    try {
      const [branchesData, tagsData] = await Promise.all([
        branchesApi.list(repo!),
        tagsApi.list(repo!)
      ])
      setBranches(branchesData)
      setTags(tagsData)

      // Load divergence and last activity for branches
      if (branchesData.length > 0) {
        loadBranchExtras(branchesData)
      }
    } catch (error) {
      console.error('Failed to load branches/tags:', error)
    } finally {
      setLoading(false)
    }
  }

  async function loadBranchExtras(branchList: Branch[]) {
    setLoadingExtras(true)
    const mainBranch = 'main'

    // Fetch divergence and last activity for all non-main branches in parallel
    // Each branch uses its upstream_branch for comparison, or falls back to main
    const divergencePromises = branchList
      .filter(b => b.name !== mainBranch)
      .map(async (branch) => {
        try {
          const baseBranch = branch.upstream_branch || mainBranch
          const divergence = await branchesApi.compare(repo!, branch.name, baseBranch)
          return { name: branch.name, divergence }
        } catch (error) {
          console.warn(`Failed to fetch divergence for ${branch.name}:`, error)
          return null
        }
      })

    const activityPromises = branchList.map(async (branch) => {
      try {
        const revisions = await revisionsApi.list(repo!, 1, 0, false, branch.name)
        if (revisions.length > 0) {
          return { name: branch.name, revision: revisions[0] }
        }
        return null
      } catch (error) {
        console.warn(`Failed to fetch last activity for ${branch.name}:`, error)
        return null
      }
    })

    const [divergenceResults, activityResults] = await Promise.all([
      Promise.all(divergencePromises),
      Promise.all(activityPromises)
    ])

    // Build maps from results
    const newDivergenceMap: Record<string, BranchDivergence> = {}
    for (const result of divergenceResults) {
      if (result) {
        newDivergenceMap[result.name] = result.divergence
      }
    }
    setDivergenceMap(newDivergenceMap)

    const newActivityMap: Record<string, Revision> = {}
    for (const result of activityResults) {
      if (result) {
        newActivityMap[result.name] = result.revision
      }
    }
    setLastActivityMap(newActivityMap)
    setLoadingExtras(false)
  }

  function handleDeleteClick(type: 'branch' | 'tag', name: string) {
    setItemToDelete({ type, name })
    setShowDeleteConfirm(true)
  }

  async function handleDelete() {
    if (!itemToDelete) return

    try {
      if (itemToDelete.type === 'branch') {
        await branchesApi.delete(repo!, itemToDelete.name)
      } else {
        await tagsApi.delete(repo!, itemToDelete.name)
      }
      await loadData()
      setShowDeleteConfirm(false)
      setItemToDelete(null)
    } catch (error) {
      console.error(`Failed to delete ${itemToDelete.type}:`, error)
      showError('Delete Failed', `Failed to delete ${itemToDelete.type}`)
    }
  }

  function formatDate(dateString: string): string {
    return new Date(dateString).toLocaleDateString('en-US', {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit'
    })
  }

  function formatTimeAgo(dateString: string): string {
    const date = new Date(dateString)
    const now = new Date()
    const diffMs = now.getTime() - date.getTime()
    const diffSeconds = Math.floor(diffMs / 1000)
    const diffMinutes = Math.floor(diffSeconds / 60)
    const diffHours = Math.floor(diffMinutes / 60)
    const diffDays = Math.floor(diffHours / 24)
    const diffWeeks = Math.floor(diffDays / 7)
    const diffMonths = Math.floor(diffDays / 30)

    if (diffSeconds < 60) return 'just now'
    if (diffMinutes < 60) return `${diffMinutes}m ago`
    if (diffHours < 24) return `${diffHours}h ago`
    if (diffDays < 7) return `${diffDays}d ago`
    if (diffWeeks < 4) return `${diffWeeks}w ago`
    return `${diffMonths}mo ago`
  }

  return (
    <div className="animate-fade-in">
      <div className="mb-8">
        <h1 className="text-4xl font-bold text-white mb-2">Branch & Tag Management</h1>
        <p className="text-zinc-400">Manage branches and tags for repository: <span className="text-primary-400">{repo}</span></p>
      </div>

      <GlassCard>
        <Tabs tabs={tabs} activeTab={activeTab} onChange={setActiveTab}>
          {/* Branches Tab */}
          {activeTab === 'branches' && (
            <div>
              <div className="flex justify-between items-center mb-6">
                <div>
                  <h3 className="text-lg font-semibold text-white">Branches</h3>
                  <p className="text-sm text-gray-400">Mutable pointers to revisions for active development</p>
                </div>
                <button
                  onClick={() => setShowCreateBranch(true)}
                  className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
                >
                  <Plus className="w-4 h-4" />
                  Create Branch
                </button>
              </div>

              {loading ? (
                <div className="text-center text-gray-400 py-12">
                  <div className="w-8 h-8 border-2 border-primary-400 border-t-transparent rounded-full animate-spin mx-auto mb-3" />
                  Loading branches...
                </div>
              ) : branches.length === 0 ? (
                <div className="text-center text-gray-400 py-12">
                  <GitBranch className="w-12 h-12 text-gray-600 mx-auto mb-3" />
                  <p>No branches yet</p>
                  <button
                    onClick={() => setShowCreateBranch(true)}
                    className="mt-4 text-primary-400 hover:text-primary-300"
                  >
                    Create your first branch
                  </button>
                </div>
              ) : (
                <div className="space-y-3">
                  {branches.map((branch) => {
                    const divergence = divergenceMap[branch.name]
                    const lastActivity = lastActivityMap[branch.name]
                    const isInSync = divergence && divergence.ahead === 0 && divergence.behind === 0

                    return (
                      <div
                        key={branch.name}
                        className="p-4 bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg transition-colors"
                      >
                        <div className="flex items-start justify-between">
                          <div className="flex-1">
                            <div className="flex items-center gap-3 mb-2 flex-wrap">
                              <GitBranch className="w-5 h-5 text-primary-400 flex-shrink-0" />
                              <h4 className="text-lg font-semibold text-white">{branch.name}</h4>
                              {branch.protected && (
                                <div className="flex items-center gap-1 px-2 py-0.5 bg-amber-500/20 border border-amber-500/30 rounded text-xs text-amber-300">
                                  <Shield className="w-3 h-3" />
                                  Protected
                                </div>
                              )}
                              {branch.name === 'main' && (
                                <div className="px-2 py-0.5 bg-primary-500/20 border border-primary-500/30 rounded text-xs text-primary-300">
                                  Default
                                </div>
                              )}
                              {/* Upstream branch indicator */}
                              {branch.name !== 'main' && branch.upstream_branch && branch.upstream_branch !== 'main' && (
                                <div className="flex items-center gap-1 px-2 py-0.5 bg-blue-500/20 border border-blue-500/30 rounded text-xs text-blue-300">
                                  <GitMerge className="w-3 h-3" />
                                  ↱ {branch.upstream_branch}
                                </div>
                              )}
                              {/* Divergence status */}
                              {branch.name !== 'main' && (
                                loadingExtras ? (
                                  <div className="w-16 h-5 bg-white/5 rounded animate-pulse" />
                                ) : divergence ? (
                                  <div className="flex items-center gap-2">
                                    {isInSync ? (
                                      <div className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 border border-green-500/30 rounded text-xs text-green-300">
                                        <Check className="w-3 h-3" />
                                        In sync
                                      </div>
                                    ) : (
                                      <>
                                        {divergence.ahead > 0 && (
                                          <div className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 border border-green-500/30 rounded text-xs text-green-300">
                                            <ArrowUp className="w-3 h-3" />
                                            {divergence.ahead} ahead
                                          </div>
                                        )}
                                        {divergence.behind > 0 && (
                                          <div className="flex items-center gap-1 px-2 py-0.5 bg-amber-500/20 border border-amber-500/30 rounded text-xs text-amber-300">
                                            <ArrowDown className="w-3 h-3" />
                                            {divergence.behind} behind
                                          </div>
                                        )}
                                      </>
                                    )}
                                  </div>
                                ) : null
                              )}
                            </div>

                            {/* Last activity display */}
                            {lastActivity && (
                              <div className="flex items-center gap-4 mb-2 ml-8 text-sm">
                                <div className="flex items-center gap-1.5 text-gray-400">
                                  <Clock className="w-3.5 h-3.5" />
                                  <span>Last commit {formatTimeAgo(lastActivity.timestamp)}</span>
                                </div>
                                {lastActivity.message && (
                                  <div className="flex items-center gap-1.5 text-gray-500 truncate max-w-[300px]">
                                    <MessageSquare className="w-3.5 h-3.5 flex-shrink-0" />
                                    <span className="truncate">{lastActivity.message}</span>
                                  </div>
                                )}
                              </div>
                            )}

                            <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-sm">
                              <div className="flex items-center gap-2 text-gray-400">
                                <Hash className="w-4 h-4" />
                                <span>HEAD: r{branch.head}</span>
                              </div>
                              <div className="flex items-center gap-2 text-gray-400">
                                <Calendar className="w-4 h-4" />
                                <span>Created {formatDate(branch.created_at)}</span>
                              </div>
                              <div className="flex items-center gap-2 text-gray-400">
                                <User className="w-4 h-4" />
                                <span>{branch.created_by}</span>
                              </div>
                            </div>
                          </div>

                          <div className="flex items-center gap-2 ml-4">
                            {!branch.protected && branch.name !== 'main' && (
                              <button
                                onClick={() => handleDeleteClick('branch', branch.name)}
                                className="p-2 text-gray-400 hover:text-red-400 hover:bg-red-500/10 rounded-lg transition-colors"
                                title="Delete branch"
                              >
                                <Trash2 className="w-4 h-4" />
                              </button>
                            )}
                            {branch.protected && (
                              <div className="p-2 text-gray-600" title="Cannot delete protected branch">
                                <Lock className="w-4 h-4" />
                              </div>
                            )}
                          </div>
                        </div>
                      </div>
                    )
                  })}
                </div>
              )}
            </div>
          )}

          {/* Tags Tab */}
          {activeTab === 'tags' && (
            <div>
              <div className="flex justify-between items-center mb-6">
                <div>
                  <h3 className="text-lg font-semibold text-white">Tags</h3>
                  <p className="text-sm text-gray-400">Immutable snapshots at specific revisions (releases, milestones)</p>
                </div>
                <button
                  onClick={() => setShowCreateTag(true)}
                  className="flex items-center gap-2 px-4 py-2 bg-amber-500 hover:bg-amber-600 text-white rounded-lg transition-colors"
                >
                  <Plus className="w-4 h-4" />
                  Create Tag
                </button>
              </div>

              {loading ? (
                <div className="text-center text-gray-400 py-12">
                  <div className="w-8 h-8 border-2 border-amber-400 border-t-transparent rounded-full animate-spin mx-auto mb-3" />
                  Loading tags...
                </div>
              ) : tags.length === 0 ? (
                <div className="text-center text-gray-400 py-12">
                  <Tag className="w-12 h-12 text-gray-600 mx-auto mb-3" />
                  <p>No tags yet</p>
                  <button
                    onClick={() => setShowCreateTag(true)}
                    className="mt-4 text-amber-400 hover:text-amber-300"
                  >
                    Create your first tag
                  </button>
                </div>
              ) : (
                <div className="space-y-3">
                  {tags.map((tag) => (
                    <div
                      key={tag.name}
                      className="p-4 bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg transition-colors"
                    >
                      <div className="flex items-start justify-between">
                        <div className="flex-1">
                          <div className="flex items-center gap-3 mb-2">
                            <Tag className="w-5 h-5 text-amber-400 flex-shrink-0" />
                            <h4 className="text-lg font-semibold text-white">{tag.name}</h4>
                            <div className="flex items-center gap-1 px-2 py-0.5 bg-amber-500/20 border border-amber-500/30 rounded text-xs text-amber-300">
                              <Lock className="w-3 h-3" />
                              Read-only
                            </div>
                            {tag.protected && (
                              <div className="flex items-center gap-1 px-2 py-0.5 bg-red-500/20 border border-red-500/30 rounded text-xs text-red-300">
                                <Shield className="w-3 h-3" />
                                Protected
                              </div>
                            )}
                          </div>
                          
                          {tag.message && (
                            <p className="text-sm text-gray-300 mb-3 ml-8">{tag.message}</p>
                          )}
                          
                          <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-sm">
                            <div className="flex items-center gap-2 text-gray-400">
                              <Hash className="w-4 h-4" />
                              <span>Revision: r{tag.revision}</span>
                            </div>
                            <div className="flex items-center gap-2 text-gray-400">
                              <Calendar className="w-4 h-4" />
                              <span>{formatDate(tag.created_at)}</span>
                            </div>
                            <div className="flex items-center gap-2 text-gray-400">
                              <User className="w-4 h-4" />
                              <span>{tag.created_by}</span>
                            </div>
                          </div>
                        </div>

                        <div className="flex items-center gap-2 ml-4">
                          {!tag.protected && (
                            <button
                              onClick={() => handleDeleteClick('tag', tag.name)}
                              className="p-2 text-gray-400 hover:text-red-400 hover:bg-red-500/10 rounded-lg transition-colors"
                              title="Delete tag"
                            >
                              <Trash2 className="w-4 h-4" />
                            </button>
                          )}
                          {tag.protected && (
                            <div className="p-2 text-gray-600" title="Cannot delete protected tag">
                              <Lock className="w-4 h-4" />
                            </div>
                          )}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </Tabs>
      </GlassCard>

      {/* Create Branch Dialog */}
      {showCreateBranch && (
        <CreateBranchDialog
          repoId={repo!}
          onClose={() => setShowCreateBranch(false)}
          onSuccess={loadData}
        />
      )}

      {/* Create Tag Dialog */}
      {showCreateTag && (
        <CreateTagDialog
          repoId={repo!}
          onClose={() => setShowCreateTag(false)}
          onSuccess={loadData}
        />
      )}

      {/* Delete Confirmation */}
      {showDeleteConfirm && itemToDelete && (
        <ConfirmDialog
          open={true}
          title={`Delete ${itemToDelete.type === 'branch' ? 'Branch' : 'Tag'}`}
          message={`Are you sure you want to delete ${itemToDelete.type} "${itemToDelete.name}"? This action cannot be undone.`}
          confirmText="Delete"
          variant="danger"
          onConfirm={handleDelete}
          onCancel={() => {
            setShowDeleteConfirm(false)
            setItemToDelete(null)
          }}
        />
      )}

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
