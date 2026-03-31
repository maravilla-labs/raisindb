/**
 * Editor Pane Component
 *
 * The main code editor area with Monaco editor.
 * Supports both Function node execution and direct file execution with SSE streaming.
 */

import { useEffect, useState, useCallback, useRef } from 'react'
import { Save, FileCode, Loader2, Plus } from 'lucide-react'
import { JavaScriptEditor, StarlarkEditor } from '@raisindb/editor'
import { SqlEditor } from '../../../../monaco/SqlEditor'
import { useFunctionsContext } from '../../hooks'
import { EditorTabs } from './EditorTabs'
import { RaisinFunctionNodeTypeEditor } from './RaisinFunctionNodeTypeEditor'
import { RaisinTriggerNodeTypeEditor } from './RaisinTriggerNodeTypeEditor'
import { RaisinFlowNodeTypeEditor } from './RaisinFlowNodeTypeEditor'
import { RaisinAgentNodeTypeEditor } from './RaisinAgentNodeTypeEditor'
import { RunConfigBar, createDefaultRunConfig, type RunConfig } from './RunConfigBar'
import { QuickPick, addToRecentNodes } from './QuickPick'
import { nodesApi } from '../../../../api/nodes'
import { functionsApi, type RunFileLogEvent, type RunFileResultEvent } from '../../../../api/functions'
import CommitDialog from '../../../../components/CommitDialog'
import type { FunctionNode, FunctionLanguage, LogEntry } from '../../types'

interface PendingCommit {
  path: string
  name: string
  code: string
}

const LANGUAGE_METADATA: Record<FunctionLanguage, { ext: string; mime: string; label: string }> = {
  javascript: { ext: 'js', mime: 'application/javascript', label: 'JavaScript' },
  starlark: { ext: 'py', mime: 'text/x-python', label: 'Python (Starlark)' },
  sql: { ext: 'sql', mime: 'application/sql', label: 'SQL' },
}

/** Detect language from file extension */
function detectLanguageFromExtension(fileName: string): FunctionLanguage {
  const ext = fileName.split('.').pop()?.toLowerCase()
  if (ext === 'py' || ext === 'star' || ext === 'bzl') return 'starlark'
  if (ext === 'sql') return 'sql'
  return 'javascript' // Default for .js, .ts, etc.
}

/** Generate default template code for a new function */
function getDefaultTemplate(functionName: string): string {
  return `/**
 * ${functionName}
 *
 * A RaisinDB serverless function.
 *
 * ============================================================================
 * AVAILABLE APIS
 * ============================================================================
 *
 * EXECUTION CONTEXT (raisin.context)
 * -----------------------------------
 * raisin.context.tenant_id      - Current tenant ID
 * raisin.context.repo_id        - Repository ID
 * raisin.context.branch         - Branch name (e.g., "main")
 * raisin.context.workspace_id   - Workspace name (e.g., "default")
 * raisin.context.actor          - User or system that triggered execution
 * raisin.context.execution_id   - Unique ID for this execution
 *
 * NODE OPERATIONS (raisin.nodes)
 * -----------------------------------
 * raisin.nodes.get(workspace, path)
 *   - Get a node by path
 *   - Returns: { id, path, node_type, properties, ... } or null
 *
 * raisin.nodes.getById(workspace, id)
 *   - Get a node by ID
 *   - Returns: { id, path, node_type, properties, ... } or null
 *
 * raisin.nodes.create(workspace, parentPath, data)
 *   - Create a new node
 *   - data: { name, type, properties: { ... } }
 *   - Returns: created node
 *
 * raisin.nodes.update(workspace, path, data)
 *   - Update an existing node
 *   - data: { properties: { ... } }
 *   - Returns: updated node
 *
 * raisin.nodes.delete(workspace, path)
 *   - Delete a node
 *   - Returns: true on success
 *
 * raisin.nodes.query(workspace, query)
 *   - Query nodes with filters
 *   - query: { node_type, path_prefix, properties, limit, offset }
 *   - Returns: array of matching nodes
 *
 * raisin.nodes.getChildren(workspace, parentPath, limit?)
 *   - Get child nodes
 *   - Returns: array of child nodes
 *
 * SQL OPERATIONS (raisin.sql)
 * -----------------------------------
 * raisin.sql.query(sql, params?)
 *   - Execute a SQL query
 *   - sql: SQL string with $1, $2, etc. placeholders
 *   - params: array of parameter values
 *   - Returns: { columns: [...], rows: [[...], ...], row_count }
 *
 * raisin.sql.execute(sql, params?)
 *   - Execute INSERT/UPDATE/DELETE
 *   - Returns: number of affected rows
 *
 * HTTP OPERATIONS (raisin.http)
 * -----------------------------------
 * raisin.http.fetch(url, options?)
 *   - Make HTTP request (URL must be in function's allowlist)
 *   - options: { method, body, headers, timeout_ms }
 *   - Returns: { status, headers, body, ok }
 *
 * EVENT OPERATIONS (raisin.events)
 * -----------------------------------
 * raisin.events.emit(eventType, data)
 *   - Emit a custom event
 *   - eventType: e.g., "custom:my-event"
 *   - data: event payload object
 *   - Returns: true on success
 *
 * CONSOLE LOGGING
 * -----------------------------------
 * console.log(...args)    - Info level logging
 * console.debug(...args)  - Debug level logging
 * console.info(...args)   - Info level logging
 * console.warn(...args)   - Warning level logging
 * console.error(...args)  - Error level logging
 *
 * HTTP TRIGGER CONTEXT (when triggered via HTTP webhook)
 * -----------------------------------
 * raisin.context.http_request.method       - HTTP method (GET, POST, etc.)
 * raisin.context.http_request.path         - Request path suffix
 * raisin.context.http_request.path_params  - Params from route pattern (e.g., { userId: "123" })
 * raisin.context.http_request.query_params - URL query parameters
 * raisin.context.http_request.headers      - Request headers
 * raisin.context.http_request.body         - Parsed request body (JSON)
 *
 * EVENT TRIGGER CONTEXT (when triggered by node events)
 * -----------------------------------
 * raisin.context.event_data.event_type  - Type of event (created, updated, deleted)
 * raisin.context.event_data.node        - The node that triggered the event
 * raisin.context.event_data.changes     - Property changes (for update events)
 *
 * ============================================================================
 */

async function handler(input) {
  // Access execution context
  const { tenant_id, branch, actor } = raisin.context;

  console.log('Function executed by:', actor);

  // Your function logic here

  return {
    success: true,
    message: 'Hello from ${functionName}!'
  };
}
`
}

