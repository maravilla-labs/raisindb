import { useCallback } from 'react'
import { ChevronRight, ChevronDown, Folder, FileText, Package, User, Calendar, Settings, Tag, Layout, Database, Check, Minus } from 'lucide-react'
import type { Node } from '../api/nodes'

/**
 * Represents a selected path for package creation
 */
export interface SelectedPath {
  workspace: string
  path: string
  nodeId: string
  nodeName: string
  isRecursive: boolean
}

interface SelectableTreeViewProps {
  /** Tree nodes to display */
  nodes: Node[]
  /** Workspace name for path tracking */
  workspace: string
  /** Currently expanded node IDs */
  expandedNodes?: Set<string>
  /** Currently selected paths */
  selectedPaths: Map<string, SelectedPath>
  /** Callback when expansion state changes */
  onNodeExpand?: (node: Node) => void
  /** Callback when selection changes */
  onSelectionChange: (selectedPaths: Map<string, SelectedPath>) => void
  /** Whether to show recursive selection option for folders */
  allowRecursiveSelection?: boolean
}

interface SelectableTreeNodeProps {
  node: Node
  level: number
  workspace: string
  expandedNodes?: Set<string>
  selectedPaths: Map<string, SelectedPath>
  onNodeExpand?: (node: Node) => void
  onSelectionChange: (selectedPaths: Map<string, SelectedPath>) => void
  allowRecursiveSelection?: boolean
}

