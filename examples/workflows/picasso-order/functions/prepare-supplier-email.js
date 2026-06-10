/**
 * prepare-supplier-email - PHASE 3 (SUPPLIER): prepare the blind
 * drop-ship order email for the selected supplier.
 *
 * THE core MTeX privacy rule (footer of the BPMN swimlane):
 *   "Supplier never sees customer name/contact/PO"
 *
 * The function receives the FULL customer object from the flow input but
 * must emit an email body with every customer detail redacted. It also
 * self-audits: redaction_check scans the produced body for each customer
 * field so the caller (and the demo's assertions) can verify the rule
 * mechanically.
 *
 * Input:  {
 *   customer: { company, contact_name, email, po_number },
 *   product, quantity, total_price,
 *   supplier, shipping_mode ("direct" | "via_mtex")
 * }
 * Output: {
 *   email_body, supplier, shipping_mode, supplier_order_ref,
 *   redaction_check: {
 *     contains_customer_name: false,
 *     contains_contact_name: false,
 *     contains_customer_email: false,
 *     contains_po_number: false,
 *   }
 * }
 */
async function handler(input) {
  const { customer, product, quantity, total_price, supplier, shipping_mode } = input;

  if (!customer || typeof customer !== 'object') {
    throw new Error('customer object is required (it is redacted, never forwarded)');
  }
  if (!supplier) {
    throw new Error('supplier is required (HITL selection result)');
  }
  if (shipping_mode !== 'direct' && shipping_mode !== 'via_mtex') {
    throw new Error('shipping_mode must be "direct" or "via_mtex", got: ' + shipping_mode);
  }

  // Simulate template rendering / ERP lookup latency (also avoids the
  // fast-function resume race documented in the example README).
  // Synchronous spin: awaiting setTimeout deadlocks in QuickJS.
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const supplierOrderRef = 'MTX-SUP-' + Date.now() + '-' + Math.floor(Math.random() * 1e4);

  // Ship-to address: NEVER the customer's. Either the supplier ships to
  // the MTeX warehouse, or "direct" ships to an anonymized fulfillment
  // label MTeX provides separately (still no customer identity).
  const shipTo =
    shipping_mode === 'via_mtex'
      ? 'MTeX GmbH warehouse, Industriestrasse 12, 8304 Wallisellen, Switzerland'
      : 'Anonymized end-delivery label ' + supplierOrderRef + '-LBL (provided by MTeX)';

  const emailBody = [
    'To: ' + supplier,
    'Subject: Purchase order ' + supplierOrderRef,
    '',
    'Dear team,',
    '',
    'Please produce and ship the following order:',
    '',
    '  Item:      ' + product,
    '  Quantity:  ' + quantity,
    '  Reference: ' + supplierOrderRef,
    '  Ship to:   ' + shipTo,
    '',
    'Invoice to MTeX GmbH as usual. This order is fulfilled under the',
    'MTeX blind drop-ship policy: end-customer identity is not disclosed.',
    '',
    'Best regards,',
    'MTeX Operations',
  ].join('\n');

  // Self-audit: scan the body for every customer field we were given.
  // (Case-insensitive; empty fields count as not contained.)
  const bodyLower = emailBody.toLowerCase();
  const contains = (value) =>
    typeof value === 'string' && value.length > 0 && bodyLower.includes(value.toLowerCase());

  const redactionCheck = {
    contains_customer_name: contains(customer.company),
    contains_contact_name: contains(customer.contact_name),
    contains_customer_email: contains(customer.email),
    contains_po_number: contains(customer.po_number),
  };

  for (const key in redactionCheck) {
    if (redactionCheck[key]) {
      throw new Error('PRIVACY VIOLATION: supplier email leaks ' + key);
    }
  }

  console.log(
    '[prepare-supplier-email] order', supplierOrderRef, 'for', supplier,
    '(' + shipping_mode + '), customer details redacted',
  );

  return {
    email_body: emailBody,
    supplier,
    shipping_mode,
    supplier_order_ref: supplierOrderRef,
    redaction_check: redactionCheck,
  };
}
