/**
 * order-laptop - order hardware for a new engineer.
 *
 * Only reached when the equipment-gate OR container's rule
 * `input.role == "engineer"` matches; everyone else skips the container.
 *
 * Input:  { account_id: string, name: string, role: string }
 * Output: { order_id, model, for_account, ordered_at }
 */
async function handler(input) {
  const { account_id, name, role } = input;

  if (!account_id) {
    throw new Error('account_id is required');
  }

  // Simulate the procurement system round-trip (also avoids the
  // fast-function resume race documented in the example README).
  // Synchronous spin: awaiting setTimeout deadlocks in QuickJS.
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const orderId = 'lap_' + Date.now() + '_' + Math.floor(Math.random() * 1e6);

  console.log('[order-laptop] ordered laptop', orderId, 'for', name, '(' + role + ',', account_id + ')');

  return {
    order_id: orderId,
    model: 'MacBook Pro 14" M5',
    for_account: account_id,
    ordered_at: new Date().toISOString(),
  };
}
