/**
 * Agent Test Chat Component
 *
 * VSCode-style split panel for testing AI agents interactively.
 * Writes a `raisin:Conversation` + user `raisin:Message` into the operator's
 * inbox/outbox (workspace `raisin:access_control`) and streams the agent's
 * reply via SSE on `/api/conversations/{repo}/events`. This is the same
 * messaging path used by the production chat UI; the trigger that fires the
 * agent (`messaging-agent-chat`) only matches this shape.
 */

import { useState, useEffect, useRef, useCallback } from 'react'
import {
  Send,
  Loader2,
  Bot,
  User,
  Brain,
  Wrench,
  ChevronDown,
  ChevronRight,
  AlertCircle,
  Trash2,
  RotateCcw,
  Play,
  PauseCircle,
} from 'lucide-react'
import { agentChatApi, type ChatEvent, type TestConversation } from '../../../../api/agent-chat'
import { agentChatPlansApi, type PlanProjection } from '../../../../api/agent-chat-plans'
import { AgentTestChatPlanCard } from './AgentTestChatPlanCard'
import MarkdownRenderer from '../../../../components/MarkdownRenderer'

interface AgentTestChatProps {
  repo: string
  branch: string
  agentPath: string
  agentName: string
  agentId: string
}

interface Message {
  id: string
  role: 'user' | 'assistant' | 'system'
  content: string
  timestamp?: string
  children?: MessageChild[]
  finishReason?: string
}

interface MessageChild {
  id: string
  type: 'thought' | 'tool_call' | 'tool_result'
  content: string
  toolName?: string
  toolInput?: unknown
  expanded?: boolean
  status?: string
}

// Safety net: if the agent emits no SSE events for this long, assume the turn
// stalled (the backend silently dropped it) and surface a retry instead of an
// endless spinner. Slightly below the agent functions' 120s execution timeout.
const INACTIVITY_TIMEOUT_MS = 90_000

