/**
 * Raisin Function Node Type Editor Component
 *
 * Form-based editor for raisin:Function node properties.
 * Opens as a tab (like files) with save, run, undo/redo functionality.
 */

import { useEffect, useCallback, useState, useRef } from 'react'
import { Save, Play, Undo2, Redo2, Loader2, Settings } from 'lucide-react'
import { useFunctionsContext, useUndoRedo } from '../../hooks'
import { FunctionPropertiesForm } from './FunctionPropertiesForm'
import { RequestBuilder, type FunctionRunConfig, type PreparedRun } from './RequestBuilder'
import { SchemaEditorDialog } from './schema'
import { nodesApi } from '../../../../api/nodes'
import { functionsApi, type RunFileLogEvent, type RunFileResultEvent } from '../../../../api/functions'
import CommitDialog from '../../../../components/CommitDialog'
import type { EditorTab, FunctionProperties, FunctionNode, LogEntry } from '../../types'

interface RaisinFunctionNodeTypeEditorProps {
  tab: EditorTab
}

/** Default function properties */
const DEFAULT_PROPERTIES: Partial<FunctionProperties> = {
  name: '',
  title: '',
  language: 'javascript',
  entry_file: 'index.js:handler',
  execution_mode: 'async',
  enabled: true,
}

function parseEntryFile(entryFile?: string) {
  const raw = entryFile?.trim() || 'index.js:handler'
  const [filePart, handlerPart] = raw.split(':')
  const file = (filePart || 'index.js').trim()
  const handler = (handlerPart || 'handler').trim() || 'handler'
  return { file, handler }
}

function resolveEntryFilePath(functionPath: string, relativeFile: string): string {
  const normalizedRelative = relativeFile.replace(/^functions:/, '')
  const base = functionPath || '/'
  const isAbsolute = normalizedRelative.startsWith('/')
  const combined = isAbsolute
    ? normalizedRelative
    : `${base.replace(/\/$/, '')}/${normalizedRelative}`

  const segments = combined.split('/')
  const stack: string[] = []
  for (const segment of segments) {
    if (!segment || segment === '.') continue
    if (segment === '..') {
      if (stack.length > 0) stack.pop()
      continue
    }
    stack.push(segment)
  }

  const prefix = combined.startsWith('/') ? '/' : ''
  return `${prefix}${stack.join('/')}`
}

