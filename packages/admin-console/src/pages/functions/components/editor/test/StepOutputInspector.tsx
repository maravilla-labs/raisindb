/**
 * Step Output Inspector
 *
 * Displays real-time step outputs during workflow test execution.
 * Shows outputs for completed steps and errors for failed steps.
 */

import { useState, useMemo } from 'react'
import {
  ChevronDown,
  ChevronRight,
  CheckCircle2,
  XCircle,
  Clock,
  Play,
  Pause,
} from 'lucide-react'

export interface StepOutput {
  stepId: string
  stepName: string
  status: 'pending' | 'running' | 'completed' | 'failed' | 'waiting'
  output?: unknown
  error?: string
  durationMs?: number
  timestamp?: string
}

export interface StepOutputInspectorProps {
  /** Map of step outputs by step ID */
  stepOutputs: Map<string, StepOutput>
  /** Currently selected step ID */
  selectedStepId?: string | null
  /** Called when a step is selected */
  onSelectStep?: (stepId: string) => void
}

const STATUS_CONFIG = {
  pending: {
    icon: Clock,
    color: 'text-gray-500',
    bg: 'bg-gray-500/10',
    label: 'Pending',
  },
  running: {
    icon: Play,
    color: 'text-blue-400',
    bg: 'bg-blue-500/10',
    label: 'Running',
  },
  completed: {
    icon: CheckCircle2,
    color: 'text-green-400',
    bg: 'bg-green-500/10',
    label: 'Completed',
  },
  failed: {
    icon: XCircle,
    color: 'text-red-400',
    bg: 'bg-red-500/10',
    label: 'Failed',
  },
  waiting: {
    icon: Pause,
    color: 'text-amber-400',
    bg: 'bg-amber-500/10',
    label: 'Waiting',
  },
}

function JsonViewer({ data, maxHeight = 200 }: { data: unknown; maxHeight?: number }) {
  const [expanded, setExpanded] = useState(true)

  const jsonString = useMemo(() => {
    try {
      return JSON.stringify(data, null, 2)
    } catch {
      return String(data)
    }
  }, [data])

  const isLong = jsonString.split('\n').length > 10

  return (
    <div className="relative">
      {isLong && (
        <button
          onClick={() => setExpanded(!expanded)}
          className="absolute top-1 right-1 p-1 text-gray-500 hover:text-white rounded hover:bg-white/10 z-10"
        >
          {expanded ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
        </button>
      )}
      <pre
        className={`text-xs font-mono text-gray-300 bg-black/40 rounded p-2 overflow-auto transition-all ${
          !expanded ? 'max-h-20' : ''
        }`}
        style={{ maxHeight: expanded ? maxHeight : 80 }}
      >
        {jsonString}
      </pre>
    </div>
  )
}

export function StepOutputInspector({
  stepOutputs,
  selectedStepId,
  onSelectStep,
}: StepOutputInspectorProps) {
  const [expandedSteps, setExpandedSteps] = useState<Set<string>>(new Set())

  // Sort outputs by timestamp (most recent first)
  const sortedOutputs = useMemo(() => {
    return Array.from(stepOutputs.values()).sort((a, b) => {
      if (!a.timestamp && !b.timestamp) return 0
      if (!a.timestamp) return 1
      if (!b.timestamp) return -1
      return new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime()
    })
  }, [stepOutputs])

  // Count by status
  const statusCounts = useMemo(() => {
    const counts = { pending: 0, running: 0, completed: 0, failed: 0, waiting: 0 }
    for (const output of stepOutputs.values()) {
      counts[output.status]++
    }
    return counts
  }, [stepOutputs])

  const toggleExpand = (stepId: string) => {
    setExpandedSteps((prev) => {
      const next = new Set(prev)
      if (next.has(stepId)) {
        next.delete(stepId)
      } else {
        next.add(stepId)
      }
      return next
    })
  }

  if (stepOutputs.size === 0) {
    return (
      <div className="flex items-center justify-center h-32 text-sm text-gray-500">
        No step outputs yet. Run the workflow to see results.
      </div>
    )
  }

  return (
    <div className="space-y-2">
      {/* Status summary */}
      <div className="flex items-center gap-3 px-2 py-1.5 bg-white/5 rounded-lg">
        {Object.entries(statusCounts)
          .filter(([, count]) => count > 0)
          .map(([status, count]) => {
            const config = STATUS_CONFIG[status as keyof typeof STATUS_CONFIG]
            return (
              <div key={status} className="flex items-center gap-1.5">
                <config.icon className={`w-3.5 h-3.5 ${config.color}`} />
                <span className="text-xs text-gray-400">
                  {count} {config.label.toLowerCase()}
                </span>
              </div>
            )
          })}
      </div>

      {/* Step list */}
      <div className="space-y-1 max-h-80 overflow-y-auto">
        {sortedOutputs.map((output) => {
          const config = STATUS_CONFIG[output.status]
          const StatusIcon = config.icon
          const isExpanded = expandedSteps.has(output.stepId)
          const isSelected = selectedStepId === output.stepId
          const hasContent = output.output !== undefined || output.error !== undefined

          return (
            <div
              key={output.stepId}
              className={`border rounded-lg overflow-hidden transition-colors ${
                isSelected
                  ? 'border-blue-500/50 bg-blue-500/5'
                  : 'border-white/10 bg-white/5 hover:bg-white/10'
              }`}
            >
              {/* Step header */}
              <button
                onClick={() => {
                  if (hasContent) toggleExpand(output.stepId)
                  onSelectStep?.(output.stepId)
                }}
                className="w-full flex items-center gap-2 px-3 py-2 text-left"
              >
                {hasContent && (
                  <span className="text-gray-500">
                    {isExpanded ? (
                      <ChevronDown className="w-3.5 h-3.5" />
                    ) : (
                      <ChevronRight className="w-3.5 h-3.5" />
                    )}
                  </span>
                )}
                <StatusIcon className={`w-4 h-4 ${config.color}`} />
                <span className="flex-1 text-sm font-medium text-white truncate">
                  {output.stepName}
                </span>
                {output.durationMs !== undefined && (
                  <span className="text-xs text-gray-500">{output.durationMs}ms</span>
                )}
              </button>

              {/* Step content */}
              {isExpanded && hasContent && (
                <div className="px-3 pb-3 space-y-2">
                  {output.error && (
                    <div className="p-2 bg-red-500/10 border border-red-500/30 rounded">
                      <p className="text-xs font-medium text-red-400 mb-1">Error</p>
                      <p className="text-xs text-red-300">{output.error}</p>
                    </div>
                  )}
                  {output.output !== undefined && (
                    <div>
                      <p className="text-xs font-medium text-gray-400 mb-1">Output</p>
                      <JsonViewer data={output.output} />
                    </div>
                  )}
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}

export default StepOutputInspector
