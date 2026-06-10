#!/usr/bin/env node
/**
 * Shiftboard demo - setup script (idempotent, re-runnable).
 *
 * NOTE: The recommended way to install shiftboard is the PACKAGE flow via the
 * raisindb CLI (works for remote instances and CI - see ci.sh):
 *
 *   raisindb login --server $RAISINDB_SERVER --username admin --password ...
 *   raisindb package deploy ./package --repo <repo> --install
 *
 * This script is the no-CLI fallback (plain HTTP, dev server). It shares the
 * package content as the single source of truth: the function sources are
 * read from package/content/functions/lib/shiftboard/<name>/index.js.
 * Package-only equivalents: workspace + seeds = package/workspaces/ +
 * package/content/staffing/, agent = package/content/functions/agents/,
 * agent home = package/content/ai/agents/.
 *
 * Creates on a running raisin-server (dev mode):
 *   1. Repository `shiftboard` (builtin packages auto-install: messaging
 *      pipeline, ai-tools incl. the agent-handler + weather tool)
 *   2. Workspace `staffing` with weekend shift + staff nodes (raisin:Node)
 *   3. Tool functions under /lib/shiftboard/ in the functions workspace:
 *        list-shifts, list-staff, assign-shift, message-staff,
 *        pick-candidates, resolve-accepter, start-shift-fill
 *   3b. The durable workflow /flows/fill-shift (raisin:Flow): asks staff
 *       via inbox accept/decline tasks, assigns the first accepter,
 *       notifies the manager
 *   4. AI agent /agents/shift-planner (Groq) wired to those tools + the
 *      builtin weather tool
 *   5. The agent's home folder in the `ai` workspace (inbox/outbox/...)
 *      so the messaging pipeline can deliver chats to it
 *   6. Identity users: planner@example.com / Planner12345! (manager) and
 *      anna@example.com + cara@example.com / Staff12345! (staff chat accounts)
 *
 * Prereqs:
 *   - raisin-server running on RAISIN_URL (default http://localhost:8081)
 *   - Groq provider configured for the tenant (admin console -> AI settings,
 *     or PUT /api/tenants/{tenant}/ai/config)
 *
 * Run: npm install && npm run setup
 */

import { readFile } from 'node:fs/promises';

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const REPO = process.env.RAISIN_REPO ?? 'shiftboard';
const ADMIN_USER = process.env.RAISIN_USER ?? 'admin';
const ADMIN_PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';
const TENANT = process.env.RAISIN_TENANT ?? 'default';
const AGENT_MODEL = process.env.SHIFTBOARD_MODEL ?? 'llama-3.3-70b-versatile';

const PLANNER_EMAIL = 'planner@example.com';
const PLANNER_PASSWORD = 'Planner12345!';

// Staff members that get real chat accounts (the agent coordinates with
// them via message-staff). Password shared by the demo/test scripts.
export const STAFF_USERS = [
  { email: 'anna@example.com', display_name: 'Anna' },
  { email: 'cara@example.com', display_name: 'Cara' },
];
export const STAFF_PASSWORD = 'Staff12345!';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let adminToken = '';

