/**
 * Function Explorer Component
 *
 * Tree view for browsing functions and folders in the functions workspace.
 * Supports drag-and-drop (via Pragmatic Drag and Drop), inline rename, and context menu.
 */

import { useState, useCallback, useEffect } from 'react'
import { createPortal } from 'react-dom'
import { monitorForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { extractInstruction } from '@atlaskit/pragmatic-drag-and-drop-hitbox/tree-item'
import { Search, Plus, Folder, SquareFunction, File, Zap, Workflow, Bot } from 'lucide-react'
import { useFunctionsContext } from '../../hooks'
import { FunctionTreeNode } from './FunctionTreeNode'
import { CreateDialog, type CreateData } from './CreateDialog'
import { nodesApi, type Node as NodeType } from '../../../../api/nodes'
import CommitDialog from '../../../../components/CommitDialog'
import type { FunctionNode } from '../../types'

/** Get placeholder content for a new file based on extension */
function getPlaceholderContent(ext: string): string {
  switch (ext) {
    case 'js': return '// JavaScript module\n'
    case 'ts': return '// TypeScript module\n'
    case 'json': return '{}\n'
    case 'md': return '# Document\n'
    default: return ''
  }
}

interface PendingOperation {
  type: 'create' | 'rename' | 'delete' | 'move' | 'reorder'
  node?: NodeType
  data: unknown
}

export function FunctionExplorer() {
  const {
    repo,
    branch,
    workspace,
    nodes,
    loading,
    loadRootNodes,
    loadNodeChildren,
    expandedNodes,
    selectedNode,
    selectNode,
    expandNode,
    collapseNode,
    openTab,
    renamingNodeId,
    setRenamingNodeId,
    renameNode,
    deleteNode,
    moveNode,
    reorderNode,
    getCreationOptions,
  } = useFunctionsContext()

  const [searchTerm, setSearchTerm] = useState('')
  const [showMenu, setShowMenu] = useState(false)
  const [createDialog, setCreateDialog] = useState<{ type: 'function' | 'folder' | 'file' | 'trigger' | 'flow' | 'agent'; parentPath?: string } | null>(null)
  const [pendingOperation, setPendingOperation] = useState<PendingOperation | null>(null)
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; node: NodeType } | null>(null)

  // Get creation options for root level
  const rootOptions = getCreationOptions(null)

  // Stable accessors so tree nodes don't rerender on unrelated state changes
  const isNodeExpandedCb = useCallback((id: string) => expandedNodes.has(id), [expandedNodes])
  const isNodeSelectedCb = useCallback((id: string) => selectedNode?.id === id, [selectedNode])
  const isNodeRenamingCb = useCallback((id: string) => renamingNodeId === id, [renamingNodeId])

  // Helper to find node by ID
  const findNodeById = useCallback((tree: NodeType[], id: string): NodeType | null => {
    for (const node of tree) {
      if (node.id === id) return node
      const childNodes = node.children as NodeType[] | undefined
      if (childNodes) {
        const found = findNodeById(childNodes, id)
        if (found) return found
      }
    }
    return null
  }, [])

  // Monitor for drop events (Pragmatic DnD)
  useEffect(() => {
    return monitorForElements({
      onDrop: ({ source, location }) => {
        const target = location.current.dropTargets[0]
        if (!target) return

        const instruction = extractInstruction(target.data)
        if (!instruction) return

        const draggedNode = source.data.node as NodeType
        const targetNodeId = target.data.id as string
        const targetNode = findNodeById(nodes, targetNodeId)

        if (!draggedNode || !targetNode) return

        // Don't allow dropping on self
        if (draggedNode.id === targetNode.id) return

        // Don't allow dropping parent into child
        if (targetNode.path.startsWith(draggedNode.path + '/')) return

        if (instruction.type === 'make-child') {
          // Move into folder
          const destinationPath = `${targetNode.path}/${draggedNode.name}`
          setPendingOperation({
            type: 'move',
            node: draggedNode,
            data: { node: draggedNode, destinationPath },
          })
        } else if (instruction.type === 'reorder-above' || instruction.type === 'reorder-below') {
          // Reorder before/after sibling
          const position = instruction.type === 'reorder-above' ? 'before' : 'after'
          setPendingOperation({
            type: 'reorder',
            node: draggedNode,
            data: { node: draggedNode, targetPath: targetNode.path, position },
          })
        }
      },
    })
  }, [nodes, findNodeById])

  // Handle create data from CreateDialog - show CommitDialog next
  const handleCreateData = (data: CreateData) => {
    setCreateDialog(null)
    setPendingOperation({ type: 'create', data })
  }

  // Execute pending operation after commit message is provided
  const executeOperation = async (message: string, actor: string) => {
    if (!pendingOperation) return

    const commit = { message, actor }

    switch (pendingOperation.type) {
      case 'create': {
        const data = pendingOperation.data as CreateData
        if (data.type === 'function') {
          await nodesApi.create(repo, branch, workspace, data.parentPath, {
            name: data.name,
            node_type: 'raisin:Function',
            properties: {
              name: data.name,
              title: data.title,
              language: data.language || 'javascript',
              execution_mode: 'async',
              entry_file: 'index.js:handler',
              enabled: true,
              version: 1,
            },
            commit,
          })
        } else if (data.type === 'trigger') {
          await nodesApi.create(repo, branch, workspace, data.parentPath, {
            name: data.name,
            node_type: 'raisin:Trigger',
            properties: {
              name: data.name,
              title: data.title,
              trigger_type: data.triggerType || 'node_event',
              config: data.triggerType === 'node_event'
                ? { event_kinds: ['Created', 'Updated'] }
                : data.triggerType === 'schedule'
                  ? { cron_expression: '0 * * * *' }
                  : {},
              filters: {},
              enabled: true,
              priority: 0,
            },
            commit,
          })
        } else if (data.type === 'flow') {
          await nodesApi.create(repo, branch, workspace, data.parentPath, {
            name: data.name,
            node_type: 'raisin:Flow',
            properties: {
              name: data.name,
              title: data.title,
              enabled: true,
              workflow_data: {
                version: 1,
                error_strategy: 'fail_fast',
                nodes: [],
              },
            },
            commit,
          })
        } else if (data.type === 'agent') {
          await nodesApi.create(repo, branch, workspace, data.parentPath, {
            name: data.name,
            node_type: 'raisin:AIAgent',
            properties: {
              name: data.name,
              title: data.title,
              system_prompt: '',
              provider: 'openai',
              model: '',
              temperature: 0.7,
              max_tokens: 4096,
              thinking_enabled: false,
              task_creation_enabled: false,
              tools: [],
              rules: [],
            },
            commit,
          })
        } else if (data.type === 'file') {
          const ext = data.name.split('.').pop()?.toLowerCase() || ''
          const mimeTypes: Record<string, string> = {
            js: 'application/javascript',
            ts: 'application/typescript',
            json: 'application/json',
            md: 'text/markdown',
            txt: 'text/plain',
            css: 'text/css',
            html: 'text/html',
          }
          const mimeType = mimeTypes[ext] || 'text/plain'
          const placeholder = getPlaceholderContent(ext)

          const basePath = data.parentPath === '/' ? '' : data.parentPath
          const assetPath = `${basePath}/${data.name}`.replace(/\/+/g, '/')

          // STEP 1: Create empty raisin:Asset node with placeholder string (NO COMMIT)
          await nodesApi.create(repo, branch, workspace, data.parentPath, {
            name: data.name,
            node_type: 'raisin:Asset',
            properties: {
              title: data.name,
              file: '',
            },
          })

          // STEP 2: Upload actual binary file (WITH COMMIT)
          const blob = new Blob([placeholder], { type: mimeType })
          await nodesApi.uploadFile(repo, branch, workspace, assetPath, {
            file: blob,
            fileName: data.name,
            inline: false,
            propertyPath: 'file',
            overrideExisting: true,
            commitMessage: commit.message,
            commitActor: commit.actor,
          })
        } else {
          await nodesApi.create(repo, branch, workspace, data.parentPath, {
            name: data.name,
            node_type: 'raisin:Folder',
            properties: {
              name: data.name,
              title: data.title,
            },
            commit,
          })
        }
        break
      }
      case 'rename': {
        const { node, newName } = pendingOperation.data as { node: NodeType; newName: string }
        await renameNode(node, newName, commit)
        break
      }
      case 'delete': {
        const node = pendingOperation.node!
        await deleteNode(node, commit)
        break
      }
      case 'move': {
        const { node, destinationPath } = pendingOperation.data as { node: NodeType; destinationPath: string }
        await moveNode(node, destinationPath, commit)
        break
      }
      case 'reorder': {
        const { node, targetPath, position } = pendingOperation.data as {
          node: NodeType
          targetPath: string
          position: 'before' | 'after'
        }
        await reorderNode(node, targetPath, position, commit)
        break
      }
    }

    setPendingOperation(null)
    loadRootNodes()
  }

  // Handle rename commit
  const handleRenameCommit = (node: NodeType, newName: string) => {
    if (newName !== node.name) {
      setPendingOperation({
        type: 'rename',
        node,
        data: { node, newName },
      })
    }
    setRenamingNodeId(null)
  }

  // Handle context menu
  const handleContextMenu = useCallback((e: React.MouseEvent, node: NodeType) => {
    e.preventDefault()
    setContextMenu({ x: e.clientX, y: e.clientY, node })
  }, [])

  // Close context menu
  const closeContextMenu = useCallback(() => {
    setContextMenu(null)
  }, [])

  // Filter nodes by search term
  const filterNodes = (nodesList: NodeType[], term: string): NodeType[] => {
    if (!term) return nodesList

    return nodesList.filter((node) => {
      const matches = node.name.toLowerCase().includes(term.toLowerCase())
      const children = node.children as NodeType[] | undefined
      if (children && children.length > 0) {
        const filteredChildren = filterNodes(children, term)
        if (filteredChildren.length > 0) {
          return true
        }
      }
      return matches
    })
  }

  const filteredNodes = filterNodes(nodes, searchTerm)

  // Get action description for commit dialog
  const getOperationDescription = () => {
    if (!pendingOperation) return ''
    switch (pendingOperation.type) {
      case 'create': {
        const data = pendingOperation.data as CreateData
        const typeLabel = data.type === 'function' ? 'function' : data.type === 'trigger' ? 'trigger' : data.type === 'flow' ? 'flow' : data.type === 'agent' ? 'agent' : data.type === 'file' ? 'file' : 'folder'
        return `Create ${typeLabel} "${data.name}"`
      }
      case 'rename': {
        const { node, newName } = pendingOperation.data as { node: NodeType; newName: string }
        return `Rename "${node.name}" to "${newName}"`
      }
      case 'delete':
        return `Delete "${pendingOperation.node?.name}"`
      case 'move': {
        const { node, destinationPath } = pendingOperation.data as { node: NodeType; destinationPath: string }
        return `Move "${node.name}" to ${destinationPath}`
      }
      case 'reorder': {
        const { node, position, targetPath } = pendingOperation.data as {
          node: NodeType
          targetPath: string
          position: 'before' | 'after'
        }
        const targetName = targetPath.split('/').pop()
        return `Move "${node.name}" ${position} "${targetName}"`
      }
      default:
        return ''
    }
  }

  useEffect(() => {
    if (!contextMenu) return

    const handleGlobalClick = () => {
      closeContextMenu()
    }

    document.addEventListener('mousedown', handleGlobalClick)
    return () => {
      document.removeEventListener('mousedown', handleGlobalClick)
    }
  }, [contextMenu, closeContextMenu])

  return (
    <div className="h-full flex flex-col" onClick={closeContextMenu}>
      {/* Header */}
      <div className="flex-shrink-0 p-3 border-b border-white/10">
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-sm font-semibold text-white">Functions</h2>
          <div className="relative">
            <button
              onClick={() => setShowMenu(!showMenu)}
              className="p-1 hover:bg-white/10 rounded text-gray-400 hover:text-white"
            >
              <Plus className="w-4 h-4" />
            </button>

            {showMenu && (
              <div className="absolute right-0 top-full mt-1 bg-zinc-800 border border-white/10 rounded shadow-lg z-10 py-1 min-w-[160px]">
                {rootOptions.canCreateFunction && (
                  <button
                    className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                    onClick={() => {
                      setShowMenu(false)
                      setCreateDialog({ type: 'function' })
                    }}
                  >
                    <SquareFunction className="w-4 h-4 text-violet-400" />
                    New Function
                  </button>
                )}
                {rootOptions.canCreateFunction && (
                  <button
                    className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                    onClick={() => {
                      setShowMenu(false)
                      setCreateDialog({ type: 'trigger' })
                    }}
                  >
                    <Zap className="w-4 h-4 text-yellow-400" />
                    New Trigger
                  </button>
                )}
                {rootOptions.canCreateFunction && (
                  <button
                    className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                    onClick={() => {
                      setShowMenu(false)
                      setCreateDialog({ type: 'flow' })
                    }}
                  >
                    <Workflow className="w-4 h-4 text-blue-400" />
                    New Flow
                  </button>
                )}
                {rootOptions.canCreateAgent && (
                  <button
                    className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                    onClick={() => {
                      setShowMenu(false)
                      setCreateDialog({ type: 'agent' })
                    }}
                  >
                    <Bot className="w-4 h-4 text-purple-400" />
                    New Agent
                  </button>
                )}
                {rootOptions.canCreateFolder && (
                  <button
                    className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                    onClick={() => {
                      setShowMenu(false)
                      setCreateDialog({ type: 'folder' })
                    }}
                  >
                    <Folder className="w-4 h-4" />
                    New Folder
                  </button>
                )}
              </div>
            )}
          </div>
        </div>

        {/* Search */}
        <div className="relative">
          <Search className="absolute left-2.5 top-1/2 transform -translate-y-1/2 w-4 h-4 text-gray-400" />
          <input
            type="text"
            placeholder="Search functions..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="w-full pl-8 pr-3 py-1.5 bg-white/10 border border-white/10 rounded text-sm text-white placeholder-gray-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
          />
        </div>
      </div>

      {/* Tree */}
      <div className="flex-1 overflow-auto py-2">
        {loading ? (
          <div className="text-center text-gray-400 py-4 text-sm">Loading...</div>
        ) : filteredNodes.length === 0 ? (
          <div className="text-center text-gray-400 py-4 text-sm">
            {searchTerm ? 'No matching functions' : 'No functions yet'}
          </div>
        ) : (
          filteredNodes.map((node, index) => (
            <FunctionTreeNode
              key={node.id}
              node={node}
              level={0}
              index={index}
              isExpanded={isNodeExpandedCb(node.id)}
              isSelected={isNodeSelectedCb(node.id)}
              isRenaming={isNodeRenamingCb(node.id)}
              isDragDisabled={!!searchTerm}
              isNodeExpanded={isNodeExpandedCb}
              isNodeSelected={isNodeSelectedCb}
              isNodeRenaming={isNodeRenamingCb}
              onSelect={(n) => selectNode(n as unknown as FunctionNode)}
              onExpand={expandNode}
              onCollapse={collapseNode}
              onLoadChildren={loadNodeChildren}
              onOpenTab={(n) => openTab(n)}
              onStartRename={setRenamingNodeId}
              onCancelRename={() => setRenamingNodeId(null)}
              onCommitRename={handleRenameCommit}
              onContextMenu={handleContextMenu}
            />
          ))
        )}
      </div>

      {/* Context Menu */}
      {contextMenu &&
        createPortal(
          <div
            className="fixed bg-zinc-800 border border-white/10 rounded shadow-lg z-50 py-1 min-w-[160px]"
            style={{ left: contextMenu.x, top: contextMenu.y }}
            onMouseDown={(e) => e.stopPropagation()}
          >
            {(() => {
              const options = getCreationOptions(contextMenu.node)
              const isFolder = contextMenu.node.node_type === 'raisin:Folder'
              const isFunction = contextMenu.node.node_type === 'raisin:Function'
              const isTrigger = contextMenu.node.node_type === 'raisin:Trigger'
              const isFlow = contextMenu.node.node_type === 'raisin:Flow'
              const canHaveFiles = isFunction || isTrigger || isFlow || isFolder
              return (
                <>
                  {/* Folder context menu - create functions/triggers/folders inside */}
                  {isFolder && options.canCreateFunction && (
                    <button
                      className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                      onClick={() => {
                        closeContextMenu()
                        setCreateDialog({ type: 'function', parentPath: contextMenu.node.path })
                      }}
                    >
                      <SquareFunction className="w-4 h-4 text-violet-400" />
                      New Function
                    </button>
                  )}
                  {isFolder && options.canCreateFunction && (
                    <button
                      className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                      onClick={() => {
                        closeContextMenu()
                        setCreateDialog({ type: 'trigger', parentPath: contextMenu.node.path })
                      }}
                    >
                      <Zap className="w-4 h-4 text-yellow-400" />
                      New Trigger
                    </button>
                  )}
                  {isFolder && options.canCreateFunction && (
                    <button
                      className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                      onClick={() => {
                        closeContextMenu()
                        setCreateDialog({ type: 'flow', parentPath: contextMenu.node.path })
                      }}
                    >
                      <Workflow className="w-4 h-4 text-blue-400" />
                      New Flow
                    </button>
                  )}
                  {isFolder && options.canCreateAgent && (
                    <button
                      className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                      onClick={() => {
                        closeContextMenu()
                        setCreateDialog({ type: 'agent', parentPath: contextMenu.node.path })
                      }}
                    >
                      <Bot className="w-4 h-4 text-purple-400" />
                      New Agent
                    </button>
                  )}
                  {isFolder && options.canCreateFolder && (
                    <button
                      className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                      onClick={() => {
                        closeContextMenu()
                        setCreateDialog({ type: 'folder', parentPath: contextMenu.node.path })
                      }}
                    >
                      <Folder className="w-4 h-4" />
                      New Folder
                    </button>
                  )}
                  {/* Context menu for nodes that can have files: Folder, Function, Trigger, Flow */}
                  {canHaveFiles && (
                    <button
                      className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                      onClick={() => {
                        closeContextMenu()
                        setCreateDialog({ type: 'file', parentPath: contextMenu.node.path })
                      }}
                    >
                      <File className="w-4 h-4" />
                      New File
                    </button>
                  )}
                  {/* Function/Trigger/Flow specific - create folders inside */}
                  {(isFunction || isTrigger || isFlow) && (
                    <>
                      <button
                        className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10 flex items-center gap-2"
                        onClick={() => {
                          closeContextMenu()
                          setCreateDialog({ type: 'folder', parentPath: contextMenu.node.path })
                        }}
                      >
                        <Folder className="w-4 h-4" />
                        New Folder
                      </button>
                      <div className="border-t border-white/10 my-1" />
                    </>
                  )}
                  <button
                    className="w-full px-3 py-1.5 text-left text-sm text-gray-300 hover:bg-white/10"
                    onClick={() => {
                      setRenamingNodeId(contextMenu.node.id)
                      closeContextMenu()
                    }}
                  >
                    Rename
                  </button>
                  <button
                    className="w-full px-3 py-1.5 text-left text-sm text-red-400 hover:bg-white/10"
                    onClick={() => {
                      setPendingOperation({
                        type: 'delete',
                        node: contextMenu.node,
                        data: null,
                      })
                      closeContextMenu()
                    }}
                  >
                    Delete
                  </button>
                </>
              )
            })()}
          </div>,
          document.body
        )}

      {/* Create Dialog - Step 1: Collect name/type info */}
      {createDialog && (
        <CreateDialog
          type={createDialog.type}
          parentPath={createDialog.parentPath}
          onClose={() => setCreateDialog(null)}
          onCreate={handleCreateData}
        />
      )}

      {/* Commit Dialog - for all operations */}
      {pendingOperation && (
        <CommitDialog
          title={
            pendingOperation.type === 'create'
              ? `Create ${(pendingOperation.data as CreateData).type === 'function'
                ? 'Function'
                : (pendingOperation.data as CreateData).type === 'trigger'
                  ? 'Trigger'
                  : (pendingOperation.data as CreateData).type === 'flow'
                    ? 'Flow'
                    : (pendingOperation.data as CreateData).type === 'agent'
                      ? 'Agent'
                      : (pendingOperation.data as CreateData).type === 'file'
                        ? 'File'
                        : 'Folder'
              }`
              : pendingOperation.type === 'rename'
                ? 'Rename'
                : pendingOperation.type === 'delete'
                  ? 'Delete'
                  : pendingOperation.type === 'move'
                    ? 'Move'
                    : 'Reorder'
          }
          action={getOperationDescription()}
          onCommit={executeOperation}
          onClose={() => setPendingOperation(null)}
        />
      )}
    </div>
  )
}
