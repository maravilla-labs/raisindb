/**
 * Client entry point for React Router 7
 *
 * This file handles client-side hydration of the server-rendered application.
 * After hydration, the useHybridClient hook will upgrade to WebSocket for real-time updates.
 */

import { startTransition, StrictMode } from 'react';
import { hydrateRoot } from 'react-dom/client';
import { HydratedRouter } from 'react-router/dom';

startTransition(() => {
  hydrateRoot(
    document,
    <StrictMode>
      <HydratedRouter />
    </StrictMode>
  );
});
