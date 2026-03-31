/**
 * FlowExecutionCard Component
 *
 * Displays details for FlowExecution job type with step-by-step visualization:
 * - Flow trigger path and execution ID
 * - Overall status and duration
 * - Collapsible step list with function results
 * - Error details per step
 */

import { useState, useMemo } from 'react'
import { Workflow, Clock, ChevronDown, ChevronRight, CheckCircle, XCircle, AlertCircle, Loader2, Circle } from 'lucide-react'
import ErrorDetails from './ErrorDetails'
import LogViewer, { parseLogs } from './LogViewer'
import { formatDuration } from '../../api/management'

// Flow execution result types
export interface FlowExecutionResult {
  flow_execution_id: string
  trigger_path: string
  status: 'completed' | 'partial_success' | 'failed' | 'timed_out' | 'running' | 'pending'
  started_at: string
  completed_at?: string
  duration_ms: number
  step_results: StepResult[]
  final_output?: unknown
  error?: string
}

export interface StepResult {
  step_id: string
  status: 'completed' | 'failed' | 'skipped' | 'running' | 'pending'
  function_results: FunctionResult[]
  duration_ms: number
  error?: string
}

export interface FunctionResult {
  function_path: string
  execution_id: string
  success: boolean
  result?: unknown
  error?: string
  duration_ms: number
  logs?: string[]
}

interface FlowExecutionCardProps {
  /** Job type containing flow execution info */
  jobType: {
    flow_execution_id?: string
    trigger_path?: string
    current_step_index?: number
  } | string
  /** Job result containing flow execution details */
  result: FlowExecutionResult | null
  /** Whether job is still running */
  isRunning?: boolean
  /** Job error message (if any) */
  jobError?: string | null
}

const STATUS_CONFIG = {
  completed: { icon: CheckCircle, color: 'text-green-400', bg: 'bg-green-500/20', label: 'Completed' },
  partial_success: { icon: AlertCircle, color: 'text-yellow-400', bg: 'bg-yellow-500/20', label: 'Partial' },
  failed: { icon: XCircle, color: 'text-red-400', bg: 'bg-red-500/20', label: 'Failed' },
  timed_out: { icon: Clock, color: 'text-orange-400', bg: 'bg-orange-500/20', label: 'Timeout' },
  running: { icon: Loader2, color: 'text-blue-400', bg: 'bg-blue-500/20', label: 'Running' },
  pending: { icon: Circle, color: 'text-zinc-400', bg: 'bg-zinc-500/20', label: 'Pending' },
  skipped: { icon: Circle, color: 'text-zinc-500', bg: 'bg-zinc-500/10', label: 'Skipped' },
}

