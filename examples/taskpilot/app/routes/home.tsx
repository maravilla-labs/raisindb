/**
 * TaskPilot — single-page workspace.
 *
 * Boot: restore the stored session (useAuth.initSession); show the login
 * card or the workspace (checklist + chat + plan panel). All RaisinDB
 * state flows through the SDK React hooks created in app/lib/raisin.ts.
 */
import { useEffect, useState } from 'react';
import { client, REPOSITORY, RaisinProvider, useAuth } from '../lib/raisin';
import { LoginCard } from '../components/LoginCard';
import { Workspace } from '../components/Workspace';

function Shell() {
  const auth = useAuth();
  const [booting, setBooting] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // Restores tokens from localStorage, reconnects + re-authenticates
        // the WebSocket; resolves null when there is no stored session.
        await auth.initSession(REPOSITORY);
      } catch (err) {
        console.warn('[app] session restore failed, showing login:', err);
      } finally {
        if (!cancelled) setBooting(false);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (booting) return <div className="splash">Loading…</div>;
  if (!auth.isAuthenticated || !auth.user) return <LoginCard auth={auth} />;
  return <Workspace auth={auth} />;
}

export default function Home() {
  return (
    <RaisinProvider client={client} repository={REPOSITORY}>
      <Shell />
    </RaisinProvider>
  );
}
