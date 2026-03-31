import { useState } from 'react'
import { Clock, GitCompare, Tag, X, FileText } from 'lucide-react'
import { useRevisions } from '../hooks/useRevisions'
import { Revision, NodeChange } from '../api/revisions'
import RevisionChangesModal from './RevisionChangesModal'

interface RevisionBrowserProps {
  repo: string
  branch: string
  workspace: string
  onSelectRevision: (revision: string) => void
  onCompareRevisions?: (from: string, to: string) => void
  onCreateTag?: (revision: string) => void
}

function formatDistanceToNow(date: Date): string {
  const seconds = Math.floor((new Date().getTime() - date.getTime()) / 1000)
  
  if (seconds < 60) return 'just now'
  if (seconds < 3600) return `${Math.floor(seconds / 60)} minutes ago`
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} hours ago`
  if (seconds < 2592000) return `${Math.floor(seconds / 86400)} days ago`
  return `${Math.floor(seconds / 2592000)} months ago`
}

export default function RevisionBrowser({
  repo,
  branch,
  workspace,
  onSelectRevision,
  onCompareRevisions,
  onCreateTag,
}: RevisionBrowserProps) {
  const { revisions, loading, error, getChanges } = useRevisions(repo, branch)
  const [selectedRevision, setSelectedRevision] = useState<string | null>(null)
  const [compareMode, setCompareMode] = useState(false)
  const [compareFrom, setCompareFrom] = useState<string | null>(null)
  const [changesModalRevision, setChangesModalRevision] = useState<string | null>(null)
  const [modalChanges, setModalChanges] = useState<NodeChange[]>([])
  const [loadingChanges, setLoadingChanges] = useState(false)

  const handleViewChanges = async (revisionNumber: string) => {
    setLoadingChanges(true)
    try {
      const changes = await getChanges(revisionNumber)
      setModalChanges(changes)
      setChangesModalRevision(revisionNumber)
    } catch (error) {
      console.error('Failed to load changes:', error)
      setModalChanges([])
    } finally {
      setLoadingChanges(false)
    }
  }
  
  const handleRevisionClick = (rev: Revision) => {
    if (compareMode) {
      if (compareFrom === null) {
        setCompareFrom(rev.number)
      } else {
        if (onCompareRevisions) {
          onCompareRevisions(compareFrom, rev.number)
        }
        setCompareMode(false)
        setCompareFrom(null)
      }
    } else {
      setSelectedRevision(rev.number)
      onSelectRevision(rev.number)
    }
  }
  
  if (loading) {
    return (
      <div className="flex items-center justify-center p-8">
        <div className="flex items-center gap-2 text-white/60">
          <div className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
          <span>Loading revisions...</span>
        </div>
      </div>
    )
  }
  
  if (error) {
    return (
      <div className="p-8 text-red-400">
        <p className="font-semibold mb-2">Error loading revisions</p>
        <p className="text-sm text-red-400/80">{error.message}</p>
      </div>
    )
  }
  
  return (
    <div className="flex flex-col h-full bg-black/20 border-l border-white/10">
      <div className="border-b border-white/10 p-4">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <Clock className="w-5 h-5 text-white/80" />
            <h2 className="text-lg font-bold text-white">History</h2>
          </div>
          {compareMode && (
            <button
              onClick={() => {
                setCompareMode(false)
                setCompareFrom(null)
              }}
              className="p-1 hover:bg-white/10 rounded transition-colors"
              title="Cancel compare mode"
            >
              <X className="w-4 h-4 text-white/60" />
            </button>
          )}
        </div>
        <button
          onClick={() => setCompareMode(!compareMode)}
          disabled={revisions.length < 2}
          className={`w-full px-3 py-2 rounded-lg text-sm font-medium transition-colors flex items-center justify-center gap-2 ${
            compareMode 
              ? 'bg-purple-500 text-white' 
              : 'bg-white/10 text-white/80 hover:bg-white/20 disabled:opacity-50 disabled:cursor-not-allowed'
          }`}
        >
          <GitCompare className="w-4 h-4" />
          {compareMode ? 'Compare Mode Active' : 'Compare Revisions'}
        </button>
        {compareMode && (
          <div className="mt-2 text-xs text-white/60 text-center">
            {compareFrom === null
              ? 'Select first revision'
              : 'Select second revision to compare'}
          </div>
        )}
      </div>
      
      <div className="flex-1 overflow-y-auto p-4 space-y-3">
        {revisions.length === 0 ? (
          <div className="text-center py-8 text-white/40">
            <Clock className="w-12 h-12 mx-auto mb-3 opacity-30" />
            <p>No revisions yet</p>
          </div>
        ) : (
          revisions.map((rev) => {
            return (
              <div
                key={rev.number}
                onClick={() => handleRevisionClick(rev)}
                className={`p-4 rounded-lg cursor-pointer transition-all ${
                  selectedRevision === rev.number
                    ? 'bg-purple-500/20 border-2 border-purple-500'
                    : compareFrom === rev.number
                    ? 'bg-blue-500/20 border-2 border-blue-500'
                    : 'bg-white/5 border border-white/10 hover:bg-white/10 hover:border-white/20'
                }`}
              >
                <div className="flex items-start justify-between">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 mb-1">
                      <span className="text-white font-mono font-bold">#{rev.number}</span>
                      {rev.is_system && (
                        <span className="px-2 py-0.5 rounded bg-blue-500/20 text-blue-300 text-xs font-medium">
                          System
                        </span>
                      )}
                    </div>
                    <p className="text-white/90 text-sm line-clamp-2 mb-2">{rev.message}</p>
                    <div className="flex items-center gap-3 text-xs text-white/50">
                      <span className="truncate">{rev.actor}</span>
                      <span>·</span>
                      <span className="whitespace-nowrap">{formatDistanceToNow(new Date(rev.timestamp))}</span>
                    </div>
                  </div>
                  
                  <div className="flex items-center gap-1 ml-2 flex-shrink-0">
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        handleViewChanges(rev.number)
                      }}
                      className="p-2 rounded-lg hover:bg-white/10 text-white/60 hover:text-blue-400 transition-colors"
                      title="View changes"
                      disabled={loadingChanges}
                    >
                      <FileText className="w-4 h-4" />
                    </button>
                    
                    {!compareMode && onCreateTag && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation()
                          onCreateTag(rev.number)
                        }}
                        className="p-2 rounded-lg hover:bg-white/10 text-white/60 hover:text-amber-400 transition-colors"
                        title="Create tag from this revision"
                      >
                        <Tag className="w-4 h-4" />
                      </button>
                    )}
                  </div>
                </div>
              </div>
            )
          })
        )}
      </div>
      
      {/* Changes Modal */}
      {changesModalRevision !== null && (
        <RevisionChangesModal
          repo={repo}
          branch={branch}
          workspace={workspace}
          revision={changesModalRevision}
          changes={modalChanges}
          onClose={() => {
            setChangesModalRevision(null)
            setModalChanges([])
          }}
        />
      )}
    </div>
  )
}
