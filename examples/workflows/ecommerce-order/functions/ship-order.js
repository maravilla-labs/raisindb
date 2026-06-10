/**
 * ship-order - hand the allocated parcel to the carrier.
 *
 * Input:  { order_id: string, allocation_id: string, address?: string,
 *           fail?: boolean }
 * Output: { tracking_number, carrier, order_id, allocation_id, shipped_at }
 *
 * `fail: true` simulates a carrier outage (scenario C): the step fails
 * permanently and the flow rolls back - release-stock then refund-payment
 * run as saga compensations in LIFO order.
 */
async function handler(input) {
  const { order_id, allocation_id, fail } = input;

  if (!order_id) {
    throw new Error('order_id is required');
  }
  if (!allocation_id) {
    throw new Error('allocation_id is required (must flow in from allocate-stock)');
  }
  if (fail) {
    throw new Error('simulated carrier outage (fail=true)');
  }

  // Simulate the carrier label API (also avoids the fast-function resume
  // race documented in the example README). Synchronous spin: awaiting
  // setTimeout deadlocks in the QuickJS runtime.
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const trackingNumber =
    'TRK-' + Date.now().toString(36).toUpperCase() + '-' +
    Math.floor(Math.random() * 1e6).toString(36).toUpperCase();

  console.log('[ship-order] shipped', order_id, '(', allocation_id, ') ->', trackingNumber);

  return {
    tracking_number: trackingNumber,
    carrier: 'swisspost',
    order_id,
    allocation_id,
    shipped_at: new Date().toISOString(),
  };
}
