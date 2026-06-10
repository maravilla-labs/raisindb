/**
 * The live chat column: message bubbles, streaming text, tool-call badges,
 * error banner, input. Bound to the shared useConversation return.
 */
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import type { UseConversationReturn } from '@raisindb/client/react';

export function ChatPanel({ chat }: { chat: UseConversationReturn }) {
  const [input, setInput] = useState('');
  const scroller = useRef<HTMLDivElement | null>(null);

  // Only user/assistant messages with text render as bubbles; tool/system
  // messages surface via the tool-call badges, and plan/task lifecycle
  // messages (messageType ai_plan / ai_task_update) belong to the plan
  // panel, not the transcript.
  const visibleMessages = chat.messages.filter(
    (m) =>
      (m.role === 'user' || m.role === 'assistant') &&
      m.content &&
      m.messageType !== 'ai_plan' &&
      m.messageType !== 'ai_task_update',
  );

  // NOTE: isWaiting must NOT feed this indicator — the store sets
  // isWaiting=true at end-of-turn ("waiting for user input"), so including
  // it shows a permanent "thinking…" bubble after every reply.
  const thinking =
    chat.isStreaming && !chat.streamingText && chat.activeToolCalls.length === 0;

  // Keep the chat scrolled to the bottom as messages/stream/tools change.
  useEffect(() => {
    scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
  }, [visibleMessages.length, chat.streamingText, chat.activeToolCalls.length]);

  function send() {
    const text = input.trim();
    if (!text || chat.isStreaming) return;
    setInput('');
    chat.sendMessage(text).catch((err) => {
      console.warn('[chat] send failed:', err);
    });
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  }

  return (
    <section className="panel chat-panel" data-testid="chat">
      <h2 className="panel-title">Pilot</h2>
      <div className="chat-messages" ref={scroller} data-testid="chat-messages">
        {visibleMessages.length === 0 && !chat.isStreaming && (
          <p className="muted chat-empty">
            Ask Pilot for a plan — e.g. “Plan and execute: mark ‘Design hero
            section’ as done, then summarize the checklist.”
          </p>
        )}

        {visibleMessages.map((message, i) => (
          <div
            key={message.id ?? `${message.role}-${i}`}
            className={`bubble ${message.role}`}
            data-testid={`bubble-${message.role}`}
          >
            {message.content}
          </div>
        ))}

        {chat.isStreaming && chat.streamingText && (
          <div className="bubble assistant streaming" data-testid="streaming">
            {chat.streamingText}
          </div>
        )}

        {chat.activeToolCalls.length > 0 && (
          <div className="tool-calls">
            {chat.activeToolCalls.map((tool) => (
              <span
                key={tool.id}
                className={`tool-badge ${tool.status}`}
                data-testid="tool-badge"
              >
                🔧 {tool.functionName}
                {tool.status === 'running' ? (
                  <span className="spinner" />
                ) : tool.status === 'completed' ? (
                  ' ✓'
                ) : (
                  ' ✗'
                )}
              </span>
            ))}
          </div>
        )}

        {thinking && <div className="bubble assistant thinking">thinking…</div>}
      </div>

      {chat.error && (
        <p className="error-banner" role="alert" data-testid="chat-error">
          {chat.error}
        </p>
      )}

      <div className="chat-input">
        <input
          type="text"
          placeholder="Message Pilot…"
          data-testid="chat-input"
          value={input}
          disabled={chat.isStreaming}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onKeyDown}
        />
        <button
          data-testid="chat-send"
          disabled={chat.isStreaming || !input.trim()}
          onClick={send}
        >
          Send
        </button>
      </div>
    </section>
  );
}
