/**
 * Shift board state (Svelte 5 runes).
 *
 * SSR handoff: +page.server.ts loads shifts + staff via SQL over HTTP and the
 * page seeds this store before first render (server and hydration alike), so
 * the board paints instantly with no client fetch. After hydration, connect()
 * keeps the shifts live with a WebSocket node subscription — when the AI
 * agent (or anyone else) updates a shift node, the matching card updates in
 * place and flashes briefly.
 *
 * SDK APIs used:
 *   db.workspace('staffing').events().subscribe(   - live updates
 *     { path: '/shifts/*', event_types: ['node:updated'], include_node: true },
 *     callback)
 *   db.executeSql(query)                           - resync after reconnect
 */
import type { EventMessage } from '@raisindb/client';
import { getClient, getDb } from '../raisin';
import {
  SHIFTS_SQL,
  STAFF_SQL,
  rowsToShifts,
  rowsToStaff,
  toShift,
  type Shift,
  type ShiftProps,
  type SqlNodeRow,
  type StaffMember,
} from '../board-data';

export { DAYS, type Shift, type StaffMember } from '../board-data';

class BoardState {
  shifts = $state<Shift[]>([]);
  staff = $state<StaffMember[]>([]);
  loading = $state(true);
  error = $state<string | null>(null);

  #connected = false;

  /** Seed from SSR data. Runs during server render and again on hydration. */
  seed(data: { shifts: Shift[]; staff: StaffMember[] }): void {
    this.shifts = data.shifts;
    this.staff = data.staff;
    this.loading = false;
    this.error = null;
  }

  /** Start the live subscription + reconnect resync. Call once after hydration. */
  async connect(): Promise<void> {
    if (this.#connected) return;
    this.#connected = true;

    try {
      // Live updates: any node:updated under /shifts patches the matching
      // card in place. The path filter glob `*` matches one path segment;
      // include_node gives us the full node in the payload, so no follow-up
      // query is needed.
      await getDb().workspace('staffing').events().subscribe(
        { path: '/shifts/*', event_types: ['node:updated'], include_node: true },
        (event) => this.#onShiftEvent(event),
      );

      // The SDK restores subscriptions after a reconnect, but events missed
      // while offline are gone — reload the board to resync.
      getClient().onReconnected(() => {
        this.#load().catch(() => {});
      });
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err);
    }
  }

  async #load(): Promise<void> {
    const db = getDb();
    const [shiftResult, staffResult] = await Promise.all([
      db.executeSql(SHIFTS_SQL),
      db.executeSql(STAFF_SQL),
    ]);

    this.shifts = rowsToShifts((shiftResult.rows ?? []) as unknown as SqlNodeRow[]);
    this.staff = rowsToStaff((staffResult.rows ?? []) as unknown as SqlNodeRow[]);
  }

  #onShiftEvent(event: EventMessage): void {
    const payload = event.payload as
      | { node?: { path?: string; properties?: ShiftProps } }
      | undefined;
    const node = payload?.node;
    if (!node?.path || !node.properties) return;

    const idx = this.shifts.findIndex((s) => s.path === node.path);
    if (idx < 0) return;

    // Replace the card data, bumping flashSeq so the UI replays the highlight.
    this.shifts[idx] = toShift(node.path, node.properties, this.shifts[idx].flashSeq + 1);
  }
}

export const board = new BoardState();
