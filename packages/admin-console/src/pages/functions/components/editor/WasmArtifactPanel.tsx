/**
 * WebAssembly Artifact Panel
 *
 * A `.wasm` artifact has no editable source on the server, so it gets this
 * panel instead of Monaco: what the artifact IS (size, content hash, when it
 * last changed, the world it must export), how to REPLACE it (upload new
 * bytes), and how to RUN the function that points at it. Output goes to the
 * shared OutputPanel through the context log, exactly like every other run.
 */

import { useCallback, useEffect, useState } from 'react'
import { Loader2, Play, RefreshCw, Upload, Binary } from 'lucide-react'
import { useFunctionsContext } from '../../hooks'
import { nodesApi, type Node as NodeType } from '../../../../api/nodes'
import { functionsApi } from '../../../../api/functions'
import CommitDialog from '../../../../components/CommitDialog'
import {
  WASM_WORLD,
  artifactHash,
  artifactSize,
  artifactUpdatedAt,
  formatBytes,
  handlerOf,
} from './wasmArtifactMeta'
import type { EditorTab, LogEntry } from '../../types'

interface WasmArtifactPanelProps {
  /** The tab holding the `.wasm` Asset node. */
  tab: EditorTab
}

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-3 py-1">
      <span className="w-32 flex-shrink-0 text-xs uppercase tracking-wide text-gray-500">{label}</span>
      <span className="text-sm text-gray-200 break-all font-mono">{value}</span>
    </div>
  )
}

