/**
 * reserve-seats - reserve seats for an event and compute the total price.
 *
 * Input:  { event_id: string, quantity: number, tier: "vip" | "standard" }
 * Output: { reservation_id, event_id, quantity, tier, price_per_seat, total_price }
 *
 * Pricing: vip = 150/seat, standard = 50/seat.
 */
const PRICE_PER_SEAT = { vip: 150, standard: 50 };

async function handler(input) {
  const { event_id, quantity, tier } = input;

  if (!event_id) {
    throw new Error('event_id is required');
  }
  const qty = Number(quantity);
  if (!Number.isInteger(qty) || qty < 1) {
    throw new Error('quantity must be a positive integer, got: ' + quantity);
  }
  const pricePerSeat = PRICE_PER_SEAT[tier];
  if (pricePerSeat === undefined) {
    throw new Error('unknown tier: ' + tier + ' (expected vip or standard)');
  }

  // Simulate talking to an external seat-inventory system. The delay also
  // sidesteps a current engine race: a flow-invoked function that finishes
  // before the flow persists its waiting state loses its resume signal and
  // the flow hangs until the wait deadline (see the example README).
  // NOTE: a synchronous spin - awaiting setTimeout deadlocks in the QuickJS
  // function runtime ("blocking on a promise resulted in a dead lock").
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const reservationId =
    'res_' + event_id + '_' + Date.now() + '_' + Math.floor(Math.random() * 1e6);

  console.log(
    '[reserve-seats] reserved', qty, tier, 'seats for', event_id,
    '->', reservationId,
  );

  return {
    reservation_id: reservationId,
    event_id,
    quantity: qty,
    tier,
    price_per_seat: pricePerSeat,
    total_price: qty * pricePerSeat,
  };
}
