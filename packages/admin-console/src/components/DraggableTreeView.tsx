import type { CSSProperties } from 'react'
import { DragDropContext, Draggable, Droppable, DropResult } from '@hello-pangea/dnd'
import { ChevronRight, ChevronDown, Folder, FileText, Package, User, Calendar, Settings, Tag, Layout, Database, GripVertical } from 'lucide-react'
import ContextMenu from './ContextMenu'
import type { Node } from '../api/nodes'

interface DraggableTreeViewProps {
  nodes: Node[]
  expandedNodes?: Set<string>
  onNodeClick?: (node: Node) => void
  onNodeExpand?: (node: Node) => void
  onEdit?: (node: Node) => void
  onAddChild?: (node: Node) => void
  onCopy?: (node: Node) => void
  onMove?: (node: Node) => void
  onPublish?: (node: Node) => void
  onUnpublish?: (node: Node) => void
  onDelete?: (node: Node) => void
  onCreateRoot?: () => void
  selectedNodeId?: string
  onDragEnd: (result: DropResult, nodes: Node[]) => void
  isDragDisabled?: boolean
}

interface DraggableTreeNodeProps {
  node: Node
  level: number
  index: number
  expandedNodes?: Set<string>
  selectedNodeId?: string
  onNodeClick?: (node: Node) => void
  onNodeExpand?: (node: Node) => void
  onEdit?: (node: Node) => void
  onAddChild?: (node: Node) => void
  onCopy?: (node: Node) => void
  onMove?: (node: Node) => void
  onPublish?: (node: Node) => void
  onUnpublish?: (node: Node) => void
  onDelete?: (node: Node) => void
  isSelected: boolean
  isDragDisabled?: boolean
}

