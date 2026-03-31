import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { X, GitMerge, GitBranch, ChevronDown, AlertCircle, CheckCircle, Info, ArrowRight, Trash2 } from 'lucide-react'
import { branchesApi, Branch, MergeStrategy, MergeResult, BranchDivergence } from '../api/branches'
import ConflictResolutionPanel from './ConflictResolutionPanel'

type MergeDirection = 'into-current' | 'from-current'

interface MergeBranchDialogProps {
  open: boolean
  onClose: () => void
  currentBranch: string // The current branch in the UI
  mainBranch: string // The default branch name
  repoId: string
  onMergeComplete: () => void // Callback when merge succeeds
}

export default function MergeBranchDialog({
  open,
  onClose,
  currentBranch,
  mainBranch,
  repoId,
  onMergeComplete,
}: MergeBranchDialogProps) {
  // State management
  const [mergeDirection, setMergeDirection] = useState<MergeDirection>('into-current')
  const [branches, setBranches] = useState<Branch[]>([])
  const [selectedBranch, setSelectedBranch] = useState<string | null>(null)
  const [divergence, setDivergence] = useState<BranchDivergence | null>(null)
  const [strategy, setStrategy] = useState<MergeStrategy>('ThreeWay')
  const [message, setMessage] = useState('')
  const [loading, setLoading] = useState(false)
  const [merging, setMerging] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [mergeResult, setMergeResult] = useState<MergeResult | null>(null)
  const [showDropdown, setShowDropdown] = useState(false)
  const [showConflictResolution, setShowConflictResolution] = useState(false)
  const [deleteAfterMerge, setDeleteAfterMerge] = useState(false)
  const [deletingBranch, setDeletingBranch] = useState(false)

  // Refs for accessibility
  const dialogRef = useRef<HTMLDivElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)

  // Calculate actual source and target based on direction
  const sourceBranch = mergeDirection === 'into-current' ? selectedBranch : currentBranch
  const targetBranch = mergeDirection === 'into-current' ? currentBranch : selectedBranch

  // Load branches on mount
  useEffect(() => {
    if (open) {
      loadBranches()
      // Set default merge direction based on whether we're on main
      setMergeDirection(currentBranch === mainBranch ? 'from-current' : 'into-current')
      // Reset state
      setSelectedBranch(null)
      setDivergence(null)
      setError(null)
      setMergeResult(null)
      setDeleteAfterMerge(false)
      setDeletingBranch(false)
    }
  }, [open, currentBranch, mainBranch])

  // Fetch divergence when source branch is selected or direction changes
  useEffect(() => {
    if (selectedBranch) {
      fetchDivergence()
      // Update commit message based on direction
      if (mergeDirection === 'into-current') {
        setMessage(`Merge ${selectedBranch} into ${currentBranch}`)
      } else {
        setMessage(`Merge ${currentBranch} into ${selectedBranch}`)
      }
    } else {
      setDivergence(null)
      setMessage('')
    }
  }, [selectedBranch, currentBranch, mergeDirection])

  // Keyboard handlers for accessibility
  useEffect(() => {
    if (!open) return

    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (showDropdown) {
          setShowDropdown(false)
        } else {
          handleClose()
        }
      }
    }

    const handleClickOutside = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setShowDropdown(false)
      }
    }

    document.addEventListener('keydown', handleEscape)
    document.addEventListener('mousedown', handleClickOutside)

    // Focus dialog on open
    dialogRef.current?.focus()

    return () => {
      document.removeEventListener('keydown', handleEscape)
      document.removeEventListener('mousedown', handleClickOutside)
    }
  }, [open, showDropdown])

  async function loadBranches() {
    setLoading(true)
    setError(null)
    try {
      const data = await branchesApi.list(repoId)
      // Exclude current branch from the list
      const filteredBranches = data.filter((b) => b.name !== currentBranch)
      setBranches(filteredBranches)

      // Auto-select main branch if available
      if (filteredBranches.some((b) => b.name === mainBranch)) {
        setSelectedBranch(mainBranch)
      }
    } catch (err: any) {
      setError(err.message || 'Failed to load branches')
    } finally {
      setLoading(false)
    }
  }

  async function fetchDivergence() {
    if (!selectedBranch || !sourceBranch || !targetBranch) return

    try {
      // Compare from the perspective of source against target
      const data = await branchesApi.compare(repoId, sourceBranch, targetBranch)
      setDivergence(data)
    } catch (err: any) {
      console.error('Failed to fetch divergence:', err)
      setDivergence(null)
    }
  }

  async function handleMerge() {
    if (!selectedBranch || !sourceBranch || !targetBranch) {
      setError('Please select a branch')
      return
    }

    setMerging(true)
    setError(null)
    setMergeResult(null)

    try {
      const result = await branchesApi.merge(repoId, targetBranch, {
        source_branch: sourceBranch,
        strategy,
        message: message.trim() || `Merge ${sourceBranch} into ${targetBranch}`,
        actor: 'user',
      })

      setMergeResult(result)

      if (result.success) {
        // Delete source branch if requested
        if (deleteAfterMerge && sourceBranch !== mainBranch) {
          setDeletingBranch(true)
          try {
            await branchesApi.delete(repoId, sourceBranch)
          } catch (deleteErr: any) {
            console.warn('Failed to delete source branch:', deleteErr)
            // Don't fail the merge if delete fails
          } finally {
            setDeletingBranch(false)
          }
        }

        // Success - call completion callback
        setTimeout(() => {
          onMergeComplete()
          handleClose()
        }, 2000)
      }
    } catch (err: any) {
      setError(err.message || 'Failed to merge branches')
    } finally {
      setMerging(false)
    }
  }

  function handleClose() {
    if (!merging) {
      onClose()
    }
  }

  function selectBranch(branchName: string) {
    setSelectedBranch(branchName)
    setShowDropdown(false)
  }

  async function handleResolveAll(resolutions: Map<string, { type: string; properties: any; translationLocale?: string }>) {
    if (!sourceBranch || !targetBranch) {
      setError('Source and target branches must be selected')
      return
    }

    setMerging(true)
    setError(null)

    try {
      const result = await branchesApi.resolveMerge(repoId, targetBranch, {
        source_branch: sourceBranch,
        resolutions: Array.from(resolutions.entries()).map(([key, res]) => {
          // Key format: "node_id" or "node_id::locale" for translation conflicts
          const nodeId = key.includes('::') ? key.split('::')[0] : key
          return {
            node_id: nodeId,
            resolution_type: res.type as 'keep-ours' | 'keep-theirs' | 'manual',
            resolved_properties: res.properties,
            translation_locale: res.translationLocale,
          }
        }),
        message: message.trim() || `Merge ${sourceBranch} into ${targetBranch} (resolved conflicts)`,
        actor: 'user',
      })

      setMergeResult(result)
      setShowConflictResolution(false)

      if (result.success) {
        // Success - call completion callback
        setTimeout(() => {
          onMergeComplete()
          handleClose()
        }, 2000)
      }
    } catch (err: any) {
      setError(err.message || 'Failed to resolve merge conflicts')
      setShowConflictResolution(false)
    } finally {
      setMerging(false)
    }
  }

  function handleCancelResolution() {
    setShowConflictResolution(false)
  }

  if (!open) return null

  const selectedBranchData = branches.find((b) => b.name === selectedBranch)
  const canMerge = selectedBranch && message.trim() && !merging

  return createPortal(
    <div
      className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4 overscroll-none"
      onClick={handleClose}
      role="dialog"
      aria-modal="true"
      aria-labelledby="merge-dialog-title"
    >
      <div
        ref={dialogRef}
        tabIndex={-1}
        className="bg-gradient-to-br from-zinc-900 to-black border border-white/20 rounded-xl shadow-2xl w-full max-w-2xl animate-slide-in overscroll-contain"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="border-b border-white/10">
          <div className="flex items-center justify-between p-6 pb-4">
            <div className="flex items-center gap-3">
              <div className="p-2 bg-primary-500/20 rounded-lg">
                <GitMerge className="w-5 h-5 text-primary-400" />
              </div>
              <div>
                <div className="flex items-center gap-2">
                  <h2 id="merge-dialog-title" className="text-xl font-semibold text-white">
                    Merge Branches
                  </h2>
                  <span className="px-2 py-0.5 bg-amber-500/20 border border-amber-400/30 rounded text-amber-300 text-xs font-medium">
                    EXP
                  </span>
                </div>
                <p className="text-sm text-gray-400">
                  Combine changes between branches
                </p>
              </div>
            </div>
            <button
              onClick={handleClose}
              disabled={merging}
              className="p-2 hover:bg-white/10 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              aria-label="Close dialog"
            >
              <X className="w-5 h-5 text-gray-400" />
            </button>
          </div>

          {/* Direction Toggle */}
          <div className="px-6 pb-4">
            <div className="inline-flex bg-black/30 border border-white/20 rounded-lg p-1" role="tablist">
              <button
                type="button"
                role="tab"
                aria-selected={mergeDirection === 'into-current'}
                onClick={() => setMergeDirection('into-current')}
                disabled={merging}
                className={`px-4 py-2 rounded-md text-sm font-medium transition-all disabled:opacity-50 disabled:cursor-not-allowed ${
                  mergeDirection === 'into-current'
                    ? 'bg-primary-500/20 text-primary-300 border border-primary-500/30'
                    : 'text-gray-400 hover:text-gray-200'
                }`}
              >
                <div className="flex items-center gap-2">
                  <span>Merge into</span>
                  <ArrowRight className="w-3 h-3" />
                  <span className="font-semibold">{currentBranch}</span>
                </div>
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={mergeDirection === 'from-current'}
                onClick={() => setMergeDirection('from-current')}
                disabled={merging}
                className={`px-4 py-2 rounded-md text-sm font-medium transition-all disabled:opacity-50 disabled:cursor-not-allowed ${
                  mergeDirection === 'from-current'
                    ? 'bg-primary-500/20 text-primary-300 border border-primary-500/30'
                    : 'text-gray-400 hover:text-gray-200'
                }`}
              >
                <div className="flex items-center gap-2">
                  <span className="font-semibold">{currentBranch}</span>
                  <ArrowRight className="w-3 h-3" />
                  <span>Merge into</span>
                </div>
              </button>
            </div>
          </div>
        </div>

        {/* Content */}
        {showConflictResolution && mergeResult && mergeResult.conflicts.length > 0 ? (
          <ConflictResolutionPanel
            conflicts={mergeResult.conflicts}
            targetBranch={targetBranch || ''}
            sourceBranch={sourceBranch || ''}
            onResolveAll={handleResolveAll}
            onCancel={handleCancelResolution}
          />
        ) : (
        <div className="p-6 space-y-5">
          {/* Branch Selector */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              {mergeDirection === 'into-current'
                ? 'Source Branch (merge from) *'
                : 'Target Branch (merge into) *'
              }
            </label>
            <div className="relative" ref={dropdownRef}>
              <button
                type="button"
                onClick={() => setShowDropdown(!showDropdown)}
                disabled={loading || merging}
                className="w-full px-4 py-3 bg-black/30 border border-white/20 rounded-lg text-white text-left flex items-center justify-between hover:bg-black/40 focus:outline-none focus:ring-2 focus:ring-primary-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                aria-haspopup="listbox"
                aria-expanded={showDropdown}
              >
                <div className="flex items-center gap-2">
                  <GitBranch className="w-4 h-4 text-gray-400" />
                  {selectedBranch ? (
                    <span className="text-white">{selectedBranch}</span>
                  ) : (
                    <span className="text-gray-500">
                      {mergeDirection === 'into-current'
                        ? 'Select a branch to merge from...'
                        : 'Select a branch to merge into...'
                      }
                    </span>
                  )}
                </div>
                <ChevronDown
                  className={`w-4 h-4 text-gray-400 transition-transform ${
                    showDropdown ? 'rotate-180' : ''
                  }`}
                />
              </button>

              {/* Dropdown Menu */}
              {showDropdown && (
                <div
                  className="absolute z-10 w-full mt-2 bg-zinc-900 border border-white/20 rounded-lg shadow-2xl max-h-64 overflow-y-auto"
                  role="listbox"
                >
                  {branches.length === 0 ? (
                    <div className="px-4 py-3 text-sm text-gray-500">
                      No other branches available
                    </div>
                  ) : (
                    branches.map((branch) => (
                      <button
                        key={branch.name}
                        type="button"
                        onClick={() => selectBranch(branch.name)}
                        className={`w-full px-4 py-3 text-left hover:bg-white/10 transition-colors flex items-center justify-between ${
                          selectedBranch === branch.name ? 'bg-primary-500/20' : ''
                        }`}
                        role="option"
                        aria-selected={selectedBranch === branch.name}
                      >
                        <div className="flex items-center gap-2">
                          <GitBranch className="w-4 h-4 text-gray-400" />
                          <span className="text-white font-medium">{branch.name}</span>
                        </div>
                        <span className="text-xs text-gray-500">r{branch.head}</span>
                      </button>
                    ))
                  )}
                </div>
              )}
            </div>
            {selectedBranchData && (
              <p className="mt-1 text-xs text-gray-500">
                HEAD at revision {selectedBranchData.head}
              </p>
            )}
          </div>

          {/* Divergence Info */}
          {divergence && (
            <div className="p-4 bg-blue-500/10 border border-blue-500/20 rounded-lg">
              <div className="flex items-start gap-3">
                <Info className="w-5 h-5 text-blue-400 flex-shrink-0 mt-0.5" />
                <div className="space-y-1">
                  <p className="text-sm font-medium text-blue-300">Branch Divergence</p>
                  <div className="flex items-center gap-4 text-xs text-gray-400">
                    <span>
                      <span className="text-green-400 font-semibold">{divergence.ahead}</span> commits ahead
                    </span>
                    <span>
                      <span className="text-orange-400 font-semibold">{divergence.behind}</span> commits behind
                    </span>
                    <span className="text-gray-500">
                      Common ancestor: r{divergence.common_ancestor}
                    </span>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* Merge Strategy */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Merge Strategy
            </label>
            <div className="grid grid-cols-2 gap-3">
              <button
                type="button"
                onClick={() => setStrategy('ThreeWay')}
                disabled={merging}
                className={`px-4 py-3 border rounded-lg text-left transition-all focus:outline-none focus:ring-2 focus:ring-primary-500 disabled:opacity-50 disabled:cursor-not-allowed ${
                  strategy === 'ThreeWay'
                    ? 'bg-primary-500/20 border-primary-500/50 text-white'
                    : 'bg-black/30 border-white/20 text-gray-400 hover:bg-black/40'
                }`}
              >
                <div className="font-medium text-sm">Three-Way Merge</div>
                <div className="text-xs mt-1 opacity-80">
                  Create merge commit (recommended)
                </div>
              </button>
              <button
                type="button"
                onClick={() => setStrategy('FastForward')}
                disabled={merging}
                className={`px-4 py-3 border rounded-lg text-left transition-all focus:outline-none focus:ring-2 focus:ring-primary-500 disabled:opacity-50 disabled:cursor-not-allowed ${
                  strategy === 'FastForward'
                    ? 'bg-primary-500/20 border-primary-500/50 text-white'
                    : 'bg-black/30 border-white/20 text-gray-400 hover:bg-black/40'
                }`}
              >
                <div className="font-medium text-sm">Fast-Forward</div>
                <div className="text-xs mt-1 opacity-80">
                  Move pointer (if possible)
                </div>
              </button>
            </div>
          </div>

          {/* Commit Message */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Merge Commit Message *
            </label>
            <textarea
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              placeholder="Describe this merge..."
              rows={3}
              disabled={merging}
              className="w-full px-4 py-3 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500 resize-none disabled:opacity-50 disabled:cursor-not-allowed"
              required
            />
            <p className="mt-1 text-xs text-gray-500">
              This message will be recorded in the commit history
            </p>
          </div>

          {/* Delete Source Branch After Merge */}
          {sourceBranch && sourceBranch !== mainBranch && !selectedBranchData?.protected && (
            <label className="flex items-center gap-3 p-3 bg-black/20 border border-white/10 rounded-lg cursor-pointer hover:bg-black/30 transition-colors">
              <input
                type="checkbox"
                checked={deleteAfterMerge}
                onChange={(e) => setDeleteAfterMerge(e.target.checked)}
                disabled={merging}
                className="w-4 h-4 rounded border-gray-600 bg-black/30 text-primary-500 focus:ring-primary-500 focus:ring-offset-0"
              />
              <div className="flex items-center gap-2">
                <Trash2 className="w-4 h-4 text-gray-400" />
                <span className="text-sm text-gray-300">
                  Delete <span className="font-medium text-white">{sourceBranch}</span> after merge
                </span>
              </div>
            </label>
          )}

          {/* Error Message */}
          {error && (
            <div className="flex items-start gap-3 p-4 bg-red-500/10 border border-red-500/20 rounded-lg">
              <AlertCircle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
              <div className="flex-1">
                <p className="text-sm font-medium text-red-300">Merge Failed</p>
                <p className="text-sm text-red-400 mt-1">{error}</p>
              </div>
            </div>
          )}

          {/* Merge Result - Conflicts */}
          {mergeResult && !mergeResult.success && mergeResult.conflicts.length > 0 && (
            <div className="bg-yellow-500/10 border border-yellow-500/20 rounded-lg p-4">
              <div className="flex items-start gap-3">
                <AlertCircle className="w-5 h-5 text-yellow-400 flex-shrink-0 mt-0.5" />
                <div className="flex-1">
                  <p className="text-sm font-medium text-yellow-300">Merge Conflicts Detected</p>
                  <p className="text-sm text-yellow-400 mt-1">
                    {mergeResult.conflicts.length} conflict{mergeResult.conflicts.length > 1 ? 's' : ''} found.
                    {' '}Resolve them to complete the merge.
                  </p>
                  <div className="mt-3 space-y-2">
                    {mergeResult.conflicts.slice(0, 3).map((conflict, idx) => (
                      <div key={idx} className="text-xs text-yellow-300/80 font-mono flex items-center gap-2">
                        <span>{conflict.path || conflict.node_id}</span>
                        {conflict.translation_locale && (
                          <span className="px-1.5 py-0.5 bg-purple-500/30 text-purple-300 rounded text-[10px]">
                            {conflict.translation_locale}
                          </span>
                        )}
                        <span className="text-yellow-400/60">- {conflict.conflict_type}</span>
                      </div>
                    ))}
                    {mergeResult.conflicts.length > 3 && (
                      <div className="text-xs text-yellow-300/60">
                        And {mergeResult.conflicts.length - 3} more conflicts...
                      </div>
                    )}
                  </div>
                  <button
                    type="button"
                    onClick={() => setShowConflictResolution(true)}
                    className="mt-4 px-4 py-2 bg-yellow-500 hover:bg-yellow-600 text-black font-medium rounded-lg transition-colors text-sm flex items-center gap-2"
                  >
                    <GitMerge className="w-4 h-4" />
                    Resolve Conflicts Now
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* Merge Result - Success */}
          {mergeResult && mergeResult.success && (
            <div className="flex items-start gap-3 p-4 bg-green-500/10 border border-green-500/20 rounded-lg">
              <CheckCircle className="w-5 h-5 text-green-400 flex-shrink-0 mt-0.5" />
              <div className="flex-1">
                <p className="text-sm font-medium text-green-300">Merge Successful!</p>
                <div className="text-sm text-green-400 mt-1 space-y-1">
                  <p>Revision: {mergeResult.revision}</p>
                  <p>Changes: {mergeResult.nodes_changed} node{mergeResult.nodes_changed !== 1 ? 's' : ''}</p>
                  {mergeResult.fast_forward && (
                    <p className="text-xs text-green-300/80">Fast-forward merge applied</p>
                  )}
                  {deleteAfterMerge && sourceBranch && sourceBranch !== mainBranch && (
                    <p className="text-xs text-green-300/80 flex items-center gap-1">
                      <Trash2 className="w-3 h-3" />
                      {deletingBranch ? 'Deleting branch...' : `Branch "${sourceBranch}" deleted`}
                    </p>
                  )}
                </div>
              </div>
            </div>
          )}
        </div>
        )}

        {/* Footer Actions */}
        {!showConflictResolution && (
        <div className="flex items-center justify-end gap-3 px-6 py-4 border-t border-white/10">
          <button
            type="button"
            onClick={handleClose}
            disabled={merging}
            className="px-4 py-2 text-gray-300 hover:text-white transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {mergeResult?.success ? 'Close' : 'Cancel'}
          </button>
          {!mergeResult?.success && (
            <button
              type="button"
              onClick={handleMerge}
              disabled={!canMerge}
              className="px-6 py-2 bg-primary-500 hover:bg-primary-600 disabled:bg-gray-600 disabled:cursor-not-allowed text-white rounded-lg transition-colors flex items-center gap-2"
              aria-label="Merge branches"
            >
              {merging ? (
                <>
                  <div className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  Merging...
                </>
              ) : (
                <>
                  <GitMerge className="w-4 h-4" />
                  Merge Branches
                </>
              )}
            </button>
          )}
        </div>
        )}
      </div>
    </div>,
    document.body
  )
}
