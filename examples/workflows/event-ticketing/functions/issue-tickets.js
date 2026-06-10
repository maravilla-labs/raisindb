/**
 * issue-tickets - issue tickets for a confirmed reservation.
 *
 * Input:  { reservation_id: string, quantity: number, fail?: boolean }
 * Output: { ticket_ids: string[], reservation_id, issued_at }
 *
 * `fail: true` simulates a downstream outage (used by the example's saga
 * compensation scenario: the flow rolls back and cancel-reservation runs).
 */
async function handler(input) {
  const { reservation_id, quantity, fail } = input;

  if (!reservation_id) {
    throw new Error('reservation_id is required');
  }
  if (fail) {
    throw new Error('simulated ticket-printer outage (fail=true)');
  }
  const qty = Number(quantity);
  if (!Number.isInteger(qty) || qty < 1) {
    throw new Error('quantity must be a positive integer, got: ' + quantity);
  }

  // Simulate ticket generation latency (also avoids the fast-function
  // resume race documented in the example README). Synchronous spin:
  // awaiting setTimeout deadlocks in the QuickJS function runtime.
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const ticketIds = [];
  for (let i = 1; i <= qty; i++) {
    ticketIds.push(reservation_id + '-T' + String(i).padStart(3, '0'));
  }

  console.log('[issue-tickets] issued', ticketIds.length, 'tickets for', reservation_id);

  return {
    ticket_ids: ticketIds,
    reservation_id,
    issued_at: new Date().toISOString(),
  };
}
