#!/usr/bin/env node
/**
 * TaskPilot demo - setup script (idempotent, re-runnable).
 *
 * NOTE: The recommended way to install taskpilot is the PACKAGE flow via the
 * raisindb CLI (works for remote instances and CI):
 *
 *   raisindb login --server $RAISINDB_SERVER --username admin --password ...
 *   raisindb package deploy ./package --repo <repo> --install
 *
 * This script is the no-CLI fallback (plain HTTP, dev server). It shares the
 * package content as the single source of truth: the function sources are
 * read from package/content/functions/lib/taskpilot/<name>/index.js.
 *
 * Creates on a running raisin-server (dev mode):
 *   1. Repository `taskpilot` (builtin packages auto-install: messaging
 *      pipeline, ai-tools incl. the agent-handler + the planning tools
 *      create-plan / add-task / update-task / get-plan-status)
 *   2. Workspace `projects` with the /checklist launch items (raisin:Node)
 *   3. Tool functions under /lib/taskpilot/ in the functions workspace:
 *        list-items, update-item, radio-check
 *   4. AI agent /agents/pilot (Groq llama-3.3-70b-versatile, max_tokens
 *      1024, task_creation_enabled, execution_mode approve_then_auto)
 *   5. The agent's home folder in the `ai` workspace (inbox/outbox/...)
 *   6. Identity user: pilot@example.com / Pilot12345!
 *
 * Flags:
 *   --mode <automatic|approve_then_auto|step_by_step|manual>
 *       Set the agent's execution_mode (default approve_then_auto). Re-run
 *       with a different mode to switch the demo behaviour.
 *   --reset-items
 *       Reset every checklist item back to status todo / empty notes.
 *
 * Prereqs:
 *   - raisin-server running on RAISIN_URL (default http://localhost:8081)
 *   - Groq provider configured for the tenant (admin console -> AI settings)
 *
 * Run: npm install && npm run setup
 */

import { readFile } from 'node:fs/promises';

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const REPO = process.env.RAISIN_REPO ?? 'taskpilot';
const ADMIN_USER = process.env.RAISIN_USER ?? 'admin';
const ADMIN_PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';
const TENANT = process.env.RAISIN_TENANT ?? 'default';
const AGENT_MODEL = process.env.TASKPILOT_MODEL ?? 'llama-3.3-70b-versatile';

const PILOT_EMAIL = 'pilot@example.com';
const PILOT_PASSWORD = 'Pilot12345!';

const VALID_MODES = ['automatic', 'approve_then_auto', 'step_by_step', 'manual'];
const ARGS = process.argv.slice(2);
const MODE = (() => {
  const i = ARGS.indexOf('--mode');
  if (i < 0) return 'approve_then_auto';
  const mode = ARGS[i + 1];
  if (!VALID_MODES.includes(mode)) {
    console.error(`--mode must be one of: ${VALID_MODES.join(', ')}`);
    process.exit(1);
  }
  return mode;
})();
const RESET_ITEMS = ARGS.includes('--reset-items');

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
  // creation - wait for the agent-handler + planning tools they provide.
  for (let i = 0; i < 60; i++) {
    const handler = await api(
      'GET',
      `/api/repository/${REPO}/main/head/functions/lib/raisin/ai/agent-handler`,
    );
    const planner = await api(
      'GET',
      `/api/repository/${REPO}/main/head/functions/lib/raisin/ai/create-plan`,
    );
    if (handler.ok && planner.ok) {
      console.log('✅ Builtin packages installed (agent-handler + create-plan present)');
      return;
    }
    await sleep(1000);
  }
  throw new Error('builtin packages did not install (agent-handler/create-plan missing after 60s)');
}

async function ensureProjectsWorkspace() {
  const res = await api('PUT', `/api/workspaces/${REPO}/projects`, {
    name: 'projects',
    description: 'Project checklists driven by the pilot planning agent',
    allowed_node_types: ['raisin:Folder', 'raisin:Node'],
    allowed_root_node_types: ['raisin:Folder'],
  });
  if (!res.ok) throw new Error(`workspace create failed: HTTP ${res.status} ${await res.text()}`);
  console.log('✅ Workspace projects ready');
}

// Keep in sync with package/content/projects/checklist/*.yaml
const ITEMS = [
  { name: 'draft-copy', title: 'Draft landing copy', owner: 'Mara', order: 1 },
  { name: 'design-hero', title: 'Design hero section', owner: 'Iker', order: 2 },
  { name: 'setup-analytics', title: 'Set up analytics', owner: 'Noa', order: 3 },
  { name: 'qa-pass', title: 'Run QA pass', owner: 'Mara', order: 4 },
  { name: 'announce-launch', title: 'Announce launch', owner: 'Noa', order: 5 },
];

