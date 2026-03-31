/**
 * AgentPicker Component
 *
 * Modal for selecting an AI agent for AI container steps.
 * Uses the reusable NodePicker component with agent-specific configuration.
 */

import { Bot } from 'lucide-react'
import { NodePicker, type NodePickerConfig } from '../../../../components/NodePicker'

interface AgentPickerProps {
  onSelect: (agentRef: {
    'raisin:ref': string
    'raisin:workspace': string
    'raisin:path': string
  }) => void
  onClose: () => void
  currentAgentPath?: string
}

export function AgentPicker({ onSelect, onClose, currentAgentPath }: AgentPickerProps) {
  const agentPickerConfig: NodePickerConfig = {
    nodeType: 'raisin:AIAgent',
    title: 'Select AI Agent',
    subtitle: 'Choose an agent for this AI container step',
    searchPlaceholder: 'Search agents by name or path...',
    emptyMessage: 'No agents found in functions workspace.',
    emptyHint: 'Create an agent in /agents folder first.',
    icon: Bot,
    iconColor: 'text-purple-400',
    selectionColor: 'purple-500',
    currentPath: currentAgentPath,
    autoExpandFolder: 'agents',
    filterTreeNodes: true,
  }

  return (
    <NodePicker
      config={agentPickerConfig}
      onSelect={(node) =>
        onSelect({
          'raisin:ref': node.id,
          'raisin:workspace': 'functions',
          'raisin:path': node.path,
        })
      }
      onClose={onClose}
    />
  )
}
