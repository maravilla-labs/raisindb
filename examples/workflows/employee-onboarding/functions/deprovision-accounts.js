/**
 * deprovision-accounts - saga compensation for create-accounts.
 *
 * Runs automatically (LIFO) when a LATER step fails unrecoverably after
 * account provisioning succeeded. Disables the accounts again so no orphan
 * identities are left behind.
 *
 * Input:  { account_id: string }
 * Output: { deprovisioned: true, account_id }
 */
async function handler(input) {
  const { account_id } = input;

  if (!account_id) {
    throw new Error('account_id is required');
  }

  // Simulate revoking the accounts in the identity provider (also avoids
  // the fast-function resume race documented in the example README).
  // Synchronous spin: awaiting setTimeout deadlocks in QuickJS.
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  console.log('[deprovision-accounts] deprovisioned', account_id);

  return {
    deprovisioned: true,
    account_id,
  };
}
