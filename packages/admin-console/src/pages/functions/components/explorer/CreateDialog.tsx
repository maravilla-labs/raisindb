/**
 * Create Function/Folder/File Dialog
 *
 * Modal dialog for creating new functions, folders, or files in the functions workspace.
 * This is step 1 of the create flow - collects name/type info.
 * Step 2 is the CommitDialog shown after this.
 */

import { useState } from 'react'
import { createPortal } from 'react-dom'
import { X, FileCode, Folder, File, AlertCircle, Zap, Workflow, Bot } from 'lucide-react'
import type { FunctionLanguage, TriggerType } from '../../types'

export interface CreateData {
  type: 'function' | 'folder' | 'file' | 'trigger' | 'flow' | 'agent'
  name: string
  title: string
  language?: FunctionLanguage
  triggerType?: TriggerType
  parentPath: string
}

interface CreateDialogProps {
  type: 'function' | 'folder' | 'file' | 'trigger' | 'flow' | 'agent'
  parentPath?: string
  onClose: () => void
  onCreate: (data: CreateData) => void
}

export function CreateDialog({ type, parentPath = '', onClose, onCreate }: CreateDialogProps) {
  const [name, setName] = useState('')
  const [title, setTitle] = useState('')
  const [language, setLanguage] = useState<FunctionLanguage>('javascript')
  const [triggerType, setTriggerType] = useState<TriggerType>('node_event')
  const [error, setError] = useState<string | null>(null)

  const handleCreate = () => {
    if (!name.trim()) {
      setError('Name is required')
      return
    }

    // Validate name - files can have extensions, functions/folders/triggers cannot
    if (type === 'file') {
      // Allow file names with extensions (e.g., utils.js, helpers.ts)
      if (!/^[a-zA-Z][a-zA-Z0-9_.-]*$/.test(name)) {
        setError('File name must start with a letter and contain only letters, numbers, underscores, hyphens, or dots')
        return
      }
    } else {
      // Validate name (no slashes, special chars, no dots for folders/functions/triggers)
      if (!/^[a-zA-Z][a-zA-Z0-9_-]*$/.test(name)) {
        setError('Name must start with a letter and contain only letters, numbers, underscores, or hyphens')
        return
      }
    }

    // Pass data to parent - CommitDialog will be shown next
    onCreate({
      type,
      name: name.trim(),
      title: title.trim() || name.trim(),
      language: type === 'function' ? language : undefined,
      triggerType: type === 'trigger' ? triggerType : undefined,
      parentPath: parentPath || '/',
    })
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleCreate()
    }
  }

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-sm"
        onClick={onClose}
      />

      {/* Dialog */}
      <div className="relative bg-zinc-900 border border-white/20 rounded-lg shadow-2xl w-full max-w-md mx-4">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
          <div className="flex items-center gap-2">
            {type === 'function' ? (
              <FileCode className="w-5 h-5 text-violet-400" />
            ) : type === 'trigger' ? (
              <Zap className="w-5 h-5 text-yellow-400" />
            ) : type === 'flow' ? (
              <Workflow className="w-5 h-5 text-blue-400" />
            ) : type === 'agent' ? (
              <Bot className="w-5 h-5 text-purple-400" />
            ) : type === 'file' ? (
              <File className="w-5 h-5 text-blue-400" />
            ) : (
              <Folder className="w-5 h-5 text-yellow-400" />
            )}
            <h2 className="text-lg font-semibold text-white">
              New {type === 'function' ? 'Function' : type === 'trigger' ? 'Trigger' : type === 'flow' ? 'Flow' : type === 'agent' ? 'Agent' : type === 'file' ? 'File' : 'Folder'}
            </h2>
          </div>
          <button
            onClick={onClose}
            className="p-1 hover:bg-white/10 rounded text-gray-400 hover:text-white"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Body */}
        <div className="p-4 space-y-4" onKeyDown={handleKeyDown}>
          {/* Parent path info */}
          {parentPath && (
            <div className="text-sm text-gray-400">
              Creating in: <span className="text-white font-mono">{parentPath}</span>
            </div>
          )}

          {/* Name */}
          <div>
            <label className="block text-sm text-gray-300 mb-1">Name *</label>
            <input
              type="text"
              value={name}
              onChange={(e) => {
                setName(e.target.value)
                setError(null)
              }}
              placeholder={type === 'function' ? 'my_function' : type === 'trigger' ? 'my_trigger' : type === 'flow' ? 'my_flow' : type === 'agent' ? 'my_agent' : type === 'file' ? 'utils.js' : 'my_folder'}
              className="w-full px-3 py-2 bg-black/30 border border-white/20 rounded text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
              autoFocus
            />
            {type === 'file' && (
              <p className="text-xs text-gray-500 mt-1">Include file extension (e.g., .js, .ts)</p>
            )}
          </div>

          {/* Title (not shown for files) */}
          {type !== 'file' && (
            <div>
              <label className="block text-sm text-gray-300 mb-1">Title</label>
              <input
                type="text"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="Display title (optional)"
                className="w-full px-3 py-2 bg-black/30 border border-white/20 rounded text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
              />
            </div>
          )}

          {/* Language (only for functions) */}
          {type === 'function' && (
            <div>
              <label className="block text-sm text-gray-300 mb-1">Language</label>
              <select
                value={language}
                onChange={(e) => setLanguage(e.target.value as FunctionLanguage)}
                className="w-full px-3 py-2 bg-black/30 border border-white/20 rounded text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
              >
                <option value="javascript">JavaScript</option>
                <option value="starlark">Starlark</option>
                <option value="sql">SQL</option>
              </select>
            </div>
          )}

          {/* Trigger Type (only for triggers) */}
          {type === 'trigger' && (
            <div>
              <label className="block text-sm text-gray-300 mb-1">Trigger Type</label>
              <select
                value={triggerType}
                onChange={(e) => setTriggerType(e.target.value as TriggerType)}
                className="w-full px-3 py-2 bg-black/30 border border-white/20 rounded text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
              >
                <option value="node_event">Node Event</option>
                <option value="schedule">Schedule</option>
                <option value="http">HTTP</option>
              </select>
              <p className="text-xs text-gray-500 mt-1">
                {triggerType === 'node_event'
                  ? 'Triggered when nodes are created, updated, or deleted'
                  : triggerType === 'schedule'
                    ? 'Triggered on a recurring schedule (cron)'
                    : 'Triggered by HTTP requests'}
              </p>
            </div>
          )}

          {/* Error */}
          {error && (
            <div className="flex items-center gap-2 text-red-400 text-sm">
              <AlertCircle className="w-4 h-4" />
              {error}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-white/10">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm text-gray-300 hover:text-white hover:bg-white/10 rounded transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleCreate}
            disabled={!name.trim()}
            className="px-4 py-2 text-sm bg-primary-500 hover:bg-primary-400 text-white rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Continue
          </button>
        </div>
      </div>
    </div>,
    document.body
  )
}
