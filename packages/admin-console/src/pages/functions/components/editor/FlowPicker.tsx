/**
 * FlowPicker Component
 *
 * Modal for selecting a flow to execute when a trigger fires.
 * Uses the reusable NodePicker component with flow-specific configuration.
 */

import { GitBranch } from 'lucide-react'
import { NodePicker, type NodePickerConfig } from '../../../../components/NodePicker'

interface FlowPickerProps {
  onSelect: (flowRef: {
    'raisin:ref': string
    'raisin:workspace': string
    'raisin:path': string
  }) => void
  onClose: () => void
  currentFlowPath?: string
}

export function FlowPicker({ onSelect, onClose, currentFlowPath }: FlowPickerProps) {
  const flowPickerConfig: NodePickerConfig = {
    nodeType: 'raisin:Flow',
    title: 'Select Flow',
    subtitle: 'Choose a flow to execute when this trigger fires',
    searchPlaceholder: 'Search flows by name or path...',
    emptyMessage: 'No flows found in functions workspace',
    emptyHint: 'Create a flow first to use it here.',
    icon: GitBranch,
    iconColor: 'text-green-400',
    selectionColor: 'green-500',
    currentPath: currentFlowPath,
  }

  return (
    <NodePicker
      config={flowPickerConfig}
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
