/**
 * Raisin Trigger Node Type Editor Component
 *
 * Form-based editor for raisin:Trigger nodes.
 * Covers properties defined in raisin_trigger.yaml and lets you attach one or more functions.
 */

import { useEffect, useCallback, useState, useRef } from 'react'
import { Save, Undo2, Redo2, Loader2, Zap, Settings, Plus, Trash2, Globe, Copy, Check, Play, GitBranch, Unlink, Link2 } from 'lucide-react'
import { useFunctionsContext, useUndoRedo } from '../../hooks'
import { nodesApi } from '../../../../api/nodes'
import { functionsApi } from '../../../../api/functions'
import CommitDialog from '../../../../components/CommitDialog'
import { FunctionPicker } from './FunctionPicker'
import { FlowPicker } from './FlowPicker'
import type {
  EditorTab,
  TriggerProperties,
  TriggerNode,
  TriggerType,
  TriggerExecutionMode,
  RaisinReference,
} from '../../types'

interface RaisinTriggerNodeTypeEditorProps {
  tab: EditorTab
}

/** Default trigger properties */
const DEFAULT_PROPERTIES: Partial<TriggerProperties> = {
  name: '',
  title: '',
  trigger_type: 'node_event',
  config: {
    event_kinds: ['Created', 'Updated'],
    cron_expression: '0 * * * *',
    methods: ['POST'],
    path_suffix: '',
  },
  filters: {},
  enabled: true,
  priority: 0,
}

function deriveFunctions(props: Partial<TriggerProperties>): string[] {
  if (props.function_flow?.steps?.length) {
    return props.function_flow.steps
      .flatMap(step => step.functions?.map(f => f.path).filter(Boolean) || [])
      .filter((v, i, arr) => !!v && arr.indexOf(v) === i) as string[]
  }
  if (props.function_path) return [props.function_path]
  return []
}

function flowFromFunctions(functions: string[]) {
  return {
    version: 1,
    error_strategy: 'fail_fast' as const,
    steps: functions.map((path, idx) => ({
      id: `step-${idx + 1}`,
      name: path.split('/').pop() || `Step ${idx + 1}`,
      functions: [{ path }],
      parallel: false,
      depends_on: [],
      on_error: 'stop' as const,
    })),
  }
}

function normalizeArrayInput(value: string): string[] | undefined {
  const items = value
    .split(',')
    .map(v => v.trim())
    .filter(Boolean)
  return items.length ? items : undefined
}

/** Parse query string into object */
function parseQueryParams(queryString: string): Record<string, string> {
  if (!queryString.trim()) return {}
  const params: Record<string, string> = {}
  queryString.split('&').forEach(pair => {
    const [key, value] = pair.split('=')
    if (key) params[key.trim()] = value?.trim() || ''
  })
  return params
}

/** Copy button component */
function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    await navigator.clipboard.writeText(text)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  return (
    <button onClick={handleCopy} className="p-1 hover:bg-white/10 rounded" title="Copy to clipboard">
      {copied ? <Check className="w-3 h-3 text-green-400" /> : <Copy className="w-3 h-3 text-gray-400" />}
    </button>
  )
}

