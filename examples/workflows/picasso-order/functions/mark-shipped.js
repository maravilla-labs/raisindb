/**
 * mark-shipped - PHASE 3 (SUPPLIER): the supplier ships and the order
 * status moves Pending -> In Transit (the "Updates Status" task in the
 * MTeX BPMN swimlane).
 *
 * Input:  { supplier_order_ref: string, supplier: string, shipping_mode: string }
 * Output: { status: "in_transit", tracking_ref, supplier_order_ref, carrier, shipped_at }
 */
async function handler(input) {
  const { supplier_order_ref, supplier, shipping_mode } = input;

  if (!supplier_order_ref) {
    throw new Error('supplier_order_ref is required');
  }

  // Simulate carrier API latency (also avoids the fast-function resume
  // race documented in the example README). Synchronous spin: awaiting
  // setTimeout deadlocks in the QuickJS runtime.
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const carrier = shipping_mode === 'direct' ? 'DHL Express' : 'Planzer (to MTeX warehouse)';
  const trackingRef = 'TRK-' + Date.now() + '-' + Math.floor(Math.random() * 1e4);

  console.log(
    '[mark-shipped]', supplier_order_ref, 'shipped by', supplier || 'supplier',
    'via', carrier, '->', trackingRef,
  );

  return {
    status: 'in_transit',
    tracking_ref: trackingRef,
    supplier_order_ref,
    carrier,
    shipped_at: new Date().toISOString(),
  };
}
