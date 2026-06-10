/**
 * check-feasibility - PHASE 1 (QUOTE): MTeX team reviews a customer's
 * quote request and checks feasibility + pricing.
 *
 * Mirrors the "Reviews Request / Checks feasibility" task in the MTeX
 * BPMN swimlane (MTeX TEAM lane).
 *
 * Input:  { product: string, quantity: number, tier: "starter" | "business" | "enterprise" }
 * Output: { feasible, product, quantity, tier, unit_price, total_price }
 *
 * Pricing: catalog base price per product, tier discount applied
 * (starter 0%, business 5%, enterprise 10%).
 */
const CATALOG = {
  'uv-dtf-roll': 80, // CHF per roll
  'uv-sticker-sheet': 12, // CHF per sheet
  'uv-phone-case': 24, // CHF per case
};

const TIER_MULTIPLIER = {
  starter: 1.0,
  business: 0.95,
  enterprise: 0.9,
};

async function handler(input) {
  const { product, quantity, tier } = input;

  if (!product) {
    throw new Error('product is required');
  }
  const qty = Number(quantity);
  if (!Number.isInteger(qty) || qty < 1) {
    throw new Error('quantity must be a positive integer, got: ' + quantity);
  }
  const multiplier = TIER_MULTIPLIER[tier];
  if (multiplier === undefined) {
    throw new Error(
      'unknown account tier: ' + tier + ' (expected starter, business or enterprise)',
    );
  }
  const basePrice = CATALOG[product];
  if (basePrice === undefined) {
    throw new Error('unknown product: ' + product);
  }

  // Simulate checking supplier availability / production capacity. The
  // delay also sidesteps a current engine race: a flow-invoked function
  // that finishes before the flow persists its waiting state loses its
  // resume signal and the flow hangs until the wait deadline (see the
  // example README). NOTE: a synchronous spin - awaiting setTimeout
  // deadlocks in the QuickJS runtime ("blocking on a promise resulted
  // in a dead lock").
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const feasible = qty <= 10000; // production capacity cap
  const unitPrice = basePrice * multiplier;
  const totalPrice = qty * unitPrice;

  console.log(
    '[check-feasibility]', qty, 'x', product, '(' + tier + ')',
    '-> feasible:', feasible, ', total:', totalPrice, 'CHF',
  );

  return {
    feasible,
    product,
    quantity: qty,
    tier,
    unit_price: unitPrice,
    total_price: totalPrice,
  };
}
