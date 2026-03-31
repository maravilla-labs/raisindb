/**
 * Function Tree Node Component
 *
 * A tree node component for the Functions IDE explorer that supports:
 * - Expand/collapse for folders
 * - Single click to select
 * - Double click to rename (inline editing)
 * - Right-click context menu
 * - Drag and drop (via Pragmatic Drag and Drop)
 */

import { memo, useState, useRef, useEffect, useCallback } from 'react'
import type { LucideIcon } from 'lucide-react'
import {
  Folder,
  FolderOpen,
  SquareFunction,
  FileJson,
  FileCode,
  FileCode2,
  FileText,
  Database,
  Braces,
  File,
  ChevronRight,
  ChevronDown,
  Zap,
  Workflow,
  Bot,
} from 'lucide-react'
import { InlineRenameInput } from './InlineRenameInput'
import { DropIndicator } from './DropIndicator'
import { useDraggableTreeNode, type DropState, type DragState } from './useDraggableTreeNode'
import type { Node as NodeType } from '../../../../api/nodes'

// Function icon color (distinctive purple/violet)
const FUNCTION_COLOR = 'text-violet-400'
// Trigger icon color (distinctive yellow)
const TRIGGER_COLOR = 'text-yellow-400'
// Flow icon color (distinctive blue)
const FLOW_COLOR = 'text-blue-400'
// Agent icon color (distinctive purple)
const AGENT_COLOR = 'text-purple-400'

// File extension to icon and color mapping
interface FileIconConfig {
  icon: LucideIcon
  color: string
}

const FILE_ICONS: Record<string, FileIconConfig> = {
  // JavaScript
  js: { icon: FileCode, color: 'text-yellow-400' },
  mjs: { icon: FileCode, color: 'text-yellow-400' },
  cjs: { icon: FileCode, color: 'text-yellow-400' },
  // TypeScript
  ts: { icon: FileCode2, color: 'text-blue-400' },
  tsx: { icon: FileCode2, color: 'text-blue-400' },
  // JSON
  json: { icon: FileJson, color: 'text-yellow-600' },
  // Python / Starlark
  py: { icon: Braces, color: 'text-sky-400' },
  star: { icon: Braces, color: 'text-sky-400' },
  // SQL
  sql: { icon: Database, color: 'text-emerald-400' },
  // Text / Markdown
  md: { icon: FileText, color: 'text-gray-400' },
  txt: { icon: FileText, color: 'text-gray-400' },
}

// Default file icon
const DEFAULT_FILE_ICON: FileIconConfig = { icon: File, color: 'text-gray-400' }

/** Get icon component and color for a file extension */
function getFileIcon(extension: string): FileIconConfig {
  return FILE_ICONS[extension.toLowerCase()] || DEFAULT_FILE_ICON
}

export interface FunctionTreeNodeProps {
  node: NodeType
  level: number
  index: number
  isExpanded: boolean
  isSelected: boolean
  isRenaming: boolean
  isDragDisabled?: boolean
  // State lookup callbacks for nested children
  isNodeExpanded: (nodeId: string) => boolean
  isNodeSelected: (nodeId: string) => boolean
  isNodeRenaming: (nodeId: string) => boolean
  onSelect: (node: NodeType) => void
  onExpand: (nodeId: string) => void
  onCollapse: (nodeId: string) => void
  onLoadChildren: (node: NodeType) => Promise<void>
  onOpenTab: (node: NodeType) => void
  onStartRename: (nodeId: string) => void
  onCancelRename: () => void
  onCommitRename: (node: NodeType, newName: string) => void
  onContextMenu: (e: React.MouseEvent, node: NodeType) => void
}