async function seedChecklist() {
  await ensureNode('projects', '/', 'checklist', 'raisin:Folder', { title: 'Launch Checklist' });
  for (const item of ITEMS) {
    const { name, ...props } = item;
    if (RESET_ITEMS) {
      await ensureNode('projects', '/checklist', name, 'raisin:Node', {
        ...props,
        status: 'todo',
        notes: '',
      });
      continue;
    }
    // Don't clobber status/notes on re-runs: only create when missing.
    const existing = await api(
      'GET',
      `/api/repository/${REPO}/main/head/projects/checklist/${name}`,
    );
    if (existing.ok) continue;
    await ensureNode('projects', '/checklist', name, 'raisin:Node', {
      ...props,
      status: 'todo',
      notes: '',
    });
  }
  console.log(
    `✅ Seeded ${ITEMS.length} checklist items${RESET_ITEMS ? ' (reset to todo)' : ''}`,
  );
}

async function deployFunction(name) {
  const dir = new URL(`./package/content/functions/lib/taskpilot/${name}/`, import.meta.url);
  // Single source of truth for the code: the package layout. The schemas/
  // descriptions below mirror package/content/functions/.../.node.yaml
  // (kept inline to avoid a YAML dependency in this fallback script).
  const code = await readFile(new URL('index.js', dir), 'utf8');

  const SCHEMAS = {
    'list-items': {
      title: 'List Checklist Items',
      description:
        'List the items on the launch checklist with path, title, status (todo/done), owner, order and notes. Optionally filter by status.',
      input_schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          status: { type: 'string', enum: ['todo', 'done'], description: 'Only items with this status' },
        },
      },
      output_schema: {
        type: 'object',
        properties: {
          items: { type: 'array', description: 'Matching items with path, title, status, owner, order, notes' },
        },
      },
    },
    'update-item': {
      title: 'Update Checklist Item',
      description:
        'Update a checklist item: set its status (todo/done) and/or replace its notes. Pass the item node path (e.g. /checklist/design-hero) plus the fields to change.',
      input_schema: {
        type: 'object',
        additionalProperties: false,
        required: ['item_path'],
        properties: {
          item_path: { type: 'string', description: 'Item node path, e.g. /checklist/design-hero' },
          status: { type: 'string', enum: ['todo', 'done'], description: 'New status for the item' },
          notes: { type: 'string', description: 'Replacement notes text for the item' },
        },
      },
      output_schema: {
        type: 'object',
        properties: {
          item_path: { type: 'string' },
          title: { type: 'string' },
          status: { type: 'string' },
          notes: { type: 'string' },
        },
      },
    },
    'radio-check': {
      title: 'Radio Check',
      description:
        'Read-only morale check for the crew: reports checklist completion as a flight-progress percentage plus a short aviation-style status call. Takes no input and never changes anything.',
      input_schema: { type: 'object', additionalProperties: false, properties: {} },
      output_schema: {
        type: 'object',
        properties: {
          callsign: { type: 'string' },
          completion_percent: { type: 'number' },
          done: { type: 'integer' },
          total: { type: 'integer' },
          message: { type: 'string' },
        },
      },
    },
  };
  const meta = SCHEMAS[name];

  await ensureNode('functions', '/lib', 'taskpilot', 'raisin:Folder', {});
  await ensureNode('functions', '/lib/taskpilot', name, 'raisin:Function', {
    name,
    title: meta.title,
    description: meta.description,
    enabled: true,
    language: 'javascript',
    execution_mode: 'async',
    entry_file: 'index.js:handler',
    input_schema: meta.input_schema,
    output_schema: meta.output_schema,
    version: 1,
  });
  await ensureNode('functions', `/lib/taskpilot/${name}`, 'index.js', 'raisin:Asset', {
    title: 'index.js',
    file: '',
    code,
  });
  console.log(`✅ Function deployed: /lib/taskpilot/${name}`);
}

