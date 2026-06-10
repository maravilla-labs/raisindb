/**
 * Raisin Flow Node Type Editor
 *
 * Visual editor for raisin:Flow nodes using the @raisindb/flow-designer package.
 */

import { useEffect, useCallback, useState, useRef, useMemo } from 'react'
import {
  Power,
  PowerOff,
  Loader2,
  Link2,
  Unlink,
  X,
  FlaskConical,
  ChevronDown,
  ChevronRight,
  UserCheck,
} from 'lucide-react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import { useFunctionsContext, useFlowRun } from '../../hooks'
import { nodesApi } from '../../../../api/nodes'
import CommitDialog from '../../../../components/CommitDialog'
import TaskDetail from '../../../../components/inbox/TaskDetail'
import { FunctionPicker } from './FunctionPicker'
import { FunctionIOPanel } from './FunctionIOPanel'
import { MockConfigEditor, type MockConfig } from './test/MockConfigEditor'
import { StepOutputInspector, type StepOutput } from './test/StepOutputInspector'
import {
  FlowDesigner,
  FlowToolbar,
  createEmptyFlow,
  isFlowStep,
  isFlowContainer,
  useFlowValidation,
  type FlowDesignerHandle,
} from '@raisindb/flow-designer'
import type {
  FlowDefinition,
  FlowNode,
  RaisinReference,
  FlowTheme,
  ValidationIssue,
} from '@raisindb/flow-designer'
import type {
  EditorTab,
  FlowProperties,
  TriggerType,
} from '../../types'

interface RaisinFlowNodeTypeEditorProps {
  tab: EditorTab
}

interface PendingCommit {
  path: string
  name: string
  properties: FlowProperties
}

import { ConditionBuilder } from './ConditionBuilder'
import { StepConfigPanel } from './StepConfigPanel'

// --- End Condition Builder ---

// Helper to convert legacy string function_ref to RaisinReference
function toRaisinReference(
  ref: string | RaisinReference | undefined,
  workspace: string
): RaisinReference | undefined {
  if (!ref) return undefined
  if (typeof ref === 'object' && 'raisin:ref' in ref) {
    return ref as RaisinReference
  }
  // Convert legacy string to RaisinReference
  return {
    'raisin:ref': ref as string,
    'raisin:workspace': workspace,
    'raisin:path': ref as string,
  }
}

// Helper to convert nodes with legacy function_ref format
function convertLegacyNodes(
  nodes: FlowDefinition['nodes'],
  workspace: string
): FlowDefinition['nodes'] {
  return nodes.map((node) => {
    if (node.node_type === 'raisin:FlowStep') {
      return {
        ...node,
        properties: {
          ...node.properties,
          function_ref: toRaisinReference(
            node.properties.function_ref as unknown as string | RaisinReference | undefined,
            workspace
          ),
        },
      }
    }
    if (node.node_type === 'raisin:FlowContainer') {
      return {
        ...node,
        children: convertLegacyNodes(node.children, workspace),
      }
    }
    return node
  })
}

