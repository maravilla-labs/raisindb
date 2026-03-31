/**
 * Builder Toolbar Component
 *
 * IDE-style toolbar with back navigation, title, undo/redo, and save actions.
 */

import { type ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { ArrowLeft, Undo2, Redo2, Save } from 'lucide-react'

interface BuilderToolbarProps {
  /** Page title */
  title: string
  /** Icon to display next to title */
  icon?: ReactNode
  /** Back link configuration */
  backLink: {
    to: string
    label: string
  }
  /** Status badge (e.g., Published/Draft) */
  status?: ReactNode
  /** Save button handler */
  onSave: () => void
  /** Whether save is in progress */
  saving?: boolean
  /** Whether undo is available */
  canUndo: boolean
  /** Whether redo is available */
  canRedo: boolean
  /** Undo handler */
  onUndo: () => void
  /** Redo handler */
  onRedo: () => void
  /** Extra actions to display (e.g., Publish button) */
  extraActions?: ReactNode
}

export function BuilderToolbar({
  title,
  icon,
  backLink,
  status,
  onSave,
  saving = false,
  canUndo,
  canRedo,
  onUndo,
  onRedo,
  extraActions,
}: BuilderToolbarProps) {
  return (
    <div className="flex-shrink-0 bg-black/30 backdrop-blur-md border-b border-white/10 select-none">
      <div className="px-4 py-2 flex items-center justify-between">
        {/* Left side: Back link, title, status */}
        <div className="flex items-center gap-3">
          <Link
            to={backLink.to}
            className="flex items-center gap-1.5 text-primary-300 hover:text-primary-200 transition-colors text-sm"
          >
            <ArrowLeft className="w-4 h-4" />
            {backLink.label}
          </Link>

          <span className="text-zinc-600">/</span>

          <div className="flex items-center gap-2">
            {icon}
            <span className="text-white font-semibold">{title}</span>
          </div>

          {status}
        </div>

        {/* Right side: Actions */}
        <div className="flex items-center gap-2">
          {/* Undo/Redo */}
          <div className="flex items-center border-r border-white/10 pr-2 mr-1">
            <button
              onClick={onUndo}
              disabled={!canUndo}
              className={`p-1.5 rounded transition-colors ${
                canUndo
                  ? 'text-zinc-400 hover:text-white hover:bg-white/10'
                  : 'text-zinc-600 cursor-not-allowed'
              }`}
              title="Undo (Ctrl+Z)"
            >
              <Undo2 className="w-4 h-4" />
            </button>
            <button
              onClick={onRedo}
              disabled={!canRedo}
              className={`p-1.5 rounded transition-colors ${
                canRedo
                  ? 'text-zinc-400 hover:text-white hover:bg-white/10'
                  : 'text-zinc-600 cursor-not-allowed'
              }`}
              title="Redo (Ctrl+Shift+Z)"
            >
              <Redo2 className="w-4 h-4" />
            </button>
          </div>

          {/* Extra actions (e.g., Publish) */}
          {extraActions}

          {/* Save button */}
          <button
            onClick={onSave}
            disabled={saving}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-primary-500 hover:bg-primary-600 disabled:bg-zinc-600 disabled:cursor-not-allowed text-white text-sm font-medium rounded transition-colors"
          >
            <Save className="w-4 h-4" />
            {saving ? 'Saving...' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  )
}