/** Generate default template code for a new Starlark function */
function getStarlarkDefaultTemplate(functionName: string): string {
  return `"""
${functionName}

A RaisinDB serverless function written in Starlark (Python subset).

============================================================================
AVAILABLE APIS
============================================================================

EXECUTION CONTEXT (raisin.context)
-----------------------------------
raisin.context.tenant_id      - Current tenant ID
raisin.context.repo_id        - Repository ID
raisin.context.branch         - Branch name (e.g., "main")
raisin.context.workspace_id   - Workspace name (e.g., "default")
raisin.context.actor          - User or system that triggered execution
raisin.context.execution_id   - Unique ID for this execution

NODE OPERATIONS (raisin.nodes)
-----------------------------------
raisin.nodes.get(workspace, path)
  - Get a node by path
  - Returns: { id, path, node_type, properties, ... } or None

raisin.nodes.get_by_id(workspace, id)
  - Get a node by ID
  - Returns: { id, path, node_type, properties, ... } or None

raisin.nodes.create(workspace, parent_path, data)
  - Create a new node
  - data: { name, type, properties: { ... } }
  - Returns: created node

raisin.nodes.update(workspace, path, data)
  - Update an existing node
  - data: { properties: { ... } }
  - Returns: updated node

raisin.nodes.delete(workspace, path)
  - Delete a node
  - Returns: True on success

raisin.nodes.query(workspace, query)
  - Query nodes with filters
  - query: { node_type, path_prefix, properties, limit, offset }
  - Returns: list of matching nodes

raisin.nodes.get_children(workspace, parent_path, limit=None)
  - Get child nodes
  - Returns: list of child nodes

SQL OPERATIONS (raisin.sql)
-----------------------------------
raisin.sql.query(sql, params=None)
  - Execute a SQL query
  - sql: SQL string with $1, $2, etc. placeholders
  - params: list of parameter values
  - Returns: { columns: [...], rows: [[...], ...], row_count }

raisin.sql.execute(sql, params=None)
  - Execute INSERT/UPDATE/DELETE
  - Returns: number of affected rows

HTTP OPERATIONS (raisin.http)
-----------------------------------
raisin.http.fetch(url, options=None)
  - Make HTTP request (URL must be in function's allowlist)
  - options: { method, body, headers, timeout_ms }
  - Returns: { status, headers, body, ok }

EVENT OPERATIONS (raisin.events)
-----------------------------------
raisin.events.emit(event_type, data)
  - Emit a custom event
  - event_type: e.g., "custom:my-event"
  - data: event payload dict
  - Returns: True on success

CONSOLE LOGGING
-----------------------------------
print(...)           - Info level logging (Starlark built-in)
raisin.log.debug()   - Debug level logging
raisin.log.info()    - Info level logging
raisin.log.warn()    - Warning level logging
raisin.log.error()   - Error level logging

STARLARK DIFFERENCES FROM PYTHON
-----------------------------------
- No imports (use load() for .bzl files)
- No classes (use struct() instead)
- No exceptions (use fail() and check return values)
- Strings are immutable, use + for concatenation
- Dicts preserve insertion order

============================================================================
"""

def handler(input):
    """Main function handler.

    Args:
        input: Input data from the function caller (dict)

    Returns:
        Response data (dict, list, or primitive)
    """
    # Access execution context
    tenant = raisin.context.tenant_id
    branch = raisin.context.branch

    print("Function executed on branch:", branch)

    # Your function logic here

    return {
        "success": True,
        "message": "Hello from ${functionName}!"
    }
`
}

