/**
 * Chat + plan panel, sharing one useConversation instance.
 *
 * Step 1 (here): resolve which conversation to open via useConversationList
 * — latest ai_chat with the pilot agent, or none (created lazily on first
 * send). Step 2 (ChatSession): the live conversation.
 */
import { useEffect, useRef, useState } from 'react';
import type { ConversationListItem } from '@raisindb/client';
import { AGENT_PATH, useConversation, useDatabase, useConversationList } from '../lib/raisin';
import { ChatPanel } from './ChatPanel';
import { PlanPanel } from './PlanPanel';

/** agent_ref arrives as a raisin reference object over HTTP; tolerate both shapes. */
function agentRefPath(item: ConversationListItem): string | undefined {
  const ref = item.agentRef as unknown;
  if (typeof ref === 'string') return ref;
  if (ref && typeof ref === 'object') return (ref as Record<string, string>)['raisin:path'];
  return undefined;
}

export function ChatWorkspace({ onTurnSettled }: { onTurnSettled?: () => void }) {
  const db = useDatabase();
  const list = useConversationList({ database: db, type: 'ai_chat' });

  const [resolved, setResolved] = useState(false);
  const [conversationPath, setConversationPath] = useState<string | null>(null);

  // useConversationList kicks off load() on mount; resolve on the first
  // transition back to !isLoading.
  useEffect(() => {
    if (list.isLoading || resolved) return;
    const latest = list.conversations
      .filter((c) => agentRefPath(c) === AGENT_PATH)
      .sort((a, b) => (b.updatedAt ?? '').localeCompare(a.updatedAt ?? ''))[0];
    setConversationPath(latest?.conversationPath ?? null);
    setResolved(true);
  }, [list.isLoading, list.conversations, resolved]);

  if (!resolved) {
    return (
      <div className="chat-plan">
        <section className="panel chat-panel">
          <h2 className="panel-title">Pilot</h2>
          <p className="muted">Opening conversation…</p>
        </section>
        <section className="panel plan-panel">
          <h2 className="panel-title">Plans</h2>
        </section>
      </div>
    );
  }

  return <ChatSession conversationPath={conversationPath} onTurnSettled={onTurnSettled} />;
}

function ChatSession({
  conversationPath,
  onTurnSettled,
}: {
  conversationPath: string | null;
  onTurnSettled?: () => void;
}) {
  const db = useDatabase();
  // With a conversationPath the store resumes it (persistent SSE
  // subscription + history); without one, createOptions makes sendMessage()
  // create the conversation with the agent on first send.
  const chat = useConversation({
    database: db,
    conversationPath: conversationPath ?? undefined,
    createOptions: { participant: AGENT_PATH },
  });

  // Load history when resuming an existing conversation.
  useEffect(() => {
    if (conversationPath) {
      chat.loadMessages().catch((err) => {
        console.warn('[chat] loading history failed:', err);
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conversationPath]);

  // Notify the workspace when a turn settles (streaming true -> false) so
  // the checklist refetches after the agent's tools ran.
  const wasStreaming = useRef(false);
  useEffect(() => {
    if (wasStreaming.current && !chat.isStreaming) onTurnSettled?.();
    wasStreaming.current = chat.isStreaming;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chat.isStreaming]);

  return (
    <div className="chat-plan">
      <ChatPanel chat={chat} />
      <PlanPanel chat={chat} />
    </div>
  );
}