export function RaisinTriggerNodeTypeEditor({ tab }: RaisinTriggerNodeTypeEditorProps) {
  const {
    repo,
    branch,
    workspace,
    nodes,
    markTabDirty,
    loadRootNodes,
    addLog,
  } = useFunctionsContext()

  // State
  const [isLoading, setIsLoading] = useState(true)
  const [pendingCommit, setPendingCommit] = useState<{
    properties: Partial<TriggerProperties>
  } | null>(null)
  const [triggerNode, setTriggerNode] = useState<TriggerNode | null>(null)
  const [showFunctionPicker, setShowFunctionPicker] = useState(false)
  const [selectedFunctions, setSelectedFunctions] = useState<string[]>([])

  // Execution mode state (functions vs flow)
  const [executionMode, setExecutionMode] = useState<TriggerExecutionMode>('functions')
  const [selectedFlow, setSelectedFlow] = useState<RaisinReference | null>(null)
  const [showFlowPicker, setShowFlowPicker] = useState(false)

  // Test execution state
  const [testMethod, setTestMethod] = useState<'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'>('GET')
  const [testQueryParams, setTestQueryParams] = useState('')
  const [testBody, setTestBody] = useState('{}')
  const [isTestRunning, setIsTestRunning] = useState(false)

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
  } = useUndoRedo<Partial<TriggerProperties>>(DEFAULT_PROPERTIES)

  // Ref for keyboard shortcuts
  const containerRef = useRef<HTMLDivElement>(null)

  // Find trigger node from tree
  const findTriggerNode = useCallback(
    (nodeList: typeof nodes, path: string): TriggerNode | null => {
      for (const node of nodeList) {
        if (node.path === path && node.node_type === 'raisin:Trigger') {
          return node as unknown as TriggerNode
        }
        if (node.children && Array.isArray(node.children)) {
          const found = findTriggerNode(node.children as typeof nodes, path)
          if (found) return found
        }
      }
      return null
    },
    []
  )

  // Load trigger node data
  useEffect(() => {
    const loadNode = async () => {
      setIsLoading(true)
      try {
        // First try to find in tree
        let node = findTriggerNode(nodes, tab.path)

        // If not found, fetch from server
        if (!node) {
          const fetchedNode = await nodesApi.getAtHead(repo, branch, workspace, tab.path)
          if (fetchedNode.node_type === 'raisin:Trigger') {
            node = fetchedNode as unknown as TriggerNode
          }
        }

        if (node) {
          setTriggerNode(node)
          const props: Partial<TriggerProperties> = {
            name: node.properties?.name || node.name,
            title: node.properties?.title || '',
            description: node.properties?.description || '',
            trigger_type: node.properties?.trigger_type || 'node_event',
            config: node.properties?.config || { event_kinds: ['Created', 'Updated'] },
            filters: node.properties?.filters || {},
            enabled: node.properties?.enabled !== false,
            priority: node.properties?.priority || 0,
            execution_mode: node.properties?.execution_mode,
            flow_ref: node.properties?.flow_ref,
            function_path: node.properties?.function_path,
            function_flow: node.properties?.function_flow,
          }
          resetProperties(props)

          // Initialize execution mode and selected target
          if (node.properties?.flow_ref) {
            setExecutionMode('flow')
            setSelectedFlow(node.properties.flow_ref as RaisinReference)
            setSelectedFunctions([])
          } else {
            setExecutionMode('functions')
            setSelectedFlow(null)
            const funcs = deriveFunctions(props)
            setSelectedFunctions(funcs)
          }
        }
      } catch (error) {
        console.error('Failed to load trigger node:', error)
      } finally {
        setIsLoading(false)
      }
    }

    loadNode()
  }, [tab.path, repo, branch, workspace, nodes, findTriggerNode, resetProperties])

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

  const syncFunctionsToFlow = useCallback((nextFunctions: string[]) => {
    setSelectedFunctions(nextFunctions)
    setProperties({
      ...properties,
      execution_mode: 'functions',
      // Only use function_flow for multiple functions - single function uses function_path
      function_flow: nextFunctions.length > 1 ? flowFromFunctions(nextFunctions) : undefined,
      function_path: nextFunctions.length === 1 ? nextFunctions[0] : undefined,
      // Clear flow_ref when switching to functions mode
      flow_ref: undefined,
    })
  }, [properties, setProperties])

  // Sync flow reference to properties (for undo/redo support)
  const syncFlowRef = useCallback((flowRef: RaisinReference | null) => {
    setSelectedFlow(flowRef)
    setProperties({
      ...properties,
      execution_mode: 'flow',
      flow_ref: flowRef || undefined,
      // Clear function properties when switching to flow mode
      function_flow: undefined,
      function_path: undefined,
    })
  }, [properties, setProperties])

  // Handle execution mode change
  const handleExecutionModeChange = useCallback((mode: TriggerExecutionMode) => {
    setExecutionMode(mode)
    setProperties({
      ...properties,
      execution_mode: mode,
    })
  }, [properties, setProperties])

  // Handle add function
  const handleAddFunction = useCallback(() => {
    setShowFunctionPicker(true)
  }, [])

  // Handle function selection from picker
  const handleFunctionSelected = useCallback(
    (functionPath: string) => {
      const exists = selectedFunctions.includes(functionPath)
      const next = exists ? selectedFunctions : [...selectedFunctions, functionPath]
      syncFunctionsToFlow(next)
      setShowFunctionPicker(false)
    },
    [selectedFunctions, syncFunctionsToFlow]
  )

  const handleRemoveFunction = useCallback((path: string) => {
    const next = selectedFunctions.filter(p => p !== path)
    syncFunctionsToFlow(next)
  }, [selectedFunctions, syncFunctionsToFlow])

  // Keep selectedFunctions in sync when undo/redo changes function_flow
  useEffect(() => {
    const derived = deriveFunctions(properties)
    if (derived.join('|') !== selectedFunctions.join('|')) {
      setSelectedFunctions(derived)
    }
  }, [properties.function_flow, properties.function_path, selectedFunctions])

  // Keep executionMode and selectedFlow in sync when undo/redo changes properties
  useEffect(() => {
    const propsMode = properties.execution_mode || (properties.flow_ref ? 'flow' : 'functions')
    if (propsMode !== executionMode) {
      setExecutionMode(propsMode)
    }
    const propsFlowRef = properties.flow_ref as RaisinReference | undefined
    const currentPath = selectedFlow?.['raisin:path']
    const propsPath = propsFlowRef?.['raisin:path']
    if (propsPath !== currentPath) {
      setSelectedFlow(propsFlowRef || null)
    }
  }, [properties.execution_mode, properties.flow_ref, executionMode, selectedFlow])

  // Handle property changes
  const handlePropertyChange = useCallback(
    (field: keyof TriggerProperties, value: unknown) => {
      setProperties({
        ...properties,
        [field]: value,
      })
    },
    [properties, setProperties]
  )

  // Compute webhook URLs for HTTP triggers
  const pathSuffix = properties.config?.path_suffix || ''
  const triggerNameUrl = properties.trigger_type === 'http' && properties.name
    ? `/api/triggers/${repo}/${properties.name}${pathSuffix}`
    : null
  const webhookIdUrl = properties.trigger_type === 'http' && properties.webhook_id
    ? `/api/webhooks/${repo}/${properties.webhook_id}${pathSuffix}`
    : null

  // Handle test execution
  const handleTestExecute = useCallback(async () => {
    if (!properties.name || !repo) return

    setIsTestRunning(true)
    const queryStr = testQueryParams ? `?${testQueryParams}` : ''
    addLog({
      level: 'info',
      message: `Testing ${testMethod} /api/triggers/${repo}/${properties.name}${queryStr}...`,
      timestamp: new Date().toISOString(),
    })

    try {
      let body: Record<string, unknown> | undefined
      if (['POST', 'PUT', 'PATCH'].includes(testMethod)) {
        try {
          body = JSON.parse(testBody)
        } catch {
          addLog({ level: 'error', message: 'Invalid JSON body', timestamp: new Date().toISOString() })
          setIsTestRunning(false)
          return
        }
      }

      const response = await functionsApi.invokeTrigger(repo, properties.name, {
        method: testMethod,
        query_params: parseQueryParams(testQueryParams),
        body,
        sync: true,
        timeout_ms: 30000,
      })

      if (response.error) {
        addLog({ level: 'error', message: `Test failed: ${response.error}`, timestamp: new Date().toISOString() })
      } else {
        addLog({ level: 'info', message: `Test completed in ${response.duration_ms}ms`, timestamp: new Date().toISOString() })
        addLog({ level: 'info', message: `Result: ${JSON.stringify(response.result, null, 2)}`, timestamp: new Date().toISOString() })
      }

      // Show logs from execution
      response.logs?.forEach(log => {
        addLog({ level: 'info', message: log, timestamp: new Date().toISOString() })
      })

    } catch (error) {
      addLog({
        level: 'error',
        message: `Test error: ${error instanceof Error ? error.message : String(error)}`,
        timestamp: new Date().toISOString(),
      })
    } finally {
      setIsTestRunning(false)
    }
  }, [repo, properties.name, testMethod, testQueryParams, testBody, addLog])

  // Handle save
  const handleSave = useCallback(() => {
    if (!isDirty) return

    // Validate based on execution mode
    if (executionMode === 'functions' && selectedFunctions.length === 0) {
      addLog({
        level: 'error',
        message: 'Select at least one function for this trigger.',
        timestamp: new Date().toISOString(),
      })
      return
    }

    if (executionMode === 'flow' && !selectedFlow) {
      addLog({
        level: 'error',
        message: 'Select a flow for this trigger.',
        timestamp: new Date().toISOString(),
      })
      return
    }

    // Build properties based on execution mode
    const updatedProperties: Partial<TriggerProperties> = {
      ...properties,
      execution_mode: executionMode,
    }

    if (executionMode === 'flow') {
      // Flow mode: set flow_ref, clear function properties
      updatedProperties.flow_ref = selectedFlow || undefined
      updatedProperties.function_flow = undefined
      updatedProperties.function_path = undefined
    } else {
      // Functions mode: set function properties, clear flow_ref
      updatedProperties.flow_ref = undefined
      updatedProperties.function_flow = selectedFunctions.length > 1 ? flowFromFunctions(selectedFunctions) : undefined
      updatedProperties.function_path = selectedFunctions.length === 1 ? selectedFunctions[0] : undefined
    }

    setPendingCommit({ properties: updatedProperties })
  }, [isDirty, properties, addLog, executionMode, selectedFunctions, selectedFlow])

  // Execute save with commit message
  const executeCommit = useCallback(
    async (message: string, actor: string) => {
      if (!pendingCommit || !triggerNode) return

      try {
        await nodesApi.update(repo, branch, workspace, tab.path, {
          properties: {
            ...triggerNode.properties,
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
          message: `Trigger "${properties.name}" saved`,
          timestamp: new Date().toISOString(),
        })
      } catch (error) {
        console.error('Failed to save trigger:', error)
        addLog({
          level: 'error',
          message: `Failed to save: ${error instanceof Error ? error.message : String(error)}`,
          timestamp: new Date().toISOString(),
        })
      } finally {
        setPendingCommit(null)
      }
    },
    [
      pendingCommit,
      triggerNode,
      repo,
      branch,
      workspace,
      tab.path,
      resetProperties,
      loadRootNodes,
      addLog,
      properties.name,
    ]
  )

  // Loading state
  if (isLoading) {
    return (
      <div className="h-full flex items-center justify-center text-gray-400">
        <Loader2 className="w-6 h-6 animate-spin mr-2" />
        Loading trigger...
      </div>
    )
  }

  // Error state - trigger not found
  if (!triggerNode) {
    return (
      <div className="h-full flex flex-col items-center justify-center text-gray-400">
        <Zap className="w-16 h-16 mb-4 opacity-50" />
        <p className="text-lg">Trigger not found</p>
      </div>
    )
  }

  return (
    <div ref={containerRef} className="h-full flex flex-col focus:outline-none" tabIndex={0}>
      {/* Toolbar */}
      <div className="flex-shrink-0 flex items-center gap-2 px-3 py-1.5 bg-black/20 border-b border-white/10">
        {/* Save Button */}
        <button
          onClick={handleSave}
          disabled={!isDirty}
          className={`flex items-center gap-1.5 px-2 py-1 rounded text-sm
            ${
              isDirty
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
            ${canUndo ? 'text-gray-300 hover:bg-white/10' : 'text-gray-600 cursor-not-allowed'}
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
            ${canRedo ? 'text-gray-300 hover:bg-white/10' : 'text-gray-600 cursor-not-allowed'}
          `}
          title="Redo (Ctrl+Shift+Z)"
        >
          <Redo2 className="w-4 h-4" />
        </button>

        <div className="flex-1" />

        {/* Enable/Disable Toggle */}
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={properties.enabled !== false}
            onChange={(e) => handlePropertyChange('enabled', e.target.checked)}
            className="rounded bg-black/20 border-white/20"
          />
          <span className={properties.enabled ? 'text-green-400' : 'text-gray-500'}>
            {properties.enabled ? 'Enabled' : 'Disabled'}
          </span>
        </label>
      </div>

      {/* Content */}
      <div className="flex-1 flex min-h-0">
        {/* Left panel - Properties */}
        <div className="w-80 border-r border-white/10 overflow-auto p-4">
          {/* Header */}
          <div className="mb-6">
            <div className="flex items-center gap-2 mb-1">
              <Zap className="w-5 h-5 text-yellow-400" />
              <h2 className="text-lg font-semibold text-white">
                {properties.name || triggerNode.name}
              </h2>
            </div>
            <p className="text-sm text-gray-400">{tab.path}</p>
          </div>

          {/* Trigger Properties */}
          <div className="space-y-4">
            <h3 className="text-sm font-medium text-gray-300 flex items-center gap-2">
              <Settings className="w-4 h-4 text-gray-400" />
              Trigger Settings
            </h3>

            {/* Name */}
            <div>
              <label className="block text-xs text-gray-400 mb-1">Name</label>
              <input
                type="text"
                value={properties.name || ''}
                onChange={(e) => handlePropertyChange('name', e.target.value)}
                className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm
                  focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
              />
            </div>

            {/* Title */}
            <div>
              <label className="block text-xs text-gray-400 mb-1">Title</label>
              <input
                type="text"
                value={properties.title || ''}
                onChange={(e) => handlePropertyChange('title', e.target.value)}
                className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm
                  focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
              />
            </div>

            {/* Trigger Type */}
            <div>
              <label className="block text-xs text-gray-400 mb-1">Trigger Type</label>
              <select
                value={properties.trigger_type || 'node_event'}
                onChange={(e) => handlePropertyChange('trigger_type', e.target.value as TriggerType)}
                className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm
                  focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
              >
                <option value="node_event">Node Event</option>
                <option value="schedule">Schedule</option>
                <option value="http">HTTP</option>
              </select>
            </div>

            {/* Event Kinds (for node_event type) */}
            {properties.trigger_type === 'node_event' && (
              <div>
                <label className="block text-xs text-gray-400 mb-1">Event Kinds</label>
                <div className="space-y-1">
                  {(['Created', 'Updated', 'Deleted', 'Published'] as const).map((kind) => {
                    const eventKinds = Array.isArray(properties.config?.event_kinds)
                      ? properties.config.event_kinds
                      : []
                    return (
                      <label key={kind} className="flex items-center gap-2 text-sm text-gray-300">
                        <input
                          type="checkbox"
                          checked={eventKinds.includes(kind)}
                          onChange={(e) => {
                            const newKinds = e.target.checked
                              ? [...eventKinds, kind]
                              : eventKinds.filter((k) => k !== kind)
                            handlePropertyChange('config', {
                              ...properties.config,
                              event_kinds: newKinds,
                            })
                          }}
                          className="rounded bg-black/20 border-white/20"
                        />
                        {kind}
                      </label>
                    )
                  })}
                </div>
              </div>
            )}

            {/* Priority */}
            <div>
              <label className="block text-xs text-gray-400 mb-1">Priority</label>
              <input
                type="number"
                value={properties.priority || 0}
                onChange={(e) => handlePropertyChange('priority', parseInt(e.target.value, 10))}
                className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm
                  focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
              />
              <p className="text-xs text-gray-500 mt-1">Higher priority triggers execute first</p>
            </div>

            {/* Max Retries */}
            <div>
              <label className="block text-xs text-gray-400 mb-1">Max Retries</label>
              <input
                type="number"
                min={0}
                max={10}
                value={properties.max_retries ?? 3}
                onChange={(e) => handlePropertyChange('max_retries', parseInt(e.target.value, 10))}
                className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm
                  focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
              />
              <p className="text-xs text-gray-500 mt-1">0 = no retries on failure (default: 3)</p>
            </div>

            {/* Description */}
            <div>
              <label className="block text-xs text-gray-400 mb-1">Description</label>
              <textarea
                value={properties.description || ''}
                onChange={(e) => handlePropertyChange('description', e.target.value)}
                rows={3}
                className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm
                  focus:border-primary-500 focus:ring-1 focus:ring-primary-500 resize-none"
              />
            </div>
          </div>
        </div>

        {/* Right panel - Functions + Config */}
        <div className="flex-1 p-4 space-y-6 overflow-auto">
          <div className="bg-black/20 border border-white/10 rounded-lg p-4">
            <div className="flex items-center gap-2 mb-3">
              <Settings className="w-4 h-4 text-gray-400" />
              <h3 className="text-sm font-medium text-gray-200">Configuration</h3>
            </div>

            {properties.trigger_type === 'node_event' && (
              <div className="space-y-2">
                <label className="block text-xs text-gray-400">Event Kinds</label>
                <div className="grid grid-cols-2 gap-2">
                  {(['Created', 'Updated', 'Deleted', 'Published', 'Unpublished', 'Moved', 'Renamed'] as const).map((kind) => {
                    const eventKinds = Array.isArray(properties.config?.event_kinds)
                      ? properties.config!.event_kinds!
                      : []
                    return (
                      <label key={kind} className="flex items-center gap-2 text-sm text-gray-200">
                        <input
                          type="checkbox"
                          checked={eventKinds.includes(kind)}
                          onChange={(e) => {
                            const newKinds = e.target.checked
                              ? [...eventKinds, kind]
                              : eventKinds.filter((k) => k !== kind)
                            handlePropertyChange('config', {
                              ...properties.config,
                              event_kinds: newKinds,
                            })
                          }}
                          className="rounded bg-black/20 border-white/20"
                        />
                        {kind}
                      </label>
                    )
                  })}
                </div>
              </div>
            )}

            {properties.trigger_type === 'schedule' && (
              <div className="space-y-2">
                <label className="block text-xs text-gray-400">Cron Expression</label>
                <input
                  type="text"
                  value={properties.config?.cron_expression || ''}
                  onChange={(e) =>
                    handlePropertyChange('config', {
                      ...properties.config,
                      cron_expression: e.target.value,
                    })
                  }
                  className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
                  placeholder="0 * * * *"
                />
              </div>
            )}

            {properties.trigger_type === 'http' && (
              <div className="space-y-4">
                {/* Webhook URLs */}
                <div className="space-y-2 p-3 bg-black/30 rounded-lg">
                  <div className="text-xs text-gray-400 font-medium flex items-center gap-2">
                    <Globe className="w-4 h-4 text-green-400" />
                    Webhook URLs
                  </div>

                  {/* By Trigger Name */}
                  {triggerNameUrl && (
                    <div className="flex items-center gap-2">
                      <span className="text-xs text-gray-500 w-20 flex-shrink-0">By Name:</span>
                      <code className="flex-1 text-xs text-green-400 font-mono truncate">{triggerNameUrl}</code>
                      <CopyButton text={triggerNameUrl} />
                    </div>
                  )}

                  {/* By Webhook ID */}
                  {webhookIdUrl && (
                    <div className="flex items-center gap-2">
                      <span className="text-xs text-gray-500 w-20 flex-shrink-0">By ID:</span>
                      <code className="flex-1 text-xs text-blue-400 font-mono truncate">{webhookIdUrl}</code>
                      <CopyButton text={webhookIdUrl} />
                    </div>
                  )}

                  {!triggerNameUrl && !webhookIdUrl && (
                    <p className="text-xs text-gray-500">Set a trigger name to see webhook URLs</p>
                  )}
                </div>

                {/* Allowed Methods */}
                <div>
                  <label className="block text-xs text-gray-400 mb-1">Allowed Methods</label>
                  <div className="grid grid-cols-3 gap-2">
                    {(['GET', 'POST', 'PUT', 'PATCH', 'DELETE'] as const).map((method) => {
                      const methods = Array.isArray(properties.config?.methods)
                        ? properties.config!.methods!
                        : []
                      return (
                        <label key={method} className="flex items-center gap-2 text-sm text-gray-200">
                          <input
                            type="checkbox"
                            checked={methods.includes(method)}
                            onChange={(e) => {
                              const next = e.target.checked
                                ? [...methods, method]
                                : methods.filter((m) => m !== method)
                              handlePropertyChange('config', {
                                ...properties.config,
                                methods: next,
                              })
                            }}
                            className="rounded bg-black/20 border-white/20"
                          />
                          {method}
                        </label>
                      )
                    })}
                  </div>
                </div>

                {/* Path Suffix */}
                <div>
                  <label className="block text-xs text-gray-400 mb-1">Path Suffix</label>
                  <input
                    type="text"
                    value={properties.config?.path_suffix || ''}
                    onChange={(e) =>
                      handlePropertyChange('config', {
                        ...properties.config,
                        path_suffix: e.target.value,
                      })
                    }
                    className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
                    placeholder="/my-hook"
                  />
                </div>

                {/* Test Webhook Section */}
                <div className="border-t border-white/10 pt-4">
                  <div className="text-sm font-medium text-white mb-3">Test Webhook</div>

                  {/* Method Selector */}
                  <div className="flex gap-2 mb-3">
                    {(['GET', 'POST', 'PUT', 'PATCH', 'DELETE'] as const).map(method => (
                      <button
                        key={method}
                        onClick={() => setTestMethod(method)}
                        className={`px-3 py-1 text-xs rounded transition-colors ${
                          testMethod === method
                            ? 'bg-primary-500 text-white'
                            : 'bg-white/10 text-gray-400 hover:bg-white/20'
                        }`}
                      >
                        {method}
                      </button>
                    ))}
                  </div>

                  {/* Query Parameters */}
                  <div className="mb-3">
                    <label className="text-xs text-gray-400 mb-1 block">Query Parameters</label>
                    <input
                      type="text"
                      value={testQueryParams}
                      onChange={(e) => setTestQueryParams(e.target.value)}
                      className="w-full bg-black/30 border border-white/10 rounded px-3 py-2 text-xs font-mono text-white focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
                      placeholder="key1=value1&key2=value2"
                    />
                  </div>

                  {/* Request Body (for POST/PUT/PATCH) */}
                  {['POST', 'PUT', 'PATCH'].includes(testMethod) && (
                    <div className="mb-3">
                      <label className="text-xs text-gray-400 mb-1 block">Request Body (JSON)</label>
                      <textarea
                        value={testBody}
                        onChange={(e) => setTestBody(e.target.value)}
                        className="w-full h-24 bg-black/30 border border-white/10 rounded p-2 text-xs font-mono text-white focus:border-primary-500 focus:ring-1 focus:ring-primary-500 resize-none"
                        placeholder='{"key": "value"}'
                      />
                    </div>
                  )}

                  {/* Execute Button */}
                  <button
                    onClick={handleTestExecute}
                    disabled={isTestRunning || !properties.enabled || !properties.name}
                    className="flex items-center gap-2 px-4 py-2 bg-green-600 hover:bg-green-500
                               disabled:opacity-50 disabled:cursor-not-allowed rounded text-sm text-white transition-colors"
                  >
                    {isTestRunning ? (
                      <Loader2 className="w-4 h-4 animate-spin" />
                    ) : (
                      <Play className="w-4 h-4" />
                    )}
                    {isTestRunning ? 'Running...' : 'Execute Test'}
                  </button>

                  {!properties.enabled && (
                    <p className="text-xs text-yellow-500 mt-2">Enable the trigger to test it</p>
                  )}
                </div>
              </div>
            )}

            {/* Filters */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3 mt-4">
              <div>
                <label className="block text-xs text-gray-400 mb-1">Workspaces (comma separated)</label>
                <input
                  type="text"
                  value={(properties.filters?.workspaces || []).join(', ')}
                  onChange={(e) =>
                    handlePropertyChange('filters', {
                      ...properties.filters,
                      workspaces: normalizeArrayInput(e.target.value),
                    })
                  }
                  className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
                  placeholder="content, functions"
                />
              </div>
              <div>
                <label className="block text-xs text-gray-400 mb-1">Paths (glob, comma separated)</label>
                <input
                  type="text"
                  value={(properties.filters?.paths || []).join(', ')}
                  onChange={(e) =>
                    handlePropertyChange('filters', {
                      ...properties.filters,
                      paths: normalizeArrayInput(e.target.value),
                    })
                  }
                  className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
                  placeholder="/content/**"
                />
              </div>
              <div>
                <label className="block text-xs text-gray-400 mb-1">Node Types (comma separated)</label>
                <input
                  type="text"
                  value={(properties.filters?.node_types || []).join(', ')}
                  onChange={(e) =>
                    handlePropertyChange('filters', {
                      ...properties.filters,
                      node_types: normalizeArrayInput(e.target.value),
                    })
                  }
                  className="w-full px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
                  placeholder="raisin:Page, raisin:Asset"
                />
              </div>

              {/* Property Filters */}
              <div className="col-span-1 md:col-span-2 border-t border-white/10 pt-3 mt-1">
                <label className="block text-xs text-gray-400 mb-2">Property Filters</label>
                <p className="text-xs text-gray-500 mb-2">
                  Only trigger when the node has these specific property values.
                </p>
                <p className="text-xs text-gray-600 mb-3">
                  Supports nested paths (e.g., <code className="text-primary-400">file.metadata.storage_key</code>) and operators like{' '}
                  <code className="text-green-400">{'{"$exists": true}'}</code>,{' '}
                  <code className="text-green-400">{'{"$eq": "value"}'}</code>,{' '}
                  <code className="text-green-400">{'{"$ne": "value"}'}</code>,{' '}
                  <code className="text-green-400">{'{"$gt": 10}'}</code>,{' '}
                  <code className="text-green-400">{'{"$in": ["a", "b"]}'}</code>
                </p>

                <div className="space-y-2">
                  {Object.entries(properties.filters?.property_filters || {}).map(([key, value]) => {
                    // Format the value display nicely
                    const isOperator = typeof value === 'object' && value !== null && !Array.isArray(value)
                    const operatorKey = isOperator ? Object.keys(value)[0] : null
                    const operatorValue = isOperator ? (value as Record<string, unknown>)[operatorKey!] : null

                    return (
                      <div key={key} className="flex items-center gap-2">
                        <div className="flex-1 px-3 py-2 rounded bg-black/30 border border-white/10 text-sm text-gray-300 font-mono">
                          {key}
                        </div>
                        <div className="flex-1 px-3 py-2 rounded bg-black/30 border border-white/10 text-sm font-mono">
                          {isOperator && operatorKey ? (
                            <span>
                              <span className="text-green-400">{operatorKey}</span>
                              <span className="text-gray-500">: </span>
                              <span className="text-yellow-300">{JSON.stringify(operatorValue)}</span>
                            </span>
                          ) : (
                            <span className="text-gray-300">{JSON.stringify(value)}</span>
                          )}
                        </div>
                        <button
                          onClick={() => {
                            const next = { ...(properties.filters?.property_filters || {}) }
                            delete next[key]
                            handlePropertyChange('filters', {
                              ...properties.filters,
                              property_filters: Object.keys(next).length > 0 ? next : undefined,
                            })
                          }}
                          className="p-2 hover:bg-white/10 rounded text-gray-400 hover:text-red-400"
                          title="Remove filter"
                        >
                          <Trash2 className="w-4 h-4" />
                        </button>
                      </div>
                    )
                  })}

                  {/* Quick Add: $exists filter */}
                  <div className="flex items-center gap-2 pt-2 border-t border-white/5">
                    <input
                      type="text"
                      id="new-prop-key"
                      placeholder="Property path (e.g. file.metadata.storage_key)"
                      className="flex-1 px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm focus:border-primary-500 focus:ring-1 focus:ring-primary-500 font-mono"
                    />
                    <select
                      id="new-prop-operator"
                      className="px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
                      defaultValue="eq"
                    >
                      <option value="eq">= (equals)</option>
                      <option value="ne">≠ (not equal)</option>
                      <option value="exists">∃ (exists)</option>
                      <option value="not_exists">∄ (not exists)</option>
                      <option value="gt">&gt; (greater than)</option>
                      <option value="gte">≥ (greater or equal)</option>
                      <option value="lt">&lt; (less than)</option>
                      <option value="lte">≤ (less or equal)</option>
                      <option value="in">∈ (in array)</option>
                    </select>
                    <input
                      type="text"
                      id="new-prop-value"
                      placeholder="Value"
                      className="flex-1 px-3 py-2 rounded bg-black/30 border border-white/10 text-white text-sm focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          const keyInput = document.getElementById('new-prop-key') as HTMLInputElement
                          const opSelect = document.getElementById('new-prop-operator') as HTMLSelectElement
                          const valInput = document.getElementById('new-prop-value') as HTMLInputElement
                          if (keyInput.value) {
                            const key = keyInput.value
                            const op = opSelect.value
                            let val: unknown = valInput.value

                            // Parse value based on operator
                            if (op === 'exists') {
                              val = { $exists: true }
                            } else if (op === 'not_exists') {
                              val = { $exists: false }
                            } else {
                              // Try JSON parse first (for arrays, objects)
                              try {
                                val = JSON.parse(valInput.value)
                              } catch {
                                // Simple type inference
                                if (valInput.value === 'true') val = true
                                else if (valInput.value === 'false') val = false
                                else if (!isNaN(Number(valInput.value)) && valInput.value.trim() !== '') val = Number(valInput.value)
                                else val = valInput.value
                              }

                              // Wrap in operator if not simple equals
                              if (op === 'ne') val = { $ne: val }
                              else if (op === 'gt') val = { $gt: val }
                              else if (op === 'gte') val = { $gte: val }
                              else if (op === 'lt') val = { $lt: val }
                              else if (op === 'lte') val = { $lte: val }
                              else if (op === 'in') {
                                // Ensure it's an array
                                if (!Array.isArray(val)) val = [val]
                                val = { $in: val }
                              }
                              // 'eq' stays as plain value for backward compatibility
                            }

                            handlePropertyChange('filters', {
                              ...properties.filters,
                              property_filters: {
                                ...(properties.filters?.property_filters || {}),
                                [key]: val
                              }
                            })

                            keyInput.value = ''
                            valInput.value = ''
                            opSelect.value = 'eq'
                            keyInput.focus()
                          }
                        }
                      }}
                    />
                    <button
                      onClick={() => {
                        const keyInput = document.getElementById('new-prop-key') as HTMLInputElement
                        const opSelect = document.getElementById('new-prop-operator') as HTMLSelectElement
                        const valInput = document.getElementById('new-prop-value') as HTMLInputElement
                        if (keyInput.value) {
                          const key = keyInput.value
                          const op = opSelect.value
                          let val: unknown = valInput.value

                          // Parse value based on operator
                          if (op === 'exists') {
                            val = { $exists: true }
                          } else if (op === 'not_exists') {
                            val = { $exists: false }
                          } else {
                            // Try JSON parse first (for arrays, objects)
                            try {
                              val = JSON.parse(valInput.value)
                            } catch {
                              // Simple type inference
                              if (valInput.value === 'true') val = true
                              else if (valInput.value === 'false') val = false
                              else if (!isNaN(Number(valInput.value)) && valInput.value.trim() !== '') val = Number(valInput.value)
                              else val = valInput.value
                            }

                            // Wrap in operator if not simple equals
                            if (op === 'ne') val = { $ne: val }
                            else if (op === 'gt') val = { $gt: val }
                            else if (op === 'gte') val = { $gte: val }
                            else if (op === 'lt') val = { $lt: val }
                            else if (op === 'lte') val = { $lte: val }
                            else if (op === 'in') {
                              // Ensure it's an array
                              if (!Array.isArray(val)) val = [val]
                              val = { $in: val }
                            }
                            // 'eq' stays as plain value for backward compatibility
                          }

                          handlePropertyChange('filters', {
                            ...properties.filters,
                            property_filters: {
                              ...(properties.filters?.property_filters || {}),
                              [key]: val
                            }
                          })

                          keyInput.value = ''
                          valInput.value = ''
                          opSelect.value = 'eq'
                          keyInput.focus()
                        }
                      }}
                      className="p-2 bg-primary-500/20 text-primary-200 rounded hover:bg-primary-500/30"
                      title="Add filter"
                    >
                      <Plus className="w-4 h-4" />
                    </button>
                  </div>

                  {/* Hint for $exists usage */}
                  <p className="text-xs text-gray-600 mt-2">
                    Tip: Use <code className="text-primary-400">file.metadata.storage_key</code> with <code className="text-green-400">∃ exists</code> to trigger only when a file upload is complete.
                  </p>
                </div>
              </div>
            </div>
          </div>

          {/* Execution Target */}
          <div className="bg-black/20 border border-white/10 rounded-lg p-4">
            <div className="flex items-center gap-2 mb-3">
              <Settings className="w-4 h-4 text-gray-400" />
              <h3 className="text-sm font-medium text-gray-200">Execution Target</h3>
            </div>

            {/* Mode Toggle */}
            <div className="flex gap-1 p-1 bg-black/30 rounded-lg mb-4">
              <button
                onClick={() => handleExecutionModeChange('functions')}
                className={`flex-1 flex items-center justify-center gap-2 px-3 py-2 text-sm rounded-md transition-colors ${
                  executionMode === 'functions'
                    ? 'bg-primary-500/30 text-primary-200'
                    : 'text-gray-400 hover:text-gray-200 hover:bg-white/5'
                }`}
              >
                <Play className="w-4 h-4" />
                Functions
              </button>
              <button
                onClick={() => handleExecutionModeChange('flow')}
                className={`flex-1 flex items-center justify-center gap-2 px-3 py-2 text-sm rounded-md transition-colors ${
                  executionMode === 'flow'
                    ? 'bg-green-500/30 text-green-200'
                    : 'text-gray-400 hover:text-gray-200 hover:bg-white/5'
                }`}
              >
                <GitBranch className="w-4 h-4" />
                Flow
              </button>
            </div>

            {/* Functions Mode Content */}
            {executionMode === 'functions' && (
              <div>
                <div className="flex items-center justify-between mb-3">
                  <span className="text-xs text-gray-500">Functions to execute in order</span>
                  <button
                    onClick={handleAddFunction}
                    className="flex items-center gap-2 px-2 py-1 text-sm bg-primary-500/20 text-primary-200 rounded hover:bg-primary-500/30"
                  >
                    <Plus className="w-4 h-4" />
                    Add Function
                  </button>
                </div>

                {selectedFunctions.length === 0 ? (
                  <div className="text-sm text-gray-500 py-4 text-center border border-dashed border-white/10 rounded">
                    No functions selected yet.
                  </div>
                ) : (
                  <div className="space-y-2">
                    {selectedFunctions.map((fnPath) => (
                      <div
                        key={fnPath}
                        className="flex items-center justify-between px-3 py-2 bg-black/30 border border-white/5 rounded"
                      >
                        <div className="flex flex-col">
                          <span className="text-sm text-white">{fnPath.split('/').pop()}</span>
                          <span className="text-xs text-gray-500">{fnPath}</span>
                        </div>
                        <button
                          onClick={() => handleRemoveFunction(fnPath)}
                          className="p-1 text-gray-400 hover:text-red-400"
                          title="Remove function"
                        >
                          <Trash2 className="w-4 h-4" />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}

            {/* Flow Mode Content */}
            {executionMode === 'flow' && (
              <div>
                <span className="text-xs text-gray-500 block mb-3">Select a flow to execute</span>

                {selectedFlow ? (
                  <div className="flex items-center justify-between px-3 py-2 bg-black/30 border border-green-500/30 rounded">
                    <div className="flex items-center gap-2">
                      <GitBranch className="w-4 h-4 text-green-400 flex-shrink-0" />
                      <div className="flex flex-col">
                        <span className="text-sm text-white">{selectedFlow['raisin:path'].split('/').pop()}</span>
                        <span className="text-xs text-gray-500">{selectedFlow['raisin:path']}</span>
                      </div>
                    </div>
                    <div className="flex items-center gap-1">
                      <button
                        onClick={() => setShowFlowPicker(true)}
                        className="p-1 text-gray-400 hover:text-white"
                        title="Change flow"
                      >
                        <Link2 className="w-4 h-4" />
                      </button>
                      <button
                        onClick={() => syncFlowRef(null)}
                        className="p-1 text-gray-400 hover:text-red-400"
                        title="Remove flow"
                      >
                        <Unlink className="w-4 h-4" />
                      </button>
                    </div>
                  </div>
                ) : (
                  <button
                    onClick={() => setShowFlowPicker(true)}
                    className="w-full flex items-center justify-center gap-2 px-3 py-3 text-sm bg-green-500/10 border border-green-500/30 rounded text-green-400 hover:bg-green-500/20 transition-colors"
                  >
                    <GitBranch className="w-4 h-4" />
                    Select Flow
                  </button>
                )}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Function Picker Modal */}
      {showFunctionPicker && (
        <FunctionPicker
          onSelect={handleFunctionSelected}
          onClose={() => setShowFunctionPicker(false)}
        />
      )}

      {/* Flow Picker Modal */}
      {showFlowPicker && (
        <FlowPicker
          onSelect={(flowRef) => {
            syncFlowRef(flowRef)
            setShowFlowPicker(false)
          }}
          onClose={() => setShowFlowPicker(false)}
          currentFlowPath={selectedFlow?.['raisin:path']}
        />
      )}

      {/* Commit Dialog */}
      {pendingCommit && (
        <CommitDialog
          title="Save Trigger"
          action={`Update trigger "${properties.name}"`}
          onCommit={executeCommit}
          onClose={() => setPendingCommit(null)}
        />
      )}
    </div>
  )
}