/** Generate default template code for a new SQL function */
function getSQLDefaultTemplate(functionName: string): string {
  return `-- ${functionName}
--
-- A RaisinDB SQL function.
--
-- ============================================================================
-- SQL FUNCTION OVERVIEW
-- ============================================================================
--
-- SQL functions execute queries directly against RaisinDB's SQL engine.
-- They are ideal for data retrieval, reporting, and simple data operations.
--
-- INPUT PARAMETERS
-- ----------------
-- Parameters are passed via the input object and accessed using $1, $2, etc.
-- Example: If input is [10, "active"], use $1 for 10 and $2 for "active"
--
-- CONTEXT (automatically applied)
-- --------------------------------
-- - tenant_id: Current tenant (applied automatically)
-- - repo_id: Repository (applied automatically)
-- - branch: Branch name (applied automatically)
--
-- SUPPORTED OPERATIONS
-- --------------------
-- - SELECT: Query data from workspaces (tables)
-- - INSERT: Create new nodes
-- - UPDATE: Modify existing nodes
-- - DELETE: Remove nodes
-- - RELATE/UNRELATE: Manage node relationships
-- - MOVE: Change node hierarchy
--
-- WORKSPACE TABLES
-- ----------------
-- Each workspace is a table. Query format:
--   SELECT * FROM workspace_name WHERE ...
--
-- Common columns: id, path, name, node_type, parent_id, properties
--
-- ============================================================================

-- Example: Query nodes from a workspace with a limit parameter
SELECT id, path, name, node_type, properties
FROM content
WHERE node_type LIKE 'raisin:%'
LIMIT $1
`
}

const HANDLER_STORAGE_KEY = 'raisindb.functions.handlersByNode'

function readStoredHandlers(): Record<string, string> {
  try {
    const raw = localStorage.getItem(HANDLER_STORAGE_KEY)
    return raw ? JSON.parse(raw) : {}
  } catch {
    return {}
  }
}

function getStoredHandler(nodeId: string): string | null {
  const handlers = readStoredHandlers()
  const value = handlers[nodeId]
  return typeof value === 'string' ? value : null
}

function saveStoredHandler(nodeId: string, handler: string) {
  const handlers = readStoredHandlers()
  handlers[nodeId] = handler
  try {
    localStorage.setItem(HANDLER_STORAGE_KEY, JSON.stringify(handlers))
  } catch {
    // Ignore storage issues; handler will fall back to defaults next load.
  }
}

