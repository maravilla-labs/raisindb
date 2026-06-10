/**
 * refund-payment - refund a charge.
 *
 * Used in TWO roles by this example:
 *   - saga compensation for charge-payment (runs during rollback, LIFO)
 *   - forward step on the fraud-review "cancel" path (void the charge)
 *
 * Input:  { charge_id: string, reason?: string }
 * Output: { refunded: true, charge_id, reason, refunded_at }
 */
async function handler(input) {
  const { charge_id } = input;
  const reason = input.reason || 'order rollback';

  if (!charge_id) {
    throw new Error('charge_id is required');
  }

  // Simulate PSP refund latency. Deliberately LONG (~2s): during a saga
  // rollback this compensation runs LAST (LIFO), and the engine persists
  // the instance after EACH compensation - the long tail gives run.mjs a
  // wide window to observe the intermediate compensation_stack state
  // (release-stock already executed, refund-payment still pending), which
  // is the live proof of LIFO ordering. Synchronous spin: awaiting
  // setTimeout deadlocks in the QuickJS runtime.
  for (const end = Date.now() + 2000; Date.now() < end; ) {
    // busy-wait
  }

  console.log('[refund-payment] refunded', charge_id, '(' + reason + ')');

  return {
    refunded: true,
    charge_id,
    reason,
    refunded_at: new Date().toISOString(),
  };
}
