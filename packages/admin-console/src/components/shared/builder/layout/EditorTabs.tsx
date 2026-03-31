/**
 * Editor Tabs Component
 *
 * Tab bar for switching between Visual and Source (YAML) modes.
 */

import { Code2, Eye } from 'lucide-react'

export type EditorMode = 'visual' | 'source'

interface EditorTabsProps {
  /** Currently active tab */
  activeTab: EditorMode
  /** Callback when tab changes */
  onTabChange: (tab: EditorMode) => void
  /** Optional error message (e.g., YAML parse error) */
  error?: string | null
}

export function EditorTabs({ activeTab, onTabChange, error }: EditorTabsProps) {
  return (
    <div className="flex-shrink-0 bg-zinc-900/50 border-b border-white/10 select-none">
      <div className="flex items-center">
        {/* Visual tab */}
        <button
          onClick={() => onTabChange('visual')}
          className={`
            flex items-center gap-1.5 px-4 py-2 text-sm font-medium transition-colors
            border-b-2 -mb-px
            ${
              activeTab === 'visual'
                ? 'text-white border-primary-500 bg-white/5'
                : 'text-zinc-400 border-transparent hover:text-white hover:bg-white/5'
            }
          `}
        >
          <Eye className="w-4 h-4" />
          Visual
        </button>

        {/* Source tab */}
        <button
          onClick={() => onTabChange('source')}
          className={`
            flex items-center gap-1.5 px-4 py-2 text-sm font-medium transition-colors
            border-b-2 -mb-px
            ${
              activeTab === 'source'
                ? 'text-white border-primary-500 bg-white/5'
                : 'text-zinc-400 border-transparent hover:text-white hover:bg-white/5'
            }
          `}
        >
          <Code2 className="w-4 h-4" />
          YAML
        </button>

        {/* Error indicator */}
        {error && (
          <div className="ml-auto px-3 py-1 text-xs text-red-400 truncate max-w-xs">
            {error}
          </div>
        )}
      </div>
    </div>
  )
}
