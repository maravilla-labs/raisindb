import { useState } from 'react'
import { ChevronLeft, ChevronRight, AlertCircle, Check } from 'lucide-react'
import { MergeConflict, ConflictType } from '../api/branches'

type ResolutionType = 'keep-ours' | 'keep-theirs' | 'manual'

interface Resolution {
  type: ResolutionType
  properties: any  // The resolved properties (for manual edits)
  translationLocale?: string  // For translation conflicts
}

/** Create a unique key for a conflict (node_id + locale for translation conflicts) */
function getConflictKey(conflict: MergeConflict): string {
  return conflict.translation_locale
    ? `${conflict.node_id}::${conflict.translation_locale}`
    : conflict.node_id
}

interface ConflictResolutionPanelProps {
  conflicts: MergeConflict[]
  targetBranch: string  // Branch being merged INTO (OURS)
  sourceBranch: string  // Branch being merged FROM (THEIRS)
  onResolveAll: (resolutions: Map<string, Resolution>) => void
  onCancel: () => void
}

export default function ConflictResolutionPanel({
  conflicts,
  targetBranch,
  sourceBranch,
  onResolveAll,
  onCancel,
}: ConflictResolutionPanelProps) {
  const [currentIndex, setCurrentIndex] = useState(0)
  const [resolutions, setResolutions] = useState<Map<string, Resolution>>(new Map())
  const [manualEdit, setManualEdit] = useState<string>('')
  const [showManualEditor, setShowManualEditor] = useState(false)
  const [manualEditError, setManualEditError] = useState<string | null>(null)

  const currentConflict = conflicts[currentIndex]
  const conflictKey = getConflictKey(currentConflict)
  const currentResolution = resolutions.get(conflictKey)
  const resolvedCount = resolutions.size
  const allResolved = resolvedCount === conflicts.length

  // Initialize manual editor with current resolution or theirs as default
  const initManualEditor = () => {
    const initialValue = currentResolution?.properties
      || currentConflict.source_properties
      || currentConflict.target_properties
      || {}

    setManualEdit(JSON.stringify(initialValue, null, 2))
    setManualEditError(null)
    setShowManualEditor(true)
  }

  const resolveWith = (type: ResolutionType, properties: any) => {
    const newResolutions = new Map(resolutions)
    newResolutions.set(conflictKey, {
      type,
      properties,
      translationLocale: currentConflict.translation_locale
    })
    setResolutions(newResolutions)
    setShowManualEditor(false)

    // Auto-advance to next unresolved conflict
    if (currentIndex < conflicts.length - 1) {
      setCurrentIndex(currentIndex + 1)
    }
  }

  const handleKeepOurs = () => {
    resolveWith('keep-ours', currentConflict.target_properties)
  }

  const handleKeepTheirs = () => {
    resolveWith('keep-theirs', currentConflict.source_properties)
  }

  const handleAcceptDeletion = () => {
    // Resolve by accepting the deletion (null properties = deletion)
    resolveWith('manual', null)
  }

  const handleManualSave = () => {
    try {
      const parsed = JSON.parse(manualEdit)
      setManualEditError(null)
      resolveWith('manual', parsed)
    } catch (e: any) {
      setManualEditError(`Invalid JSON: ${e.message}`)
    }
  }

  const getConflictTypeLabel = (type: ConflictType): string => {
    switch (type) {
      case 'BothModified':
        return 'Both branches modified this node'
      case 'DeletedBySourceModifiedByTarget':
        return 'Source branch deleted, target branch modified'
      case 'ModifiedBySourceDeletedByTarget':
        return 'Source branch modified, target branch deleted'
      case 'BothAdded':
        return 'Both branches added this node'
      default:
        return 'Conflict'
    }
  }

  const formatProperties = (props: any): string => {
    if (!props) return 'null (deleted)'
    return JSON.stringify(props, null, 2)
  }

  return (
    <div className="flex flex-col h-full max-h-[80vh]">
      {/* Fixed Header */}
      <div className="flex-shrink-0 p-4 border-b border-white/10">
        <div className="flex items-center justify-between mb-2">
          <h3 className="text-lg font-semibold text-white flex items-center gap-2">
            <AlertCircle className="w-5 h-5 text-yellow-400" />
            Resolve Merge Conflicts
          </h3>
          <span className="text-sm text-gray-400">
            {resolvedCount} / {conflicts.length} resolved
          </span>
        </div>

        {/* Progress bar */}
        <div className="w-full bg-black/30 rounded-full h-2 overflow-hidden">
          <div
            className="h-full bg-primary-500 transition-all duration-300"
            style={{ width: `${(resolvedCount / conflicts.length) * 100}%` }}
          />
        </div>
      </div>

      {/* Fixed Conflict Info */}
      <div className="flex-shrink-0 px-4 pt-4">
        <div className="bg-yellow-500/10 border border-yellow-500/20 rounded-lg p-3">
          <div className="flex items-start gap-3">
            <AlertCircle className="w-5 h-5 text-yellow-400 flex-shrink-0 mt-0.5" />
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium text-yellow-300">
                Conflict {currentIndex + 1} of {conflicts.length}
              </p>
              <p className="text-sm text-yellow-400 mt-1">
                {getConflictTypeLabel(currentConflict.conflict_type)}
                {currentConflict.translation_locale && (
                  <span className="ml-2 px-2 py-0.5 bg-purple-500/30 text-purple-300 rounded text-xs font-medium">
                    Locale: {currentConflict.translation_locale}
                  </span>
                )}
              </p>
              {/* Deletion indicators */}
              {(!currentConflict.target_properties || !currentConflict.source_properties) && (
                <div className="flex gap-2 mt-1">
                  {!currentConflict.target_properties && (
                    <span className="px-2 py-0.5 bg-red-500/30 text-red-300 rounded text-xs font-medium">
                      OURS: Deleted
                    </span>
                  )}
                  {!currentConflict.source_properties && (
                    <span className="px-2 py-0.5 bg-red-500/30 text-red-300 rounded text-xs font-medium">
                      THEIRS: Deleted
                    </span>
                  )}
                </div>
              )}
              <p className="text-xs text-gray-400 mt-1 font-mono truncate">
                Path: {currentConflict.path || currentConflict.node_id}
              </p>
            </div>
            {currentResolution && (
              <div className="flex-shrink-0 flex items-center gap-2 bg-green-500/20 px-3 py-1 rounded-md">
                <Check className="w-4 h-4 text-green-400" />
                <span className="text-xs text-green-300">Resolved</span>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Scrollable 3-Way Diff Display */}
      <div className="flex-1 min-h-0 overflow-y-auto px-4 py-4">
        {!showManualEditor ? (
          <div className="grid grid-cols-1 lg:grid-cols-3 gap-4 h-full">
            {/* Base (Common Ancestor) */}
            <div className="bg-black/30 border border-white/10 rounded-lg p-4 flex flex-col min-h-[200px] max-h-full">
              <h4 className="flex-shrink-0 text-sm font-medium text-gray-300 mb-2 flex items-center gap-2">
                <div className="w-2 h-2 rounded-full bg-gray-500" />
                BASE (Common Ancestor)
              </h4>
              <pre className="flex-1 text-xs text-gray-400 overflow-auto whitespace-pre-wrap break-words">
                {formatProperties(currentConflict.base_properties)}
              </pre>
            </div>

            {/* Ours (Target Branch) */}
            <div className="bg-blue-500/10 border border-blue-500/30 rounded-lg p-4 flex flex-col min-h-[200px] max-h-full">
              <h4 className="flex-shrink-0 text-sm font-medium text-blue-300 mb-2 flex items-center gap-2">
                <div className="w-2 h-2 rounded-full bg-blue-500" />
                OURS ({targetBranch})
              </h4>
              <pre className="flex-1 text-xs text-blue-200 overflow-auto whitespace-pre-wrap break-words">
                {formatProperties(currentConflict.target_properties)}
              </pre>
            </div>

            {/* Theirs (Source Branch) */}
            <div className="bg-green-500/10 border border-green-500/30 rounded-lg p-4 flex flex-col min-h-[200px] max-h-full">
              <h4 className="flex-shrink-0 text-sm font-medium text-green-300 mb-2 flex items-center gap-2">
                <div className="w-2 h-2 rounded-full bg-green-500" />
                THEIRS ({sourceBranch})
              </h4>
              <pre className="flex-1 text-xs text-green-200 overflow-auto whitespace-pre-wrap break-words">
                {formatProperties(currentConflict.source_properties)}
              </pre>
            </div>
          </div>
        ) : (
          /* Manual Editor */
          <div className="bg-black/30 border border-white/20 rounded-lg p-4 h-full flex flex-col">
            <h4 className="flex-shrink-0 text-sm font-medium text-gray-300 mb-3">
              Manually Edit Resolved Properties
            </h4>
            <textarea
              value={manualEdit}
              onChange={(e) => setManualEdit(e.target.value)}
              className="flex-1 min-h-[200px] px-3 py-2 bg-black/40 border border-white/20 rounded-lg text-white font-mono text-xs focus:outline-none focus:ring-2 focus:ring-primary-500 resize-none"
              placeholder="Enter JSON properties..."
            />
            {manualEditError && (
              <p className="flex-shrink-0 text-sm text-red-400 mt-2">{manualEditError}</p>
            )}
          </div>
        )}
      </div>

      {/* Fixed Resolution Controls */}
      <div className="flex-shrink-0 px-4 pb-4">
        <div className="bg-black/20 border border-white/10 rounded-lg p-4">
          <h4 className="text-sm font-medium text-gray-300 mb-3">
            Choose Resolution
          </h4>
          <div className="flex flex-wrap gap-3">
            {/* Keep Ours - always visible, disabled if target deleted */}
            <button
              type="button"
              onClick={handleKeepOurs}
              disabled={!currentConflict.target_properties || currentResolution?.type === 'keep-ours'}
              title={!currentConflict.target_properties ? 'Target branch deleted this node' : undefined}
              className="px-4 py-2 bg-blue-500/20 border border-blue-500/30 rounded-lg text-blue-300 hover:bg-blue-500/30 disabled:opacity-50 disabled:cursor-not-allowed transition-colors text-sm font-medium"
            >
              Keep Ours ({targetBranch}){!currentConflict.target_properties && ' - Deleted'}
            </button>
            {/* Keep Theirs - always visible, disabled if source deleted */}
            <button
              type="button"
              onClick={handleKeepTheirs}
              disabled={!currentConflict.source_properties || currentResolution?.type === 'keep-theirs'}
              title={!currentConflict.source_properties ? 'Source branch deleted this node' : undefined}
              className="px-4 py-2 bg-green-500/20 border border-green-500/30 rounded-lg text-green-300 hover:bg-green-500/30 disabled:opacity-50 disabled:cursor-not-allowed transition-colors text-sm font-medium"
            >
              Keep Theirs ({sourceBranch}){!currentConflict.source_properties && ' - Deleted'}
            </button>
            {/* Accept Deletion - show when either side deleted the node */}
            {(!currentConflict.target_properties || !currentConflict.source_properties) && (
              <button
                type="button"
                onClick={handleAcceptDeletion}
                disabled={currentResolution?.type === 'manual' && currentResolution.properties === null}
                className="px-4 py-2 bg-red-500/20 border border-red-500/30 rounded-lg text-red-300 hover:bg-red-500/30 disabled:opacity-50 disabled:cursor-not-allowed transition-colors text-sm font-medium"
              >
                Accept Deletion
              </button>
            )}
            {!showManualEditor ? (
              <button
                type="button"
                onClick={initManualEditor}
                className="px-4 py-2 bg-purple-500/20 border border-purple-500/30 rounded-lg text-purple-300 hover:bg-purple-500/30 transition-colors text-sm font-medium"
              >
                Manual Edit
              </button>
            ) : (
              <>
                <button
                  type="button"
                  onClick={handleManualSave}
                  className="px-4 py-2 bg-purple-500 border border-purple-400 rounded-lg text-white hover:bg-purple-600 transition-colors text-sm font-medium"
                >
                  Save Manual Edit
                </button>
                <button
                  type="button"
                  onClick={handleAcceptDeletion}
                  className="px-4 py-2 bg-red-500/20 border border-red-500/30 rounded-lg text-red-300 hover:bg-red-500/30 transition-colors text-sm font-medium"
                  title="Permanently delete this node in the merged result"
                >
                  Delete This Node
                </button>
                <button
                  type="button"
                  onClick={() => setShowManualEditor(false)}
                  className="px-4 py-2 bg-gray-600 border border-gray-500 rounded-lg text-white hover:bg-gray-700 transition-colors text-sm font-medium"
                >
                  Cancel Edit
                </button>
              </>
            )}
          </div>
        </div>
      </div>

      {/* Fixed Footer */}
      <div className="flex-shrink-0 border-t border-white/10 p-4 flex items-center justify-between">
        {/* Navigation */}
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setCurrentIndex(Math.max(0, currentIndex - 1))}
            disabled={currentIndex === 0}
            className="p-2 bg-black/30 border border-white/20 rounded-lg text-gray-300 hover:bg-black/40 disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
            aria-label="Previous conflict"
          >
            <ChevronLeft className="w-4 h-4" />
          </button>
          <span className="text-sm text-gray-400 min-w-[80px] text-center">
            {currentIndex + 1} / {conflicts.length}
          </span>
          <button
            type="button"
            onClick={() => setCurrentIndex(Math.min(conflicts.length - 1, currentIndex + 1))}
            disabled={currentIndex === conflicts.length - 1}
            className="p-2 bg-black/30 border border-white/20 rounded-lg text-gray-300 hover:bg-black/40 disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
            aria-label="Next conflict"
          >
            <ChevronRight className="w-4 h-4" />
          </button>
        </div>

        {/* Action Buttons */}
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={onCancel}
            className="px-4 py-2 text-gray-300 hover:text-white transition-colors text-sm"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => onResolveAll(resolutions)}
            disabled={!allResolved}
            className="px-6 py-2 bg-primary-500 hover:bg-primary-600 disabled:bg-gray-600 disabled:cursor-not-allowed text-white rounded-lg transition-colors text-sm font-medium flex items-center gap-2"
          >
            <Check className="w-4 h-4" />
            {allResolved ? 'Complete Merge' : `Resolve All (${resolvedCount}/${conflicts.length})`}
          </button>
        </div>
      </div>
    </div>
  )
}