export function RaisinFunctionNodeTypeEditor({ tab }: RaisinFunctionNodeTypeEditorProps) {
  const {
    repo,
    branch,
    workspace,
    nodes,
    markTabDirty,
    loadRootNodes,
    addLog,
    addExecution,
  } = useFunctionsContext()

  // State
  const [isLoading, setIsLoading] = useState(true)
  const [isRunning, setIsRunning] = useState(false)
  const [pendingCommit, setPendingCommit] = useState<{
    properties: Partial<FunctionProperties>
  } | null>(null)
  const [functionNode, setFunctionNode] = useState<FunctionNode | null>(null)
  const [runConfig, setRunConfig] = useState<FunctionRunConfig>({
    inputType: 'json',
    inputJson: '{}',
    inputNodeId: null,
    inputNodePath: null,
    inputWorkspace: 'content',
    sync: true,
    timeout_ms: 30000,
  })

  // Schema editor dialog state
  const [schemaEditorOpen, setSchemaEditorOpen] = useState<'input' | 'output' | null>(null)

  // Undo/redo for properties
  const {
    value: properties,
    setValue: setProperties,
    undo,
    redo,
    canUndo,
    canRedo,
    reset: resetProperties,
    isDirty,
  } = useUndoRedo<Partial<FunctionProperties>>(DEFAULT_PROPERTIES)

  // Ref for keyboard shortcuts
  const containerRef = useRef<HTMLDivElement>(null)

  // Find function node from tree
  const findFunctionNode = useCallback((nodeList: typeof nodes, path: string): FunctionNode | null => {
    for (const node of nodeList) {
      if (node.path === path && node.node_type === 'raisin:Function') {
        return node as unknown as FunctionNode
      }
      if (node.children && Array.isArray(node.children)) {
        const found = findFunctionNode(node.children as typeof nodes, path)
        if (found) return found
      }
    }
    return null
  }, [])

  // Load function node data
  useEffect(() => {
    const loadNode = async () => {
      setIsLoading(true)
      try {
        // First try to find in tree
        let node = findFunctionNode(nodes, tab.path)

        // If not found, fetch from server
        if (!node) {
          const fetchedNode = await nodesApi.getAtHead(repo, branch, workspace, tab.path)
          if (fetchedNode.node_type === 'raisin:Function') {
            node = fetchedNode as unknown as FunctionNode
          }
        }

        if (node) {
          setFunctionNode(node)
          const props: Partial<FunctionProperties> = {
            name: node.properties?.name || node.name,
            title: node.properties?.title || '',
            description: node.properties?.description || '',
            language: node.properties?.language || 'javascript',
            entry_file: node.properties?.entry_file || 'index.js:handler',
            execution_mode: node.properties?.execution_mode || 'async',
            enabled: node.properties?.enabled !== false,
            input_schema: node.properties?.input_schema,
            output_schema: node.properties?.output_schema,
            network_policy: node.properties?.network_policy,
            resource_limits: node.properties?.resource_limits,
          }
          resetProperties(props)
        }
      } catch (error) {
        console.error('Failed to load function node:', error)
      } finally {
        setIsLoading(false)
      }
    }

    loadNode()
  }, [tab.path, repo, branch, workspace, nodes, findFunctionNode, resetProperties])

  // Sync dirty state with tab
  useEffect(() => {
    markTabDirty(tab.id, isDirty)
  }, [isDirty, tab.id, markTabDirty])

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const isMac = navigator.platform.toUpperCase().indexOf('MAC') >= 0
      const cmdOrCtrl = isMac ? e.metaKey : e.ctrlKey

      if (cmdOrCtrl && e.key === 's') {
        e.preventDefault()
        if (isDirty) {
          handleSave()
        }
      } else if (cmdOrCtrl && e.key === 'z' && !e.shiftKey) {
        e.preventDefault()
        if (canUndo) {
          undo()
        }
      } else if (cmdOrCtrl && e.shiftKey && e.key === 'z') {
        e.preventDefault()
        if (canRedo) {
          redo()
        }
      } else if (cmdOrCtrl && e.key === 'y') {
        // Alternative redo shortcut
        e.preventDefault()
        if (canRedo) {
          redo()
        }
      }
    }

    const container = containerRef.current
    if (container) {
      container.addEventListener('keydown', handleKeyDown)
      return () => container.removeEventListener('keydown', handleKeyDown)
    }
  }, [isDirty, canUndo, canRedo, undo, redo])

  // Handle property changes
  const handlePropertiesChange = useCallback((newProps: Partial<FunctionProperties>) => {
    setProperties(newProps)
  }, [setProperties])

  // Handle save
  const handleSave = useCallback(() => {
    if (!isDirty) return
    setPendingCommit({ properties })
  }, [isDirty, properties])

  // Execute save with commit message
  const executeCommit = useCallback(async (message: string, actor: string) => {
    if (!pendingCommit || !functionNode) return

    try {
      await nodesApi.update(repo, branch, workspace, tab.path, {
        properties: {
          ...functionNode.properties,
          ...pendingCommit.properties,
        },
        commit: { message, actor },
      })

      // Reset undo history with new saved state
      resetProperties(pendingCommit.properties)

      // Refresh the tree to get updated data
      await loadRootNodes()

      addLog({
        level: 'info',
        message: `Function "${properties.name}" properties saved`,
        timestamp: new Date().toISOString(),
      })
    } catch (error) {
      console.error('Failed to save function properties:', error)
      addLog({
        level: 'error',
        message: `Failed to save: ${error instanceof Error ? error.message : String(error)}`,
        timestamp: new Date().toISOString(),
      })
    } finally {
      setPendingCommit(null)
    }
  }, [pendingCommit, functionNode, repo, branch, workspace, tab.path, resetProperties, loadRootNodes, addLog, properties.name])

  const prepareRun = useCallback((config: FunctionRunConfig): PreparedRun | null => {
    if (config.inputType === 'json') {
      try {
        const parsed = config.inputJson.trim() ? JSON.parse(config.inputJson) : {}
        if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
          addLog({
            level: 'error',
            message: 'Input must be a JSON object',
            timestamp: new Date().toISOString(),
          })
          return null
        }
        return {
          inputType: 'json',
          input: parsed,
          inputNodeId: null,
          inputNodePath: null,
          inputWorkspace: config.inputWorkspace,
          sync: config.sync,
          timeout_ms: config.timeout_ms,
        }
      } catch (error) {
        addLog({
          level: 'error',
          message: `Invalid JSON: ${error instanceof Error ? error.message : String(error)}`,
          timestamp: new Date().toISOString(),
        })
        return null
      }
    }

    if (!config.inputNodeId) {
      addLog({
        level: 'error',
        message: 'Select a node to use as input before running.',
        timestamp: new Date().toISOString(),
      })
      return null
    }

    return {
      inputType: 'node',
      input: null,
      inputNodeId: config.inputNodeId,
      inputNodePath: config.inputNodePath,
      inputWorkspace: config.inputWorkspace,
      sync: config.sync,
      timeout_ms: config.timeout_ms,
    }
  }, [addLog])

  const runFunction = useCallback(async (prepared: PreparedRun) => {
    if (!functionNode || isRunning || !repo || !branch) return

    const entryRaw = properties.entry_file || functionNode.properties?.entry_file || functionNode.properties?.entrypoint || 'index.js:handler'
    const { file, handler } = parseEntryFile(entryRaw)
    const resolvedPath = resolveEntryFilePath(functionNode.path, file)
    const displayName = properties.name || functionNode.name

    setIsRunning(true)
    const startTime = Date.now()
    const collectedLogs: LogEntry[] = []

    addLog({
      level: 'info',
      message: `Executing "${displayName}" using ${resolvedPath}:${handler}...`,
      timestamp: new Date().toISOString(),
    })

    let assetNode
    try {
      assetNode = await nodesApi.getAtHead(repo, branch, workspace, resolvedPath)
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      addLog({
        level: 'error',
        message: `Entry file not found at ${resolvedPath}: ${errorMessage}`,
        timestamp: new Date().toISOString(),
      })
      setIsRunning(false)
      return
    }

    if (!assetNode || assetNode.node_type !== 'raisin:Asset') {
      addLog({
        level: 'error',
        message: `Entry file at ${resolvedPath} is missing or not a file node.`,
        timestamp: new Date().toISOString(),
      })
      setIsRunning(false)
      return
    }

    functionsApi.runFileStream(
      repo,
      {
        node_id: assetNode.id,
        function_path: functionNode.path,
        handler,
        input: prepared.inputType === 'json' ? prepared.input ?? {} : undefined,
        input_node_id: prepared.inputType === 'node' ? prepared.inputNodeId ?? undefined : undefined,
        input_workspace: prepared.inputType === 'node' ? prepared.inputWorkspace : undefined,
        timeout_ms: prepared.timeout_ms,
      },
      {
        onStarted: (event) => {
          addLog({
            level: 'info',
            message: `Started execution ${event.execution_id}`,
            timestamp: new Date().toISOString(),
          })
        },
        onLog: (event: RunFileLogEvent) => {
          const entry: LogEntry = {
            level: event.level as LogEntry['level'],
            message: event.message,
            timestamp: event.timestamp,
          }
          collectedLogs.push(entry)
          addLog(entry)
        },
        onResult: (event: RunFileResultEvent) => {
          const execution = {
            id: event.execution_id,
            execution_id: event.execution_id,
            function_path: functionNode.path,
            trigger_name: 'manual',
            status: event.success ? 'completed' as const : 'failed' as const,
            started_at: new Date(startTime).toISOString(),
            completed_at: new Date().toISOString(),
            duration_ms: event.duration_ms,
            result: event.result,
            error: event.error,
            logs: collectedLogs,
          }
          addExecution(execution)

          if (event.error) {
            addLog({
              level: 'error',
              message: `Execution failed: ${event.error}`,
              timestamp: new Date().toISOString(),
            })
          } else {
            addLog({
              level: 'info',
              message: `Execution completed in ${event.duration_ms}ms`,
              timestamp: new Date().toISOString(),
            })
            if (event.result !== undefined) {
              addLog({
                level: 'info',
                message: `Result: ${JSON.stringify(event.result, null, 2)}`,
                timestamp: new Date().toISOString(),
              })
            }
          }
        },
        onDone: () => {
          setIsRunning(false)
        },
        onError: (error) => {
          addLog({
            level: 'error',
            message: `Execution error: ${error.message}`,
            timestamp: new Date().toISOString(),
          })
          setIsRunning(false)
        },
      }
    )
  }, [functionNode, isRunning, properties.entry_file, properties.name, repo, branch, workspace, addLog, addExecution])

  const handleRun = useCallback((prepared: PreparedRun) => {
    runFunction(prepared)
  }, [runFunction])

  const handleRunConfigChange = useCallback((next: FunctionRunConfig) => {
    setRunConfig(next)
  }, [])

  const handleQuickRun = useCallback(() => {
    const prepared = prepareRun(runConfig)
    if (prepared) {
      runFunction(prepared)
    }
  }, [prepareRun, runConfig, runFunction])

  // Loading state
  if (isLoading) {
    return (
      <div className="h-full flex items-center justify-center text-gray-400">
        <Loader2 className="w-6 h-6 animate-spin mr-2" />
        Loading function properties...
      </div>
    )
  }

  // Error state - function not found
  if (!functionNode) {
    return (
      <div className="h-full flex items-center justify-center text-gray-400">
        <Settings className="w-16 h-16 mb-4 opacity-50" />
        <p className="text-lg">Function not found</p>
      </div>
    )
  }

  return (
    <div
      ref={containerRef}
      className="h-full flex flex-col focus:outline-none"
      tabIndex={0}
    >
      {/* Toolbar */}
      <div className="flex-shrink-0 flex items-center gap-2 px-3 py-1.5 bg-black/20 border-b border-white/10">
        {/* Save Button */}
        <button
          onClick={handleSave}
          disabled={!isDirty}
          className={`flex items-center gap-1.5 px-2 py-1 rounded text-sm
            ${isDirty
              ? 'bg-primary-500/20 text-primary-300 hover:bg-primary-500/30'
              : 'text-gray-500 cursor-not-allowed'
            }
          `}
          title="Save (Ctrl+S)"
        >
          <Save className="w-4 h-4" />
          Save
        </button>

        {/* Undo Button */}
        <button
          onClick={undo}
          disabled={!canUndo}
          className={`p-1.5 rounded text-sm
            ${canUndo
              ? 'text-gray-300 hover:bg-white/10'
              : 'text-gray-600 cursor-not-allowed'
            }
          `}
          title="Undo (Ctrl+Z)"
        >
          <Undo2 className="w-4 h-4" />
        </button>

        {/* Redo Button */}
        <button
          onClick={redo}
          disabled={!canRedo}
          className={`p-1.5 rounded text-sm
            ${canRedo
              ? 'text-gray-300 hover:bg-white/10'
              : 'text-gray-600 cursor-not-allowed'
            }
          `}
          title="Redo (Ctrl+Shift+Z)"
        >
          <Redo2 className="w-4 h-4" />
        </button>

        <div className="flex-1" />

        {/* Run Button (Quick Run) */}
        <button
          onClick={handleQuickRun}
          disabled={isRunning}
          className={`flex items-center gap-1.5 px-2 py-1 rounded text-sm
            ${isRunning
              ? 'bg-yellow-500/20 text-yellow-300'
              : 'bg-green-500/20 text-green-300 hover:bg-green-500/30'
            }
            ${isRunning ? 'cursor-not-allowed' : ''}
          `}
          title="Quick Run (Ctrl+Enter)"
        >
          {isRunning ? (
            <Loader2 className="w-4 h-4 animate-spin" />
          ) : (
            <Play className="w-4 h-4" />
          )}
          {isRunning ? 'Running...' : 'Run'}
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-auto">
        <div className="max-w-2xl mx-auto p-6">
          {/* Header */}
          <div className="mb-6">
            <h2 className="text-lg font-semibold text-white">{properties.name || functionNode.name}</h2>
            <p className="text-sm text-gray-400">{tab.path}</p>
          </div>

          {/* Properties Form */}
          <div className="mb-6">
            <h3 className="text-sm font-medium text-gray-300 mb-4 flex items-center gap-2">
              <Settings className="w-4 h-4 text-gray-400" />
              Function Properties
            </h3>
            <FunctionPropertiesForm
              properties={properties}
              onChange={handlePropertiesChange}
              disabled={isRunning}
              onOpenSchemaEditor={setSchemaEditorOpen}
            />
          </div>

          {/* Request Builder */}
          <RequestBuilder
            functionName={properties.name || functionNode.name}
            config={runConfig}
            onConfigChange={handleRunConfigChange}
            onRun={handleRun}
            isRunning={isRunning}
            disabled={false}
          />
        </div>
      </div>

      {/* Commit Dialog */}
      {pendingCommit && (
        <CommitDialog
          title="Save Function Properties"
          action={`Update properties for "${properties.name}"`}
          onCommit={executeCommit}
          onClose={() => setPendingCommit(null)}
        />
      )}

      {/* Schema Editor Dialog */}
      {schemaEditorOpen && (
        <SchemaEditorDialog
          title={schemaEditorOpen === 'input' ? 'Input Schema' : 'Output Schema'}
          schema={
            schemaEditorOpen === 'input'
              ? (properties.input_schema as Record<string, unknown> | undefined)
              : (properties.output_schema as Record<string, unknown> | undefined)
          }
          onSave={(schema) => {
            handlePropertiesChange({
              ...properties,
              [schemaEditorOpen === 'input' ? 'input_schema' : 'output_schema']: schema,
            })
            setSchemaEditorOpen(null)
          }}
          onClose={() => setSchemaEditorOpen(null)}
        />
      )}
    </div>
  )
}