function FunctionTreeNodeComponent({
  node,
  level,
  index: _index,
  isExpanded,
  isSelected,
  isRenaming,
  isDragDisabled = false,
  isNodeExpanded,
  isNodeSelected,
  isNodeRenaming,
  onSelect,
  onExpand,
  onCollapse,
  onLoadChildren,
  onOpenTab,
  onStartRename,
  onCancelRename,
  onCommitRename,
  onContextMenu,
}: FunctionTreeNodeProps) {
  const [lastClickTime, setLastClickTime] = useState(0)
  const clickTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const nodeRef = useRef<HTMLDivElement>(null)

  // Drag and drop state
  const [dragState, setDragState] = useState<DragState>({ isDragging: false })
  const [dropState, setDropState] = useState<DropState>({ instruction: null, isDraggedOver: false })

  const isFunction = node.node_type === 'raisin:Function'
  const isTrigger = node.node_type === 'raisin:Trigger'
  const isFlow = node.node_type === 'raisin:Flow'
  const isAgent = node.node_type === 'raisin:AIAgent'
  const isFolder = node.node_type === 'raisin:Folder'
  const isAsset = node.node_type === 'raisin:Asset'
  const children = node.children as NodeType[] | undefined

  // Node is expandable if it's a folder, OR a function/flow with children/has_children
  const isExpandable = isFolder || ((isFunction || isFlow) && !!(node.has_children || (children && children.length > 0)))

  // Get file extension and icon config for assets
  const fileExtension = isAsset ? (node?.properties?.title as string || node.name).split('.').pop()?.toLowerCase() || '' : ''
  const fileIconConfig = getFileIcon(fileExtension)

  // Use Pragmatic DnD hook
  useDraggableTreeNode({
    ref: nodeRef,
    node,
    level,
    isFolder: isExpandable,
    isExpanded,
    isDragDisabled: isDragDisabled || isRenaming,
    onDragStateChange: setDragState,
    onDropStateChange: setDropState,
  })

  // Clean up timeout on unmount
  useEffect(() => {
    return () => {
      if (clickTimeoutRef.current) {
        clearTimeout(clickTimeoutRef.current)
      }
    }
  }, [])

  const handleClick = useCallback(async () => {
    const now = Date.now()
    const timeDiff = now - lastClickTime

    // Clear any pending single click action
    if (clickTimeoutRef.current) {
      clearTimeout(clickTimeoutRef.current)
      clickTimeoutRef.current = null
    }

    if (timeDiff < 300) {
      // Double click behavior:
      // - Function/Trigger/Flow/Agent: open in editor tab AND expand to show files
      // - Others: start rename
      setLastClickTime(0)
      if (isFunction || isTrigger || isFlow || isAgent) {
        // Open function/trigger/flow/agent in editor tab
        onOpenTab(node)
        // Expand to show children (for functions/flows with children)
        if ((isFunction || isFlow) && !isExpanded) {
          onExpand(node.id)
          await onLoadChildren(node)
        }
      } else {
        onStartRename(node.id)
      }
    } else {
      // Potential single click - delay to check for double click
      setLastClickTime(now)
      clickTimeoutRef.current = setTimeout(() => {
        // Single click behavior:
        // - Function/Trigger/Flow/Agent: select to show properties panel
        // - Asset (file): open in code editor tab
        // - Folder: just select
        if (isFunction || isTrigger || isFlow || isAgent) {
          onSelect(node)
        } else if (isAsset) {
          // Select to update URL, then open in editor tab
          onSelect(node)
          onOpenTab(node)
        } else if (isFolder) {
          onSelect(node)
        }
        clickTimeoutRef.current = null
      }, 300)
    }
  }, [lastClickTime, node, isFunction, isTrigger, isFlow, isAgent, isFolder, isAsset, isExpanded, onSelect, onOpenTab, onStartRename, onExpand, onLoadChildren])

  const handleExpandToggle = async (e: React.MouseEvent) => {
    e.stopPropagation()

    // Check if children are actually loaded (as objects, not just IDs)
    const childrenLoaded = children && children.length > 0 && typeof children[0] === 'object'

    if (isExpanded) {
      if (childrenLoaded) {
        // Children are loaded and visible - collapse
        onCollapse(node.id)
      } else {
        // Marked as expanded (from localStorage) but children not loaded yet - load them
        await onLoadChildren(node)
      }
    } else {
      // Not expanded - expand and load children
      onExpand(node.id)
      await onLoadChildren(node)
    }
  }

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    onContextMenu(e, node)
  }

  const handleRenameComplete = (newName: string) => {
    onCommitRename(node, newName)
  }

  return (
    <div>
      {/* Node row */}
      <div
        ref={nodeRef}
        className={`
          relative flex items-center gap-1 px-2 py-1 rounded cursor-pointer
          transition-colors duration-100
          ${isSelected ? 'bg-primary-500/30 text-white' : 'text-gray-300 hover:bg-white/10'}
          ${dragState.isDragging ? 'opacity-50' : ''}
        `}
        style={{ paddingLeft: `${level * 12 + 8}px` }}
        onClick={handleClick}
        onContextMenu={handleContextMenu}
      >
        {/* Drop indicator */}
        {dropState.instruction && (
          <DropIndicator instruction={dropState.instruction} level={level} />
        )}

        {/* Expand/collapse button for expandable items (folders and functions with children) */}
        {isExpandable ? (
          <button
            onClick={handleExpandToggle}
            className="p-0.5 hover:bg-white/10 rounded"
          >
            {isExpanded ? (
              <ChevronDown className="w-4 h-4" />
            ) : (
              <ChevronRight className="w-4 h-4" />
            )}
          </button>
        ) : (
          <span className="w-5" /> // Spacer for alignment
        )}

        {/* Icon */}
        {isFolder ? (
          isExpanded ? (
            <FolderOpen className="w-4 h-4 text-yellow-400 flex-shrink-0" />
          ) : (
            <Folder className="w-4 h-4 text-yellow-400 flex-shrink-0" />
          )
        ) : isFunction ? (
          <SquareFunction className={`w-4 h-4 flex-shrink-0 ${FUNCTION_COLOR}`} />
        ) : isTrigger ? (
          <Zap className={`w-4 h-4 flex-shrink-0 ${TRIGGER_COLOR}`} />
        ) : isFlow ? (
          <Workflow className={`w-4 h-4 flex-shrink-0 ${FLOW_COLOR}`} />
        ) : isAgent ? (
          <Bot className={`w-4 h-4 flex-shrink-0 ${AGENT_COLOR}`} />
        ) : isAsset ? (
          <fileIconConfig.icon className={`w-4 h-4 flex-shrink-0 ${fileIconConfig.color}`} />
        ) : (
          <File className="w-4 h-4 flex-shrink-0 text-gray-400" />
        )}

        {/* Name - inline editable when renaming */}
        {isRenaming ? (
          <InlineRenameInput
            initialValue={node.name}
            onSave={handleRenameComplete}
            onCancel={onCancelRename}
            isFile={isAsset}
          />
        ) : (
          <span className="flex-1 truncate text-sm select-none">{node.name}</span>
        )}

        {/* Enabled indicator for functions, triggers, flows, and agents */}
        {(isFunction || isTrigger || isFlow || isAgent) && !isRenaming && (
          <span
            className={`w-2 h-2 rounded-full flex-shrink-0 ${
              (node.properties as { enabled?: boolean })?.enabled !== false
                ? 'bg-green-400'
                : 'bg-red-400'
            }`}
            title={(node.properties as { enabled?: boolean })?.enabled !== false ? 'Enabled' : 'Disabled'}
          />
        )}
      </div>

      {/* Children (for folders and expandable functions) */}
      {isExpandable && isExpanded && children && children.length > 0 && (
        <div>
          {children.map((child, childIndex) => (
            <FunctionTreeNode
              key={child.id}
              node={child}
              level={level + 1}
              index={childIndex}
              isExpanded={isNodeExpanded(child.id)}
              isSelected={isNodeSelected(child.id)}
              isRenaming={isNodeRenaming(child.id)}
              isDragDisabled={isDragDisabled}
              isNodeExpanded={isNodeExpanded}
              isNodeSelected={isNodeSelected}
              isNodeRenaming={isNodeRenaming}
              onSelect={onSelect}
              onExpand={onExpand}
              onCollapse={onCollapse}
              onLoadChildren={onLoadChildren}
              onOpenTab={onOpenTab}
              onStartRename={onStartRename}
              onCancelRename={onCancelRename}
              onCommitRename={onCommitRename}
              onContextMenu={onContextMenu}
            />
          ))}
        </div>
      )}
    </div>
  )
}

const propsAreEqual = (prev: FunctionTreeNodeProps, next: FunctionTreeNodeProps) => {
  return (
    prev.node === next.node &&
    prev.level === next.level &&
    prev.isExpanded === next.isExpanded &&
    prev.isSelected === next.isSelected &&
    prev.isRenaming === next.isRenaming &&
    prev.isDragDisabled === next.isDragDisabled &&
    prev.isNodeExpanded === next.isNodeExpanded &&
    prev.isNodeSelected === next.isNodeSelected &&
    prev.isNodeRenaming === next.isNodeRenaming &&
    prev.onSelect === next.onSelect &&
    prev.onExpand === next.onExpand &&
    prev.onCollapse === next.onCollapse &&
    prev.onLoadChildren === next.onLoadChildren &&
    prev.onOpenTab === next.onOpenTab &&
    prev.onStartRename === next.onStartRename &&
    prev.onCancelRename === next.onCancelRename &&
    prev.onCommitRename === next.onCommitRename &&
    prev.onContextMenu === next.onContextMenu
  )
}

export const FunctionTreeNode = memo(FunctionTreeNodeComponent, propsAreEqual)