export function AgentTestChat({ repo, branch: _branch, agentPath, agentName, agentId }: AgentTestChatProps) {
  const [conversation, setConversation] = useState<TestConversation | null>(null)
  const [messages, setMessages] = useState<Message[]>([])
  const [inputText, setInputText] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const [isSending, setIsSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [isWaitingForResponse, setIsWaitingForResponse] = useState(false)
  // Plan/task projection for this conversation (mirrors ConversationStore.plans
  // in the JS SDK). Refreshed on SSE events and polled while a plan is active.
  const [plans, setPlans] = useState<PlanProjection[]>([])
  // Why the agent is paused: 'awaiting_plan_approval' (waiting for an
  // approve/reject) or 'step_by_step' (waiting for a continue signal).
  const [waitingReason, setWaitingReason] = useState<string | null>(null)
  const [planActionBusy, setPlanActionBusy] = useState<string | null>(null)

  const messagesEndRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const streamAbortRef = useRef<AbortController | null>(null)
  const assistantMessageIdRef = useRef<string | null>(null)
  const inactivityTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const conversationRef = useRef<TestConversation | null>(null)
  const plansRefreshInFlightRef = useRef(false)
  conversationRef.current = conversation

  const clearInactivityTimer = useCallback(() => {
    if (inactivityTimerRef.current) {
      clearTimeout(inactivityTimerRef.current)
      inactivityTimerRef.current = null
    }
  }, [])

  // (Re)start the inactivity timer. Called when a turn begins and reset on
  // every received event, so long-but-active tool runs aren't killed — only a
  // genuine silent stall (zero events) trips it.
  const armInactivityTimer = useCallback(() => {
    clearInactivityTimer()
    inactivityTimerRef.current = setTimeout(() => {
      inactivityTimerRef.current = null
      streamAbortRef.current?.abort()
      streamAbortRef.current = null
      assistantMessageIdRef.current = null
      setIsWaitingForResponse(false)
      // Don't leave tool calls visually spinning forever: their
      // tool_call_completed events will never arrive on this stream.
      setMessages(prev =>
        prev.map(m =>
          m.children?.some(c => c.type === 'tool_call' && c.status === 'running')
            ? {
                ...m,
                children: m.children.map(c =>
                  c.type === 'tool_call' && c.status === 'running'
                    ? { ...c, status: 'stalled', content: 'Status: stalled (no response from agent)' }
                    : c,
                ),
              }
            : m,
        ),
      )
      setError('The agent did not respond in time. Send your message again to retry.')
    }, INACTIVITY_TIMEOUT_MS)
  }, [clearInactivityTimer])

  // Re-project plan/task state from the persisted ai_plan / ai_task_update
  // message cards (same contract the SDK's ConversationStore.plans exposes).
  const refreshPlans = useCallback(async () => {
    const conv = conversationRef.current
    if (!conv || plansRefreshInFlightRef.current) return
    plansRefreshInFlightRef.current = true
    try {
      const next = await agentChatPlansApi.loadPlans(repo, conv.conversationPath)
      setPlans(next)
    } catch (err) {
      console.error('Failed to load plans:', err)
    } finally {
      plansRefreshInFlightRef.current = false
    }
  }, [repo])

  // Plan/task cards are delivered to the user conversation asynchronously
  // (outbox -> process-chat), often landing AFTER the SSE waiting/done event
  // of the turn that produced them. Schedule a few delayed refreshes so the
  // projection catches late deliveries even when polling has stopped.
  const schedulePlanRefreshes = useCallback(() => {
    for (const delay of [1_500, 4_000, 8_000]) {
      setTimeout(() => void refreshPlans(), delay)
    }
  }, [refreshPlans])

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages, plans, waitingReason])

  // Clear any pending inactivity timer when the component unmounts.
  useEffect(() => () => clearInactivityTimer(), [clearInactivityTimer])

  const createConversation = useCallback(async () => {
    setIsLoading(true)
    setError(null)
    try {
      const conv = await agentChatApi.createTestConversation({
        repo,
        agentPath,
        agentId,
        agentName,
      })
      setConversation(conv)
      setMessages([])
      setPlans([])
      setWaitingReason(null)
    } catch (err) {
      console.error('Failed to create conversation:', err)
      setError(
        err instanceof Error
          ? `Failed to create conversation: ${err.message}`
          : 'Failed to create conversation. Verify that raisin-messaging is installed and you have a user identity.',
      )
    } finally {
      setIsLoading(false)
    }
  }, [repo, agentPath, agentId, agentName])

  useEffect(() => {
    createConversation()
    return () => {
      streamAbortRef.current?.abort()
      streamAbortRef.current = null
    }
  }, [createConversation])

  const ensureAssistantBubble = useCallback((): string => {
    if (assistantMessageIdRef.current) return assistantMessageIdRef.current
    const id = `assistant-${Date.now()}`
    assistantMessageIdRef.current = id
    setMessages(prev => [
      ...prev,
      {
        id,
        role: 'assistant',
        content: '',
        timestamp: new Date().toISOString(),
        children: [],
      },
    ])
    return id
  }, [])

  const applyToAssistant = useCallback((mutator: (msg: Message) => Message) => {
    const id = ensureAssistantBubble()
    setMessages(prev => prev.map(m => (m.id === id ? mutator(m) : m)))
  }, [ensureAssistantBubble])

  const handleEvent = useCallback((event: ChatEvent) => {
    // Any event means the turn is still alive — push the stall deadline out.
    armInactivityTimer()
    switch (event.type) {
      case 'text_chunk':
        applyToAssistant(m => ({ ...m, content: m.content + event.text }))
        break
      case 'thought_chunk':
        applyToAssistant(m => {
          const children = m.children ? [...m.children] : []
          const lastIdx = children.length - 1
          if (lastIdx >= 0 && children[lastIdx].type === 'thought') {
            children[lastIdx] = {
              ...children[lastIdx],
              content: children[lastIdx].content + event.text,
            }
          } else {
            children.push({
              id: `thought-${Date.now()}-${children.length}`,
              type: 'thought',
              content: event.text,
            })
          }
          return { ...m, children }
        })
        break
      case 'tool_call_started':
        applyToAssistant(m => ({
          ...m,
          children: [
            ...(m.children ?? []),
            {
              id: event.toolCallId,
              type: 'tool_call',
              content: 'Status: running',
              toolName: event.functionName,
              toolInput: event.arguments,
              status: 'running',
            },
          ],
        }))
        break
      case 'tool_call_completed':
        applyToAssistant(m => {
          const children = (m.children ?? []).map(c =>
            c.id === event.toolCallId
              ? {
                  ...c,
                  status: event.error ? 'failed' : 'completed',
                  content: event.error ? `Error: ${event.error}` : 'Status: completed',
                }
              : c,
          )
          children.push({
            id: `result-${event.toolCallId}`,
            type: 'tool_result',
            content: JSON.stringify(event.error ?? event.result ?? '', null, 2),
            toolName: event.functionName,
          })
          return { ...m, children }
        })
        break
      case 'message_saved':
        // A new assistant message landed in storage. If it's an assistant
        // turn and we don't have a bubble yet, open one for subsequent
        // chunks to attach to. Plan/task cards land as messages too, so
        // refresh the plan projection.
        if (event.role === 'assistant') ensureAssistantBubble()
        void refreshPlans()
        break
      case 'done':
        applyToAssistant(m => ({
          ...m,
          content: m.content || event.content || '',
          finishReason: event.finishReason ?? 'stop',
        }))
        assistantMessageIdRef.current = null
        setIsWaitingForResponse(false)
        // Pause-style terminal events: awaiting_step_continue (step_by_step
        // pause) and awaiting_plan_approval (safety-net done after the
        // waiting event) keep their waiting affordances visible.
        setWaitingReason(
          event.finishReason === 'awaiting_step_continue'
            ? 'step_by_step'
            : event.finishReason === 'awaiting_plan_approval'
              ? 'awaiting_plan_approval'
              : null,
        )
        clearInactivityTimer()
        void refreshPlans()
        schedulePlanRefreshes()
        break
      case 'waiting':
        // Turn paused on purpose: 'awaiting_plan_approval' (render the plan
        // card with Approve/Reject) or 'step_by_step' (render Continue).
        // Stop the spinner and the stall timer — the agent is waiting on us.
        assistantMessageIdRef.current = null
        setIsWaitingForResponse(false)
        setWaitingReason(event.reason ?? 'awaiting_plan_approval')
        clearInactivityTimer()
        void refreshPlans()
        schedulePlanRefreshes()
        break
      case 'log':
        // Surface backend logs to the dev console; don't render in the chat.
        console.debug(`[agent-handler ${event.level}]`, event.message, event.module ?? '')
        break
    }
  }, [applyToAssistant, ensureAssistantBubble, armInactivityTimer, clearInactivityTimer, refreshPlans, schedulePlanRefreshes])

  const consumeStream = useCallback(async (streamChannel: string) => {
    streamAbortRef.current?.abort()
    const abort = new AbortController()
    streamAbortRef.current = abort
    try {
      for await (const event of agentChatApi.streamEvents({
        repo,
        streamChannel,
        signal: abort.signal,
      })) {
        handleEvent(event)
      }
    } catch (err) {
      if (abort.signal.aborted) return
      console.error('SSE stream failed:', err)
      setError('Lost connection to agent stream. Send again to retry.')
      setIsWaitingForResponse(false)
      clearInactivityTimer()
    }
  }, [repo, handleEvent, clearInactivityTimer])

  // Poll the plan projection while something is in flight: a paused turn, a
  // running turn, or a plan that is still pending/in progress. Task status
  // updates are persisted as messages without dedicated SSE events, so this
  // is what makes the task list progress live during execution.
  const hasActivePlan = plans.some(
    p => p.status === 'pending_approval' || p.status === 'in_progress',
  )
  useEffect(() => {
    if (!conversation) return
    if (!waitingReason && !isWaitingForResponse && !hasActivePlan) return
    const timer = setInterval(() => {
      void refreshPlans()
    }, 3_000)
    return () => clearInterval(timer)
  }, [conversation, waitingReason, isWaitingForResponse, hasActivePlan, refreshPlans])

  const sendMessage = async (messageContent: string) => {
    if (!messageContent || !conversation || isSending) return

    setIsSending(true)
    setError(null)
    setWaitingReason(null)

    try {
      // Open the SSE stream BEFORE writing the user message so we don't miss
      // early text_chunk events.
      setIsWaitingForResponse(true)
      assistantMessageIdRef.current = null
      armInactivityTimer()
      const streamPromise = consumeStream(conversation.streamChannel)

      await agentChatApi.sendUserMessage({
        repo,
        conversation,
        content: messageContent,
      })

      setMessages(prev => [
        ...prev,
        {
          id: `user-${Date.now()}`,
          role: 'user',
          content: messageContent,
          timestamp: new Date().toISOString(),
        },
      ])

      void streamPromise
    } catch (err) {
      console.error('Failed to send message:', err)
      setError(err instanceof Error ? `Failed to send message: ${err.message}` : 'Failed to send message')
      setIsWaitingForResponse(false)
      clearInactivityTimer()
      streamAbortRef.current?.abort()
    } finally {
      setIsSending(false)
    }
  }

  const handleSend = () => {
    const messageContent = inputText.trim()
    if (!messageContent) return
    setInputText('')
    void sendMessage(messageContent)
  }

  // Resume a step_by_step turn: a plain user message is the continue signal.
  const handleContinue = () => {
    void sendMessage('continue')
  }

  // Approve/reject mirror the SDK's ConversationStore.approvePlan/rejectPlan:
  // SQL INVOKE of /lib/raisin/ai/plan-approval-handler. Approval may start a
  // continuation turn (approve_then_auto / step_by_step), so keep the SSE
  // stream open to capture it; the plan poller shows task progress.
  const handlePlanAction = async (
    action: 'approve' | 'reject',
    planPath: string,
    feedback?: string,
  ) => {
    if (!conversation || planActionBusy) return
    setPlanActionBusy(planPath)
    setError(null)
    try {
      if (action === 'approve') {
        await agentChatPlansApi.approvePlan(repo, planPath)
      } else {
        await agentChatPlansApi.rejectPlan(repo, planPath, feedback)
      }
      setWaitingReason(null)
      assistantMessageIdRef.current = null
      // (Re)open the stream so the continuation turn's events render. In
      // manual mode no continuation follows an approval — that's fine, the
      // stream just stays idle and the plan card reflects the new status.
      void consumeStream(conversation.streamChannel)
      await refreshPlans()
    } catch (err) {
      console.error(`Failed to ${action} plan:`, err)
      setError(err instanceof Error ? `Failed to ${action} plan: ${err.message}` : `Failed to ${action} plan`)
    } finally {
      setPlanActionBusy(null)
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  const handleClearChat = async () => {
    streamAbortRef.current?.abort()
    streamAbortRef.current = null
    clearInactivityTimer()
    setIsWaitingForResponse(false)
    if (conversation) {
      try {
        await agentChatApi.deleteConversation(repo, conversation.conversationPath)
      } catch (err) {
        console.error('Failed to delete conversation:', err)
      }
    }
    await createConversation()
  }

  const toggleChildExpanded = (messageId: string, childId: string) => {
    setMessages(prev => prev.map(msg => {
      if (msg.id === messageId && msg.children) {
        return {
          ...msg,
          children: msg.children.map(child =>
            child.id === childId ? { ...child, expanded: !child.expanded } : child,
          ),
        }
      }
      return msg
    }))
  }

  if (isLoading) {
    return (
      <div className="h-full flex flex-col items-center justify-center text-zinc-400">
        <Loader2 className="w-6 h-6 animate-spin mb-2" />
        <p className="text-sm">Creating test conversation...</p>
      </div>
    )
  }

  if (error && !conversation) {
    return (
      <div className="h-full flex flex-col items-center justify-center text-red-400 p-4">
        <AlertCircle className="w-8 h-8 mb-2" />
        <p className="text-sm text-center">{error}</p>
        <button
          onClick={createConversation}
          className="mt-4 px-3 py-1.5 bg-purple-500/20 text-purple-300 rounded text-sm hover:bg-purple-500/30 flex items-center gap-2"
        >
          <RotateCcw className="w-4 h-4" />
          Retry
        </button>
      </div>
    )
  }

  return (
    <div className="h-full flex flex-col bg-zinc-900/50">
      {/* Header */}
      <div className="flex-shrink-0 flex items-center justify-between px-3 py-2 border-b border-white/10 bg-black/20">
        <div className="flex items-center gap-2">
          <Bot className="w-4 h-4 text-purple-400" />
          <span className="text-sm font-medium text-zinc-300">Test Chat</span>
        </div>
        <button
          onClick={handleClearChat}
          className="p-1.5 hover:bg-white/10 rounded text-zinc-400 hover:text-zinc-200"
          title="Clear chat and start over"
        >
          <Trash2 className="w-4 h-4" />
        </button>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto p-3 space-y-4">
        {messages.length === 0 ? (
          <div className="h-full flex flex-col items-center justify-center text-zinc-500">
            <Bot className="w-12 h-12 mb-3 opacity-30" />
            <p className="text-sm">Send a message to test the agent</p>
          </div>
        ) : (
          messages.map((message) => (
            <div key={message.id} className={`flex gap-3 ${message.role === 'user' ? 'flex-row-reverse' : ''}`}>
              <div className={`flex-shrink-0 w-7 h-7 rounded-full flex items-center justify-center ${
                message.role === 'user' ? 'bg-blue-500/20' : 'bg-purple-500/20'
              }`}>
                {message.role === 'user' ? (
                  <User className="w-4 h-4 text-blue-400" />
                ) : (
                  <Bot className="w-4 h-4 text-purple-400" />
                )}
              </div>

              <div className={`flex-1 min-w-0 ${message.role === 'user' ? 'text-right' : ''}`}>
                <div className={`inline-block max-w-full px-3 py-2 rounded-lg text-sm ${
                  message.role === 'user'
                    ? 'bg-blue-500/20 text-blue-100'
                    : 'bg-white/5 text-zinc-200'
                }`}>
                  {message.role === 'assistant' && message.content ? (
                    // Render the agent's reply as markdown (code, lists, tables, etc.).
                    <div className="text-left text-sm [&_p]:mb-2 [&_p:first-child]:mt-0 [&_:last-child]:!mb-0">
                      <MarkdownRenderer content={message.content} />
                    </div>
                  ) : (
                    <p className="whitespace-pre-wrap break-words">
                      {message.content || (message.children?.some(c => c.type === 'tool_call')
                        ? <span className="text-zinc-400 italic">Using tools...</span>
                        : '')}
                    </p>
                  )}
                </div>

                {message.children && message.children.length > 0 && (
                  <div className="mt-2 space-y-1">
                    {message.children.map((child) => (
                      <div key={child.id} className="text-left">
                        <button
                          onClick={() => toggleChildExpanded(message.id, child.id)}
                          className="flex items-center gap-1.5 text-xs text-zinc-400 hover:text-zinc-300"
                        >
                          {child.expanded ? (
                            <ChevronDown className="w-3 h-3" />
                          ) : (
                            <ChevronRight className="w-3 h-3" />
                          )}
                          {child.type === 'thought' && (
                            <>
                              <Brain className="w-3 h-3 text-purple-400" />
                              <span>Thought</span>
                            </>
                          )}
                          {child.type === 'tool_call' && (
                            <>
                              <Wrench className={`w-3 h-3 ${
                                child.status === 'completed' ? 'text-green-400' :
                                child.status === 'failed' ? 'text-red-400' :
                                child.status === 'running' ? 'text-blue-400 animate-pulse' :
                                'text-yellow-400'
                              }`} />
                              <span>Tool: {child.toolName}</span>
                              {child.status && child.status !== 'completed' && (
                                <span className={`text-xs px-1.5 py-0.5 rounded ${
                                  child.status === 'running' ? 'bg-blue-500/20 text-blue-300' :
                                  child.status === 'failed' ? 'bg-red-500/20 text-red-300' :
                                  'bg-yellow-500/20 text-yellow-300'
                                }`}>
                                  {child.status}
                                </span>
                              )}
                            </>
                          )}
                          {child.type === 'tool_result' && (
                            <>
                              <Wrench className="w-3 h-3 text-green-400" />
                              <span>Result: {child.toolName}</span>
                            </>
                          )}
                        </button>
                        {child.expanded && (
                          <div className="mt-1 ml-4 p-2 bg-black/30 rounded text-xs text-zinc-400 font-mono overflow-x-auto">
                            <pre className="whitespace-pre-wrap break-words">
                              {child.type === 'tool_call' && child.toolInput
                                ? JSON.stringify(child.toolInput, null, 2)
                                : child.content}
                            </pre>
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          ))
        )}

        {/* Plan proposal / progress cards (projected from ai_plan +
            ai_task_update message cards, like ConversationStore.plans) */}
        {plans.length > 0 && (
          <div className="space-y-2 ml-10">
            {plans.map((plan) => (
              <AgentTestChatPlanCard
                key={plan.key}
                plan={plan}
                busy={planActionBusy === plan.planPath}
                onApprove={(planPath) => void handlePlanAction('approve', planPath)}
                onReject={(planPath, feedback) => void handlePlanAction('reject', planPath, feedback)}
              />
            ))}
          </div>
        )}

        {/* Paused awaiting approval but the plan card hasn't been delivered
            yet (outbox delivery is async) — show a hint while polling. */}
        {waitingReason === 'awaiting_plan_approval' &&
          !plans.some(p => p.status === 'pending_approval') && (
          <div className="flex items-center gap-2 ml-10 text-yellow-300/80 text-xs">
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
            <span>Agent proposed a plan — loading it…</span>
          </div>
        )}

        {/* step_by_step pause: agent completed one task and waits for the
            continue signal (a plain user message). */}
        {waitingReason === 'step_by_step' && !isWaitingForResponse && (
          <div className="flex items-center gap-3 ml-10" data-testid="step-continue">
            <div className="flex items-center gap-1.5 text-zinc-400 text-xs">
              <PauseCircle className="w-3.5 h-3.5 text-blue-400" />
              <span>Paused after this step</span>
            </div>
            <button
              onClick={handleContinue}
              disabled={isSending}
              className="px-2.5 py-1 bg-blue-500/20 text-blue-300 rounded text-xs hover:bg-blue-500/30 disabled:opacity-50 flex items-center gap-1.5"
              data-testid="step-continue-button"
            >
              <Play className="w-3 h-3" />
              Continue next step
            </button>
          </div>
        )}

        {isWaitingForResponse && !assistantMessageIdRef.current && (
          <div className="flex gap-3">
            <div className="flex-shrink-0 w-7 h-7 rounded-full flex items-center justify-center bg-purple-500/20">
              <Bot className="w-4 h-4 text-purple-400" />
            </div>
            <div className="flex items-center gap-2 text-zinc-400 text-sm">
              <Loader2 className="w-4 h-4 animate-spin" />
              <span>Thinking...</span>
            </div>
          </div>
        )}

        <div ref={messagesEndRef} />
      </div>

      {error && (
        <div className="flex-shrink-0 px-3 py-2 bg-red-500/10 border-t border-red-500/20">
          <p className="text-xs text-red-400">{error}</p>
        </div>
      )}

      <div className="flex-shrink-0 p-3 border-t border-white/10 bg-black/20">
        <div className="flex gap-2">
          <textarea
            ref={inputRef}
            value={inputText}
            onChange={(e) => setInputText(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Type a message..."
            rows={1}
            className="flex-1 px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm placeholder-zinc-500 focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20 resize-none"
            disabled={isSending || !conversation}
          />
          <button
            onClick={handleSend}
            disabled={!inputText.trim() || isSending || !conversation}
            className="px-3 py-2 bg-purple-500 hover:bg-purple-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
          >
            {isSending ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Send className="w-4 h-4" />
            )}
          </button>
        </div>
        <p className="text-xs text-zinc-500 mt-1">Press Enter to send, Shift+Enter for new line</p>
      </div>
    </div>
  )
}