async function api(method, path, body) {
  const res = await fetch(`${BASE_URL}${path}`, {
    method,
    headers: {
      'Content-Type': 'application/json',
      ...(adminToken ? { Authorization: `Bearer ${adminToken}` } : {}),
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  return res;
}

async function loginAdmin() {
  const res = await api('POST', `/api/raisindb/sys/${TENANT}/auth`, {
    username: ADMIN_USER,
    password: ADMIN_PASSWORD,
  });
  if (!res.ok) throw new Error(`admin login failed: HTTP ${res.status} ${await res.text()}`);
  adminToken = (await res.json()).token;
  console.log('✅ Admin authenticated');
}

/** Create a node in a workspace; on "already exists" refresh properties. */
async function ensureNode(workspace, parentPath, name, nodeType, properties) {
  const base = `/api/repository/${REPO}/main/head/${workspace}`;
  let lastError = '';
  for (let i = 0; i < 20; i++) {
    const created = await api('POST', `${base}${parentPath}`, {
      node: { name, node_type: nodeType, properties },
    });
    if (created.ok) return 'created';
    lastError = await created.text();
    if (/exists|conflict/i.test(lastError)) {
      const childPath =
        parentPath === '/' ? `/${name}` : `${parentPath.replace(/\/$/, '')}/${name}`;
      const updated = await api('PUT', `${base}${childPath}`, { properties });
      return updated.ok ? 'updated' : 'reused';
    }
    await sleep(1000);
  }
  throw new Error(`Failed to create ${workspace}:${parentPath}/${name}: ${lastError}`);
}

async function ensureRepository() {
  const res = await api('POST', '/api/repositories', { repo_id: REPO });
  if (res.ok) {
    console.log(`✅ Repository '${REPO}' created`);
  } else {
    const text = await res.text();
    if (!/exists/i.test(text)) throw new Error(`repo create failed: ${text}`);
    console.log(`ℹ️  Repository '${REPO}' already exists`);
  }

  // Builtin packages (messaging, ai-tools) install asynchronously on repo
  // creation - wait for the agent-handler function they provide.
  for (let i = 0; i < 60; i++) {
    const res = await api(
      'GET',
      `/api/repository/${REPO}/main/head/functions/lib/raisin/ai/agent-handler`,
    );
    if (res.ok) {
      console.log('✅ Builtin packages installed (agent-handler present)');
      return;
    }
    await sleep(1000);
  }
  throw new Error('builtin packages did not install (agent-handler missing after 60s)');
}

async function ensureStaffingWorkspace() {
  const res = await api('PUT', `/api/workspaces/${REPO}/staffing`, {
    name: 'staffing',
    description: 'Shift planning board',
    allowed_node_types: ['raisin:Folder', 'raisin:Node'],
    allowed_root_node_types: ['raisin:Folder'],
  });
  if (!res.ok) throw new Error(`workspace create failed: HTTP ${res.status} ${await res.text()}`);
  console.log('✅ Workspace staffing ready');
}

const SHIFTS = [
  { name: 'fri-evening', title: 'Friday Evening', day: 'friday',   start: '17:00', end: '23:00', location: 'Main bar',     outdoor: false },
  { name: 'sat-morning', title: 'Saturday Morning', day: 'saturday', start: '08:00', end: '14:00', location: 'Terrace',      outdoor: true },
  { name: 'sat-evening', title: 'Saturday Evening', day: 'saturday', start: '16:00', end: '23:00', location: 'Main bar',     outdoor: false },
  { name: 'sun-morning', title: 'Sunday Morning', day: 'sunday',   start: '09:00', end: '14:00', location: 'Terrace',      outdoor: true },
  { name: 'sun-evening', title: 'Sunday Evening', day: 'sunday',   start: '15:00', end: '21:00', location: 'Restaurant',   outdoor: false },
];

const STAFF = [
  { name: 'anna', title: 'Anna', role: 'barista',  email: 'anna@example.com', available_days: ['friday', 'saturday'] },
  { name: 'ben',  title: 'Ben',  role: 'waiter',   email: 'ben@example.com',  available_days: ['saturday', 'sunday'] },
  { name: 'cara', title: 'Cara', role: 'all-round', email: 'cara@example.com', available_days: ['friday', 'saturday', 'sunday'] },
  { name: 'dave', title: 'Dave', role: 'waiter',   email: 'dave@example.com', available_days: ['sunday'] },
];

async function seedStaffingNodes() {
  await ensureNode('staffing', '/', 'shifts', 'raisin:Folder', { title: 'Shifts' });
  await ensureNode('staffing', '/', 'staff', 'raisin:Folder', { title: 'Staff' });

  for (const s of SHIFTS) {
    const { name, ...props } = s;
    await ensureNode('staffing', '/shifts', name, 'raisin:Node', {
      ...props,
      status: 'open',
      assignee: null,
    });
  }
  for (const m of STAFF) {
    const { name, ...props } = m;
    await ensureNode('staffing', '/staff', name, 'raisin:Node', props);
  }
  console.log(`✅ Seeded ${SHIFTS.length} shifts + ${STAFF.length} staff (all shifts open)`);
}

async function deployFunction(name, title, description, inputSchema, outputSchema) {
  // Single source of truth: the package layout (see package/ + ci.sh)
  const code = await readFile(
    new URL(`./package/content/functions/lib/shiftboard/${name}/index.js`, import.meta.url),
    'utf8',
  );
  await ensureNode('functions', '/lib', 'shiftboard', 'raisin:Folder', {});
  await ensureNode('functions', '/lib/shiftboard', name, 'raisin:Function', {
    name,
    title,
    description,
    enabled: true,
    language: 'javascript',
    execution_mode: 'async',
    entry_file: 'index.js:handler',
    input_schema: inputSchema,
    output_schema: outputSchema,
    version: 1,
  });
  await ensureNode('functions', `/lib/shiftboard/${name}`, 'index.js', 'raisin:Asset', {
    title: 'index.js',
    file: '',
    code,
  });
  console.log(`✅ Function deployed: /lib/shiftboard/${name}`);
}

async function deployTools() {
  await deployFunction(
    'list-shifts',
    'List Shifts',
    'List the shifts on the staffing board with day, time, location, whether the spot is outdoor, status (open/filled) and current assignee. Optionally filter by day or status.',
    {
      type: 'object',
      additionalProperties: false,
      properties: {
        day: { type: 'string', enum: ['friday', 'saturday', 'sunday'], description: 'Only shifts on this day' },
        status: { type: 'string', enum: ['open', 'filled'], description: 'Only shifts with this status' },
      },
    },
    {
      type: 'object',
      properties: {
        shifts: { type: 'array', description: 'Matching shifts with path, title, day, start, end, location, outdoor, status, assignee' },
      },
    },
  );

  await deployFunction(
    'list-staff',
    'List Staff',
    'List all staff members with their role, their email address, whether they are reachable by chat (reachable=true means message-staff works for that email) and which days they are available to work.',
    { type: 'object', additionalProperties: false, properties: {} },
    {
      type: 'object',
      properties: {
        staff: { type: 'array', description: 'Staff members with name, role, email, reachable, available_days' },
      },
    },
  );

  await deployFunction(
    'assign-shift',
    'Assign Shift',
    'Assign a staff member to a shift by setting the assignee. Pass the shift node path (e.g. /shifts/sat-morning) and the staff member name. Pass staff_name null to clear an assignment and re-open the shift.',
    {
      type: 'object',
      additionalProperties: false,
      required: ['shift_path'],
      properties: {
        shift_path: { type: 'string', description: 'Shift node path, e.g. /shifts/sat-morning' },
        staff_name: { type: 'string', description: 'Staff member name to assign, or null to clear' },
      },
    },
    {
      type: 'object',
      properties: {
        shift_path: { type: 'string' },
        assignee: { type: 'string' },
        status: { type: 'string', description: 'open or filled after the change' },
      },
    },
  );

  await deployFunction(
    'pick-candidates',
    'Pick Candidates',
    'Find the candidates for filling a shift: staff members who are available on the shift day AND reachable (their email belongs to a registered identity user). Returns the shift details plus the ordered candidate list with each candidate\'s identity-user home path (used as the inbox task assignee by the fill-shift workflow).',
    {
      type: 'object',
      additionalProperties: false,
      required: ['shift_path'],
      properties: {
        shift_path: { type: 'string', description: 'Shift node path, e.g. /shifts/sun-evening' },
      },
    },
    {
      type: 'object',
      properties: {
        shift_path: { type: 'string' },
        shift_title: { type: 'string' },
        day: { type: 'string' },
        start: { type: 'string' },
        end: { type: 'string' },
        location: { type: 'string' },
        candidates: { type: 'array', description: 'Ordered candidates with name, email, user_path' },
      },
    },
  );

  await deployFunction(
    'resolve-accepter',
    'Resolve Accepter',
    'Workflow helper for /flows/fill-shift: pairs the ask-each loop results (one human-task response per asked candidate, in order) back with the candidate list to determine who accepted, who declined, and the outcome summary sent to the manager.',
    {
      type: 'object',
      additionalProperties: true,
      properties: {
        candidates: { type: 'array', description: 'Ordered candidates from pick-candidates' },
        results: { type: 'array', description: 'Loop results - one human response per asked candidate' },
        shift_title: { type: 'string' },
        day: { type: 'string' },
        start: { type: 'string' },
        end: { type: 'string' },
        location: { type: 'string' },
        shift_path: { type: 'string' },
      },
    },
    {
      type: 'object',
      properties: {
        accepted: { type: 'boolean' },
        accepter_name: { type: 'string' },
        accepter_email: { type: 'string' },
        asked: { type: 'integer' },
        declined_names: { type: 'array' },
        summary: { type: 'string' },
      },
    },
  );

  await deployFunction(
    'start-shift-fill',
    'Start Shift Fill Workflow',
    'Start the durable fill-shift workflow for an open shift. The workflow engine asks each available and reachable staff member via an inbox approval task (accept/decline buttons with a deadline), assigns the first accepter to the shift, and messages the manager with the outcome. Use this instead of chatting with staff yourself when the manager wants a tracked, auditable process ("via tasks", "with a workflow"). Pass the shift node path (e.g. /shifts/sun-evening). Returns the workflow instance id immediately - the coordination continues in the background.',
    {
      type: 'object',
      additionalProperties: false,
      required: ['shift_path'],
      properties: {
        shift_path: { type: 'string', description: 'Shift node path, e.g. /shifts/sun-evening' },
      },
    },
    {
      type: 'object',
      properties: {
        instance_id: { type: 'string', description: 'Flow instance id of the started workflow' },
        status: { type: 'string' },
        message: { type: 'string' },
      },
    },
  );

  await deployFunction(
    'message-staff',
    'Message Staff',
    'Send a short chat message to a person by email address - a staff member (to ask if they can take a shift) or the manager (to report an outcome). The message is delivered into their inbox; when they reply, you receive the reply as a new chat message in that same conversation. Always restate which shift the message is about.',
    {
      type: 'object',
      additionalProperties: false,
      required: ['staff_email', 'message'],
      properties: {
        staff_email: { type: 'string', description: 'Email address of the recipient, e.g. anna@example.com' },
        message: { type: 'string', description: 'The chat message text to send. Mention the shift (name, day, time, location) so the thread keeps context.' },
      },
    },
    {
      type: 'object',
      properties: {
        delivered_to: { type: 'string', description: 'Email the message was sent to' },
        conversation_id: { type: 'string', description: 'Conversation thread the message was placed in' },
      },
    },
  );
}

// The /flows/fill-shift workflow in DESIGNER format.
// Keep in sync with package/content/functions/flows/fill-shift/.node.yaml
const FILL_SHIFT_WORKFLOW = {
  version: 1,
  error_strategy: 'fail_fast',
  nodes: [
    // 1. Who could take this shift? Available on the day + registered
    //    identity user (their home path becomes the task assignee).
    {
      id: 'pick_candidates',
      node_type: 'raisin:FlowStep',
      properties: {
        action: 'Pick candidates for {{ input.shift_path }}',
        function_ref: '/lib/shiftboard/pick-candidates',
        arguments: { shift_path: '${input.shift_path}' },
        // Function JOB retries run ~40s on a fixed backoff - the step's
        // wait deadline must outlive them (see docs/workflows.md).
        timeout_ms: 120000,
      },
    },
    // 2. Ask each candidate in order via an inbox approval task; `until`
    //    stops the loop after the first accept.
    {
      id: 'ask_each',
      node_type: 'raisin:FlowContainer',
      container_type: 'loop',
      loop: {
        over: '${steps.pick_candidates.candidates}',
        item: 'candidate',
        index: 'candidate_index',
        max_iterations: 10,
        until: 'steps.ask_candidate.action == "accept"',
      },
      children: [
        {
          id: 'ask_candidate',
          node_type: 'raisin:FlowStep',
          properties: {
            action:
              'Can you take {{ steps.pick_candidates.shift_title }} ' +
              '({{ steps.pick_candidates.day }} {{ steps.pick_candidates.start }}-{{ steps.pick_candidates.end }})?',
            step_type: 'human_task',
            task_type: 'approval',
            assignee: '${candidate.user_path}',
            task_description:
              'Hi {{ candidate.name }} - the {{ steps.pick_candidates.shift_title }} shift ' +
              '({{ steps.pick_candidates.day }} {{ steps.pick_candidates.start }}-{{ steps.pick_candidates.end }}, ' +
              '{{ steps.pick_candidates.location }}, {{ input.shift_path }}) is open. Can you take it?',
            priority: 4,
            // Deadline doubles as the wait timeout; on expiry the task is
            // marked expired and the flow jumps to resolve_accepter.
            due_in_seconds: 300,
            timeout_edge: 'resolve_accepter',
            options: [
              { value: 'accept', label: 'Accept', style: 'success' },
              { value: 'decline', label: 'Decline', style: 'danger' },
            ],
          },
        },
      ],
    },
    // 3. Pair the loop results with the candidate list.
    {
      id: 'resolve_accepter',
      node_type: 'raisin:FlowStep',
      properties: {
        action: 'Determine who accepted',
        function_ref: '/lib/shiftboard/resolve-accepter',
        arguments: {
          candidates: '${steps.pick_candidates.candidates}',
          results: '${steps.ask_each.results}',
          shift_path: '${input.shift_path}',
          shift_title: '${steps.pick_candidates.shift_title}',
          day: '${steps.pick_candidates.day}',
          start: '${steps.pick_candidates.start}',
          end: '${steps.pick_candidates.end}',
          location: '${steps.pick_candidates.location}',
        },
        timeout_ms: 120000,
      },
    },
    // 4. Somebody accepted -> assign them; otherwise the container is skipped.
    {
      id: 'assign_or_report',
      node_type: 'raisin:FlowContainer',
      container_type: 'or',
      rules: [{ condition: 'steps.resolve_accepter.accepted == true', next_step: 'assign_shift' }],
      children: [
        {
          id: 'assign_shift',
          node_type: 'raisin:FlowStep',
          properties: {
            action: 'Assign {{ steps.resolve_accepter.accepter_name }} to {{ input.shift_path }}',
            function_ref: '/lib/shiftboard/assign-shift',
            arguments: {
              shift_path: '${input.shift_path}',
              staff_name: '${steps.resolve_accepter.accepter_name}',
            },
            timeout_ms: 120000,
          },
        },
      ],
    },
    // 5. Always report the outcome to the manager - best effort.
    {
      id: 'notify_manager',
      node_type: 'raisin:FlowStep',
      properties: {
        action: 'Report the outcome to the manager',
        function_ref: '/lib/shiftboard/message-staff',
        arguments: {
          staff_email: 'planner@example.com',
          message: '${steps.resolve_accepter.summary}',
        },
        timeout_ms: 120000,
        continue_on_fail: true,
      },
    },
  ],
};

async function deployFlow() {
  await ensureNode('functions', '/', 'flows', 'raisin:Folder', {});
  await ensureNode('functions', '/flows', 'fill-shift', 'raisin:Flow', {
    name: 'fill-shift',
    title: 'Fill Shift',
    description:
      'Durable shift-filling workflow: ask available, reachable staff one by one ' +
      'via inbox accept/decline tasks, assign the first accepter, notify the manager.',
    enabled: true,
    workflow_data: FILL_SHIFT_WORKFLOW,
  });
  console.log('✅ Flow deployed: /flows/fill-shift');
}

async function deployAgent() {
  await ensureNode('functions', '/', 'agents', 'raisin:Folder', {});
  await ensureNode('functions', '/agents', 'shift-planner', 'raisin:AIAgent', {
    title: 'Shift Planner',
    // Keep in sync with package/content/functions/agents/shift-planner/.node.yaml
    system_prompt: [
      'You are the shift planning assistant for a small cafe. You help the',
      'manager fill the weekend shift board, and you coordinate with staff',
      'directly over chat.',
      '',
      'You have tools to list shifts, list staff (role, availability, email),',
      'assign or clear shift assignments, check the weather for outdoor',
      'shifts, and send a chat message to a person by email (message-staff).',
      '',
      'If the manager explicitly names who to assign (e.g. "assign Ben to the',
      'Saturday evening shift"), do it directly with assign-shift - no chat',
      'coordination needed. Use the coordination protocol below when the',
      'manager asks you to FILL a shift or find someone for it.',
      '',
      'If the manager asks to fill a shift "via tasks", "with a workflow", or',
      'asks for a tracked/auditable process, use start-shift-fill instead of',
      'chatting with staff yourself - it asks each candidate through inbox',
      'tasks and reports the outcome to the manager automatically.',
      '',
      'Coordination protocol - when the manager asks you to fill a shift:',
      '1. Check the shift and staff availability with your tools first.',
      '2. Do NOT assign anyone yet. Pick ONE candidate who is available on',
      '   that day AND reachable (list-staff reachable=true) and send them a',
      '   short chat message via message-staff asking if they can take the',
      '   shift. Address them by name and include the shift title, day,',
      '   start-end time, location AND the shift node path (e.g.',
      '   /shifts/sat-morning). Then tell the manager who you asked.',
      "3. A staff member's reply arrives in the same chat thread as your",
      '   question to them. When a staff member replies:',
      '   - If they ACCEPT: in one step, assign them to that shift with',
      '     assign-shift AND send the manager (planner@example.com) a short',
      '     confirmation via message-staff. Then thank them in your reply.',
      '   - If they DECLINE: call list-staff, then send the next available',
      '     AND reachable candidate a message-staff request for the same',
      '     shift. Mention in that message which colleague(s) already',
      '     declined, so the thread carries the state. Never ask someone who',
      '     already declined this shift, and never ask the same person twice',
      '     for the same shift.',
      '   - If no candidates are left to ask: inform the manager',
      '     (planner@example.com) via message-staff.',
      '4. Restate which shift you are talking about (title, day, time, shift',
      '   path) in every message you send, so each thread keeps its context.',
      '',
      'Rules:',
      '- Always check current shift and staff data with your tools before',
      '  answering questions about the board - never guess.',
      '- Only assign staff who are available on the shift day.',
      '- When you mention a shift, use ONLY the exact title, day, time and',
      '  location from your tool results. Never invent venue names, times,',
      '  or people.',
      '- A shift that is already filled stays as it is - never reassign or',
      '  overwrite an existing assignee unless the manager explicitly asks.',
      '  For status questions, report the live board from list-shifts; take',
      '  no action.',
      '- When the manager asked you to coordinate via chat, only assign a',
      '  staff member after they confirmed in chat.',
      '- Never use message-staff to write to the person you are currently',
      '  chatting with - your normal reply already reaches them. Use',
      '  message-staff only to contact OTHER people (the next candidate or',
      '  the manager).',
      '- Only message people who are reachable (list-staff reachable=true).',
      '  If message-staff still returns a not-reachable error, pick another',
      '  candidate.',
      '- Keep answers short and factual.',
    ].join('\n'),
    provider: 'groq',
    model: AGENT_MODEL,
    temperature: 0.2,
    max_tokens: 1024,
    task_creation_enabled: false,
    thinking_enabled: false,
    execution_mode: 'automatic',
    execution_context: 'system',
    tools: [
      '/lib/shiftboard/list-shifts',
      '/lib/shiftboard/list-staff',
      '/lib/shiftboard/assign-shift',
      '/lib/shiftboard/message-staff',
      '/lib/shiftboard/start-shift-fill',
      '/lib/raisin/ai/weather',
    ],
    rules: ['Never assign a staff member to a day they are not available on.'],
  });
  console.log(`✅ Agent /agents/shift-planner deployed (groq ${AGENT_MODEL})`);

  // Agent home in the `ai` workspace - the messaging pipeline delivers chat
  // messages into <agent>/inbox/chats/ and the agent-handler picks them up.
  await ensureNode('ai', '/agents', 'shift-planner', 'raisin:Folder', {
    title: 'Shift Planner',
    agent_ref: {
      'raisin:ref': '',
      'raisin:workspace': 'functions',
      'raisin:path': '/agents/shift-planner',
    },
    user_id: 'agent:shift-planner',
    display_name: 'Shift Planner',
    max_turns: 10,
  });
  for (const [parent, name, title] of [
    ['/agents/shift-planner', 'inbox', 'Inbox'],
    ['/agents/shift-planner', 'outbox', 'Outbox'],
    ['/agents/shift-planner', 'memory', 'User Memory'],
    ['/agents/shift-planner', 'sent', 'Sent'],
    ['/agents/shift-planner/inbox', 'chats', 'Chats'],
    ['/agents/shift-planner/inbox', 'notifications', 'Notifications'],
  ]) {
    await ensureNode('ai', parent, name, 'raisin:Folder', { title });
  }
  console.log('✅ Agent home created in ai workspace (inbox/outbox/...)');
}

/**
 * The PLAN-enabled coordinator agent (the "Planner" tab in the frontend).
 *
 * Composition demo: task_creation_enabled + execution_mode approve_then_auto
 * means the agent proposes a raisin:AIPlan (one task per open shift) that the
 * manager must APPROVE; each approved task then runs start-shift-fill, i.e.
 * starts the durable /flows/fill-shift workflow for that shift. A "completed"
 * plan task means the workflow was STARTED - the board fills as staff accept
 * their inbox tasks.
 */
async function deployCoordinatorAgent() {
  await ensureNode('functions', '/', 'agents', 'raisin:Folder', {});
  await ensureNode('functions', '/agents', 'shift-coordinator', 'raisin:AIAgent', {
    title: 'Shift Coordinator',
    // Keep in sync with package/content/functions/agents/shift-coordinator/.node.yaml
    system_prompt: [
      'You are the shift coordinator for a small cafe. You help the manager',
      'fill the weekend shift board by starting the durable fill-shift',
      'workflow - you never chat with staff yourself and you never assign',
      'anyone directly.',
      '',
      'You have tools to list shifts, list staff, start the fill-shift',
      'workflow for one shift (start-shift-fill), and the planning tools',
      '(create-plan, add-task, update-task, get-plan-status).',
      '',
      'When the manager asks to fill MULTIPLE shifts (e.g. "fill all open',
      'shifts", "fill the weekend", "fill Saturday"):',
      '1. Call list-shifts with status "open" to get the open shifts.',
      '2. Call create-plan with ONE task per open shift. Each task title',
      '   MUST be the shift title followed by its path, e.g.',
      '   "Fill Saturday Morning (/shifts/sat-morning)". Order the tasks',
      '   Friday -> Saturday -> Sunday.',
      '3. When executing a task: mark it in_progress with update-task,',
      "   call start-shift-fill with that task's shift path, then mark the",
      '   task completed with update-task (use the task_id values returned',
      '   by create-plan). One start-shift-fill call per task - never skip',
      '   a task and never call start-shift-fill twice for the same shift.',
      '4. After the last task, summarize: which shifts got a workflow',
      '   started, with their instance ids.',
      '',
      'Be honest about what "completed" means: completing a task means the',
      'fill-shift WORKFLOW WAS STARTED for that shift - staff still have to',
      'accept their inbox tasks before the shift is actually filled. Say so',
      'in your summary; never claim a shift is filled unless list-shifts',
      'shows status "filled".',
      '',
      'When the manager names a SINGLE specific shift ("fill the Sunday',
      'evening shift"), do NOT create a plan - just call start-shift-fill',
      'for that shift directly and report the instance id.',
      '',
      'Rules:',
      '- Always check the live board with list-shifts before planning -',
      '  never guess which shifts are open.',
      '- Only plan tasks for shifts whose status is "open"; leave filled',
      '  shifts alone.',
      '- Keep answers short and factual.',
    ].join('\n'),
    provider: 'groq',
    model: AGENT_MODEL,
    temperature: 0.2,
    max_tokens: 1024,
    task_creation_enabled: true,
    thinking_enabled: false,
    execution_mode: 'approve_then_auto',
    execution_context: 'system',
    tools: [
      '/lib/shiftboard/list-shifts',
      '/lib/shiftboard/list-staff',
      '/lib/shiftboard/start-shift-fill',
      '/lib/raisin/ai/create-plan',
      '/lib/raisin/ai/add-task',
      '/lib/raisin/ai/update-task',
      '/lib/raisin/ai/get-plan-status',
    ],
    rules: ['Only start fill-shift workflows for shifts that are currently open.'],
  });
  console.log(`✅ Agent /agents/shift-coordinator deployed (groq ${AGENT_MODEL}, approve_then_auto)`);

  await ensureNode('ai', '/agents', 'shift-coordinator', 'raisin:Folder', {
    title: 'Shift Coordinator',
    agent_ref: {
      'raisin:ref': '',
      'raisin:workspace': 'functions',
      'raisin:path': '/agents/shift-coordinator',
    },
    user_id: 'agent:shift-coordinator',
    display_name: 'Shift Coordinator',
    max_turns: 10,
  });
  for (const [parent, name, title] of [
    ['/agents/shift-coordinator', 'inbox', 'Inbox'],
    ['/agents/shift-coordinator', 'outbox', 'Outbox'],
    ['/agents/shift-coordinator', 'memory', 'User Memory'],
    ['/agents/shift-coordinator', 'sent', 'Sent'],
    ['/agents/shift-coordinator/inbox', 'chats', 'Chats'],
    ['/agents/shift-coordinator/inbox', 'notifications', 'Notifications'],
  ]) {
    await ensureNode('ai', parent, name, 'raisin:Folder', { title });
  }
  console.log('✅ Coordinator agent home created in ai workspace (inbox/outbox/...)');
}

/**
 * Allow raisin:InboxTask children under raisin:MessageFolder.
 *
 * The workflow engine creates human tasks at {assignee}/inbox/task-* and a
 * registered user's inbox folder is a raisin:MessageFolder, whose builtin
 * allowed_children only lists Message/Conversation - completing a task then
 * fails parent-child validation on the node update. Patched upstream in
 * crates/raisin-core/global_nodetypes/raisin_message_folder.yaml; this
 * applies the same fix to repos created by older server builds (idempotent).
 */
async function ensureInboxTasksAllowed() {
  const base = `/api/management/${REPO}/main/nodetypes/raisin:MessageFolder`;
  const res = await api('GET', base);
  if (!res.ok) {
    console.log('ℹ️  raisin:MessageFolder node type not found - skipping inbox-task patch');
    return;
  }
  const nodeType = await res.json();
  const allowed = nodeType.allowed_children ?? [];
  if (allowed.includes('raisin:InboxTask')) {
    console.log('ℹ️  raisin:MessageFolder already allows raisin:InboxTask');
    return;
  }
  nodeType.allowed_children = [...allowed, 'raisin:InboxTask'];
  const updated = await api('PUT', base, { node_type: nodeType });
  if (!updated.ok) {
    throw new Error(
      `failed to allow raisin:InboxTask under raisin:MessageFolder: HTTP ${updated.status} ${await updated.text()}`,
    );
  }
  console.log('✅ raisin:MessageFolder now allows raisin:InboxTask children (workflow tasks)');
}

async function registerIdentityUser(email, password, displayName) {
  const res = await api('POST', `/auth/${REPO}/register`, {
    email,
    password,
    display_name: displayName,
  });
  if (res.ok) {
    console.log(`✅ Identity user ${email} registered`);
  } else {
    const text = await res.text();
    if (/exists|registered|conflict/i.test(text)) {
      console.log(`ℹ️  Identity user ${email} already exists`);
    } else {
      throw new Error(`register failed: HTTP ${res.status} ${text}`);
    }
  }
}

async function ensurePlannerUser() {
  await registerIdentityUser(PLANNER_EMAIL, PLANNER_PASSWORD, 'Planner');
}

async function ensureStaffUsers() {
  for (const staff of STAFF_USERS) {
    await registerIdentityUser(staff.email, STAFF_PASSWORD, staff.display_name);
  }
}

async function main() {
  console.log(`Shiftboard setup against ${BASE_URL} (repo: ${REPO})\n`);
  await loginAdmin();
  await ensureRepository();
  await ensureStaffingWorkspace();
  await seedStaffingNodes();
  await deployTools();
  await deployFlow();
  await ensureInboxTasksAllowed();
  await deployAgent();
  await deployCoordinatorAgent();
  await ensurePlannerUser();
  await ensureStaffUsers();
  console.log('\n🎉 Setup complete.');
  console.log(`   Agents: /agents/shift-planner + /agents/shift-coordinator (${AGENT_MODEL} via Groq)`);
  console.log(`   Login: ${PLANNER_EMAIL} / ${PLANNER_PASSWORD}`);
  console.log(`   Staff: ${STAFF_USERS.map((s) => s.email).join(', ')} / ${STAFF_PASSWORD}`);
  console.log('   Next: npm run smoke              (backend chat smoke test)');
  console.log('         npm run negotiation-test   (agent<->staff chat coordination test)');
  console.log('         npm run workflow-test      (durable fill-shift workflow + inbox tasks)');
}

main().catch((err) => {
  console.error('❌ Setup failed:', err.message);
  process.exit(1);
});
