/**
 * FunctionPicker Component
 *
 * Modal for selecting functions to add to a trigger flow.
 * Uses the reusable NodePicker component with function-specific configuration.
 */

import { Play } from 'lucide-react'
import { NodePicker, type NodePickerConfig } from '../../../../components/NodePicker'

interface FunctionPickerProps {
  onSelect: (functionPath: string) => void
  onClose: () => void
  currentFunctionPath?: string
}

export function FunctionPicker({ onSelect, onClose, currentFunctionPath }: FunctionPickerProps) {
  const functionPickerConfig: NodePickerConfig = {
    nodeType: 'raisin:Function',
    title: 'Add Function to Flow',
    subtitle: 'Search or browse functions to add to the execution flow',
    searchPlaceholder: 'Search by name, title, or path...',
    emptyMessage: 'No content in functions workspace',
    icon: Play,
    iconColor: 'text-blue-400',
    selectionColor: 'primary-500',
    currentPath: currentFunctionPath,
  }

  return (
    <NodePicker
      config={functionPickerConfig}
      onSelect={(node) => onSelect(node.path)}
      onClose={onClose}
    />
  )
}
