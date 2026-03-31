/**
 * FlowInstanceDetail Component
 *
 * Detailed view of a single flow instance showing:
 * - Instance properties and metadata
 * - Step-by-step execution timeline
 * - Variables/context at each step
 * - Error details and stack traces
 * - Compensation stack (if any)
 * - Real-time updates for running flows
 */

import { useEffect, useState } from 'react'
import {
  X,
  Workflow,
  Clock,
  Code,
  AlertTriangle,
  ChevronLeft,
  Copy,
  Check,
  ArrowRight,
  Undo2,
  Package
} from 'lucide-react'
import ErrorDetails from './ErrorDetails'
import FlowStepTimeline from './FlowStepTimeline'
import type { FlowInstance } from '../../pages/management/FlowExecutionMonitor'

export interface FlowStepExecution {
  id: string
  node_id: string
  step_name: string
  status: 'pending' | 'running' | 'completed' | 'failed' | 'skipped'
  input: Record<string, unknown>
  output?: Record<string, unknown>
  error?: string
  started_at: string
  completed_at?: string
  iteration?: number
}

interface FlowInstanceDetailProps {
  instance: FlowInstance
  onClose: () => void
}

export default function FlowInstanceDetail({ instance, onClose }: FlowInstanceDetailProps) {
  const [stepExecutions, setStepExecutions] = useState<FlowStepExecution[]>([])
  const [loading, setLoading] = useState(true)
  const [copiedField, setCopiedField] = useState<string | null>(null)

  // Fetch step executions (children of flow instance)
  useEffect(() => {
    const fetchStepExecutions = async () => {
      try {
        // TODO: Implement API call to get child nodes of type raisin:FlowStepExecution
        // const response = await nodesApi.getChildren({
        //   path: instance.path,
        //   node_type: 'raisin:FlowStepExecution'
        // })

        // Mock data for now
        setStepExecutions([])
      } catch (err) {
        console.error('Failed to fetch step executions:', err)
      } finally {
        setLoading(false)
      }
    }
    fetchStepExecutions()
  }, [instance.path])

  const copyToClipboard = (text: string, field: string) => {
    navigator.clipboard.writeText(text).then(() => {
      setCopiedField(field)
      setTimeout(() => setCopiedField(null), 2000)
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

  const CopyButton = ({ text, field }: { text: string; field: string }) => (
    <button
      onClick={() => copyToClipboard(text, field)}
      className="p-1.5 text-zinc-400 hover:text-white hover:bg-white/10 rounded transition-colors"
      title="Copy to clipboard"
    >
      {copiedField === field ? (
        <Check className="w-3.5 h-3.5 text-green-400" />
      ) : (
        <Copy className="w-3.5 h-3.5" />
      )}
    </button>
  )

  return (
    <div className="fixed inset-0 z-50 bg-black/50 backdrop-blur-sm flex items-start justify-center overflow-auto p-4"
         onClick={onClose}>
      <div
        className="bg-zinc-900 border border-white/10 rounded-2xl shadow-2xl max-w-6xl w-full my-8"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <div className="flex items-center gap-3">
            <button
              onClick={onClose}
              className="p-2 text-zinc-400 hover:text-white hover:bg-white/10 rounded-lg transition-colors"
              aria-label="Go back"
            >
              <ChevronLeft className="w-5 h-5" />
            </button>
            <Workflow className="w-6 h-6 text-purple-400" />
            <div>
              <h2 className="text-xl font-bold text-white">
                {instance.name || 'Flow Instance'}
              </h2>
              <p className="text-sm text-zinc-400">{instance.path}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 text-zinc-400 hover:text-white hover:bg-white/10 rounded-lg transition-colors"
            aria-label="Close"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Content */}
        <div className="p-6 space-y-6 max-h-[calc(100vh-200px)] overflow-y-auto">
          {/* Instance Properties */}
          <section>
            <h3 className="text-lg font-semibold text-white mb-3 flex items-center gap-2">
              <Package className="w-5 h-5 text-zinc-400" />
              Instance Properties
            </h3>
            <div className="bg-white/5 border border-white/10 rounded-lg p-4">
              <div className="grid grid-cols-2 gap-4">
                <InfoField label="Status" value={instance.status} />
                <InfoField label="Flow Version" value={`v${instance.flow_version}`} />
                <InfoField label="Started At" value={new Date(instance.started_at).toLocaleString()} />
                {instance.completed_at && (
                  <InfoField label="Completed At" value={new Date(instance.completed_at).toLocaleString()} />
                )}
                <InfoField
                  label="Duration"
                  value={formatDuration(instance.started_at, instance.completed_at)}
                />
                {instance.current_node_id && (
                  <InfoField label="Current Node" value={instance.current_node_id} />
                )}
                {instance.metrics && (
                  <>
                    <InfoField label="Steps Executed" value={instance.metrics.step_count.toString()} />
                    <InfoField label="Retry Count" value={instance.metrics.retry_count.toString()} />
                  </>
                )}
              </div>

              {/* Wait Info */}
              {instance.wait_info && (
                <div className="mt-4 pt-4 border-t border-white/10">
                  <div className="text-sm font-medium text-yellow-300 mb-2">
                    Waiting for: {instance.wait_info.wait_type}
                  </div>
                  {instance.wait_info.target_path && (
                    <div className="text-xs text-zinc-400">
                      Target: <code className="bg-black/30 px-1 rounded">{instance.wait_info.target_path}</code>
                    </div>
                  )}
                  {instance.wait_info.timeout_at && (
                    <div className="text-xs text-zinc-400">
                      Timeout: {new Date(instance.wait_info.timeout_at).toLocaleString()}
                    </div>
                  )}
                </div>
              )}
            </div>
          </section>

          {/* Flow Input */}
          <section>
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-lg font-semibold text-white flex items-center gap-2">
                <ArrowRight className="w-5 h-5 text-zinc-400" />
                Input
              </h3>
              <CopyButton text={JSON.stringify(instance.input, null, 2)} field="input" />
            </div>
            <pre className="bg-black/30 border border-white/10 rounded-lg p-4 text-xs text-zinc-300 font-mono overflow-x-auto max-h-48 overflow-y-auto">
              {JSON.stringify(instance.input, null, 2)}
            </pre>
          </section>

          {/* Variables (Context) */}
          <section>
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-lg font-semibold text-white flex items-center gap-2">
                <Code className="w-5 h-5 text-zinc-400" />
                Variables (Current Context)
              </h3>
              <CopyButton text={JSON.stringify(instance.variables, null, 2)} field="variables" />
            </div>
            <pre className="bg-black/30 border border-white/10 rounded-lg p-4 text-xs text-zinc-300 font-mono overflow-x-auto max-h-48 overflow-y-auto">
              {JSON.stringify(instance.variables, null, 2)}
            </pre>
          </section>

          {/* Step Execution Timeline */}
          <section>
            <h3 className="text-lg font-semibold text-white mb-3 flex items-center gap-2">
              <Clock className="w-5 h-5 text-zinc-400" />
              Execution Timeline
            </h3>
            {loading ? (
              <div className="bg-white/5 border border-white/10 rounded-lg p-8 text-center">
                <div className="animate-spin w-8 h-8 border-2 border-purple-500 border-t-transparent rounded-full mx-auto mb-2"></div>
                <p className="text-zinc-400">Loading steps...</p>
              </div>
            ) : (
              <FlowStepTimeline
                steps={stepExecutions}
                currentNodeId={instance.current_node_id}
              />
            )}
          </section>

          {/* Error Details */}
          {instance.error && (
            <section>
              <h3 className="text-lg font-semibold text-white mb-3 flex items-center gap-2">
                <AlertTriangle className="w-5 h-5 text-red-400" />
                Error Details
              </h3>
              <ErrorDetails error={instance.error} />
            </section>
          )}

          {/* Compensation Stack */}
          {instance.compensation_stack && instance.compensation_stack.length > 0 && (
            <section>
              <h3 className="text-lg font-semibold text-white mb-3 flex items-center gap-2">
                <Undo2 className="w-5 h-5 text-purple-400" />
                Compensation Stack ({instance.compensation_stack.length})
              </h3>
              <div className="bg-white/5 border border-white/10 rounded-lg overflow-hidden">
                <div className="divide-y divide-white/5">
                  {instance.compensation_stack.map((entry, idx) => (
                    <div key={idx} className="p-3 hover:bg-white/5">
                      <div className="flex items-center justify-between mb-2">
                        <span className="text-sm font-medium text-white">
                          Step: {entry.step_id}
                        </span>
                        <span className={`px-2 py-0.5 text-xs rounded ${
                          entry.compensation_status === 'executed'
                            ? 'bg-green-500/20 text-green-300'
                            : entry.compensation_status === 'failed'
                            ? 'bg-red-500/20 text-red-300'
                            : 'bg-zinc-500/20 text-zinc-300'
                        }`}>
                          {entry.compensation_status}
                        </span>
                      </div>
                      <div className="text-xs text-zinc-400">
                        Compensation: <code className="bg-black/30 px-1 rounded">{entry.compensation_fn}</code>
                      </div>
                      <div className="text-xs text-zinc-500 mt-1">
                        Completed: {new Date(entry.completed_at).toLocaleString()}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </section>
          )}

          {/* Output (if completed) */}
          {instance.output && (
            <section>
              <div className="flex items-center justify-between mb-3">
                <h3 className="text-lg font-semibold text-white flex items-center gap-2">
                  <ArrowRight className="w-5 h-5 text-green-400" />
                  Output
                </h3>
                <CopyButton text={JSON.stringify(instance.output, null, 2)} field="output" />
              </div>
              <pre className="bg-black/30 border border-white/10 rounded-lg p-4 text-xs text-zinc-300 font-mono overflow-x-auto max-h-48 overflow-y-auto">
                {JSON.stringify(instance.output, null, 2)}
              </pre>
            </section>
          )}
        </div>
      </div>
    </div>
  )
}

function InfoField({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs text-zinc-500 mb-1">{label}</div>
      <div className="text-sm text-white font-medium">{value}</div>
    </div>
  )
}
