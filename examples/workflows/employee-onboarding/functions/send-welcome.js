/**
 * send-welcome - compose (and "send") the welcome email for a new hire.
 *
 * Branches on the manager's approval decision: an approved onboarding gets
 * the welcome text, a rejected one gets the rejection notice (the flow
 * still completes either way - the decision is data, not control flow).
 *
 * Input:  { name, email, decision: "approve"|"reject", fail?: boolean }
 * Output: { welcome_text, sent, decision, email }
 *
 * `fail: true` simulates a mail-gateway outage (used by the saga scenario:
 * the flow rolls back and deprovision-accounts compensates create-accounts).
 */
async function handler(input) {
  const { name, email, decision, fail } = input;

  if (!name) {
    throw new Error('name is required');
  }
  if (!email) {
    throw new Error('email is required');
  }
  if (fail) {
    throw new Error('simulated mail-gateway outage (fail=true)');
  }

  // Simulate the mail gateway latency (also avoids the fast-function
  // resume race documented in the example README). Synchronous spin:
  // awaiting setTimeout deadlocks in the QuickJS function runtime.
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const rejected = decision === 'reject';
  const welcomeText = rejected
    ? 'Hi ' +
      name +
      ', unfortunately your onboarding was not approved by your manager. ' +
      'Your account ' +
      email +
      ' will be deactivated. HR will contact you with next steps.'
    : 'Welcome aboard, ' +
      name +
      '! Your account ' +
      email +
      ' is ready - sign in on your first day and follow the onboarding checklist.';

  console.log('[send-welcome]', rejected ? 'rejection notice' : 'welcome email', 'for', email);

  return {
    welcome_text: welcomeText,
    sent: !rejected,
    decision: decision === undefined ? null : decision,
    email,
  };
}