function DraggableTreeNode({
  node, level, index, expandedNodes, selectedNodeId,
  onNodeClick, onNodeExpand, onEdit, onAddChild, onCopy, onMove, onPublish, onUnpublish, onDelete,
  isSelected, isDragDisabled
}: DraggableTreeNodeProps) {
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

  return (
    <Draggable draggableId={node.id} index={index} isDragDisabled={isDragDisabled}>
      {(provided, snapshot) => (
        <div
          ref={provided.innerRef}
          {...provided.draggableProps}
          style={provided.draggableProps.style}
        >
          <Droppable droppableId={node.id} type="NODE">
            {(droppableProvided, droppableSnapshot) => {
              const dragHandleProps = !isDragDisabled ? provided.dragHandleProps : undefined
              const combinedStyle: CSSProperties = {
                marginLeft: indent,
                paddingLeft: 12,
              }

              return (
                <div>
                  <div
                    ref={droppableProvided.innerRef}
                    {...droppableProvided.droppableProps}
                    {...(dragHandleProps || {})}
                    className={`flex items-center gap-2 px-3 py-2 rounded-lg group transition-colors select-none ${
                      isSelected ? 'bg-primary-500/30 text-white' : 'hover:bg-white/10 text-zinc-300'
                    } ${snapshot.isDragging ? 'bg-primary-500/20 shadow-lg' : ''} ${
                      droppableSnapshot.isDraggingOver ? 'bg-green-500/20 ring-2 ring-green-500/50' : ''
                    } ${isDragDisabled ? '' : snapshot.isDragging ? 'cursor-grabbing' : 'cursor-grab'}`}
                    style={combinedStyle}
                    onClick={() => {
                      if (onNodeClick) onNodeClick(node)
                    }}
                  >
                    {/* Drag handle */}
                    <div className={`transition-opacity ${isDragDisabled ? 'opacity-0' : 'opacity-0 group-hover:opacity-100'}`}>
                      <GripVertical className="w-4 h-4 text-zinc-500 pointer-events-none" />
                    </div>

                    {showExpandChevron ? (
                      <button
                        onClick={(e) => {
                          e.stopPropagation()
                          if (onNodeExpand) onNodeExpand(node)
                        }}
                        onMouseDown={(e) => e.stopPropagation()}
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

                    {getNodeIcon()}

                    <span className="flex-1 truncate font-medium">{node.name}</span>

                    <span className="text-xs text-zinc-500">{node.node_type}</span>

                    <div className="opacity-0 group-hover:opacity-100 transition-opacity">
                      <ContextMenu
                        node={node}
                        onEdit={() => onEdit?.(node)}
                        onAddChild={() => onAddChild?.(node)}
                        onCopy={() => onCopy?.(node)}
                        onMove={() => onMove?.(node)}
                        onPublish={() => onPublish?.(node)}
                        onUnpublish={() => onUnpublish?.(node)}
                        onDelete={() => onDelete?.(node)}
                      />
                    </div>
                  </div>
                  {droppableProvided.placeholder}
                </div>
              )
            }}
          </Droppable>

          {/* Render children */}
          {isExpanded && hasChildren && (
            <Droppable droppableId={`${node.id}-children`} type="NODE">
              {(childrenProvided, childrenSnapshot) => (
                <div
                  ref={childrenProvided.innerRef}
                  {...childrenProvided.droppableProps}
                  className={childrenSnapshot.isDraggingOver ? 'bg-green-500/10 rounded-lg' : ''}
                >
                  {node.children!.map((child, childIndex) => (
                    <DraggableTreeNode
                      key={child.id}
                      node={child}
                      level={level + 1}
                      index={childIndex}
                      expandedNodes={expandedNodes}
                      selectedNodeId={selectedNodeId}
                      onNodeClick={onNodeClick}
                      onNodeExpand={onNodeExpand}
                      onEdit={onEdit}
                      onAddChild={onAddChild}
                      onCopy={onCopy}
                      onMove={onMove}
                      onPublish={onPublish}
                      onUnpublish={onUnpublish}
                      onDelete={onDelete}
                      isSelected={selectedNodeId ? child.id === selectedNodeId : false}
                      isDragDisabled={isDragDisabled}
                    />
                  ))}
                  {childrenProvided.placeholder}
                </div>
              )}
            </Droppable>
          )}
        </div>
      )}
    </Draggable>
  )
}

export default function DraggableTreeView({
  nodes, expandedNodes, onNodeClick, onNodeExpand,
  onEdit, onAddChild, onCopy, onMove, onPublish, onUnpublish, onDelete,
  onCreateRoot, selectedNodeId, onDragEnd, isDragDisabled
}: DraggableTreeViewProps) {
  if (nodes.length === 0) {
    return (
      <div className="text-center py-12 text-zinc-400">
        <Folder className="w-12 h-12 mx-auto mb-2 opacity-50" />
        <p className="mb-4">No content yet</p>
        {onCreateRoot && (
          <button
            onClick={onCreateRoot}
            className="px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors inline-flex items-center gap-2"
          >
            <FileText className="w-4 h-4" />
            Create First Node
          </button>
        )}
      </div>
    )
  }

  return (
    <DragDropContext onDragEnd={(result) => onDragEnd(result, nodes)}>
      <Droppable droppableId="root" type="NODE">
        {(provided, snapshot) => (
          <div
            ref={provided.innerRef}
            {...provided.droppableProps}
            className={`space-y-1 ${snapshot.isDraggingOver ? 'bg-green-500/10 rounded-lg' : ''}`}
          >
            {nodes.map((node, index) => (
              <DraggableTreeNode
                key={node.id}
                node={node}
                level={0}
                index={index}
                expandedNodes={expandedNodes}
                selectedNodeId={selectedNodeId}
                onNodeClick={onNodeClick}
                onNodeExpand={onNodeExpand}
                onEdit={onEdit}
                onAddChild={onAddChild}
                onCopy={onCopy}
                onMove={onMove}
                onPublish={onPublish}
                onUnpublish={onUnpublish}
                onDelete={onDelete}
                isSelected={selectedNodeId ? node.id === selectedNodeId : false}
                isDragDisabled={isDragDisabled}
              />
            ))}
            {provided.placeholder}
          </div>
        )}
      </Droppable>
    </DragDropContext>
  )
}
