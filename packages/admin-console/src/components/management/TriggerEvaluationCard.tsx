/**
 * TriggerEvaluationCard Component
 *
 * Displays detailed debug information for TriggerEvaluation job type including:
 * - Event details (type, node_id, node_type, path, workspace)
 * - Summary stats (triggers evaluated, matched, duration)
 * - Expandable list of each trigger with filter check details
 */

import { useState } from 'react'
import { ChevronDown, ChevronRight, Clock, CheckCircle, XCircle, Zap, Target, FileCode, Box } from 'lucide-react'
import { formatDuration } from '../../api/management'

export interface FilterCheckResult {
  filter_name: string
  passed: boolean
  expected: unknown
  actual: unknown
  reason: string
}

export interface TriggerEvaluationResult {
  trigger_path: string
  trigger_name: string
  matched: boolean
  filter_checks: FilterCheckResult[]
  enqueued_job_id: string | null
}

export interface TriggerEventInfo {
  event_type: string
  node_id: string
  node_type: string
  node_path: string
  workspace: string
  node_properties: unknown
}

export interface TriggerEvaluationReport {
  event: TriggerEventInfo
  triggers_evaluated: number
  triggers_matched: number
  trigger_results: TriggerEvaluationResult[]
  duration_ms: number
}

interface TriggerEvaluationCardProps {
  result: TriggerEvaluationReport
}