/** Extract function names from JS code (best-effort heuristics) */
function extractFunctionNames(code: string): string[] {
  const names: string[] = []
  const add = (name?: string) => {
    if (!name) return
    if (!names.includes(name)) names.push(name)
  }

  const functionDecl = /\bfunction\s+([A-Za-z_$][\w$]*)\s*\(/g
  let match: RegExpExecArray | null
  while ((match = functionDecl.exec(code)) !== null) {
    add(match[1])
  }

  const arrowWithParens = /\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?\([^=]*\)\s*=>/g
  while ((match = arrowWithParens.exec(code)) !== null) {
    add(match[1])
  }

  const arrowSingleParam = /\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?[A-Za-z_$][\w$]*\s*=>/g
  while ((match = arrowSingleParam.exec(code)) !== null) {
    add(match[1])
  }

  const exportsAssign = /(?:exports|module\.exports)\.([A-Za-z_$][\w$]*)\s*=/g
  while ((match = exportsAssign.exec(code)) !== null) {
    add(match[1])
  }

  const objectMethod = /([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*\{/g
  const reserved = new Set(['if', 'for', 'while', 'switch', 'catch', 'function', 'return', 'const', 'let', 'var', 'class', 'export', 'default', 'async'])
  while ((match = objectMethod.exec(code)) !== null) {
    if (!reserved.has(match[1])) {
      add(match[1])
    }
  }

  return names
}

export function EditorPane() {
  const {
    repo,
    branch,
    workspace,
    openTabs,
    activeTabId,
    selectedNode,
    setCode,
    loadCode,
    markTabDirty,
    updateTabPath,
    preferences,
    addLog,
    addExecution,
  } = useFunctionsContext()

  const [localCode, setLocalCode] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const [isRunning, setIsRunning] = useState(false)
  const [pendingCommit, setPendingCommit] = useState<PendingCommit | null>(null)
  const [hasNoFiles, setHasNoFiles] = useState(false)
  const [isCreatingFile, setIsCreatingFile] = useState(false)
  const [showCreateFileDialog, setShowCreateFileDialog] = useState(false)
  const [showEmptyFilePrompt, setShowEmptyFilePrompt] = useState(false)

  // Run configuration state
  const [runConfig, setRunConfig] = useState<RunConfig>(createDefaultRunConfig)
  const [availableHandlers, setAvailableHandlers] = useState<string[]>([])
  const [showNodePicker, setShowNodePicker] = useState(false)
  const abortControllerRef = useRef<AbortController | null>(null)

  const activeTab = openTabs.find((t) => t.id === activeTabId)

  // Determine if the current file is a standalone Asset (not under a Function parent)
  const isStandaloneFile = activeTab?.node_type === 'raisin:Asset' && !selectedNode
  const isJavaScriptAsset = activeTab?.node_type === 'raisin:Asset' && activeTab?.language === 'javascript'

  const handleRunConfigChange = useCallback((nextConfig: RunConfig) => {
    setRunConfig(nextConfig)
  }, [])

  const applyHandlerChange = useCallback((handler: string, persist = true) => {
    setRunConfig((prev) => (prev.handler === handler ? prev : { ...prev, handler }))
    if (persist && activeTab?.id && isJavaScriptAsset) {
      saveStoredHandler(activeTab.id, handler)
    }
  }, [activeTab?.id, isJavaScriptAsset])

  const handleHandlerChange = useCallback((handler: string) => {
    applyHandlerChange(handler, true)
  }, [applyHandlerChange])

  // Load code when active tab changes (only for Asset/file nodes)
  useEffect(() => {
    if (!activeTab) {
      setHasNoFiles(false)
      return
    }

    // Skip code loading for non-Asset nodes (Function, Trigger, Flow, Agent)
    // These node types have their own dedicated editors and don't need code loading
    // IMPORTANT: Do NOT modify tab.path for these nodes - they need their original path for saving
    if (activeTab.node_type !== 'raisin:Asset') {
      setHasNoFiles(false)
      setLocalCode('')
      return
    }

    const loadActiveCode = async () => {
      // Construct a minimal node object from the tab info for loadCode
      const tabNode = {
        id: activeTab.id,
        path: activeTab.path,
        name: activeTab.name,
        node_type: activeTab.node_type,
        properties: {},
      }

      // Load from server (loadCode handles caching internally)
      setIsLoading(true)
      setShowEmptyFilePrompt(false)
      try {
        const result = await loadCode(tabNode)
        if (result !== null) {
          // File exists - update tab path to the actual file path and set code
          updateTabPath(activeTab.id, result.filePath)
          setLocalCode(result.code)
          setHasNoFiles(false)
          // Check if file is empty or nearly empty (only whitespace) - offer template
          if (result.code.trim().length === 0) {
            setShowEmptyFilePrompt(true)
          }
        } else {
          // No files at all - this shouldn't happen for Asset nodes that exist
          setHasNoFiles(false)
        }
      } finally {
        setIsLoading(false)
      }
    }

    loadActiveCode()
  }, [activeTab?.id, activeTab?.path, activeTab?.name, activeTab?.node_type, loadCode, updateTabPath])

  const handleCodeChange = useCallback(
    (newCode: string) => {
      setLocalCode(newCode)
      if (activeTab) {
        setCode(activeTab.path, newCode)
        markTabDirty(activeTab.id, true)
      }
    },
    [activeTab, setCode, markTabDirty]
  )

  /** Apply template code to an empty file */
  const handleApplyTemplate = useCallback(() => {
    if (!activeTab) return

    // Detect language from file extension
    const language = detectLanguageFromExtension(activeTab.name)

    // Extract function name from file name or parent path
    const fileName = activeTab.name.replace(/\.[^.]+$/, '') // Remove extension
    const functionName = fileName === 'index'
      ? activeTab.path.split('/').filter(Boolean).slice(-2, -1)[0] || 'MyFunction'
      : fileName

    // Get appropriate template
    const template = language === 'sql'
      ? getSQLDefaultTemplate(functionName)
      : language === 'starlark'
        ? getStarlarkDefaultTemplate(functionName)
        : getDefaultTemplate(functionName)

    // Apply template
    setLocalCode(template)
    setCode(activeTab.path, template)
    markTabDirty(activeTab.id, true)
    setShowEmptyFilePrompt(false)
  }, [activeTab, setCode, markTabDirty])

  const handleSave = useCallback((valueFromEditor?: string) => {
    if (!activeTab) return
    // Use value from Monaco editor if provided (keyboard shortcut),
    // otherwise fall back to localCode (toolbar button)
    const codeToSave = valueFromEditor ?? localCode
    // Show commit dialog instead of saving directly
    setPendingCommit({
      path: activeTab.path,
      name: activeTab.name,
      code: codeToSave,
    })
  }, [activeTab, localCode])

  /** Create the first file (index.js) for a new function */
  const handleCreateFirstFile = useCallback(() => {
    setShowCreateFileDialog(true)
  }, [])

  const executeCreateFirstFile = useCallback(
    async (message: string, actor: string) => {
      if (!selectedNode || !repo || !branch) return

      setIsCreatingFile(true)
      setShowCreateFileDialog(false)

      const functionNode = selectedNode as FunctionNode
      const language = (functionNode.properties?.language as FunctionLanguage) || 'javascript'
      const metadata = LANGUAGE_METADATA[language] || LANGUAGE_METADATA.javascript
      const fileName = `index.${metadata.ext}`

      try {
        const defaultCode = language === 'sql'
          ? getSQLDefaultTemplate(functionNode.name)
          : language === 'starlark'
            ? getStarlarkDefaultTemplate(functionNode.name)
            : getDefaultTemplate(functionNode.name)
        const blob = new Blob([defaultCode], { type: metadata.mime })
        const assetPath = `${functionNode.path}/${fileName}`

        // Upload default template code and auto-create Asset node using binary storage
        await nodesApi.uploadFile(repo, branch, workspace, assetPath, {
          file: blob,
          fileName,
          inline: false, // Store as PropertyValue::Resource (binary storage)
          propertyPath: 'file',
          nodeType: 'raisin:Asset',
          overrideExisting: true,
          commitMessage: message,
          commitActor: actor,
        })

        // Update entry_file property on the function
        await nodesApi.update(repo, branch, workspace, functionNode.path, {
          properties: {
            ...functionNode.properties,
            entry_file: `${fileName}:handler`,
          },
          commit: { message: `Set entry file to ${fileName}:handler`, actor },
        })

        // Cache the code and load it
        setCode(assetPath, defaultCode)
        setLocalCode(defaultCode)
        setHasNoFiles(false)
      } catch (error) {
        console.error('Failed to create first file:', error)
      } finally {
        setIsCreatingFile(false)
      }
    },
    [selectedNode, repo, branch, workspace, setCode]
  )

  const executeCommit = useCallback(async (message: string, actor: string) => {
    if (!pendingCommit || !repo || !branch) return

    // pendingCommit.path is the file node's path (set by updateTabPath after loading)
    const filePath = pendingCommit.path
    const fileName = pendingCommit.name

    // Determine MIME type from file name
    const ext = fileName.split('.').pop()?.toLowerCase() || 'js'
    const mimeTypes: Record<string, string> = {
      js: 'application/javascript',
      ts: 'application/typescript',
      json: 'application/json',
      md: 'text/markdown',
      txt: 'text/plain',
    }
    const mimeType = mimeTypes[ext] || 'text/plain'

    // Upload the code directly to the file path
    const blob = new Blob([pendingCommit.code], { type: mimeType })

    await nodesApi.uploadFile(repo, branch, workspace, filePath, {
      file: blob,
      fileName,
      inline: false, // Store as PropertyValue::Resource in binary storage
      propertyPath: 'file',
      overrideExisting: true,
      commitMessage: message,
      commitActor: actor,
    })

    if (activeTab) {
      markTabDirty(activeTab.id, false)
    }

    setPendingCommit(null)
  }, [pendingCommit, repo, branch, workspace, activeTab, markTabDirty])

  const handleRun = useCallback(async () => {
    if (!activeTab || isRunning) return

    setIsRunning(true)
    const startTime = Date.now()
    const collectedLogs: LogEntry[] = []

    // Check if we should use direct file execution (standalone Asset) or function invocation
    const useDirectExecution = activeTab.node_type === 'raisin:Asset'

    if (useDirectExecution) {
      // Direct file execution with SSE streaming
      const isDirty = activeTab.isDirty

      addLog({
        level: 'info',
        message: `Executing ${isDirty ? '(unsaved) ' : ''}file "${activeTab.name}" with handler "${runConfig.handler}"...`,
        timestamp: new Date().toISOString(),
      })

      // Prepare input
      let input: Record<string, unknown> | undefined
      let inputNodeId: string | undefined

      if (runConfig.inputType === 'json') {
        try {
          input = JSON.parse(runConfig.inputJson || '{}')
        } catch {
          input = {}
        }
      } else if (runConfig.inputNodeId) {
        inputNodeId = runConfig.inputNodeId
      }

      // Build request - use inline code for dirty files, node_id for saved files
      const request: Parameters<typeof functionsApi.runFileStream>[1] = {
        handler: runConfig.handler,
        input,
        input_node_id: inputNodeId,
        input_workspace: runConfig.inputWorkspace,
        timeout_ms: 30000,
      }

      // Compute parent function path (e.g., /lib/raisin/weather/index.js -> /lib/raisin/weather)
      const parentPath = activeTab.path.split('/').slice(0, -1).join('/')
      request.function_path = parentPath

      if (isDirty) {
        // Send inline code for unsaved files
        request.code = localCode
        request.file_name = activeTab.name
      } else {
        // Use node_id for saved files
        request.node_id = activeTab.id
      }

      // Start SSE stream
      abortControllerRef.current = functionsApi.runFileStream(
        repo,
        request,
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
            // Create execution record
            const execution = {
              id: event.execution_id,
              execution_id: event.execution_id,
              function_path: activeTab.path,
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
            abortControllerRef.current = null
          },
          onError: (error) => {
            addLog({
              level: 'error',
              message: `Execution error: ${error.message}`,
              timestamp: new Date().toISOString(),
            })
            setIsRunning(false)
            abortControllerRef.current = null
          },
        }
      )
    } else {
      // Function invocation (existing logic)
      let functionName: string
      if (selectedNode) {
        functionName = (selectedNode as FunctionNode).properties?.name as string || selectedNode.name
      } else {
        // Get parent function name from path
        const pathParts = activeTab.path.split('/').filter(Boolean)
        functionName = pathParts.length > 1 ? pathParts[pathParts.length - 2] : activeTab.name
      }

      addLog({
        level: 'info',
        message: `Executing function "${functionName}"...`,
        timestamp: new Date().toISOString(),
      })

      try {
        // Get input based on config
        let input: Record<string, unknown> = {}
        if (runConfig.inputType === 'json') {
          try {
            input = JSON.parse(runConfig.inputJson || '{}')
          } catch {
            input = {}
          }
        }

        const response = await functionsApi.invokeFunction(repo, functionName, {
          input,
          sync: true,
          timeout_ms: 30000,
        })

        // Parse and display logs from response
        const parsedLogs: LogEntry[] = []
        if (response.logs && response.logs.length > 0) {
          response.logs.forEach((log) => {
            const match = log.match(/^\[(\w+)\]\s*(.*)$/s)
            const levelStr = match ? match[1].toLowerCase() : 'info'
            const message = match ? match[2] : log
            const level = ['debug', 'info', 'warn', 'error'].includes(levelStr)
              ? levelStr as LogEntry['level']
              : 'info'
            const entry: LogEntry = {
              level,
              message,
              timestamp: new Date().toISOString(),
            }
            parsedLogs.push(entry)
            addLog(entry)
          })
        }

        const execution = {
          id: response.execution_id,
          execution_id: response.execution_id,
          function_path: activeTab.path,
          trigger_name: 'manual',
          status: response.error ? 'failed' as const : 'completed' as const,
          started_at: new Date(startTime).toISOString(),
          completed_at: new Date().toISOString(),
          duration_ms: response.duration_ms || Date.now() - startTime,
          result: response.result,
          error: response.error,
          logs: parsedLogs,
        }

        addExecution(execution)

        if (response.error) {
          addLog({
            level: 'error',
            message: `Execution failed: ${response.error}`,
            timestamp: new Date().toISOString(),
          })
        } else {
          addLog({
            level: 'info',
            message: `Execution completed in ${execution.duration_ms}ms`,
            timestamp: new Date().toISOString(),
          })
          if (response.result !== undefined) {
            addLog({
              level: 'info',
              message: `Result: ${JSON.stringify(response.result, null, 2)}`,
              timestamp: new Date().toISOString(),
            })
          }
        }
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : String(error)
        addLog({
          level: 'error',
          message: `Failed to invoke function: ${errorMessage}`,
          timestamp: new Date().toISOString(),
        })
      } finally {
        setIsRunning(false)
      }
    }
  }, [activeTab, selectedNode, isRunning, repo, runConfig, addLog, addExecution])

  // Handle node selection from QuickPick
  const handleNodeSelect = useCallback((nodeId: string, nodePath: string, workspace: string) => {
    // Extract node name from path for display
    const parts = nodePath.split('/')
    const name = parts[parts.length - 1] || nodePath

    // Add to recent nodes
    addToRecentNodes({
      id: nodeId,
      path: nodePath.replace(`${workspace}:`, ''),
      workspace,
      name,
      nodeType: 'unknown',
    })

    setRunConfig(prev => ({
      ...prev,
      inputNodeId: nodeId,
      inputNodePath: nodePath,
      inputWorkspace: workspace,
    }))
    setShowNodePicker(false)
  }, [])

  // Discover functions in JS files and remember the last-used handler per node
  useEffect(() => {
    if (!activeTab || !isJavaScriptAsset) {
      setAvailableHandlers([])
      return
    }

    if (isLoading) {
      setAvailableHandlers([])
      return
    }

    const functionsInFile = extractFunctionNames(localCode)
    setAvailableHandlers(functionsInFile)

    const savedHandler = getStoredHandler(activeTab.id)
    let nextHandler = runConfig.handler

    if (savedHandler && (!functionsInFile.length || functionsInFile.includes(savedHandler))) {
      nextHandler = savedHandler
    } else if (functionsInFile.length > 0 && !functionsInFile.includes(nextHandler)) {
      nextHandler = functionsInFile[0]
    }

    if (nextHandler !== runConfig.handler) {
      applyHandlerChange(nextHandler, true)
    }
  }, [activeTab?.id, isJavaScriptAsset, isLoading, localCode, runConfig.handler, applyHandlerChange])

  // Empty state - no tabs open
  if (openTabs.length === 0) {
    return (
      <div className="h-full flex flex-col items-center justify-center text-gray-400">
        <FileCode className="w-16 h-16 mb-4 opacity-50" />
        <p className="text-lg">No function open</p>
        <p className="text-sm mt-2">Select a function from the explorer to edit</p>
      </div>
    )
  }

  // Route to function editor for raisin:Function nodes
  if (activeTab?.node_type === 'raisin:Function') {
    return (
      <div className="h-full flex flex-col">
        <EditorTabs />
        <div className="flex-1 min-h-0">
          <RaisinFunctionNodeTypeEditor tab={activeTab} />
        </div>
      </div>
    )
  }

  // Route to trigger editor for raisin:Trigger nodes
  if (activeTab?.node_type === 'raisin:Trigger') {
    return (
      <div className="h-full flex flex-col">
        <EditorTabs />
        <div className="flex-1 min-h-0">
          <RaisinTriggerNodeTypeEditor tab={activeTab} />
        </div>
      </div>
    )
  }

  // Route to flow editor for raisin:Flow nodes
  if (activeTab?.node_type === 'raisin:Flow') {
    return (
      <div className="h-full flex flex-col">
        <EditorTabs />
        <div className="flex-1 min-h-0">
          <RaisinFlowNodeTypeEditor tab={activeTab} />
        </div>
      </div>
    )
  }

  // Route to agent editor for raisin:AIAgent nodes
  if (activeTab?.node_type === 'raisin:AIAgent') {
    return (
      <div className="h-full flex flex-col">
        <EditorTabs />
        <div className="flex-1 min-h-0">
          <RaisinAgentNodeTypeEditor tab={activeTab} />
        </div>
      </div>
    )
  }

  return (
    <div className="h-full flex flex-col">
      {/* Tabs */}
      <EditorTabs />

      {/* Toolbar */}
      <div className="flex-shrink-0 flex items-center gap-2 px-3 py-1.5 bg-black/20 border-b border-white/10">
        <button
          onClick={() => handleSave()}
          disabled={!activeTab?.isDirty}
          className={`
            flex items-center gap-1.5 px-2 py-1 rounded text-sm
            ${activeTab?.isDirty
              ? 'bg-primary-500/20 text-primary-300 hover:bg-primary-500/30'
              : 'text-gray-500 cursor-not-allowed'
            }
          `}
          title="Save (Ctrl+S)"
        >
          <Save className="w-4 h-4" />
          Save
        </button>

        <div className="h-5 w-px bg-white/10" />

        <div className="flex-1" />

        {activeTab && (
          <span className="text-xs text-gray-500">
            {(() => {
              const effectiveLanguage = activeTab.language || detectLanguageFromExtension(activeTab.name)
              return LANGUAGE_METADATA[effectiveLanguage]?.label || effectiveLanguage
            })()}
          </span>
        )}
      </div>

      {/* Run Configuration Bar */}
      <RunConfigBar
        showHandler={isStandaloneFile}
        config={runConfig}
        onConfigChange={handleRunConfigChange}
        onHandlerChange={handleHandlerChange}
        handlerOptions={isJavaScriptAsset ? availableHandlers : []}
        onRun={handleRun}
        isRunning={isRunning}
        onOpenNodePicker={() => setShowNodePicker(true)}
        disabled={!activeTab}
      />

      {/* Editor */}
      <div className="flex-1 min-h-0">
        {isLoading ? (
          <div className="h-full flex items-center justify-center text-gray-400">
            <Loader2 className="w-6 h-6 animate-spin mr-2" />
            Loading...
          </div>
        ) : hasNoFiles && activeTab ? (
          /* Empty state - function has no files yet (only for Function nodes) */
          (() => {
            const language = (selectedNode as FunctionNode | undefined)?.properties?.language as FunctionLanguage || 'javascript'
            const metadata = LANGUAGE_METADATA[language] || LANGUAGE_METADATA.javascript
            return (
              <div className="h-full flex flex-col items-center justify-center text-gray-400">
                <FileCode className="w-20 h-20 mb-6 opacity-30" />
                <h3 className="text-xl font-medium text-white mb-2">No source files yet</h3>
                <p className="text-sm text-gray-500 mb-6 max-w-md text-center">
                  This function doesn&apos;t have any source files. Create your first file to get started with coding.
                </p>
                <button
                  onClick={handleCreateFirstFile}
                  disabled={isCreatingFile || !selectedNode}
                  className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-400 text-white rounded-lg transition-colors disabled:opacity-50"
                >
                  {isCreatingFile ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : (
                    <Plus className="w-4 h-4" />
                  )}
                  {isCreatingFile ? 'Creating...' : `Create index.${metadata.ext}`}
                </button>
              </div>
            )
          })()
        ) : activeTab ? (
          (() => {
            // Detect language from file extension as fallback
            const effectiveLanguage = activeTab.language || detectLanguageFromExtension(activeTab.name)
            const languageLabel = LANGUAGE_METADATA[effectiveLanguage]?.label || effectiveLanguage
            return (
              <div className="h-full flex flex-col">
                {/* Empty file template prompt */}
                {showEmptyFilePrompt && (
                  <div className="flex-shrink-0 flex items-center gap-3 px-4 py-3 bg-primary-500/10 border-b border-primary-500/20">
                    <FileCode className="w-5 h-5 text-primary-400" />
                    <span className="text-sm text-gray-300">
                      This file is empty. Would you like to add a {languageLabel} function template?
                    </span>
                    <div className="flex items-center gap-2 ml-auto">
                      <button
                        onClick={handleApplyTemplate}
                        className="px-3 py-1 text-sm bg-primary-500 hover:bg-primary-400 text-white rounded transition-colors"
                      >
                        Add Template
                      </button>
                      <button
                        onClick={() => setShowEmptyFilePrompt(false)}
                        className="px-3 py-1 text-sm text-gray-400 hover:text-gray-300 transition-colors"
                      >
                        Dismiss
                      </button>
                    </div>
                  </div>
                )}
                <div className="flex-1 min-h-0">
                  {effectiveLanguage === 'sql' ? (
                    <SqlEditor
                      value={localCode}
                      onChange={handleCodeChange}
                      height="100%"
                      enableValidation={true}
                      repo={repo}
                      options={{
                        fontSize: preferences.fontSize,
                      }}
                    />
                  ) : effectiveLanguage === 'starlark' ? (
                    <StarlarkEditor
                      value={localCode}
                      onChange={handleCodeChange}
                      onSave={handleSave}
                      onRun={handleRun}
                      options={{
                        fontSize: preferences.fontSize,
                      }}
                    />
                  ) : (
                    <JavaScriptEditor
                      value={localCode}
                      onChange={handleCodeChange}
                      onSave={handleSave}
                      onRun={handleRun}
                      options={{
                        fontSize: preferences.fontSize,
                      }}
                    />
                  )}
                </div>
              </div>
            )
          })()
        ) : null}
      </div>

      {/* Commit Dialog */}
      {pendingCommit && (
        <CommitDialog
          title="Save Function"
          action={`Saving "${pendingCommit.name}"`}
          onCommit={executeCommit}
          onClose={() => setPendingCommit(null)}
        />
      )}

      {/* Create First File Dialog */}
      {showCreateFileDialog && selectedNode && (
        <CommitDialog
          title="Create Entry File"
          action={`Creating index file for "${selectedNode.name}"`}
          onCommit={executeCreateFirstFile}
          onClose={() => setShowCreateFileDialog(false)}
        />
      )}

      {/* Node Picker Modal */}
      {showNodePicker && (
        <QuickPick
          onSelect={handleNodeSelect}
          onClose={() => setShowNodePicker(false)}
          initialWorkspace={runConfig.inputWorkspace}
        />
      )}
    </div>
  )
}
