/**
 * cancel-reservation - saga compensation for reserve-seats.
 *
 * Runs automatically (LIFO) when a later step fails unrecoverably after
 * the reservation succeeded. Releases the held seats.
 *
 * Input:  { reservation_id: string }
 * Output: { cancelled: true, reservation_id }
 */
async function handler(input) {
  const { reservation_id } = input;

  if (!reservation_id) {
    throw new Error('reservation_id is required');
  }

  // Simulate releasing the hold in the external inventory system (also
  // avoids the fast-function resume race documented in the example README).
  // Synchronous spin: awaiting setTimeout deadlocks in the QuickJS runtime.
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  console.log('[cancel-reservation] released seats for', reservation_id);

  return {
    cancelled: true,
    reservation_id,
  };
}
