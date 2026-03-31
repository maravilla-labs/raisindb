/**
 * Agent Test Chat Component
 *
 * VSCode-style split panel for testing AI agents interactively.
 * Creates a temporary conversation in the AI workspace and allows
 * sending messages to test the agent's behavior.
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
import { nodesApi } from '../../../../api/nodes'

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
  finishReason?: string  // 'stop', 'tool_calls', etc.
}

interface MessageChild {
  id: string
  type: 'thought' | 'tool_call' | 'tool_result'
  content: string
  toolName?: string
  toolInput?: unknown
  expanded?: boolean
  status?: string  // For tool calls: pending, running, completed, failed
}

const AI_WORKSPACE = 'ai'
const POLL_INTERVAL = 1500 // Poll every 1.5 seconds for responses

export function AgentTestChat({ repo, branch, agentPath, agentName, agentId }: AgentTestChatProps) {
  const [conversationPath, setConversationPath] = useState<string | null>(null)
  const [messages, setMessages] = useState<Message[]>([])
  const [inputText, setInputText] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const [isSending, setIsSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [isWaitingForResponse, setIsWaitingForResponse] = useState(false)

  const messagesEndRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const pollIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // Auto-scroll to bottom when messages change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages])

  // Create conversation on mount
  useEffect(() => {
    createConversation()
    return () => {
      // Clean up polling on unmount
      if (pollIntervalRef.current) {
        clearInterval(pollIntervalRef.current)
      }
    }
  }, [agentPath])

  // Create a new conversation for this test session
  const createConversation = useCallback(async () => {
    setIsLoading(true)
    setError(null)
    try {
      // Generate a unique conversation name
      const timestamp = new Date().toISOString().replace(/[:.]/g, '-')
      const conversationName = `test-${agentName}-${timestamp}`

      // Create conversation in AI workspace under /conversations
      const response = await nodesApi.create(repo, branch, AI_WORKSPACE, '/conversations', {
        name: conversationName,
        node_type: 'raisin:AIConversation',
        properties: {
          agent_ref: {
            'raisin:ref': agentId,
            'raisin:workspace': 'functions',
            'raisin:path': agentPath,
          },
          status: 'active',
          title: `Test: ${agentName}`,
        },
        commit: {
          message: `Create test conversation for agent ${agentName}`,
          actor: 'admin',
        },
      }) as unknown as { node: { path: string } }

      setConversationPath(response.node.path)
      setMessages([])
    } catch (err) {
      console.error('Failed to create conversation:', err)
      setError('Failed to create conversation. Make sure the AI workspace exists.')
    } finally {
      setIsLoading(false)
    }
  }, [repo, branch, agentPath, agentName])

  // Poll for new messages from the assistant
  const pollForResponses = useCallback(async () => {
    if (!conversationPath) return

    try {
      const children = await nodesApi.listChildrenAtHead(repo, branch, AI_WORKSPACE, conversationPath)

      const newMessages: Message[] = []

      for (const child of children) {
        if (child.node_type === 'raisin:AIMessage') {
          const props = child.properties || {}
          const role = props.role as 'user' | 'assistant' | 'system'
          const content = props.content as string || ''
          const finishReason = props.finish_reason as string | undefined

          // Check for child nodes (thoughts, tool calls)
          const messageChildren: MessageChild[] = []
          if (child.has_children) {
            try {
              const messageChildNodes = await nodesApi.listChildrenAtHead(repo, branch, AI_WORKSPACE, child.path)
              for (const mc of messageChildNodes) {
                const mcProps = mc.properties || {}
                if (mc.node_type === 'raisin:AIThought') {
                  messageChildren.push({
                    id: mc.id,
                    type: 'thought',
                    content: mcProps.content as string || '',
                  })
                } else if (mc.node_type === 'raisin:AIToolCall') {
                  // AIToolCall has: function_ref, arguments, status, tool_call_id
                  const toolStatus = mcProps.status as string || 'pending'
                  const functionRef = mcProps.function_ref as Record<string, unknown> | undefined
                  messageChildren.push({
                    id: mc.id,
                    type: 'tool_call',
                    content: `Status: ${toolStatus}`,
                    toolName: (functionRef?.["raisin:path"] as string)?.split('/').pop() || 'unknown',
                    toolInput: mcProps.arguments,
                    status: toolStatus,
                  })

                  // Also fetch tool results that are children of this tool call
                  if (mc.has_children) {
                    try {
                      const toolChildren = await nodesApi.listChildrenAtHead(repo, branch, AI_WORKSPACE, mc.path)
                      for (const tc of toolChildren) {
                        if (tc.node_type === 'raisin:AIToolResult') {
                          const tcProps = tc.properties || {}
                          messageChildren.push({
                            id: tc.id,
                            type: 'tool_result',
                            content: JSON.stringify(tcProps.result ?? tcProps.error ?? '', null, 2),
                            toolName: (functionRef?.["raisin:path"] as string)?.split('/').pop() || 'unknown',
                          })
                        }
                      }
                    } catch (e) {
                      console.error('Failed to load tool result children:', e)
                    }
                  }
                } else if (mc.node_type === 'raisin:AIToolResult') {
                  // AIToolResult has: result or error
                  const functionRef = mcProps.function_ref as Record<string, unknown> | undefined
                  messageChildren.push({
                    id: mc.id,
                    type: 'tool_result',
                    content: JSON.stringify(mcProps.result ?? mcProps.error ?? '', null, 2),
                    toolName: (functionRef?.["raisin:path"] as string)?.split('/').pop() as string,
                  })
                }
              }
            } catch (e) {
              console.error('Failed to load message children:', e)
            }
          }

          newMessages.push({
            id: child.id,
            role,
            content,
            timestamp: child.created_at,
            children: messageChildren,
            finishReason,
          })
        }
      }

      // Sort by timestamp
      newMessages.sort((a, b) => {
        if (!a.timestamp || !b.timestamp) return 0
        return new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime()
      })

      setMessages(newMessages)

      // Check if we got a complete response
      // Keep polling until we have a FINAL assistant message with finish_reason: 'stop'
      // This ensures we wait for the continuation message after tool execution
      if (newMessages.length > 0) {
        const lastMessage = newMessages[newMessages.length - 1]

        if (lastMessage.role === 'assistant') {
          // Check for incomplete tool calls (still running)
          const hasIncompleteToolCalls = lastMessage.children?.some(
            child => child.type === 'tool_call' &&
                     child.status !== 'completed' &&
                     child.status !== 'failed'
          )

          // Only stop polling when:
          // 1. finish_reason is 'stop' or 'error' (terminal states)
          // 2. Has actual content (the final response text)
          // 3. No incomplete tool calls
          //
          // Keep polling if:
          // - finish_reason is 'tool_calls' (waiting for continuation message)
          // - There are incomplete tool calls
          // - No finish_reason but has tool calls (legacy, wait for completion)
          const isTerminalState = lastMessage.finishReason === 'stop' ||
                                  lastMessage.finishReason === 'error'
          const isFinalResponse = isTerminalState &&
                                  lastMessage.content &&
                                  lastMessage.content.trim() !== ''

          if (isFinalResponse && !hasIncompleteToolCalls) {
            setIsWaitingForResponse(false)
            if (pollIntervalRef.current) {
              clearInterval(pollIntervalRef.current)
              pollIntervalRef.current = null
            }
          }
          // If finish_reason is 'tool_calls' or there are incomplete tools, keep polling
        }
      }
    } catch (err) {
      console.error('Failed to poll for responses:', err)
    }
  }, [repo, branch, conversationPath])

  // Start polling when waiting for response
  useEffect(() => {
    if (isWaitingForResponse && !pollIntervalRef.current) {
      pollIntervalRef.current = setInterval(pollForResponses, POLL_INTERVAL)
    }
    return () => {
      if (!isWaitingForResponse && pollIntervalRef.current) {
        clearInterval(pollIntervalRef.current)
        pollIntervalRef.current = null
      }
    }
  }, [isWaitingForResponse, pollForResponses])

  // Send a message
  const handleSend = async () => {
    if (!inputText.trim() || !conversationPath || isSending) return

    const messageContent = inputText.trim()
    setInputText('')
    setIsSending(true)
    setError(null)

    try {
      // Create user message
      await nodesApi.create(repo, branch, AI_WORKSPACE, conversationPath, {
        name: `msg-${Date.now()}`,
        node_type: 'raisin:AIMessage',
        properties: {
          role: 'user',
          content: messageContent,
        },
        commit: {
          message: 'Send test message',
          actor: 'admin',
        },
      })

      // Immediately add to local state for responsive UI
      setMessages(prev => [...prev, {
        id: `temp-${Date.now()}`,
        role: 'user',
        content: messageContent,
        timestamp: new Date().toISOString(),
      }])

      // Start polling for assistant response
      setIsWaitingForResponse(true)

      // Initial poll after a short delay
      setTimeout(pollForResponses, 500)
    } catch (err) {
      console.error('Failed to send message:', err)
      setError('Failed to send message')
    } finally {
      setIsSending(false)
    }
  }

  // Handle Enter key to send
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  // Clear conversation and start fresh
  const handleClearChat = async () => {
    if (conversationPath) {
      try {
        // Delete the conversation
        await nodesApi.delete(repo, branch, AI_WORKSPACE, conversationPath, {
          commit: {
            message: 'Delete test conversation',
            actor: 'admin',
          },
        })
      } catch (err) {
        console.error('Failed to delete conversation:', err)
      }
    }
    // Create a new conversation
    await createConversation()
  }

  // Toggle expand/collapse for message children
  const toggleChildExpanded = (messageId: string, childId: string) => {
    setMessages(prev => prev.map(msg => {
      if (msg.id === messageId && msg.children) {
        return {
          ...msg,
          children: msg.children.map(child => {
            if (child.id === childId) {
              return { ...child, expanded: !child.expanded }
            }
            return child
          }),
        }
      }
      return msg
    }))
  }

  // Loading state
  if (isLoading) {
    return (
      <div className="h-full flex flex-col items-center justify-center text-zinc-400">
        <Loader2 className="w-6 h-6 animate-spin mb-2" />
        <p className="text-sm">Creating test conversation...</p>
      </div>
    )
  }

  // Error state
  if (error && !conversationPath) {
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
              {/* Avatar */}
              <div className={`flex-shrink-0 w-7 h-7 rounded-full flex items-center justify-center ${
                message.role === 'user' ? 'bg-blue-500/20' : 'bg-purple-500/20'
              }`}>
                {message.role === 'user' ? (
                  <User className="w-4 h-4 text-blue-400" />
                ) : (
                  <Bot className="w-4 h-4 text-purple-400" />
                )}
              </div>

              {/* Content */}
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

                {/* Children (thoughts, tool calls) */}
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

        {/* Waiting indicator */}
        {isWaitingForResponse && (
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

      {/* Error message */}
      {error && (
        <div className="flex-shrink-0 px-3 py-2 bg-red-500/10 border-t border-red-500/20">
          <p className="text-xs text-red-400">{error}</p>
        </div>
      )}

      {/* Input */}
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
            disabled={isSending || !conversationPath}
          />
          <button
            onClick={handleSend}
            disabled={!inputText.trim() || isSending || !conversationPath}
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
