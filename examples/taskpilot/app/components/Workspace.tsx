/**
 * Authenticated workspace: header (connection dot, agent mode chip, user),
 * then the three panels — checklist, chat, plan panel. The chat and plan
 * panel share one useConversation instance (ChatWorkspace).
 */
import { useState } from 'react';
import type { UseAuthReturn } from '@raisindb/client/react';
import { AGENT_PATH, useConnection, useSql } from '../lib/raisin';
import { ChecklistPanel } from './ChecklistPanel';
import { ChatWorkspace } from './ChatWorkspace';

const MODE_LABELS: Record<string, string> = {
  automatic: 'Automatic',
  approve_then_auto: 'Approve, then auto',
  step_by_step: 'Step by step',
  manual: 'Manual',
};

function ModeChip() {
  // The agent definition is a node in the `functions` workspace — read its
  // execution_mode live so the chip always reflects the deployed config
  // (switch modes with `node setup.mjs --mode step_by_step`).
  const { data } = useSql<{ mode: string }>(
    "SELECT properties->>'execution_mode'::String AS mode FROM 'functions' WHERE path = $1",
    [AGENT_PATH],
  );
  const mode = data?.[0]?.mode;
  if (!mode) return null;
  return (
    <span className="mode-chip" data-testid="mode-chip" data-mode={mode} title="Agent execution_mode">
      {MODE_LABELS[mode] ?? mode}
    </span>
  );
}

export function Workspace({ auth }: { auth: UseAuthReturn }) {
  const connection = useConnection();
  const [refreshSeq, setRefreshSeq] = useState(0);

  async function signOut() {
    try {
      await auth.logout({ disconnect: false, reconnect: true });
    } catch (err) {
      console.warn('[app] logout failed:', err);
    }
  }

  return (
    <div className="app" data-testid="workspace">
      <header className="app-header">
        <div className="brand">
          <span className="logo-mark">✈</span>
          <span className="brand-name">TaskPilot</span>
          <ModeChip />
        </div>
        <div className="header-right">
          <span
            className={`conn-dot ${connection.isReady ? 'connected' : ''}`}
            data-testid="conn-dot"
            title={connection.isReady ? 'Connected' : 'Connecting…'}
          />
          <span className="user-email">{auth.user?.email}</span>
          <button className="ghost" onClick={signOut}>
            Sign out
          </button>
        </div>
      </header>
      <main className="layout">
        <ChecklistPanel refreshSeq={refreshSeq} />
        <ChatWorkspace onTurnSettled={() => setRefreshSeq((n) => n + 1)} />
      </main>
    </div>
  );
}
