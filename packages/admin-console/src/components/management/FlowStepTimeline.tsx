/**
 * FlowStepTimeline Component
 *
 * Visual timeline showing step-by-step execution of a flow:
 * - Vertical timeline with status indicators
 * - Expandable steps to view input/output/errors
 * - Color-coded by status
 * - Duration display
 * - Current step highlighting for running flows
 */

import { useState } from 'react'
import {
  ChevronDown,
  ChevronRight,
  CheckCircle,
  XCircle,
  Circle,
  Loader2,
  Clock,
  ArrowRight,
  AlertTriangle,
  Copy,
  Check
} from 'lucide-react'
import ErrorDetails from './ErrorDetails'
import type { FlowStepExecution } from './FlowInstanceDetail'

interface FlowStepTimelineProps {
  steps: FlowStepExecution[]
  currentNodeId?: string
}

const STATUS_CONFIG = {
  pending: {
    icon: Circle,
    color: 'text-zinc-400',
    bg: 'bg-zinc-500/20',
    border: 'border-zinc-500/30',
    label: 'Pending'
  },
  running: {
    icon: Loader2,
    color: 'text-blue-400',
    bg: 'bg-blue-500/20',
    border: 'border-blue-500/30',
    label: 'Running'
  },
  completed: {
    icon: CheckCircle,
    color: 'text-green-400',
    bg: 'bg-green-500/20',
    border: 'border-green-500/30',
    label: 'Completed'
  },
  failed: {
    icon: XCircle,
    color: 'text-red-400',
    bg: 'bg-red-500/20',
    border: 'border-red-500/30',
    label: 'Failed'
  },
  skipped: {
    icon: Circle,
    color: 'text-zinc-500',
    bg: 'bg-zinc-500/10',
    border: 'border-zinc-500/20',
    label: 'Skipped'
  },
}

