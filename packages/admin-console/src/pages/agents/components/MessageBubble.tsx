import { useState } from 'react'
import { ChevronDown, ChevronRight, AlertTriangle, Brain, Coins } from 'lucide-react'

import type { ConversationMessage } from '../../../api/agent-conversations'
import RawJsonViewer from './RawJsonViewer'
import ToolCallDetail from './ToolCallDetail'
import PlanDetail from './PlanDetail'

interface MessageBubbleProps {
  message: ConversationMessage
}

const TERMINAL_FALLBACK_TEXT = 'I could not generate a complete response'

function FinishReasonBadge({ reason }: { reason?: string }) {
  if (!reason) return null
  const colors: Record<string, string> = {
    stop: 'bg-green-500/20 text-green-300',
    tool_calls: 'bg-blue-500/20 text-blue-300',
    error: 'bg-red-500/20 text-red-300',
    awaiting_plan_approval: 'bg-yellow-500/20 text-yellow-300',
    max_continuation_depth: 'bg-orange-500/20 text-orange-300',
  }
  return (
    <span className={`px-2 py-0.5 text-xs rounded-full ${colors[reason] || 'bg-zinc-500/20 text-zinc-300'}`}>
      {reason}
    </span>
  )
}

export default function MessageBubble({ message }: MessageBubbleProps) {
  const hasDiagnostics = !!message.executionDiagnostics
  const hasChildren = message.toolCalls.length > 0 || message.thoughts.length > 0 || message.plans.length > 0 || message.costRecords.length > 0 || hasDiagnostics
  const isWarning = message.content?.includes(TERMINAL_FALLBACK_TEXT)
  const isError = message.finishReason === 'error'
  const hasFailedTools = message.toolCalls.some(tc => tc.error)

  // Auto-expand children when there are errors or warnings so failures are immediately visible
  const [showChildren, setShowChildren] = useState(isError || isWarning || hasFailedTools)

  // System messages
  if (message.role === 'system') {
    return (
      <div className="flex justify-center my-2">
        <div className="px-4 py-2 bg-zinc-800/50 border border-white/5 rounded-full text-xs text-zinc-500 max-w-lg text-center">
          {message.content || 'System message'}
        </div>
      </div>
    )
  }

  // User messages
  if (message.role === 'user') {
    return (
      <div className="flex justify-end my-3">
        <div className="max-w-2xl">
          <div className="bg-primary-500/20 border border-primary-500/30 rounded-2xl rounded-br-sm px-4 py-3">
            {message.senderDisplayName && (
              <div className="text-xs text-primary-300 mb-1">{message.senderDisplayName}</div>
            )}
            <div className="text-sm text-white whitespace-pre-wrap">{message.content || '(empty message)'}</div>
          </div>
          {message.createdAt && (
            <div className="text-xs text-zinc-600 mt-1 text-right">
              {new Date(message.createdAt).toLocaleTimeString()}
            </div>
          )}
        </div>
      </div>
    )
  }

  // Assistant messages
  return (
    <div className="flex justify-start my-3">
      <div className="max-w-3xl w-full">
        <div className={`bg-zinc-800 border rounded-2xl rounded-bl-sm px-4 py-3 ${
          isError ? 'border-red-500/40' : isWarning ? 'border-yellow-500/40' : 'border-white/10'
        }`}>
          {/* Warning banner */}
          {isWarning && (
            <div className="mb-2 p-2 bg-yellow-500/10 border border-yellow-500/20 rounded text-xs text-yellow-300">
              <div className="flex items-center gap-2 font-medium">
                <AlertTriangle className="w-4 h-4" />
                Terminal fallback response
              </div>
              {message.errorDetails && (
                <div className="mt-1 text-yellow-200/80">
                  {message.errorDetails.type && <span className="font-medium">{message.errorDetails.type}: </span>}
                  {message.errorDetails.message || 'No error message'}
                  {message.errorDetails.finish_reason && <span className="ml-2 text-zinc-400">(finish_reason: {message.errorDetails.finish_reason})</span>}
                </div>
              )}
              {!message.errorDetails && message.finishReason && message.finishReason !== 'stop' && (
                <div className="mt-1 text-yellow-200/80">finish_reason: {message.finishReason}</div>
              )}
              {hasFailedTools && (
                <div className="mt-1 text-yellow-200/80">
                  {message.toolCalls.filter(tc => tc.error).length} tool call{message.toolCalls.filter(tc => tc.error).length > 1 ? 's' : ''} failed
                </div>
              )}
              {message.continuationDepth !== undefined && message.continuationDepth > 0 && (
                <div className="mt-1 text-zinc-400">continuation depth: {message.continuationDepth}</div>
              )}
            </div>
          )}

          {/* Content */}
          {message.content && (
            <div className="text-sm text-zinc-200 whitespace-pre-wrap">{message.content}</div>
          )}

          {/* Error details */}
          {message.errorDetails && (
            <div className="mt-2 p-2 bg-red-500/10 border border-red-500/20 rounded text-xs text-red-300">
              <span className="font-medium">Error: </span>
              {message.errorDetails.type || 'unknown'}{message.errorDetails.message ? ` — ${message.errorDetails.message}` : ''}
              {message.errorDetails.was_retry && <span className="ml-2 text-yellow-300">(retry attempted)</span>}
            </div>
          )}

          {/* Metadata badges */}
          <div className="flex flex-wrap items-center gap-2 mt-2">
            <FinishReasonBadge reason={message.finishReason} />
            {message.model && (
              <span className="px-2 py-0.5 bg-zinc-700 text-zinc-400 text-xs rounded-full">{message.model}</span>
            )}
            {message.tokens && (message.tokens.input || message.tokens.output) && (
              <span className="px-2 py-0.5 bg-zinc-700 text-zinc-400 text-xs rounded-full">
                {message.tokens.input ?? 0}↓ {message.tokens.output ?? 0}↑ tokens
              </span>
            )}
            {message.continuationDepth !== undefined && message.continuationDepth > 0 && (
              <span className="px-2 py-0.5 bg-zinc-700 text-zinc-400 text-xs rounded-full">
                depth: {message.continuationDepth}
              </span>
            )}
            {message.planActionId && (
              <span className="px-2 py-0.5 bg-indigo-500/20 text-indigo-300 text-xs rounded-full" title={message.planActionId}>
                plan action
              </span>
            )}
          </div>

          {/* Expandable children */}
          {hasChildren && (
            <div className="mt-3 border-t border-white/5 pt-2">
              <button
                onClick={() => setShowChildren(!showChildren)}
                className="flex items-center gap-1 text-xs text-zinc-400 hover:text-zinc-300"
              >
                {showChildren ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
                {message.toolCalls.length > 0 && `${message.toolCalls.length} tool call${message.toolCalls.length > 1 ? 's' : ''}`}
                {message.thoughts.length > 0 && `${message.toolCalls.length > 0 ? ', ' : ''}${message.thoughts.length} thought${message.thoughts.length > 1 ? 's' : ''}`}
                {message.plans.length > 0 && `${(message.toolCalls.length > 0 || message.thoughts.length > 0) ? ', ' : ''}${message.plans.length} plan${message.plans.length > 1 ? 's' : ''}`}
              </button>

              {showChildren && (
                <div className="mt-2 space-y-3">
                  {/* Thoughts */}
                  {message.thoughts.map((thought, i) => (
                    <div key={thought.path || i} className="flex items-start gap-2 p-2 bg-white/5 rounded-lg">
                      <Brain className="w-4 h-4 text-purple-400 mt-0.5 flex-shrink-0" />
                      <p className="text-sm text-zinc-400 italic">{thought.content}</p>
                    </div>
                  ))}

                  {/* Tool calls */}
                  {message.toolCalls.map(tc => (
                    <ToolCallDetail key={tc.path} toolCall={tc} />
                  ))}

                  {/* Plans */}
                  {message.plans.map(plan => (
                    <PlanDetail key={plan.path} plan={plan} />
                  ))}

                  {/* Cost records */}
                  {message.costRecords.length > 0 && (
                    <div className="flex items-center gap-2 p-2 bg-white/5 rounded-lg text-xs text-zinc-500">
                      <Coins className="w-4 h-4" />
                      {message.costRecords.map((cr, i) => (
                        <span key={cr.path || i}>
                          {cr.model && <span className="text-zinc-400">{cr.model}: </span>}
                          {cr.inputTokens ?? 0}↓ {cr.outputTokens ?? 0}↑
                        </span>
                      ))}
                    </div>
                  )}

                  {/* Execution diagnostics */}
                  {message.executionDiagnostics && (
                    <RawJsonViewer
                      data={message.executionDiagnostics}
                      title="Execution Diagnostics"
                      defaultCollapsed={!isError && !isWarning}
                    />
                  )}
                </div>
              )}
            </div>
          )}
        </div>

        {message.createdAt && (
          <div className="text-xs text-zinc-600 mt-1">
            {new Date(message.createdAt).toLocaleTimeString()}
          </div>
        )}
      </div>
    </div>
  )
}
