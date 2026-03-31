/**
 * Editor Tabs Component
 *
 * Tab bar for switching between open function files.
 */

import { X, FileCode, Circle, SquareFunction } from 'lucide-react'
import { useFunctionsContext } from '../../hooks'
import type { EditorTab, FunctionLanguage } from '../../types'

const LANGUAGE_COLORS: Record<FunctionLanguage, string> = {
  javascript: 'text-yellow-400',
  starlark: 'text-blue-400',
  sql: 'text-green-400',
}

// Function icon color (distinctive purple/violet)
const FUNCTION_COLOR = 'text-violet-400'

interface TabProps {
  tab: EditorTab
  isActive: boolean
}

function Tab({ tab, isActive }: TabProps) {
  const { setActiveTab, closeTab } = useFunctionsContext()

  const isFunction = tab.node_type === 'raisin:Function'

  return (
    <div
      className={`
        group flex items-center gap-2 px-3 py-1.5 cursor-pointer
        border-r border-white/10 min-w-0 max-w-[200px]
        ${isActive
          ? 'bg-[#1a1a2e] text-white border-t-2 border-t-primary-500'
          : 'bg-black/20 text-gray-400 hover:bg-white/5 border-t-2 border-t-transparent'
        }
      `}
      onClick={() => setActiveTab(tab.id)}
    >
      {isFunction ? (
        <SquareFunction className={`w-4 h-4 flex-shrink-0 ${FUNCTION_COLOR}`} />
      ) : (
        <FileCode className={`w-4 h-4 flex-shrink-0 ${LANGUAGE_COLORS[tab.language]}`} />
      )}
      <span className="truncate text-sm">{tab.name}</span>

      {/* Dirty indicator or close button */}
      <div className="flex-shrink-0 w-4 h-4 flex items-center justify-center">
        {tab.isDirty ? (
          <Circle className="w-2 h-2 fill-current text-primary-400" />
        ) : (
          <button
            onClick={(e) => {
              e.stopPropagation()
              closeTab(tab.id)
            }}
            className="opacity-0 group-hover:opacity-100 hover:bg-white/10 rounded p-0.5"
          >
            <X className="w-3 h-3" />
          </button>
        )}
      </div>
    </div>
  )
}

export function EditorTabs() {
  const { openTabs, activeTabId } = useFunctionsContext()

  if (openTabs.length === 0) {
    return null
  }

  return (
    <div className="flex items-center bg-black/30 border-b border-white/10 overflow-x-auto select-none">
      {openTabs.map((tab) => (
        <Tab key={tab.id} tab={tab} isActive={tab.id === activeTabId} />
      ))}
    </div>
  )
}
