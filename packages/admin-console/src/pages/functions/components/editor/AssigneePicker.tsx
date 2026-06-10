/**
 * AssigneePicker Component
 *
 * Modal for selecting a human-task assignee. The assignee can be either
 * a user (raisin:User from the raisin:access_control workspace) or an
 * AI agent (raisin:AIAgent from the functions workspace).
 *
 * Uses the reusable NodePicker component with a Users / AI Agents toggle.
 * Returns the selected node's path as a plain string (backward compatible
 * with manually typed paths).
 */

import { useState } from 'react'
import { User, Bot } from 'lucide-react'
import { NodePicker, type NodePickerConfig } from '../../../../components/NodePicker'

type AssigneeTab = 'users' | 'agents'

interface AssigneePickerProps {
  /** Called with the selected node's path (e.g. /users/alice or /agents/support) */
  onSelect: (path: string) => void
  onClose: () => void
  /** Currently assigned path, used to highlight + pick the initial tab */
  currentPath?: string
}

export function AssigneePicker({ onSelect, onClose, currentPath }: AssigneePickerProps) {
  const [tab, setTab] = useState<AssigneeTab>(
    currentPath?.startsWith('/agents/') ? 'agents' : 'users'
  )

  const userConfig: NodePickerConfig = {
    nodeType: 'raisin:User',
    workspace: 'raisin:access_control',
    title: 'Select Assignee',
    subtitle: 'Choose a user or AI agent to assign this task to',
    searchPlaceholder: 'Search users by name or path...',
    emptyMessage: 'No users found in access control workspace',
    emptyHint: 'Create users in the Access Control section first.',
    icon: User,
    iconColor: 'text-amber-400',
    selectionColor: 'primary-500',
    currentPath: tab === 'users' ? currentPath : undefined,
    filterTreeNodes: true,
  }

  const agentConfig: NodePickerConfig = {
    nodeType: 'raisin:AIAgent',
    workspace: 'functions',
    title: 'Select Assignee',
    subtitle: 'Choose a user or AI agent to assign this task to',
    searchPlaceholder: 'Search agents by name or path...',
    emptyMessage: 'No agents found in functions workspace.',
    emptyHint: 'Create an agent in /agents folder first.',
    icon: Bot,
    iconColor: 'text-purple-400',
    selectionColor: 'purple-500',
    currentPath: tab === 'agents' ? currentPath : undefined,
    autoExpandFolder: 'agents',
    filterTreeNodes: true,
  }

  const tabToggle = (
    <div className="flex items-center gap-1 p-0.5 bg-white/5 border border-white/10 rounded-lg w-fit">
      <button
        onClick={() => setTab('users')}
        className={`flex items-center gap-1.5 px-3 py-1 rounded-md text-xs transition-colors ${
          tab === 'users'
            ? 'bg-amber-500/20 text-amber-400'
            : 'text-gray-400 hover:text-white'
        }`}
      >
        <User className="w-3.5 h-3.5" />
        Users
      </button>
      <button
        onClick={() => setTab('agents')}
        className={`flex items-center gap-1.5 px-3 py-1 rounded-md text-xs transition-colors ${
          tab === 'agents'
            ? 'bg-purple-500/20 text-purple-400'
            : 'text-gray-400 hover:text-white'
        }`}
      >
        <Bot className="w-3.5 h-3.5" />
        AI Agents
      </button>
    </div>
  )

  return (
    <NodePicker
      key={tab}
      config={tab === 'users' ? userConfig : agentConfig}
      onSelect={(node) => onSelect(node.path)}
      onClose={onClose}
      headerExtra={tabToggle}
    />
  )
}
