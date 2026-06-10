/**
 * allocate-stock - reserve warehouse stock for the order's line items.
 *
 * Input:  { order_id: string, charge_id: string, items: [{sku, qty}, ...] }
 * Output: { allocation_id, order_id, charge_id, items, units_total, allocated_at }
 *
 * `items` arrives via the whole-string template ${input.items} - the
 * function validates it is a REAL array (asserting that arrays survive
 * flow templates), and echoes charge_id so run.mjs can assert the value
 * chain charge-payment -> allocate-stock.
 *
 * Saga: release-stock runs with { allocation_id: ${output.allocation_id} }
 * if a later step (ship-order) fails unrecoverably.
 */
async function handler(input) {
  const { order_id, charge_id, items } = input;

  if (!order_id) {
    throw new Error('order_id is required');
  }
  if (!charge_id) {
    throw new Error('charge_id is required (must flow in from the charge-payment step)');
  }
  if (!Array.isArray(items) || items.length === 0) {
    // If templates mangled the array this fires - scenario A asserts on it.
    throw new Error(
      'items must be a non-empty array, got: ' + Object.prototype.toString.call(items),
    );
  }

  let unitsTotal = 0;
  for (const item of items) {
    const qty = Number(item && item.qty);
    if (!item || !item.sku || !Number.isInteger(qty) || qty < 1) {
      throw new Error('each item needs {sku, qty>=1}, got: ' + JSON.stringify(item));
    }
    unitsTotal += qty;
  }

  // Simulate the WMS reservation round-trip (also avoids the fast-function
  // resume race documented in the example README). Synchronous spin:
  // awaiting setTimeout deadlocks in the QuickJS runtime.
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const allocationId =
    'alloc_' + order_id + '_' + Date.now() + '_' + Math.floor(Math.random() * 1e6);

  console.log(
    '[allocate-stock] allocated', unitsTotal, 'units in', items.length,
    'lines for', order_id, '->', allocationId,
  );

  return {
    allocation_id: allocationId,
    order_id,
    charge_id,
    items,
    units_total: unitsTotal,
    allocated_at: new Date().toISOString(),
  };
}
