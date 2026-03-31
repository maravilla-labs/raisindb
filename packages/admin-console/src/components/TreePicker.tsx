import { useState } from 'react'
import { ChevronRight, ChevronDown, Folder, CheckCircle } from 'lucide-react'
import type { Node } from '../api/nodes'

interface TreePickerProps {
  nodes: Node[]
  selectedPath: string | null
  onSelect: (path: string) => void
  excludePath?: string  // Don't show this node (e.g., source node when moving)
}

interface TreeNodeProps {
  node: Node
  level: number
  selectedPath: string | null
  excludePath?: string
  onSelect: (path: string) => void
  onExpand: (node: Node) => void
  isExpanded: boolean
}

function TreePickerNode({ node, level, selectedPath, excludePath, onSelect, onExpand, isExpanded }: TreeNodeProps) {
  if (excludePath && node.path === excludePath) {
    return null  // Don't show excluded node
  }

  const hasChildren = node.children && node.children.length > 0
  const isSelected = selectedPath === node.path
  const indent = level * 20

  return (
    <div>
      <div
        className={`flex items-center gap-2 px-3 py-2 rounded-lg cursor-pointer transition-colors ${
          isSelected ? 'bg-purple-500/30 text-white' : 'hover:bg-white/10 text-gray-300'
        }`}
        style={{ paddingLeft: `${indent + 12}px` }}
        onClick={() => onSelect(node.path)}
      >
        {hasChildren ? (
          <button
            onClick={(e) => {
              e.stopPropagation()
              onExpand(node)
            }}
            className="p-1 hover:bg-white/10 rounded"
          >
            {isExpanded ? (
              <ChevronDown className="w-4 h-4" />
            ) : (
              <ChevronRight className="w-4 h-4" />
            )}
          </button>
        ) : (
          <div className="w-6" />
        )}

        <Folder className="w-4 h-4 text-yellow-400 flex-shrink-0" />
        <span className="flex-1 truncate font-medium">{node.name}</span>
        {isSelected && <CheckCircle className="w-4 h-4 text-green-400" />}
      </div>

      {isExpanded && hasChildren && (
        <div>
          {node.children!.map((child) => (
            <TreePickerNode
              key={child.id}
              node={child}
              level={level + 1}
              selectedPath={selectedPath}
              excludePath={excludePath}
              onSelect={onSelect}
              onExpand={onExpand}
              isExpanded={false}  // Simplified - could track expanded state per node
            />
          ))}
        </div>
      )}
    </div>
  )
}

export default function TreePicker({ nodes, selectedPath, onSelect, excludePath }: TreePickerProps) {
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set())

  function handleExpand(node: Node) {
    setExpandedNodes(prev => {
      const next = new Set(prev)
      if (next.has(node.id)) {
        next.delete(node.id)
      } else {
        next.add(node.id)
      }
      return next
    })
  }

  return (
    <div className="space-y-1 max-h-96 overflow-y-auto">
      {/* Root option */}
      <div
        className={`flex items-center gap-2 px-3 py-2 rounded-lg cursor-pointer transition-colors ${
          selectedPath === '/' ? 'bg-purple-500/30 text-white' : 'hover:bg-white/10 text-gray-300'
        }`}
        onClick={() => onSelect('/')}
      >
        <Folder className="w-4 h-4 text-yellow-400" />
        <span className="flex-1">/ (Root)</span>
        {selectedPath === '/' && <CheckCircle className="w-4 h-4 text-green-400" />}
      </div>

      {nodes.map((node) => (
        <TreePickerNode
          key={node.id}
          node={node}
          level={0}
          selectedPath={selectedPath}
          excludePath={excludePath}
          onSelect={onSelect}
          onExpand={handleExpand}
          isExpanded={expandedNodes.has(node.id)}
        />
      ))}
    </div>
  )
}
