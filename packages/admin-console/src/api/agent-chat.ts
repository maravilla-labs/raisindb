// SPDX-License-Identifier: BSL-1.1

import { sqlApi } from './sql'
import { requestRaw } from './client'

const CHAT_WORKSPACE = 'raisin:access_control'

export interface CurrentUser {
  userId: string
  userHome: string
}

export interface TestConversation {
  conversationPath: string
  conversationId: string
  streamChannel: string
  userId: string
  userHome: string
}

export interface CreateTestConversationOptions {
  repo: string
  agentPath: string
  agentId: string
  agentName: string
}

export interface SendUserMessageOptions {
  repo: string
  conversation: TestConversation
  agentId: string
  content: string
}

export interface StreamEventsOptions {
  repo: string
  streamChannel: string
  signal?: AbortSignal
}

export type ChatEvent =
  | { type: 'text_chunk'; text: string; timestamp: string }
  | { type: 'thought_chunk'; text: string; timestamp: string }
  | {
      type: 'tool_call_started'
      toolCallId: string
      functionName: string
      arguments: unknown
      timestamp: string
    }
  | {
      type: 'tool_call_completed'
      toolCallId: string
      functionName?: string
      result: unknown
      error?: string
      durationMs?: number
      timestamp: string
    }
  | {
      type: 'message_saved'
      messagePath: string
      role: string
      conversationPath: string
      timestamp: string
    }
  | {
      type: 'done'
      conversationPath: string
      content?: string
      role?: string
      senderDisplayName?: string
      finishReason?: string
      timestamp: string
    }
  | {
      type: 'waiting'
      conversationPath: string
      reason?: string
      timestamp: string
    }
  | { type: 'log'; level: string; message: string; module?: string; timestamp: string }

async function ensureFolderNode(repo: string, path: string, title: string): Promise<void> {
  await sqlApi.executeQuery(
    repo,
    `UPSERT INTO '${CHAT_WORKSPACE}' (path, node_type, properties) VALUES ($1, 'raisin:Folder', $2::jsonb)`,
    [path, JSON.stringify({ title })],
  )
}