export function RaisinFlowNodeTypeEditor({ tab }: RaisinFlowNodeTypeEditorProps) {
  const { repo, branch, workspace, markTabDirty, loadRootNodes, addLog, clearLogs, setProblems } = useFunctionsContext()

  // Ref for FlowDesigner imperative handle
  const flowDesignerRef = useRef<FlowDesignerHandle>(null)

  // Flow properties state
  const [properties, setProperties] = useState<FlowProperties>({
    name: '',
    title: '',
    description: '',
    enabled: true,
  })

  // Workflow data (for FlowDesigner)
  const [workflowData, setWorkflowData] = useState<FlowDefinition>(createEmptyFlow())

  // Flow validation
  const { validation } = useFlowValidation(workflowData, { debounceMs: 300 })

  // Sync validation issues to the Problems panel
  useEffect(() => {
    // Combine all issues (errors, warnings, suggestions)
    const allIssues: ValidationIssue[] = [
      ...validation.errors,
      ...validation.warnings,
      ...validation.suggestions,
    ]

    const problems = allIssues.map((issue: ValidationIssue, index: number) => ({
      id: `flow-${tab.id}-${issue.nodeId}-${issue.code}-${index}`,
      source: tab.path,
      nodeId: issue.nodeId,
      field: issue.field,
      code: issue.code,
      message: issue.message,
      severity: issue.severity,
    }))
    setProblems(problems)

    // Clear problems when component unmounts
    return () => {
      setProblems([])
    }
  }, [validation, tab.id, tab.path, setProblems])

  // UI state
  const [isLoading, setIsLoading] = useState(true)
  const [pendingCommit, setPendingCommit] = useState<PendingCommit | null>(null)
  const [selectedStepId, setSelectedStepId] = useState<string | null>(null)
  const [showFunctionPicker, setShowFunctionPicker] = useState(false)
  const [targetStepId, setTargetStepId] = useState<string | null>(null)

  // Trigger type for StartNode display (from child triggers)
  // TODO: Load child triggers to determine this value
  const [triggerType] = useState<TriggerType | undefined>(undefined)

  // Panel preferences (persisted to localStorage)
  const STORAGE_KEY = 'raisindb-flow-editor-preferences'

  const getStoredPreferences = useCallback(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY)
      if (stored) {
        return JSON.parse(stored) as {
          sidebarVisible?: boolean
          propertiesVisible?: boolean
          sidebarWidth?: number
          propertiesWidth?: number
          canvasTheme?: FlowTheme
        }
      }
    } catch {
      // Ignore parse errors
    }
    return {}
  }, [])

  const storedPrefs = useMemo(() => getStoredPreferences(), [getStoredPreferences])

  // Panel visibility
  const [sidebarVisible, setSidebarVisible] = useState(storedPrefs.sidebarVisible ?? true)
  const [propertiesVisible, setPropertiesVisible] = useState(storedPrefs.propertiesVisible ?? true)

  // Canvas theme
  const [canvasTheme, setCanvasTheme] = useState<FlowTheme>(storedPrefs.canvasTheme ?? 'dark')

  // Panel sizes (for Allotment)
  const [sidebarWidth, setSidebarWidth] = useState(storedPrefs.sidebarWidth ?? 320)
  const [propertiesWidth, setPropertiesWidth] = useState(storedPrefs.propertiesWidth ?? 288)

  // Persist preferences to localStorage
  useEffect(() => {
    const prefs = {
      sidebarVisible,
      propertiesVisible,
      sidebarWidth,
      propertiesWidth,
      canvasTheme,
    }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs))
  }, [sidebarVisible, propertiesVisible, sidebarWidth, propertiesWidth, canvasTheme])

  // Run dialog state
  const [showRunDialog, setShowRunDialog] = useState(false)
  const [runPayload, setRunPayload] = useState('')
  const [testMode, setTestMode] = useState(false)
  const [mockConfig, setMockConfig] = useState<MockConfig>({})
  const [agentMockConfig, setAgentMockConfig] = useState<MockConfig>({})
  const [taskBusy, setTaskBusy] = useState(false)
  const [stepOutputsExpanded, setStepOutputsExpanded] = useState(true)

  // Run + SSE wiring (execution state, events, step outputs, waiting task)
  const flowRun = useFlowRun({
    repo,
    flowPath: tab.path,
    flowName: properties.name || tab.name,
    addLog,
    clearLogs,
  })
  const { executionState } = flowRun
  const isExecuting = flowRun.running

  // Toolbar state (synced from FlowDesigner)
  const [toolbarState, setToolbarState] = useState({
    canUndo: false,
    canRedo: false,
    canDelete: false,
    zoom: 100,
    toolMode: 'select' as 'select' | 'pan',
  })

  // Toggle sidebar (Flow Properties panel)
  const handleToggleSidebar = useCallback(() => {
    setSidebarVisible((prev) => !prev)
  }, [])

  // Toggle properties panel (Step Properties)
  const handleToggleProperties = useCallback(() => {
    setPropertiesVisible((prev) => !prev)
  }, [])

  // Toggle canvas theme
  const handleToggleCanvasTheme = useCallback(() => {
    setCanvasTheme((prev) => (prev === 'dark' ? 'light' : 'dark'))
  }, [])

  // Open run dialog
  const handleRun = useCallback(() => {
    setShowRunDialog(true)
  }, [])

  // Sync toolbar state from FlowDesigner
  const syncToolbarState = useCallback(() => {
    if (flowDesignerRef.current) {
      setToolbarState({
        canUndo: flowDesignerRef.current.canUndo(),
        canRedo: flowDesignerRef.current.canRedo(),
        canDelete: flowDesignerRef.current.canDelete(),
        zoom: flowDesignerRef.current.getZoom(),
        toolMode: flowDesignerRef.current.getToolMode(),
      })
    }
  }, [])

  // Toolbar action handlers
  const handleToolbarUndo = useCallback(() => {
    flowDesignerRef.current?.undo()
    syncToolbarState()
  }, [syncToolbarState])

  const handleToolbarRedo = useCallback(() => {
    flowDesignerRef.current?.redo()
    syncToolbarState()
  }, [syncToolbarState])

  const handleToolbarDelete = useCallback(() => {
    flowDesignerRef.current?.deleteSelected()
    syncToolbarState()
  }, [syncToolbarState])

  const handleToolbarZoomIn = useCallback(() => {
    flowDesignerRef.current?.zoomIn()
    syncToolbarState()
  }, [syncToolbarState])

  const handleToolbarZoomOut = useCallback(() => {
    flowDesignerRef.current?.zoomOut()
    syncToolbarState()
  }, [syncToolbarState])

  const handleToolModeChange = useCallback((mode: 'select' | 'pan') => {
    flowDesignerRef.current?.setToolMode(mode)
    syncToolbarState()
  }, [syncToolbarState])

  // Handle selection change and sync toolbar
  const handleSelectNode = useCallback((nodeId: string | null) => {
    setSelectedStepId(nodeId)
    // Sync toolbar state after selection changes
    setTimeout(syncToolbarState, 0)
  }, [syncToolbarState])

  // Sync toolbar when workflow changes
  useEffect(() => {
    syncToolbarState()
  }, [workflowData, syncToolbarState])

  // Find node by ID helper
  const findNodeById = useCallback(
    (nodes: FlowNode[], id: string): FlowNode | null => {
      for (const node of nodes) {
        if (node.id === id) return node
        if (isFlowContainer(node) && node.children) {
          const found = findNodeById(node.children, id)
          if (found) return found
        }
      }
      return null
    },
    []
  )

  // Get the selected node
  const selectedNode = useMemo(() => {
    if (!selectedStepId) return null
    return findNodeById(workflowData.nodes, selectedStepId)
  }, [selectedStepId, workflowData.nodes, findNodeById])

  // Get current function path for the target step (for FunctionPicker)
  const targetStepFunctionPath = useMemo(() => {
    if (!targetStepId) return undefined
    const node = findNodeById(workflowData.nodes, targetStepId)
    if (node && isFlowStep(node) && node.properties.function_ref) {
      return node.properties.function_ref['raisin:path']
    }
    return undefined
  }, [targetStepId, workflowData.nodes, findNodeById])

  // Adapt the hook's step outputs to the StepOutputInspector's Map shape
  const stepOutputsMap = useMemo(() => {
    const map = new Map<string, StepOutput>()
    for (const [stepId, record] of Object.entries(flowRun.stepOutputs)) {
      const node = findNodeById(workflowData.nodes, stepId)
      const stepName =
        record.stepName ||
        (node && isFlowStep(node) ? node.properties.action : undefined) ||
        stepId
      map.set(stepId, {
        stepId,
        stepName,
        status: record.status,
        output: record.output,
        error: record.error,
        durationMs: record.durationMs,
        timestamp: record.timestamp,
      })
    }
    return map
  }, [flowRun.stepOutputs, workflowData.nodes, findNodeById])

  // Start a run (real or test) from the Run dialog
  const handleStartRun = useCallback(async () => {
    let payload: unknown
    try {
      payload = runPayload.trim() ? JSON.parse(runPayload) : {}
    } catch (error) {
      addLog({ level: 'error', message: `Failed to start flow: ${error}`, timestamp: new Date().toISOString() })
      return
    }
    await flowRun.start({
      test: testMode,
      input: payload,
      mockConfig: testMode ? mockConfig : undefined,
      agentMockConfig: testMode ? agentMockConfig : undefined,
    })
  }, [runPayload, testMode, mockConfig, agentMockConfig, flowRun, addLog])

  // Complete the waiting human task inline
  const handleCompleteTask = useCallback(
    async (response: Record<string, unknown>) => {
      setTaskBusy(true)
      try {
        await flowRun.completeWaitingTask(response)
      } catch (error) {
        addLog({ level: 'error', message: `Failed to complete task: ${error}`, timestamp: new Date().toISOString() })
      } finally {
        setTaskBusy(false)
      }
    },
    [flowRun, addLog]
  )

  // Load flow data
  useEffect(() => {
    const loadFlowData = async () => {
      setIsLoading(true)
      try {
        const response = await nodesApi.getAtHead(repo, branch, workspace, tab.path)
        const props = (response.properties || {}) as unknown as FlowProperties

        setProperties({
          name: props.name || '',
          title: props.title || '',
          description: props.description || '',
          enabled: props.enabled ?? true,
          timeout_ms: props.timeout_ms,
        })

        // Convert workflow_data to FlowDefinition (handling legacy string function_ref)
        if (props.workflow_data) {
          const legacyNodes = props.workflow_data.nodes || []
          setWorkflowData({
            version: props.workflow_data.version || 1,
            error_strategy: props.workflow_data.error_strategy || 'fail_fast',
            timeout_ms: props.workflow_data.timeout_ms,
            nodes: convertLegacyNodes(legacyNodes as unknown as FlowDefinition['nodes'], workspace),
          })
        } else {
          setWorkflowData(createEmptyFlow())
        }

        // TODO: Load child triggers to determine trigger type
      } catch (error) {
        console.error('Failed to load flow:', error)
      } finally {
        setIsLoading(false)
      }
    }

    loadFlowData()
  }, [repo, branch, workspace, tab.path])

  // Handle workflow changes from FlowDesigner
  const handleWorkflowChange = useCallback(
    (flow: FlowDefinition) => {
      setWorkflowData(flow)
      markTabDirty(tab.id, true)
    },
    [tab.id, markTabDirty]
  )

  // Handle property changes
  const handlePropertyChange = useCallback(
    (field: keyof FlowProperties, value: unknown) => {
      setProperties((prev) => ({ ...prev, [field]: value }))
      markTabDirty(tab.id, true)
    },
    [tab.id, markTabDirty]
  )

  // Handle save
  const handleSave = useCallback(() => {
    const updatedProperties: FlowProperties = {
      ...properties,
      workflow_data: workflowData as FlowProperties['workflow_data'],
    }
    setPendingCommit({
      path: tab.path,
      name: tab.name,
      properties: updatedProperties,
    })
  }, [tab.path, tab.name, properties, workflowData])

  // Handle commit
  const handleCommit = useCallback(
    async (message: string, actor: string) => {
      if (!pendingCommit) return

      try {
        await nodesApi.update(repo, branch, workspace, pendingCommit.path, {
          properties: pendingCommit.properties as unknown as Record<string, unknown>,
          commit: { message, actor },
        })
        markTabDirty(tab.id, false)
        loadRootNodes()
      } catch (error) {
        console.error('Failed to save flow:', error)
      } finally {
        setPendingCommit(null)
      }
    },
    [repo, branch, workspace, pendingCommit, tab.id, markTabDirty, loadRootNodes]
  )

  // Handle function picker
  const handleOpenFunctionPicker = useCallback((stepId: string) => {
    setTargetStepId(stepId)
    setShowFunctionPicker(true)
  }, [])

  const handleFunctionSelect = useCallback(
    (functionPath: string) => {
      if (targetStepId && flowDesignerRef.current) {
        // Use the FlowDesigner's setStepFunction method with RaisinReference format
        const functionRef: RaisinReference = {
          'raisin:ref': functionPath,
          'raisin:workspace': workspace,
          'raisin:path': functionPath,
        }
        flowDesignerRef.current.setStepFunction(targetStepId, functionRef)
        markTabDirty(tab.id, true)
      }
      setShowFunctionPicker(false)
      setTargetStepId(null)
    },
    [targetStepId, workspace, tab.id, markTabDirty]
  )

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 's') {
        e.preventDefault()
        handleSave()
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [handleSave])

  if (isLoading) {
    return (
      <div className="h-full flex items-center justify-center bg-gray-900">
        <Loader2 className="w-8 h-8 text-gray-400 animate-spin" />
      </div>
    )
  }

  return (
    <div className="h-full flex flex-col bg-gray-900">
      {/* Full-width toolbar at top */}
      <FlowToolbar
        sidebarVisible={sidebarVisible}
        onToggleSidebar={handleToggleSidebar}
        onSave={handleSave}
        onUndo={handleToolbarUndo}
        onRedo={handleToolbarRedo}
        canUndo={toolbarState.canUndo}
        canRedo={toolbarState.canRedo}
        onDelete={handleToolbarDelete}
        canDelete={toolbarState.canDelete}
        toolMode={toolbarState.toolMode}
        onToolModeChange={handleToolModeChange}
        onZoomIn={handleToolbarZoomIn}
        onZoomOut={handleToolbarZoomOut}
        currentZoom={toolbarState.zoom}
        propertiesVisible={propertiesVisible}
        onToggleProperties={handleToggleProperties}
        onRun={handleRun}
        canvasTheme={canvasTheme}
        onToggleCanvasTheme={handleToggleCanvasTheme}
      />

      {/* Resizable three-panel layout below toolbar */}
      <div className="flex-1 min-h-0">
        <Allotment
          onChange={(sizes) => {
            if (sizes.length >= 2) {
              if (sidebarVisible) setSidebarWidth(sizes[0])
              if (propertiesVisible) setPropertiesWidth(sizes[sizes.length - 1])
            }
          }}
        >
          {/* Left panel: Flow properties */}
          {sidebarVisible && (
            <Allotment.Pane preferredSize={sidebarWidth} minSize={200} maxSize={500}>
              <div className="h-full bg-black/30 backdrop-blur-md border-r border-white/10 overflow-y-auto">
                <div className="p-4 space-y-6">
                  {/* Header */}
                  <h2 className="text-lg font-semibold text-white">Flow Properties</h2>

                  {/* Name field */}
                  <div className="space-y-2">
                    <label className="block text-sm text-gray-400">Name</label>
                    <input
                      type="text"
                      value={properties.name}
                      onChange={(e) => handlePropertyChange('name', e.target.value)}
                      className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                      placeholder="my_flow"
                    />
                  </div>

                  {/* Title field */}
                  <div className="space-y-2">
                    <label className="block text-sm text-gray-400">Title</label>
                    <input
                      type="text"
                      value={properties.title || ''}
                      onChange={(e) => handlePropertyChange('title', e.target.value)}
                      className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                      placeholder="My Flow"
                    />
                  </div>

                  {/* Description field */}
                  <div className="space-y-2">
                    <label className="block text-sm text-gray-400">Description</label>
                    <textarea
                      value={properties.description || ''}
                      onChange={(e) => handlePropertyChange('description', e.target.value)}
                      rows={3}
                      className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 resize-none"
                      placeholder="Describe what this flow does..."
                    />
                  </div>

                  {/* Enabled toggle */}
                  <div className="flex items-center justify-between">
                    <div>
                      <label className="block text-sm text-white">Enabled</label>
                      <p className="text-xs text-gray-500">Allow this flow to be executed</p>
                    </div>
                    <button
                      onClick={() => handlePropertyChange('enabled', !properties.enabled)}
                      className={`p-2 rounded-lg transition-colors ${
                        properties.enabled
                          ? 'bg-green-500/20 text-green-400'
                          : 'bg-red-500/20 text-red-400'
                      }`}
                    >
                      {properties.enabled ? (
                        <Power className="w-5 h-5" />
                      ) : (
                        <PowerOff className="w-5 h-5" />
                      )}
                    </button>
                  </div>

                  {/* Timeout */}
                  <div className="space-y-2">
                    <label className="block text-sm text-gray-400">Timeout (ms)</label>
                    <input
                      type="number"
                      value={properties.timeout_ms || ''}
                      onChange={(e) =>
                        handlePropertyChange(
                          'timeout_ms',
                          e.target.value ? parseInt(e.target.value) : undefined
                        )
                      }
                      className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                      placeholder="60000"
                    />
                  </div>

                  {/* Info section */}
                  <div className="pt-4 border-t border-white/10">
                    <h3 className="text-sm font-medium text-gray-400 mb-2">About Flows</h3>
                    <p className="text-xs text-gray-500">
                      Flows define visual workflows with triggers and function execution.
                      Add triggers as children to define when the flow executes.
                      Use the visual editor to design the execution flow.
                    </p>
                  </div>
                </div>
              </div>
            </Allotment.Pane>
          )}

          {/* Center panel: Visual workflow designer */}
          <Allotment.Pane minSize={300}>
            <div className="h-full relative">
              {/* TEST indicator while a test run is active */}
              {flowRun.running && flowRun.isTestRun && (
                <div className="absolute top-3 right-3 z-10 flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-amber-500/20 border border-amber-500/40 text-amber-300 text-xs font-semibold tracking-wider pointer-events-none">
                  <FlaskConical className="w-3.5 h-3.5" />
                  TEST
                </div>
              )}
              <FlowDesigner
                ref={flowDesignerRef}
                flow={workflowData}
                onChange={handleWorkflowChange}
                onSelect={handleSelectNode}
                selectedNodeId={selectedStepId}
                onOpenFunctionPicker={handleOpenFunctionPicker}
                triggerType={triggerType}
                showToolbar={false}
                theme={canvasTheme}
                className="h-full"
                executionState={executionState}
              />
            </div>
          </Allotment.Pane>

          {/* Right panel: Step properties (collapsible) */}
          {propertiesVisible && (
            <Allotment.Pane preferredSize={propertiesWidth} minSize={200} maxSize={400}>
              <div className="h-full bg-black/30 backdrop-blur-md border-l border-white/10 overflow-y-auto">
                <div className="p-4">
                  <h3 className="text-sm font-medium text-gray-400 mb-4">Properties</h3>
                  {selectedNode && isFlowStep(selectedNode) ? (
                    <div className="space-y-4">
                      {/* Step title */}
                      <div className="space-y-2">
                        <label className="block text-xs text-gray-500">Title</label>
                        <input
                          type="text"
                          value={selectedNode.properties.action || ''}
                          onChange={(e) => {
                            flowDesignerRef.current?.updateStepProperty(selectedNode.id, { action: e.target.value })
                            markTabDirty(tab.id, true)
                          }}
                          className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                          placeholder="Step title"
                        />
                      </div>

                      {/* Function reference */}
                      <div className="space-y-2">
                        <label className="block text-xs text-gray-500">Linked Function</label>
                        {selectedNode.properties.function_ref ? (
                          <div className="flex items-center gap-2 px-3 py-2 bg-white/5 border border-white/10 rounded-lg">
                            <Link2 className="w-4 h-4 text-blue-400 flex-shrink-0" />
                            <span className="text-sm text-white truncate flex-1">
                              {selectedNode.properties.function_ref['raisin:path']}
                            </span>
                            <button
                              onClick={() => handleOpenFunctionPicker(selectedNode.id)}
                              className="text-gray-400 hover:text-white"
                              title="Change function"
                            >
                              <Link2 className="w-4 h-4" />
                            </button>
                            <button
                              onClick={() => {
                                if (flowDesignerRef.current) {
                                  flowDesignerRef.current.setStepFunction(selectedNode.id, undefined as unknown as RaisinReference)
                                  markTabDirty(tab.id, true)
                                }
                              }}
                              className="text-gray-400 hover:text-red-400"
                              title="Unlink function"
                            >
                              <Unlink className="w-4 h-4" />
                            </button>
                          </div>
                        ) : (
                          <button
                            onClick={() => handleOpenFunctionPicker(selectedNode.id)}
                            className="w-full flex items-center gap-2 px-3 py-2 bg-blue-500/10 border border-blue-500/30 rounded-lg text-blue-400 hover:bg-blue-500/20 transition-colors"
                          >
                            <Link2 className="w-4 h-4" />
                            <span className="text-sm">Link Function</span>
                          </button>
                        )}
                      </div>

                      {/* Function input/output schemas + arguments editor */}
                      {selectedNode.properties.function_ref &&
                        (!selectedNode.properties.step_type ||
                          selectedNode.properties.step_type === 'default') && (
                          <div className="pt-4 border-t border-white/10">
                            <FunctionIOPanel
                              repo={repo}
                              functionPath={
                                selectedNode.properties.function_ref['raisin:path'] ||
                                selectedNode.properties.function_ref['raisin:ref']
                              }
                              stepId={selectedNode.id}
                              args={
                                selectedNode.properties.arguments as
                                  | Record<string, unknown>
                                  | undefined
                              }
                              onChangeArguments={(args) => {
                                flowDesignerRef.current?.updateStepProperty(selectedNode.id, {
                                  arguments: args,
                                })
                                markTabDirty(tab.id, true)
                              }}
                            />
                          </div>
                        )}

                      {/* Disabled toggle */}
                      <div className="flex items-center justify-between pt-2">
                        <span className="text-xs text-gray-500">Disabled</span>
                        <div
                          className={`w-8 h-4 rounded-full relative cursor-pointer transition-colors ${
                            selectedNode.properties.disabled ? 'bg-red-500' : 'bg-gray-600'
                          }`}
                          onClick={() => {
                            flowDesignerRef.current?.updateStepProperty(selectedNode.id, {
                              disabled: !selectedNode.properties.disabled
                            })
                            markTabDirty(tab.id, true)
                          }}
                        >
                          <div
                            className={`w-3 h-3 rounded-full bg-white absolute top-0.5 transition-all ${
                              selectedNode.properties.disabled ? 'left-4' : 'left-0.5'
                            }`}
                          />
                        </div>
                      </div>

                      {/* Step Configuration Panel */}
                      <div className="pt-4 border-t border-white/10">
                        <StepConfigPanel
                          node={selectedNode}
                          onUpdateStep={(updates) => {
                            flowDesignerRef.current?.updateStepProperty(selectedNode.id, updates)
                            markTabDirty(tab.id, true)
                          }}
                          onDirty={() => markTabDirty(tab.id, true)}
                        />
                      </div>
                    </div>
                  ) : selectedNode && isFlowContainer(selectedNode) ? (
                    <div className="space-y-4">
                      <div className="space-y-2">
                        <label className="block text-xs text-gray-500">Container Type</label>
                        <div className="px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm">
                          {selectedNode.container_type.toUpperCase()}
                        </div>
                      </div>

                      {/* Children Conditions - Only for OR containers */}
                      {selectedNode.container_type === 'or' && (
                        <div className="space-y-4 pt-4 border-t border-white/10">
                          <h4 className="text-xs font-medium text-gray-400">Branch Conditions</h4>
                          {selectedNode.children.map((child, index) => {
                            if (!isFlowStep(child)) return null;
                            return (
                              <div key={child.id} className="space-y-2">
                                <label className="block text-xs text-gray-500">
                                  {child.properties.action || `Step ${index + 1}`}
                                </label>
                                <ConditionBuilder
                                  condition={child.properties.condition || ''}
                                  onChange={(newCondition) => {
                                    flowDesignerRef.current?.updateStepProperty(child.id, { condition: newCondition })
                                    markTabDirty(tab.id, true)
                                  }}
                                />
                              </div>
                            );
                          })}
                          {selectedNode.children.length === 0 && (
                            <p className="text-xs text-gray-500 italic">No steps in this container</p>
                          )}
                        </div>
                      )}

                      {/* Container Configuration (AI config, router, referee, merge strategy) */}
                      <div className="pt-4 border-t border-white/10">
                        <StepConfigPanel
                          node={selectedNode}
                          onUpdateStep={() => {}}
                          onUpdateContainer={(updates) => {
                            // Update container (ai_config / router / referee / prompt / timeout)
                            flowDesignerRef.current?.updateContainer(selectedNode.id, updates as any)
                            markTabDirty(tab.id, true)
                          }}
                          onDirty={() => markTabDirty(tab.id, true)}
                        />
                      </div>
                    </div>
                  ) : (
                    <p className="text-sm text-gray-500">
                      Select a step or container to view properties
                    </p>
                  )}
                </div>
              </div>
            </Allotment.Pane>
          )}
        </Allotment>
      </div>

      {/* Commit dialog */}
      {pendingCommit && (
        <CommitDialog
          title="Update Flow"
          action={`Save changes to ${pendingCommit.name}`}
          onCommit={handleCommit}
          onClose={() => setPendingCommit(null)}
        />
      )}

      {/* Function picker modal */}
      {showFunctionPicker && (
        <FunctionPicker
          onSelect={handleFunctionSelect}
          onClose={() => {
            setShowFunctionPicker(false)
            setTargetStepId(null)
          }}
          currentFunctionPath={targetStepFunctionPath}
        />
      )}

      {/* Run flow dialog */}
      {showRunDialog && (
        <div className="fixed inset-0 z-50 flex items-center justify-center">
          {/* Backdrop */}
          <div
            className="absolute inset-0 bg-black/60 backdrop-blur-sm"
            onClick={() => setShowRunDialog(false)}
          />

          {/* Dialog */}
          <div className="relative bg-gray-900 border border-white/10 rounded-xl shadow-2xl w-full max-w-2xl flex flex-col max-h-[85vh]">
            {/* Header */}
            <div className="flex items-center justify-between px-6 py-4 border-b border-white/10">
              <div className="flex items-center gap-3">
                <h2 className="text-lg font-semibold text-white">Run Flow</h2>
                {flowRun.running && flowRun.isTestRun && (
                  <span className="flex items-center gap-1.5 px-2 py-0.5 rounded-md bg-amber-500/20 border border-amber-500/40 text-amber-300 text-xs font-semibold tracking-wider">
                    <FlaskConical className="w-3.5 h-3.5" />
                    TEST
                  </span>
                )}
              </div>
              <button
                onClick={() => setShowRunDialog(false)}
                className="p-1 text-gray-400 hover:text-white rounded hover:bg-white/10"
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            {/* Content */}
            <div className="p-6 space-y-4 overflow-y-auto">
              <div className="space-y-2">
                <label className="block text-sm text-gray-400">Input Payload (JSON)</label>
                <textarea
                  value={runPayload}
                  onChange={(e) => setRunPayload(e.target.value)}
                  rows={8}
                  className="w-full px-4 py-3 bg-black/30 border border-white/10 rounded-lg text-white font-mono text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 resize-none"
                  placeholder='{\n  "key": "value"\n}'
                />
              </div>

              {/* Test run toggle */}
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <FlaskConical className="w-4 h-4 text-amber-400" />
                  <div>
                    <label className="block text-sm text-white">Test run</label>
                    <p className="text-xs text-gray-500">
                      Execute via the test endpoint with optional function mocking
                    </p>
                  </div>
                </div>
                <div
                  className={`w-10 h-5 rounded-full relative cursor-pointer transition-colors ${
                    testMode ? 'bg-amber-500' : 'bg-gray-600'
                  }`}
                  onClick={() => setTestMode((prev) => !prev)}
                >
                  <div
                    className={`w-4 h-4 rounded-full bg-white absolute top-0.5 transition-all ${
                      testMode ? 'left-5' : 'left-0.5'
                    }`}
                  />
                </div>
              </div>

              {/* Mock configuration (test runs only) */}
              {testMode && (
                <MockConfigEditor
                  workflow={workflowData}
                  mockConfig={mockConfig}
                  onChange={setMockConfig}
                  agentMockConfig={agentMockConfig}
                  onAgentChange={setAgentMockConfig}
                />
              )}

              <div className="bg-blue-500/10 border border-blue-500/30 rounded-lg p-3">
                <p className="text-sm text-blue-400">
                  Flow: <span className="font-medium text-white">{properties.name || tab.name}</span>
                </p>
                <p className="text-xs text-gray-500 mt-1">
                  The flow will be executed with the provided payload.
                </p>
              </div>

              {/* Waiting human task: complete inline without leaving the editor */}
              {flowRun.waitingTask && (
                <div className="border border-amber-500/40 bg-amber-500/5 rounded-lg p-4 space-y-3">
                  <div className="flex items-center gap-2 text-amber-400">
                    <UserCheck className="w-4 h-4" />
                    <span className="text-sm font-medium">Flow is waiting on a human task</span>
                  </div>
                  <TaskDetail
                    task={flowRun.waitingTask}
                    onComplete={handleCompleteTask}
                    busy={taskBusy}
                  />
                </div>
              )}

              {/* Step outputs (collapsible) */}
              {flowRun.instanceId && (
                <div className="border border-white/10 rounded-lg overflow-hidden">
                  <button
                    onClick={() => setStepOutputsExpanded((prev) => !prev)}
                    className="w-full flex items-center justify-between px-4 py-3 bg-white/5 hover:bg-white/10 transition-colors"
                  >
                    <div className="flex items-center gap-2">
                      {stepOutputsExpanded ? (
                        <ChevronDown className="w-4 h-4 text-gray-400" />
                      ) : (
                        <ChevronRight className="w-4 h-4 text-gray-400" />
                      )}
                      <span className="text-sm font-medium text-white">Step Outputs</span>
                    </div>
                    <span className="text-xs text-gray-500">
                      {flowRun.running ? 'Running…' : 'Finished'}
                    </span>
                  </button>
                  {stepOutputsExpanded && (
                    <div className="p-3 bg-black/20">
                      <StepOutputInspector
                        stepOutputs={stepOutputsMap}
                        selectedStepId={selectedStepId}
                        onSelectStep={handleSelectNode}
                      />
                    </div>
                  )}
                </div>
              )}
            </div>

            {/* Footer */}
            <div className="flex items-center justify-end gap-3 px-6 py-4 border-t border-white/10">
              <button
                onClick={() => setShowRunDialog(false)}
                className="px-4 py-2 text-sm text-gray-400 hover:text-white rounded-lg hover:bg-white/10 transition-colors"
              >
                Close
              </button>
              <button
                onClick={() => void handleStartRun()}
                disabled={isExecuting}
                className="px-4 py-2 text-sm font-medium text-white bg-green-600 hover:bg-green-700 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {isExecuting ? 'Running...' : testMode ? 'Run Test' : 'Run Flow'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
