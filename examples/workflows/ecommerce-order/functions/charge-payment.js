/**
 * charge-payment - charge the customer's payment method for an order.
 *
 * Input:  { order_id: string, amount: number, currency?: string }
 * Output: { charge_id, order_id, amount, currency, charged_at }
 *
 * Saga: if any LATER step fails unrecoverably, refund-payment runs with
 * { charge_id: ${output.charge_id} } via compensation_input_mapping.
 */
async function handler(input) {
  const { order_id, amount } = input;
  const currency = input.currency || 'CHF';

  if (!order_id) {
    throw new Error('order_id is required');
  }
  const amt = Number(amount);
  if (!Number.isFinite(amt) || amt <= 0) {
    throw new Error('amount must be a positive number, got: ' + amount);
  }

  // Simulate the PSP round-trip. The delay also sidesteps a current engine
  // race: a flow-invoked function that finishes before the flow persists
  // its waiting state loses its resume signal and the flow hangs until the
  // wait deadline (see the example README). NOTE: synchronous spin -
  // awaiting setTimeout deadlocks in the QuickJS function runtime
  // ("blocking on a promise resulted in a dead lock").
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const chargeId =
    'ch_' + order_id + '_' + Date.now() + '_' + Math.floor(Math.random() * 1e6);

  console.log('[charge-payment] charged', amt, currency, 'for', order_id, '->', chargeId);

  return {
    charge_id: chargeId,
    order_id,
    amount: amt,
    currency,
    charged_at: new Date().toISOString(),
  };
}
