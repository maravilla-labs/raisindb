/**
 * FunctionExecutionCard Component
 *
 * Displays details for FunctionExecution job type including:
 * - Function path and trigger name
 * - Execution duration
 * - Logs with level coloring
 * - Error details with stack trace
 */

import { useMemo } from 'react'
import { Code, Clock, Zap, CheckCircle, XCircle, Terminal } from 'lucide-react'
import LogViewer, { parseLogs, type LogEntry } from './LogViewer'
import ErrorDetails from './ErrorDetails'
import { formatDuration } from '../../api/management'

export interface FunctionExecutionResult {
  execution_id: string
  success: boolean
  result?: unknown
  error?: string
  duration_ms: number
  logs: string[] | LogEntry[]
}

interface FunctionExecutionCardProps {
  /** Job type containing function_path, trigger_name, execution_id */
  jobType: {
    function_path?: string
    trigger_name?: string
    execution_id?: string
  } | string
  /** Job result containing execution details */
  result: FunctionExecutionResult | null
  /** Whether job is still running */
  isRunning?: boolean
}

export default function FunctionExecutionCard({
  jobType,
  result,
  isRunning = false,
}: FunctionExecutionCardProps) {
  // Extract function info from jobType
  const { functionPath, triggerName, executionId } = parseJobType(jobType)

  // Parse logs from result - memoized to prevent timestamp regeneration on re-renders
  const logs = useMemo(() => result ? parseLogs(result.logs) : [], [result])

  return (
    <div className="mt-3 space-y-3">
      {/* Function Info */}
      <div className="p-3 bg-white/5 rounded-lg border border-white/10">
        <div className="flex items-center gap-2 mb-2">
          <Code className="w-4 h-4 text-primary-400" />
          <span className="text-sm font-semibold text-white">Function Execution</span>
          {isRunning ? (
            <span className="px-2 py-0.5 bg-blue-500/20 text-blue-300 text-xs rounded-full animate-pulse">
              Running...
            </span>
          ) : result?.success ? (
            <span className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 text-green-300 text-xs rounded-full">
              <CheckCircle className="w-3 h-3" />
              Success
            </span>
          ) : result ? (
            <span className="flex items-center gap-1 px-2 py-0.5 bg-red-500/20 text-red-300 text-xs rounded-full">
              <XCircle className="w-3 h-3" />
              Failed
            </span>
          ) : null}
        </div>

        <div className="grid grid-cols-2 gap-x-4 gap-y-2 text-sm">
          <div className="flex items-center gap-2">
            <span className="text-zinc-500">Function:</span>
            <code className="text-primary-300 bg-primary-500/10 px-1 rounded font-mono text-xs">
              {functionPath || 'Unknown'}
            </code>
          </div>

          {triggerName && (
            <div className="flex items-center gap-2">
              <Zap className="w-3 h-3 text-yellow-400" />
              <span className="text-zinc-500">Trigger:</span>
              <span className="text-yellow-300">{triggerName}</span>
            </div>
          )}

          {result?.duration_ms !== undefined && (
            <div className="flex items-center gap-2">
              <Clock className="w-3 h-3 text-zinc-400" />
              <span className="text-zinc-500">Duration:</span>
              <span className="text-zinc-300">{formatDuration(result.duration_ms)}</span>
            </div>
          )}

          {executionId && (
            <div className="flex items-center gap-2">
              <span className="text-zinc-500">Execution ID:</span>
              <code className="text-zinc-400 font-mono text-xs">{executionId}</code>
            </div>
          )}
        </div>

        {/* Return Value (if success and has result) */}
        {result?.success && result.result !== undefined && result.result !== null && (
          <div className="mt-3 pt-3 border-t border-white/10">
            <div className="text-xs text-zinc-500 mb-1">Return Value:</div>
            <pre className="text-xs text-zinc-300 bg-black/30 p-2 rounded font-mono overflow-x-auto max-h-32 overflow-y-auto">
              {JSON.stringify(result.result, null, 2)}
            </pre>
          </div>
        )}
      </div>

      {/* Error Details */}
      {result?.error && (
        <ErrorDetails error={result.error} />
      )}

      {/* Logs */}
      {logs.length > 0 && (
        <div className="bg-white/5 rounded-lg border border-white/10 overflow-hidden">
          <div className="flex items-center gap-2 px-3 py-2 border-b border-white/10 bg-white/5">
            <Terminal className="w-4 h-4 text-zinc-400" />
            <span className="text-sm text-zinc-300">Console Output</span>
          </div>
          <LogViewer
            logs={logs}
            showHeader={false}
            maxHeight="200px"
            autoScroll={isRunning}
            showCopyButton={!isRunning}
          />
        </div>
      )}
    </div>
  )
}

/**
 * Parse job type to extract function execution details
 */
function parseJobType(jobType: FunctionExecutionCardProps['jobType']): {
  functionPath: string
  triggerName: string | null
  executionId: string | null
} {
  if (typeof jobType === 'string') {
    // Parse from string format: "FunctionExecution(/lib/func/trigger/exec-id)"
    const match = jobType.match(/FunctionExecution\(([^)]+)\)/)
    if (match) {
      const parts = match[1].split('/')
      if (parts.length >= 2) {
        // Last part is execution_id, second to last might be trigger_name
        const executionId = parts[parts.length - 1]
        const triggerName = parts.length >= 3 ? parts[parts.length - 2] : null
        const functionPath = parts.slice(0, parts.length - (triggerName ? 2 : 1)).join('/')
        return { functionPath: '/' + functionPath, triggerName, executionId }
      }
    }
    return { functionPath: jobType, triggerName: null, executionId: null }
  }

  return {
    functionPath: jobType.function_path || 'Unknown',
    triggerName: jobType.trigger_name || null,
    executionId: jobType.execution_id || null,
  }
}
