/**
 * release-stock - saga compensation for allocate-stock.
 *
 * Runs automatically (LIFO: FIRST, before refund-payment) when a later
 * step fails unrecoverably after the allocation succeeded.
 *
 * Input:  { allocation_id: string }
 * Output: { released: true, allocation_id, released_at }
 */
async function handler(input) {
  const { allocation_id } = input;

  if (!allocation_id) {
    throw new Error('allocation_id is required');
  }

  // Simulate the WMS release round-trip. Long-ish (~2s) on purpose: the
  // engine saves the instance after each compensation, so the gap between
  // "release-stock executed" and "refund-payment executed" stays wide
  // enough for run.mjs to observe the LIFO ordering live. Synchronous
  // spin: awaiting setTimeout deadlocks in the QuickJS runtime.
  for (const end = Date.now() + 2000; Date.now() < end; ) {
    // busy-wait
  }

  console.log('[release-stock] released allocation', allocation_id);

  return {
    released: true,
    allocation_id,
    released_at: new Date().toISOString(),
  };
}