export default function FlowExecutionCard({
  jobType,
  result,
  isRunning = false,
  jobError,
}: FlowExecutionCardProps) {
  const [expandedSteps, setExpandedSteps] = useState<Set<string>>(new Set())

  // Extract flow info from jobType
  const { flowExecutionId, triggerPath, currentStepIndex } = parseJobType(jobType)

  // Determine overall status
  const status = isRunning ? 'running' : (result?.status || 'pending')
  const statusConfig = STATUS_CONFIG[status] || STATUS_CONFIG.pending
  const StatusIcon = statusConfig.icon

  const toggleStep = (stepId: string) => {
    setExpandedSteps(prev => {
      const next = new Set(prev)
      if (next.has(stepId)) {
        next.delete(stepId)
      } else {
        next.add(stepId)
      }
      return next
    })
  }

  // Expand all failed steps by default
  const effectiveExpandedSteps = new Set(expandedSteps)
  if (result?.step_results) {
    result.step_results.forEach(step => {
      if (step.status === 'failed') {
        effectiveExpandedSteps.add(step.step_id)
      }
    })
  }

  return (
    <div className="mt-3 space-y-3">
      {/* Flow Info Header */}
      <div className="p-3 bg-white/5 rounded-lg border border-white/10">
        <div className="flex items-center gap-2 mb-2">
          <Workflow className="w-4 h-4 text-purple-400" />
          <span className="text-sm font-semibold text-white">Flow Execution</span>
          <span className={`flex items-center gap-1 px-2 py-0.5 ${statusConfig.bg} ${statusConfig.color} text-xs rounded-full`}>
            <StatusIcon className={`w-3 h-3 ${status === 'running' ? 'animate-spin' : ''}`} />
            {statusConfig.label}
          </span>
        </div>

        <div className="grid grid-cols-2 gap-x-4 gap-y-2 text-sm">
          {triggerPath && (
            <div className="flex items-center gap-2 col-span-2">
              <span className="text-zinc-500">Trigger:</span>
              <code className="text-purple-300 bg-purple-500/10 px-1 rounded font-mono text-xs">
                {triggerPath}
              </code>
            </div>
          )}

          {result?.duration_ms !== undefined && (
            <div className="flex items-center gap-2">
              <Clock className="w-3 h-3 text-zinc-400" />
              <span className="text-zinc-500">Duration:</span>
              <span className="text-zinc-300">{formatDuration(result.duration_ms)}</span>
            </div>
          )}

          {flowExecutionId && (
            <div className="flex items-center gap-2">
              <span className="text-zinc-500">Flow ID:</span>
              <code className="text-zinc-400 font-mono text-xs">{flowExecutionId}</code>
            </div>
          )}

          {isRunning && currentStepIndex !== undefined && (
            <div className="flex items-center gap-2">
              <span className="text-zinc-500">Current Step:</span>
              <span className="text-blue-300">{currentStepIndex + 1}</span>
            </div>
          )}
        </div>
      </div>

      {/* Flow-level Error */}
      {(result?.error || jobError) && (
        <ErrorDetails error={result?.error || jobError!} />
      )}

      {/* Step List */}
      {result?.step_results && result.step_results.length > 0 && (
        <div className="bg-white/5 rounded-lg border border-white/10 overflow-hidden">
          <div className="px-3 py-2 border-b border-white/10 bg-white/5">
            <span className="text-sm text-zinc-300">Steps ({result.step_results.length})</span>
          </div>
          <div className="divide-y divide-white/5">
            {result.step_results.map((step, idx) => {
              const stepStatus = STATUS_CONFIG[step.status] || STATUS_CONFIG.pending
              const StepIcon = stepStatus.icon
              const isExpanded = effectiveExpandedSteps.has(step.step_id)

              return (
                <div key={step.step_id} className="bg-white/0 hover:bg-white/5">
                  {/* Step Header */}
                  <button
                    onClick={() => toggleStep(step.step_id)}
                    className="w-full px-3 py-2 flex items-center gap-2 text-left"
                  >
                    {isExpanded ? (
                      <ChevronDown className="w-4 h-4 text-zinc-500" />
                    ) : (
                      <ChevronRight className="w-4 h-4 text-zinc-500" />
                    )}
                    <StepIcon className={`w-4 h-4 ${stepStatus.color} ${step.status === 'running' ? 'animate-spin' : ''}`} />
                    <span className="text-sm text-white">
                      Step {idx + 1}: {step.step_id}
                    </span>
                    <span className={`text-xs ${stepStatus.color}`}>
                      {stepStatus.label}
                    </span>
                    {step.duration_ms > 0 && (
                      <span className="text-xs text-zinc-500 ml-auto">
                        {formatDuration(step.duration_ms)}
                      </span>
                    )}
                  </button>

                  {/* Step Details (expanded) */}
                  {isExpanded && (
                    <div className="px-3 pb-3 ml-6 space-y-2">
                      {/* Step Error */}
                      {step.error && (
                        <ErrorDetails error={step.error} compact />
                      )}

                      {/* Function Results */}
                      {step.function_results.map((func, fidx) => (
                        <FunctionResultItem key={fidx} func={func} />
                      ))}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        </div>
      )}

      {/* Final Output */}
      {result?.final_output !== undefined && result.final_output !== null && (
        <div className="p-3 bg-white/5 rounded-lg border border-white/10">
          <div className="text-xs text-zinc-500 mb-1">Final Output:</div>
          <pre className="text-xs text-zinc-300 bg-black/30 p-2 rounded font-mono overflow-x-auto max-h-32 overflow-y-auto">
            {JSON.stringify(result.final_output, null, 2)}
          </pre>
        </div>
      )}
    </div>
  )
}

/**
 * Individual function result within a step
 */
function FunctionResultItem({ func }: { func: FunctionResult }) {
  const [showLogs, setShowLogs] = useState(false)
  // Memoize parsed logs to prevent timestamp regeneration on re-renders
  const logs = useMemo(() => parseLogs(func.logs || []), [func.logs])

  return (
    <div className={`p-2 rounded border ${func.success ? 'border-green-500/20 bg-green-500/5' : 'border-red-500/20 bg-red-500/5'}`}>
      <div className="flex items-center gap-2">
        {func.success ? (
          <CheckCircle className="w-3 h-3 text-green-400" />
        ) : (
          <XCircle className="w-3 h-3 text-red-400" />
        )}
        <code className="text-xs font-mono text-zinc-300">{func.function_path}</code>
        <span className="text-xs text-zinc-500">{formatDuration(func.duration_ms)}</span>
        {logs.length > 0 && (
          <button
            onClick={() => setShowLogs(!showLogs)}
            className="ml-auto text-xs text-zinc-500 hover:text-zinc-300"
          >
            {showLogs ? 'Hide' : 'Show'} logs ({logs.length})
          </button>
        )}
      </div>

      {/* Function Error */}
      {func.error && (
        <div className="mt-2">
          <ErrorDetails error={func.error} compact />
        </div>
      )}

      {/* Function Logs */}
      {showLogs && logs.length > 0 && (
        <div className="mt-2">
          <LogViewer
            logs={logs}
            showHeader={false}
            maxHeight="150px"
            compact
            autoScroll={false}
            showCopyButton={true}
          />
        </div>
      )}

      {/* Function Result */}
      {func.success && func.result !== undefined && func.result !== null && (
        <div className="mt-2">
          <div className="text-xs text-zinc-500 mb-1">Result:</div>
          <pre className="text-xs text-zinc-400 bg-black/20 p-1 rounded font-mono overflow-x-auto max-h-20 overflow-y-auto">
            {JSON.stringify(func.result, null, 2)}
          </pre>
        </div>
      )}
    </div>
  )
}

/**
 * Parse job type to extract flow execution details
 */
function parseJobType(jobType: FlowExecutionCardProps['jobType']): {
  flowExecutionId: string | null
  triggerPath: string | null
  currentStepIndex: number | undefined
} {
  if (typeof jobType === 'string') {
    // Parse from string format: "FlowExecution(trigger_path/flow_id/step:N)"
    const match = jobType.match(/FlowExecution\(([^)]+)\)/)
    if (match) {
      const parts = match[1].split('/')
      const stepPart = parts.find(p => p.startsWith('step:'))
      const stepIndex = stepPart ? parseInt(stepPart.replace('step:', ''), 10) : undefined

      // Remove step part and get flow_id (second to last)
      const nonStepParts = parts.filter(p => !p.startsWith('step:'))
      const flowId = nonStepParts[nonStepParts.length - 1]
      const triggerPath = nonStepParts.slice(0, nonStepParts.length - 1).join('/')

      return {
        flowExecutionId: flowId,
        triggerPath: '/' + triggerPath,
        currentStepIndex: stepIndex,
      }
    }
    return { flowExecutionId: null, triggerPath: jobType, currentStepIndex: undefined }
  }

  return {
    flowExecutionId: jobType.flow_execution_id || null,
    triggerPath: jobType.trigger_path || null,
    currentStepIndex: jobType.current_step_index,
  }
}