export const agentChatApi = {
  /**
   * Resolve the current user's home node path via RAISIN_CURRENT_USER().
   * Required to write into `${userHome}/inbox/...` and `${userHome}/outbox/...`.
   */
  async getCurrentUser(repo: string): Promise<CurrentUser> {
    const result = await sqlApi.executeQuery(
      repo,
      `SELECT RAISIN_CURRENT_USER()->>'path'::String as home, RAISIN_CURRENT_USER()->>'id'::String as user_id`,
    )
    const row = result?.rows?.[0]
    const userHome = row?.home as string | undefined
    const userId = (row?.user_id as string | undefined) || userHome
    if (!userHome) throw new Error('Current user has no home path — cannot start test chat')
    return { userId: userId!, userHome }
  },

  /**
   * Create a `raisin:Conversation` node in the user's inbox that targets the
   * given agent. Triggers `messaging-chat` → `handle-chat` on subsequent
   * outbox writes.
   */
  async createTestConversation(options: CreateTestConversationOptions): Promise<TestConversation> {
    const { repo, agentPath, agentId, agentName } = options
    const { userId, userHome } = await agentChatApi.getCurrentUser(repo)

    const conversationId = `chat-${crypto.randomUUID()}`
    const conversationPath = `${userHome}/inbox/chats/${conversationId}`
    const streamChannel = `chat:${conversationId}`

    await ensureFolderNode(repo, `${userHome}/inbox`, 'Inbox')
    await ensureFolderNode(repo, `${userHome}/inbox/chats`, 'Chats')

    const properties = {
      conversation_type: 'ai_chat',
      conversation_id: conversationId,
      subject: `Test: ${agentName}`,
      participants: [userId, `agent:${agentId}`],
      stream_channel: streamChannel,
      agent_ref: { 'raisin:path': agentPath, 'raisin:workspace': 'functions' },
      unread_count: 0,
    }

    await sqlApi.executeQuery(
      repo,
      `INSERT INTO '${CHAT_WORKSPACE}' (path, node_type, properties) VALUES ($1, 'raisin:Conversation', $2::jsonb)`,
      [conversationPath, JSON.stringify(properties)],
    )

    return { conversationPath, conversationId, streamChannel, userId, userHome }
  },

  /**
   * Write a user message into `${userHome}/outbox/msg-<uuid>`. The
   * `messaging-chat` trigger picks it up, `handle-chat` delivers a copy to
   * the agent's inbox with status="delivered", and `messaging-agent-chat`
   * then fires the agent.
   */
  async sendUserMessage(options: SendUserMessageOptions): Promise<void> {
    const { repo, conversation, agentId, content } = options
    const { userId, userHome, conversationId } = conversation
    const messageId = crypto.randomUUID()
    const messagePath = `${userHome}/outbox/msg-${messageId}`

    const properties = {
      role: 'user',
      message_type: 'chat',
      status: 'pending',
      sender_id: userId,
      sender_path: userHome,
      recipient_id: `agent:${agentId}`,
      body: { content, message_text: content, thread_id: conversationId },
      conversation_id: conversationId,
      client_id: messageId,
      created_at: new Date().toISOString(),
    }

    await sqlApi.executeQuery(
      repo,
      `INSERT INTO '${CHAT_WORKSPACE}' (path, node_type, properties) VALUES ($1, 'raisin:Message', $2::jsonb)`,
      [messagePath, JSON.stringify(properties)],
    )
  },

  /**
   * Stream `ChatEvent`s from `POST /api/conversations/{repo}/events`.
   *
   * Yields parsed `conversation-event` payloads until either the server
   * sends a `done` event, the response closes, or the caller aborts.
   */
  async *streamEvents(options: StreamEventsOptions): AsyncIterable<ChatEvent> {
    const { repo, streamChannel, signal } = options
    const response = await requestRaw(`/api/conversations/${repo}/events`, {
      method: 'POST',
      body: JSON.stringify({ channel: streamChannel }),
      headers: { Accept: 'text/event-stream', 'Content-Type': 'application/json' },
      signal,
    })

    if (!response.body) return

    const reader = response.body.getReader()
    const decoder = new TextDecoder()
    let buffer = ''

    try {
      while (true) {
        const { value, done } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })

        // SSE frames are separated by double newlines (LF or CRLF).
        let sepIndex: number
        while ((sepIndex = findFrameSeparator(buffer)) !== -1) {
          const frame = buffer.slice(0, sepIndex)
          buffer = buffer.slice(sepIndex).replace(/^(\r?\n)+/, '')
          const event = parseSseFrame(frame)
          if (event) {
            yield event
            if (event.type === 'done') return
          }
        }
      }
    } finally {
      try {
        reader.releaseLock()
      } catch {
        // ignore
      }
    }
  },

  /**
   * Delete a test conversation and all its descendants.
   */
  async deleteConversation(repo: string, conversationPath: string): Promise<void> {
    await sqlApi.executeQuery(
      repo,
      `DELETE FROM '${CHAT_WORKSPACE}' WHERE path = $1 OR path LIKE $2`,
      [conversationPath, `${conversationPath}/%`],
    )
  },
}

function findFrameSeparator(buffer: string): number {
  const lf = buffer.indexOf('\n\n')
  const crlf = buffer.indexOf('\r\n\r\n')
  if (lf === -1) return crlf
  if (crlf === -1) return lf
  return Math.min(lf, crlf)
}

function parseSseFrame(frame: string): ChatEvent | null {
  let eventName: string | null = null
  const dataLines: string[] = []
  for (const rawLine of frame.split(/\r?\n/)) {
    if (!rawLine || rawLine.startsWith(':')) continue
    const colon = rawLine.indexOf(':')
    const field = colon === -1 ? rawLine : rawLine.slice(0, colon)
    const value = colon === -1 ? '' : rawLine.slice(colon + 1).replace(/^ /, '')
    if (field === 'event') eventName = value
    else if (field === 'data') dataLines.push(value)
  }
  if (eventName !== 'conversation-event' || dataLines.length === 0) return null
  try {
    return JSON.parse(dataLines.join('\n')) as ChatEvent
  } catch {
    return null
  }
}
