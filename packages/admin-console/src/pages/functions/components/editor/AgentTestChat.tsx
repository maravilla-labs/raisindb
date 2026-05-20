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
} from 'lucide-react'
import { agentChatApi, type ChatEvent, type TestConversation } from '../../../../api/agent-chat'

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

export function AgentTestChat({ repo, branch: _branch, agentPath, agentName, agentId }: AgentTestChatProps) {
  const [conversation, setConversation] = useState<TestConversation | null>(null)
  const [messages, setMessages] = useState<Message[]>([])
  const [inputText, setInputText] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const [isSending, setIsSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [isWaitingForResponse, setIsWaitingForResponse] = useState(false)

  const messagesEndRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const streamAbortRef = useRef<AbortController | null>(null)
  const assistantMessageIdRef = useRef<string | null>(null)

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages])

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
        // chunks to attach to.
        if (event.role === 'assistant') ensureAssistantBubble()
        break
      case 'done':
        applyToAssistant(m => ({
          ...m,
          content: m.content || event.content || '',
          finishReason: event.finishReason ?? 'stop',
        }))
        assistantMessageIdRef.current = null
        setIsWaitingForResponse(false)
        break
      case 'waiting':
        // Turn paused (e.g. plan approval). Keep the spinner up.
        break
      case 'log':
        // Surface backend logs to the dev console; don't render in the chat.
        console.debug(`[agent-handler ${event.level}]`, event.message, event.module ?? '')
        break
    }
  }, [applyToAssistant, ensureAssistantBubble])

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
    }
  }, [repo, handleEvent])

  const handleSend = async () => {
    if (!inputText.trim() || !conversation || isSending) return

    const messageContent = inputText.trim()
    setInputText('')
    setIsSending(true)
    setError(null)

    try {
      // Open the SSE stream BEFORE writing the user message so we don't miss
      // early text_chunk events.
      setIsWaitingForResponse(true)
      assistantMessageIdRef.current = null
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
      streamAbortRef.current?.abort()
    } finally {
      setIsSending(false)
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
                  <p className="whitespace-pre-wrap break-words">
                    {message.content || (message.children?.some(c => c.type === 'tool_call')
                      ? <span className="text-zinc-400 italic">Using tools...</span>
                      : '')}
                  </p>
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
