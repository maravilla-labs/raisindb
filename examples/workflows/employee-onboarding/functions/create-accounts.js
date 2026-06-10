/**
 * create-accounts - provision email + system accounts for a new employee.
 *
 * Input:  { name: string, role: string, start_date: string }
 * Output: { account_id, email, username, name, role, start_date, systems }
 *
 * The email/username are derived from the employee name. Engineers get
 * extra system accounts (github, vpn) on top of the base set.
 */
const BASE_SYSTEMS = ['email', 'sso', 'hr-portal'];
const ENGINEER_SYSTEMS = ['github', 'vpn'];

async function handler(input) {
  const { name, role, start_date } = input;

  if (!name || typeof name !== 'string') {
    throw new Error('name is required');
  }
  if (!role || typeof role !== 'string') {
    throw new Error('role is required');
  }
  if (!start_date) {
    throw new Error('start_date is required');
  }

  // Simulate talking to the external identity provider. The delay also
  // sidesteps a current engine race: a flow-invoked function that finishes
  // before the flow persists its waiting state loses its resume signal and
  // the flow hangs until the wait deadline (see the example README).
  // NOTE: a synchronous spin - awaiting setTimeout deadlocks in the QuickJS
  // function runtime ("blocking on a promise resulted in a dead lock").
  for (const end = Date.now() + 250; Date.now() < end; ) {
    // busy-wait
  }

  const username = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '.')
    .replace(/^\.+|\.+$/g, '');
  const email = username + '@example-corp.com';
  const accountId =
    'acct_' + username + '_' + Date.now() + '_' + Math.floor(Math.random() * 1e6);

  const systems = BASE_SYSTEMS.concat(role === 'engineer' ? ENGINEER_SYSTEMS : []);

  console.log('[create-accounts] provisioned', email, 'as', accountId, 'systems:', systems.join(','));

  return {
    account_id: accountId,
    email,
    username,
    name,
    role,
    start_date,
    systems,
  };
}