export function WasmArtifactPanel({ tab }: WasmArtifactPanelProps) {
  const { repo, branch, workspace, addLog, addExecution } = useFunctionsContext()

  const [artifact, setArtifact] = useState<NodeType | null>(null)
  const [functionNode, setFunctionNode] = useState<NodeType | null>(null)
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [pendingFile, setPendingFile] = useState<File | null>(null)
  const [isUploading, setIsUploading] = useState(false)
  const [isRunning, setIsRunning] = useState(false)
  const [inputJson, setInputJson] = useState('{}')

  const parentPath = tab.path.split('/').slice(0, -1).join('/') || '/'

  const load = useCallback(async () => {
    if (!repo || !branch) return
    setLoading(true)
    setLoadError(null)
    try {
      const node = await nodesApi.getAtHead(repo, branch, workspace, tab.path)
      setArtifact(node)
      try {
        const parent = await nodesApi.getAtHead(repo, branch, workspace, parentPath)
        setFunctionNode(parent.node_type === 'raisin:Function' ? parent : null)
      } catch {
        setFunctionNode(null)
      }
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : String(error))
    } finally {
      setLoading(false)
    }
  }, [repo, branch, workspace, tab.path, parentPath])

  useEffect(() => {
    load()
  }, [load])

  const executeReplace = useCallback(
    async (message: string, actor: string) => {
      if (!pendingFile || !repo || !branch) return
      setIsUploading(true)
      try {
        const blob = new Blob([await pendingFile.arrayBuffer()], { type: 'application/wasm' })
        await nodesApi.uploadFile(repo, branch, workspace, tab.path, {
          file: blob,
          fileName: tab.name,
          inline: false,
          propertyPath: 'file',
          overrideExisting: true,
          commitMessage: message,
          commitActor: actor,
        })
        addLog({
          level: 'info',
          message: `Replaced ${tab.name} (${formatBytes(pendingFile.size)}) from ${pendingFile.name}`,
          timestamp: new Date().toISOString(),
        })
        setPendingFile(null)
        await load()
      } catch (error) {
        addLog({
          level: 'error',
          message: `Failed to replace artifact: ${error instanceof Error ? error.message : String(error)}`,
          timestamp: new Date().toISOString(),
        })
      } finally {
        setIsUploading(false)
      }
    },
    [pendingFile, repo, branch, workspace, tab.path, tab.name, addLog, load]
  )

  const handleRun = useCallback(async () => {
    if (isRunning || !repo) return
    const name =
      (typeof functionNode?.properties?.name === 'string' && functionNode.properties.name) ||
      functionNode?.name ||
      parentPath.split('/').filter(Boolean).slice(-1)[0]
    if (!name) {
      addLog({
        level: 'error',
        message: 'This artifact has no parent function node to invoke.',
        timestamp: new Date().toISOString(),
      })
      return
    }

    let input: Record<string, unknown> = {}
    try {
      input = JSON.parse(inputJson || '{}')
    } catch (error) {
      addLog({
        level: 'error',
        message: `Input is not valid JSON: ${error instanceof Error ? error.message : String(error)}`,
        timestamp: new Date().toISOString(),
      })
      return
    }

    setIsRunning(true)
    const startTime = Date.now()
    addLog({
      level: 'info',
      message: `Invoking wasm function "${name}" (handler "${handlerOf(functionNode)}")...`,
      timestamp: new Date().toISOString(),
    })

    try {
      const response = await functionsApi.invokeFunction(repo, name, {
        input,
        sync: true,
        timeout_ms: 30000,
      })

      const parsedLogs: LogEntry[] = (response.logs || []).map((log) => {
        const match = log.match(/^\[(\w+)\]\s*(.*)$/s)
        const levelStr = match ? match[1].toLowerCase() : 'info'
        const level = (['debug', 'info', 'warn', 'error'].includes(levelStr)
          ? levelStr
          : 'info') as LogEntry['level']
        return { level, message: match ? match[2] : log, timestamp: new Date().toISOString() }
      })
      parsedLogs.forEach(addLog)

      addExecution({
        id: response.execution_id,
        execution_id: response.execution_id,
        function_path: parentPath,
        trigger_name: 'manual',
        status: response.error ? 'failed' : 'completed',
        started_at: new Date(startTime).toISOString(),
        completed_at: new Date().toISOString(),
        duration_ms: response.duration_ms || Date.now() - startTime,
        result: response.result,
        error: response.error,
        logs: parsedLogs,
      })

      addLog({
        level: response.error ? 'error' : 'info',
        message: response.error
          ? `Execution failed: ${response.error}`
          : `Execution completed in ${response.duration_ms ?? Date.now() - startTime}ms`,
        timestamp: new Date().toISOString(),
      })
      if (!response.error && response.result !== undefined) {
        addLog({
          level: 'info',
          message: `Result: ${JSON.stringify(response.result, null, 2)}`,
          timestamp: new Date().toISOString(),
        })
      }
    } catch (error) {
      addLog({
        level: 'error',
        message: `Failed to invoke function: ${error instanceof Error ? error.message : String(error)}`,
        timestamp: new Date().toISOString(),
      })
    } finally {
      setIsRunning(false)
    }
  }, [isRunning, repo, functionNode, parentPath, inputJson, addLog, addExecution])

  const size = artifactSize(artifact)
  const hash = artifactHash(artifact)
  const updatedAt = artifactUpdatedAt(artifact)

  return (
    <div className="h-full overflow-auto p-6">
      <div className="max-w-2xl">
        <div className="flex items-center gap-3 mb-1">
          <Binary className="w-6 h-6 text-orange-400" />
          <h2 className="text-lg font-medium text-white">{tab.name}</h2>
          <button
            onClick={load}
            disabled={loading}
            className="ml-auto p-1.5 text-gray-400 hover:text-white hover:bg-white/10 rounded disabled:opacity-50"
            title="Reload artifact metadata"
          >
            <RefreshCw className={`w-4 h-4 ${loading ? 'animate-spin' : ''}`} />
          </button>
        </div>
        <p className="text-sm text-gray-500 mb-6">
          A WebAssembly component has no editable source here. Build it locally, then replace the
          artifact.
        </p>

        {loadError && (
          <div className="mb-4 px-3 py-2 rounded bg-red-500/10 border border-red-500/30 text-sm text-red-300">
            {loadError}
          </div>
        )}

        <div className="rounded-lg border border-white/10 bg-black/20 p-4 mb-6">
          <Row label="Path" value={tab.path} />
          <Row label="Size" value={size === null ? '—' : `${formatBytes(size)} (${size} bytes)`} />
          <Row label="Content hash" value={hash || '—'} />
          <Row label="Updated" value={updatedAt ? new Date(updatedAt).toLocaleString() : '—'} />
          <Row label="World" value={WASM_WORLD} />
          <Row label="Handler" value={handlerOf(functionNode)} />
        </div>

        <div className="flex flex-wrap items-center gap-3 mb-6">
          <label
            className={`flex items-center gap-2 px-3 py-2 rounded bg-primary-500/20 text-primary-300 text-sm cursor-pointer hover:bg-primary-500/30 ${
              isUploading ? 'opacity-50 pointer-events-none' : ''
            }`}
          >
            {isUploading ? <Loader2 className="w-4 h-4 animate-spin" /> : <Upload className="w-4 h-4" />}
            Replace artifact
            <input
              type="file"
              accept=".wasm,application/wasm"
              className="hidden"
              onChange={(e) => {
                const file = e.target.files?.[0]
                e.target.value = ''
                if (file) setPendingFile(file)
              }}
            />
          </label>

          <button
            onClick={handleRun}
            disabled={isRunning}
            className="flex items-center gap-2 px-3 py-2 rounded bg-green-500/20 text-green-300 text-sm hover:bg-green-500/30 disabled:opacity-50"
          >
            {isRunning ? <Loader2 className="w-4 h-4 animate-spin" /> : <Play className="w-4 h-4" />}
            Run
          </button>
        </div>

        <label className="block text-xs uppercase tracking-wide text-gray-500 mb-1">Input (JSON)</label>
        <textarea
          value={inputJson}
          onChange={(e) => setInputJson(e.target.value)}
          rows={6}
          spellCheck={false}
          className="w-full px-3 py-2 font-mono text-sm bg-black/30 border border-white/20 rounded text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
        />
      </div>

      {pendingFile && (
        <CommitDialog
          title="Replace Artifact"
          action={`Uploading "${pendingFile.name}" to ${tab.path}`}
          onCommit={executeReplace}
          onClose={() => setPendingFile(null)}
        />
      )}
    </div>
  )
}