function SelectableTreeNode({
  node,
  level,
  workspace,
  expandedNodes,
  selectedPaths,
  onNodeExpand,
  onSelectionChange,
  allowRecursiveSelection = true
}: SelectableTreeNodeProps) {
  const isExpanded = expandedNodes?.has(node.id) || false

  // Use server-provided has_children when available, fall back to checking children array
  const hasChildren = node.has_children !== undefined
    ? node.has_children
    : (node.children && node.children.length > 0)

  // For showing expand chevron: use has_children or assume true if children not loaded yet
  const showExpandChevron = node.has_children !== undefined
    ? node.has_children
    : !node.children // If children not loaded yet, assume it might have children

  const indent = level * 20

  // Check selection state
  const isSelected = selectedPaths.has(node.id)
  const selectedPath = selectedPaths.get(node.id)
  const isRecursiveSelected = selectedPath?.isRecursive || false

  // Check if any children are selected (for intermediate state)
  const hasSelectedChildren = useCallback(() => {
    if (!node.children) return false
    return node.children.some(child => {
      if (selectedPaths.has(child.id)) return true
      // Check recursively
      const checkChildren = (n: Node): boolean => {
        if (!n.children) return false
        return n.children.some(c => selectedPaths.has(c.id) || checkChildren(c))
      }
      return checkChildren(child)
    })
  }, [node.children, selectedPaths])

  const isIntermediate = !isSelected && hasSelectedChildren()

  // Get appropriate icon based on node type
  function getNodeIcon() {
    const nodeType = node.node_type?.toLowerCase() || ''

    if (nodeType.includes('folder')) return <Folder className="w-4 h-4 text-amber-400 flex-shrink-0" />
    if (nodeType.includes('page')) return <Layout className="w-4 h-4 text-secondary-400 flex-shrink-0" />
    if (nodeType.includes('asset')) return <Package className="w-4 h-4 text-green-400 flex-shrink-0" />
    if (nodeType.includes('user')) return <User className="w-4 h-4 text-primary-400 flex-shrink-0" />
    if (nodeType.includes('settings')) return <Settings className="w-4 h-4 text-zinc-400 flex-shrink-0" />
    if (nodeType.includes('event')) return <Calendar className="w-4 h-4 text-accent-400 flex-shrink-0" />
    if (nodeType.includes('tag')) return <Tag className="w-4 h-4 text-accent-400 flex-shrink-0" />
    if (nodeType.includes('data')) return <Database className="w-4 h-4 text-secondary-400 flex-shrink-0" />

    // Default icon based on whether it has children
    if (showExpandChevron) {
      return <Folder className="w-4 h-4 text-amber-400 flex-shrink-0" />
    }
    return <FileText className="w-4 h-4 text-zinc-400 flex-shrink-0" />
  }

  // Handle checkbox click
  const handleCheckboxClick = (e: React.MouseEvent) => {
    e.stopPropagation()
    const newSelectedPaths = new Map(selectedPaths)

    if (isSelected) {
      // Deselect
      newSelectedPaths.delete(node.id)
    } else {
      // Select
      newSelectedPaths.set(node.id, {
        workspace,
        path: node.path,
        nodeId: node.id,
        nodeName: node.name,
        isRecursive: Boolean(hasChildren && allowRecursiveSelection)
      })
    }

    onSelectionChange(newSelectedPaths)
  }

  // Handle recursive toggle (only for selected folders)
  const handleRecursiveToggle = (e: React.MouseEvent) => {
    e.stopPropagation()
    if (!isSelected || !hasChildren) return

    const newSelectedPaths = new Map(selectedPaths)
    const currentPath = newSelectedPaths.get(node.id)
    if (currentPath) {
      newSelectedPaths.set(node.id, {
        ...currentPath,
        isRecursive: !currentPath.isRecursive
      })
      onSelectionChange(newSelectedPaths)
    }
  }

  // Handle row click (expand/collapse for folders, select for files)
  const handleRowClick = () => {
    if (hasChildren && onNodeExpand) {
      onNodeExpand(node)
    } else {
      // For leaf nodes, toggle selection
      handleCheckboxClick({ stopPropagation: () => {} } as React.MouseEvent)
    }
  }

  return (
    <div>
      <div
        className={`flex items-center gap-2 px-3 py-2 rounded-lg cursor-pointer group transition-colors ${
          isSelected ? 'bg-primary-500/20 text-white' : 'hover:bg-white/10 text-zinc-300'
        }`}
        style={{ paddingLeft: `${indent + 12}px` }}
        onClick={handleRowClick}
      >
        {/* Expand chevron */}
        {showExpandChevron ? (
          <button
            onClick={(e) => {
              e.stopPropagation()
              if (onNodeExpand) onNodeExpand(node)
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

        {/* Checkbox */}
        <button
          onClick={handleCheckboxClick}
          className={`w-5 h-5 rounded border-2 flex items-center justify-center transition-colors flex-shrink-0 ${
            isSelected
              ? 'bg-primary-500 border-primary-500'
              : isIntermediate
                ? 'bg-primary-500/50 border-primary-500'
                : 'border-zinc-500 hover:border-zinc-400'
          }`}
        >
          {isSelected && <Check className="w-3 h-3 text-white" />}
          {isIntermediate && !isSelected && <Minus className="w-3 h-3 text-white" />}
        </button>

        {/* Node icon */}
        {getNodeIcon()}

        {/* Node name */}
        <span className="flex-1 truncate font-medium">{node.name}</span>

        {/* Node type badge */}
        <span className="text-xs text-zinc-500">{node.node_type}</span>

        {/* Recursive indicator for selected folders */}
        {isSelected && hasChildren && allowRecursiveSelection && (
          <button
            onClick={handleRecursiveToggle}
            className={`text-xs px-2 py-0.5 rounded transition-colors ${
              isRecursiveSelected
                ? 'bg-primary-500 text-white'
                : 'bg-zinc-700 text-zinc-300 hover:bg-zinc-600'
            }`}
            title={isRecursiveSelected ? 'Click to include only this folder' : 'Click to include all children'}
          >
            {isRecursiveSelected ? 'recursive' : 'only this'}
          </button>
        )}
      </div>

      {/* Render children if expanded */}
      {isExpanded && hasChildren && node.children && (
        <div>
          {node.children.map((child) => (
            <SelectableTreeNode
              key={child.id}
              node={child}
              level={level + 1}
              workspace={workspace}
              expandedNodes={expandedNodes}
              selectedPaths={selectedPaths}
              onNodeExpand={onNodeExpand}
              onSelectionChange={onSelectionChange}
              allowRecursiveSelection={allowRecursiveSelection}
            />
          ))}
        </div>
      )}
    </div>
  )
}

export default function SelectableTreeView({
  nodes,
  workspace,
  expandedNodes,
  selectedPaths,
  onNodeExpand,
  onSelectionChange,
  allowRecursiveSelection = true
}: SelectableTreeViewProps) {
  if (nodes.length === 0) {
    return (
      <div className="text-center py-8 text-zinc-400">
        <Folder className="w-10 h-10 mx-auto mb-2 opacity-50" />
        <p>No content in this workspace</p>
      </div>
    )
  }

  return (
    <div className="space-y-1">
      {nodes.map((node) => (
        <SelectableTreeNode
          key={node.id}
          node={node}
          level={0}
          workspace={workspace}
          expandedNodes={expandedNodes}
          selectedPaths={selectedPaths}
          onNodeExpand={onNodeExpand}
          onSelectionChange={onSelectionChange}
          allowRecursiveSelection={allowRecursiveSelection}
        />
      ))}
    </div>
  )
}
