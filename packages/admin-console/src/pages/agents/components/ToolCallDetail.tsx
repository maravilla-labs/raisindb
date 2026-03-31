import { useState } from 'react'
import { ChevronDown, ChevronRight, CheckCircle, XCircle, Clock, Code } from 'lucide-react'

import type { ToolCallTrace } from '../../../api/agent-conversations'

interface ToolCallDetailProps {
  toolCall: ToolCallTrace
}

function StatusBadge({ status }: { status: string }) {
  switch (status) {
    case 'completed':
      return (
        <span className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 text-green-300 text-xs rounded-full">
          <CheckCircle className="w-3 h-3" /> Completed
        </span>
      )
    case 'error':
      return (
        <span className="flex items-center gap-1 px-2 py-0.5 bg-red-500/20 text-red-300 text-xs rounded-full">
          <XCircle className="w-3 h-3" /> Error
        </span>
      )
    case 'pending':
      return (
        <span className="flex items-center gap-1 px-2 py-0.5 bg-yellow-500/20 text-yellow-300 text-xs rounded-full">
          <Clock className="w-3 h-3" /> Pending
        </span>
      )
    default:
      return (
        <span className="px-2 py-0.5 bg-zinc-500/20 text-zinc-300 text-xs rounded-full">
          {status}
        </span>
      )
  }
}

export default function ToolCallDetail({ toolCall }: ToolCallDetailProps) {
  const [showArgs, setShowArgs] = useState(false)
  const [showResult, setShowResult] = useState(false)

  const hasError = toolCall.error || (toolCall.result && typeof toolCall.result === 'object' && toolCall.result !== null && 'error' in toolCall.result)
  const errorText = toolCall.error || (hasError && toolCall.result && typeof toolCall.result === 'object' && toolCall.result !== null ? (toolCall.result as Record<string, unknown>).error as string : undefined)

  let parsedArgs: unknown = null
  if (toolCall.arguments) {
    if (typeof toolCall.arguments === 'string') {
      try {
        parsedArgs = JSON.parse(toolCall.arguments)
      } catch {
        parsedArgs = toolCall.arguments
      }
    } else {
      parsedArgs = toolCall.arguments
    }
  }

  return (
    <div className={`bg-white/5 border rounded-lg overflow-hidden ${hasError ? 'border-red-500/30' : 'border-white/10'}`}>
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2">
        <Code className="w-4 h-4 text-primary-400" />
        <code className="text-sm text-primary-300 font-mono">{toolCall.functionName}</code>
        <StatusBadge status={toolCall.status} />
        {toolCall.durationMs !== undefined && (
          <span className="text-xs text-zinc-500">{toolCall.durationMs}ms</span>
        )}
      </div>

      {/* Error highlight */}
      {errorText && (
        <div className="mx-3 mb-2 p-2 bg-red-500/10 border border-red-500/20 rounded text-sm text-red-300">
          {typeof errorText === 'string' ? errorText : JSON.stringify(errorText)}
        </div>
      )}

      {/* Collapsible sections */}
      <div className="border-t border-white/5">
        {/* Arguments */}
        {parsedArgs !== null && (
          <div>
            <button
              onClick={() => setShowArgs(!showArgs)}
              className="flex items-center gap-2 w-full px-3 py-2 text-xs text-zinc-400 hover:text-zinc-300 hover:bg-white/5"
            >
              {showArgs ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
              Arguments
            </button>
            {showArgs && (
              <pre className="px-3 pb-2 text-xs text-zinc-300 font-mono overflow-x-auto max-h-48 overflow-y-auto">
                {typeof parsedArgs === 'string' ? parsedArgs : JSON.stringify(parsedArgs, null, 2)}
              </pre>
            )}
          </div>
        )}

        {/* Result */}
        {toolCall.result !== undefined && (
          <div>
            <button
              onClick={() => setShowResult(!showResult)}
              className="flex items-center gap-2 w-full px-3 py-2 text-xs text-zinc-400 hover:text-zinc-300 hover:bg-white/5"
            >
              {showResult ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
              Result
            </button>
            {showResult && (
              <pre className="px-3 pb-2 text-xs text-zinc-300 font-mono overflow-x-auto max-h-48 overflow-y-auto">
                {typeof toolCall.result === 'string' ? toolCall.result : JSON.stringify(toolCall.result, null, 2)}
              </pre>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