export default function FlowStepTimeline({ steps, currentNodeId }: FlowStepTimelineProps) {
  const [expandedSteps, setExpandedSteps] = useState<Set<string>>(new Set())
  const [copiedField, setCopiedField] = useState<string | null>(null)

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

  const formatDuration = (startedAt: string, completedAt?: string): string => {
    const start = new Date(startedAt).getTime()
    const end = completedAt ? new Date(completedAt).getTime() : Date.now()
    const ms = end - start

    if (ms < 1000) return `${ms}ms`
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
    if (ms < 3600000) return `${Math.floor(ms / 60000)}m ${Math.floor((ms % 60000) / 1000)}s`
    return `${Math.floor(ms / 3600000)}h ${Math.floor((ms % 3600000) / 60000)}m`
  }

  const copyToClipboard = (text: string, field: string) => {
    navigator.clipboard.writeText(text).then(() => {
      setCopiedField(field)
      setTimeout(() => setCopiedField(null), 2000)
    })
  }

  const CopyButton = ({ text, field }: { text: string; field: string }) => (
    <button
      onClick={(e) => {
        e.stopPropagation()
        copyToClipboard(text, field)
      }}
      className="p-1 text-zinc-400 hover:text-white hover:bg-white/10 rounded transition-colors"
      title="Copy to clipboard"
    >
      {copiedField === field ? (
        <Check className="w-3 h-3 text-green-400" />
      ) : (
        <Copy className="w-3 h-3" />
      )}
    </button>
  )

  if (steps.length === 0) {
    return (
      <div className="bg-white/5 border border-white/10 rounded-lg p-8 text-center">
        <Circle className="w-12 h-12 text-zinc-500 mx-auto mb-3" />
        <p className="text-zinc-400">No steps executed yet</p>
      </div>
    )
  }

  return (
    <div className="bg-white/5 border border-white/10 rounded-lg overflow-hidden">
      <div className="divide-y divide-white/5">
        {steps.map((step, idx) => {
          const statusConfig = STATUS_CONFIG[step.status] || STATUS_CONFIG.pending
          const StatusIcon = statusConfig.icon
          const isExpanded = expandedSteps.has(step.id)
          const isCurrent = step.node_id === currentNodeId
          const isLast = idx === steps.length - 1

          return (
            <div
              key={step.id}
              className={`relative ${isCurrent ? 'bg-purple-500/10' : 'hover:bg-white/5'}`}
            >
              {/* Timeline connector */}
              {!isLast && (
                <div className="absolute left-6 top-10 bottom-0 w-0.5 bg-white/10"></div>
              )}

              {/* Step Header */}
              <button
                onClick={() => toggleStep(step.id)}
                className="w-full px-4 py-3 flex items-start gap-3 text-left"
              >
                {/* Status Icon */}
                <div className={`relative z-10 p-1.5 rounded-full ${statusConfig.bg} border ${statusConfig.border}`}>
                  <StatusIcon className={`w-4 h-4 ${statusConfig.color} ${
                    step.status === 'running' ? 'animate-spin' : ''
                  }`} />
                </div>

                {/* Step Info */}
                <div className="flex-1 min-w-0 pt-0.5">
                  <div className="flex items-center gap-2 flex-wrap mb-1">
                    <span className="font-medium text-white">
                      {step.step_name || step.node_id}
                    </span>
                    <span className={`text-xs px-2 py-0.5 rounded ${statusConfig.bg} ${statusConfig.color}`}>
                      {statusConfig.label}
                    </span>
                    {isCurrent && (
                      <span className="text-xs px-2 py-0.5 rounded bg-purple-500/20 text-purple-300">
                        Current
                      </span>
                    )}
                    {step.iteration !== undefined && (
                      <span className="text-xs px-2 py-0.5 rounded bg-zinc-500/20 text-zinc-400">
                        Iteration {step.iteration}
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-zinc-500 flex items-center gap-3 flex-wrap">
                    <span className="flex items-center gap-1">
                      <Clock className="w-3 h-3" />
                      {formatDuration(step.started_at, step.completed_at)}
                    </span>
                    <span>{new Date(step.started_at).toLocaleString()}</span>
                  </div>
                </div>

                {/* Expand Icon */}
                {isExpanded ? (
                  <ChevronDown className="w-5 h-5 text-zinc-400 flex-shrink-0" />
                ) : (
                  <ChevronRight className="w-5 h-5 text-zinc-400 flex-shrink-0" />
                )}
              </button>

              {/* Expanded Details */}
              {isExpanded && (
                <div className="px-4 pb-4 ml-12 space-y-3">
                  {/* Input */}
                  {Object.keys(step.input).length > 0 && (
                    <div>
                      <div className="flex items-center justify-between mb-1">
                        <div className="text-xs font-medium text-zinc-400 flex items-center gap-1">
                          <ArrowRight className="w-3 h-3" />
                          Input
                        </div>
                        <CopyButton
                          text={JSON.stringify(step.input, null, 2)}
                          field={`input-${step.id}`}
                        />
                      </div>
                      <pre className="bg-black/30 border border-white/10 rounded p-2 text-xs text-zinc-300 font-mono overflow-x-auto max-h-32 overflow-y-auto">
                        {JSON.stringify(step.input, null, 2)}
                      </pre>
                    </div>
                  )}

                  {/* Output */}
                  {step.output && Object.keys(step.output).length > 0 && (
                    <div>
                      <div className="flex items-center justify-between mb-1">
                        <div className="text-xs font-medium text-zinc-400 flex items-center gap-1">
                          <ArrowRight className="w-3 h-3 text-green-400" />
                          Output
                        </div>
                        <CopyButton
                          text={JSON.stringify(step.output, null, 2)}
                          field={`output-${step.id}`}
                        />
                      </div>
                      <pre className="bg-black/30 border border-white/10 rounded p-2 text-xs text-zinc-300 font-mono overflow-x-auto max-h-32 overflow-y-auto">
                        {JSON.stringify(step.output, null, 2)}
                      </pre>
                    </div>
                  )}

                  {/* Error */}
                  {step.error && (
                    <div>
                      <div className="text-xs font-medium text-red-400 mb-1 flex items-center gap-1">
                        <AlertTriangle className="w-3 h-3" />
                        Error
                      </div>
                      <ErrorDetails error={step.error} compact />
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