async function deployAgent() {
  // Keep in sync with package/content/functions/agents/pilot/.node.yaml
  const systemPrompt = [
    'You are Pilot, the planning copilot for a small project launch',
    'checklist. Checklist items live under /checklist (e.g.',
    '/checklist/design-hero) and have a title, a status (todo or done),',
    'an owner, and notes.',
    '',
    "Your tools: list-items reads the checklist, update-item changes an",
    "item's status or notes, and radio-check gives a read-only progress",
    'report in flight lingo.',
    '',
    'When the user asks you to plan something or says "Plan and execute",',
    'you MUST call the create-plan tool with one task per step (2 to 4',
    'short, imperative task titles). Do not do the work before the plan',
    'exists.',
    '',
    'When executing plan tasks, always follow the Task Planning',
    'instructions: mark each task in_progress with update-task before',
    'working on it and completed when done (use the task_id values',
    'returned by create-plan). Use list-items / update-item / radio-check',
    "to actually do the work. To find an item's path, match its title",
    'against list-items output.',
    '',
    'If the request does not need a plan (a simple question or a single',
    'small action), answer directly without creating a plan.',
    '',
    'Keep every text response under two sentences, factual, no filler.',
  ].join('\n');

  await ensureNode('functions', '/', 'agents', 'raisin:Folder', {});
  await ensureNode('functions', '/agents', 'pilot', 'raisin:AIAgent', {
    title: 'Pilot',
    system_prompt: systemPrompt,
    provider: 'groq',
    model: AGENT_MODEL,
    temperature: 0.1,
    max_tokens: 1024,
    task_creation_enabled: true,
    thinking_enabled: false,
    execution_mode: MODE,
    execution_context: 'system',
    tools: [
      '/lib/raisin/ai/create-plan',
      '/lib/raisin/ai/add-task',
      '/lib/raisin/ai/update-task',
      '/lib/raisin/ai/get-plan-status',
      '/lib/taskpilot/list-items',
      '/lib/taskpilot/update-item',
      '/lib/taskpilot/radio-check',
    ],
    rules: ['Never invent checklist items - only work with paths returned by list-items.'],
  });
  console.log(`✅ Agent /agents/pilot deployed (groq ${AGENT_MODEL}, mode ${MODE})`);

  // Agent home in the `ai` workspace - the messaging pipeline delivers chat
  // messages into <agent>/inbox/chats/ and the agent-handler picks them up.
  await ensureNode('ai', '/agents', 'pilot', 'raisin:Folder', {
    title: 'Pilot',
    agent_ref: {
      'raisin:ref': '',
      'raisin:workspace': 'functions',
      'raisin:path': '/agents/pilot',
    },
    user_id: 'agent:pilot',
    display_name: 'Pilot',
    max_turns: 10,
  });
  for (const [parent, name, title] of [
    ['/agents/pilot', 'inbox', 'Inbox'],
    ['/agents/pilot', 'outbox', 'Outbox'],
    ['/agents/pilot', 'memory', 'User Memory'],
    ['/agents/pilot', 'sent', 'Sent'],
    ['/agents/pilot/inbox', 'chats', 'Chats'],
    ['/agents/pilot/inbox', 'notifications', 'Notifications'],
  ]) {
    await ensureNode('ai', parent, name, 'raisin:Folder', { title });
  }
  console.log('✅ Agent home created in ai workspace (inbox/outbox/...)');
}

async function ensurePilotUser() {
  const res = await api('POST', `/auth/${REPO}/register`, {
    email: PILOT_EMAIL,
    password: PILOT_PASSWORD,
    display_name: 'Pilot User',
  });
  if (res.ok) {
    console.log(`✅ Identity user ${PILOT_EMAIL} registered`);
  } else {
    const text = await res.text();
    if (/exists|registered|conflict/i.test(text)) {
      console.log(`ℹ️  Identity user ${PILOT_EMAIL} already exists`);
    } else {
      throw new Error(`register failed: HTTP ${res.status} ${text}`);
    }
  }
}

async function main() {
  console.log(`TaskPilot setup against ${BASE_URL} (repo: ${REPO}, mode: ${MODE})\n`);
  await loginAdmin();
  await ensureRepository();
  await ensureProjectsWorkspace();
  await seedChecklist();
  await deployFunction('list-items');
  await deployFunction('update-item');
  await deployFunction('radio-check');
  await deployAgent();
  await ensurePilotUser();
  console.log('\n🎉 Setup complete.');
  console.log(`   Agent: /agents/pilot (${AGENT_MODEL} via Groq, ${MODE})`);
  console.log(`   Login: ${PILOT_EMAIL} / ${PILOT_PASSWORD}`);
  console.log('   Next:  npm run dev    (app on http://localhost:5177)');
  console.log('          npm run check  (headless proof, costs one Groq run)');
}

main().catch((err) => {
  console.error('❌ Setup failed:', err.message);
  process.exit(1);
});
