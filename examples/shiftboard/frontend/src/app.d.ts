import type { IdentityUser } from '@raisindb/client';

declare global {
  namespace App {
    interface Locals {
      /**
       * Auth session resolved from the httpOnly cookies in hooks.server.ts.
       * `null` when not logged in (or the token could not be refreshed).
       */
      session: { token: string; user: IdentityUser } | null;
    }
  }
}

export {};