export default function TriggerEvaluationCard({ result }: TriggerEvaluationCardProps) {
  const [expandedTriggers, setExpandedTriggers] = useState<Set<string>>(new Set())

  const toggleTrigger = (triggerId: string) => {
    setExpandedTriggers(prev => {
      const next = new Set(prev)
      if (next.has(triggerId)) {
        next.delete(triggerId)
      } else {
        next.add(triggerId)
      }
      return next
    })
  }

  return (
    <div className="mt-3 space-y-3">
      {/* Event Details */}
      <div className="p-3 bg-white/5 rounded-lg border border-white/10">
        <div className="flex items-center gap-2 mb-3">
          <Target className="w-4 h-4 text-primary-400" />
          <span className="text-sm font-semibold text-white">Event Details</span>
        </div>

        <div className="grid grid-cols-2 gap-x-4 gap-y-2 text-sm">
          <div className="flex items-center gap-2">
            <Zap className="w-3 h-3 text-yellow-400" />
            <span className="text-zinc-500">Type:</span>
            <span className="text-yellow-300">{result.event.event_type}</span>
          </div>

          <div className="flex items-center gap-2">
            <Box className="w-3 h-3 text-blue-400" />
            <span className="text-zinc-500">Node Type:</span>
            <code className="text-blue-300 bg-blue-500/10 px-1 rounded font-mono text-xs">
              {result.event.node_type}
            </code>
          </div>

          <div className="flex items-center gap-2 col-span-2">
            <FileCode className="w-3 h-3 text-zinc-400" />
            <span className="text-zinc-500">Path:</span>
            <code className="text-zinc-300 font-mono text-xs truncate">{result.event.node_path}</code>
          </div>

          <div className="flex items-center gap-2">
            <span className="text-zinc-500">Workspace:</span>
            <span className="text-zinc-300">{result.event.workspace}</span>
          </div>

          <div className="flex items-center gap-2">
            <span className="text-zinc-500">Node ID:</span>
            <code className="text-zinc-400 font-mono text-xs truncate">{result.event.node_id}</code>
          </div>
        </div>
      </div>

      {/* Summary Stats */}
      <div className="p-3 bg-white/5 rounded-lg border border-white/10">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-4 text-sm">
            <span className="text-zinc-400">
              Evaluated: <span className="text-white font-medium">{result.triggers_evaluated}</span> triggers
            </span>
            <span className="text-zinc-400">
              Matched:{' '}
              <span className={result.triggers_matched > 0 ? 'text-green-400 font-medium' : 'text-zinc-300 font-medium'}>
                {result.triggers_matched}
              </span>
            </span>
          </div>
          <div className="flex items-center gap-2 text-sm text-zinc-400">
            <Clock className="w-3 h-3" />
            {formatDuration(result.duration_ms)}
          </div>
        </div>
      </div>

      {/* Trigger Results */}
      {result.trigger_results.length > 0 && (
        <div className="bg-white/5 rounded-lg border border-white/10 overflow-hidden">
          <div className="px-3 py-2 border-b border-white/10 bg-white/5">
            <span className="text-sm text-zinc-300">Trigger Evaluation Results</span>
          </div>
          <div className="divide-y divide-white/5">
            {result.trigger_results.map((triggerResult, index) => {
              const triggerId = `${triggerResult.trigger_path}-${index}`
              const isExpanded = expandedTriggers.has(triggerId)

              return (
                <div key={triggerId}>
                  <button
                    onClick={() => toggleTrigger(triggerId)}
                    className="w-full px-3 py-2 flex items-center gap-2 hover:bg-white/5 transition-colors text-left"
                  >
                    {isExpanded ? (
                      <ChevronDown className="w-4 h-4 text-zinc-400" />
                    ) : (
                      <ChevronRight className="w-4 h-4 text-zinc-400" />
                    )}
                    {triggerResult.matched ? (
                      <CheckCircle className="w-4 h-4 text-green-400" />
                    ) : (
                      <XCircle className="w-4 h-4 text-red-400" />
                    )}
                    <span className="text-sm text-white font-medium">{triggerResult.trigger_name}</span>
                    <span className="text-xs text-zinc-500 truncate flex-1">
                      {triggerResult.trigger_path}
                    </span>
                    {triggerResult.enqueued_job_id && (
                      <span className="px-1.5 py-0.5 bg-green-500/20 text-green-300 text-[10px] rounded">
                        Job: {triggerResult.enqueued_job_id.slice(0, 8)}...
                      </span>
                    )}
                  </button>

                  {isExpanded && (
                    <div className="px-4 py-2 bg-black/20 space-y-1">
                      {triggerResult.filter_checks.map((check, checkIndex) => (
                        <FilterCheckRow key={checkIndex} check={check} />
                      ))}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        </div>
      )}

      {result.trigger_results.length === 0 && (
        <div className="p-4 bg-white/5 rounded-lg border border-white/10 text-center">
          <span className="text-sm text-zinc-500">No triggers found in the functions workspace</span>
        </div>
      )}
    </div>
  )
}

function FilterCheckRow({ check }: { check: FilterCheckResult }) {
  const [showDetails, setShowDetails] = useState(false)

  return (
    <div className="text-xs">
      <button
        onClick={() => setShowDetails(!showDetails)}
        className="w-full flex items-center gap-2 py-1 hover:bg-white/5 rounded px-1 -mx-1 transition-colors text-left"
      >
        {check.passed ? (
          <CheckCircle className="w-3 h-3 text-green-400 shrink-0" />
        ) : (
          <XCircle className="w-3 h-3 text-red-400 shrink-0" />
        )}
        <span className={`font-mono ${check.passed ? 'text-zinc-300' : 'text-red-300'}`}>
          {check.filter_name}
        </span>
        <span className="text-zinc-500 truncate flex-1">
          {check.reason}
        </span>
        {(check.expected !== null || check.actual !== null) && (
          <ChevronDown className={`w-3 h-3 text-zinc-500 shrink-0 transition-transform ${showDetails ? '' : '-rotate-90'}`} />
        )}
      </button>

      {showDetails && (check.expected !== null || check.actual !== null) && (
        <div className="ml-5 pl-2 border-l border-white/10 mt-1 space-y-1">
          {check.expected !== null && (
            <div className="flex gap-2">
              <span className="text-zinc-500 shrink-0">Expected:</span>
              <code className="text-zinc-300 bg-black/30 px-1 rounded font-mono overflow-x-auto">
                {JSON.stringify(check.expected)}
              </code>
            </div>
          )}
          {check.actual !== null && (
            <div className="flex gap-2">
              <span className="text-zinc-500 shrink-0">Actual:</span>
              <code className={`${check.passed ? 'text-green-300' : 'text-red-300'} bg-black/30 px-1 rounded font-mono overflow-x-auto`}>
                {JSON.stringify(check.actual)}
              </code>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
