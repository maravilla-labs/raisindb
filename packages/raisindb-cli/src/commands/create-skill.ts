/**
 * `raisindb create skill <name> [--scope package|local|agent] [--path <dir>]`
 *
 * Scaffolds an agent skill as a `SKILL.md` (Agent Skills format: `---` YAML
 * frontmatter + a Markdown body) inside a package. The installer and
 * `sync --push` both turn it into a `raisin:Skill` node named after its
 * directory. Where it goes decides WHO gets it — the runtime's three layers
 * (ai-tools agent-shared/skills.js):
 *
 *   package  content/functions/skills/<name>        every agent, shipped with the package
 *   local    content/functions/local/skills/<name>  every agent, the installation's own layer
 *   agent    content/functions/<path>/<name>        only agents that list it in `skills:`
 *
 * With no `--scope` on an interactive terminal it asks; otherwise it defaults
 * to `package`.
 */

import fs from 'fs';
import path from 'path';
import readline from 'readline/promises';
import { findPackageRoot } from '../wasm-fn/discover.js';

/** Mirrors the runtime rule: an invalid name makes the skill silently unusable. */
export const SKILL_NAME = /^[a-z0-9]+(-[a-z0-9]+)*$/;
export const SKILL_NAME_MAX = 64;

export type SkillScope = 'package' | 'local' | 'agent';

export interface CreateSkillOptions {
  scope?: string;
  /** Directory under content/functions for `--scope agent`, e.g. lib/acme/skills */
  path?: string;
  dir?: string;
  description?: string;
}

/** Where a skill of this scope lives, relative to content/functions. */
export function skillDir(scope: SkillScope, name: string, agentPath?: string): string {
  if (scope === 'package') return path.posix.join('skills', name);
  if (scope === 'local') return path.posix.join('local', 'skills', name);
  const base = String(agentPath || '').replace(/^\/+|\/+$/g, '');
  if (!base) throw new Error('--scope agent needs --path <dir under content/functions>, e.g. lib/acme/skills');
  if (base === 'skills' || base === 'local/skills') {
    throw new Error(`${base} is a GLOBAL root (every agent gets it) — use --scope ${base === 'skills' ? 'package' : 'local'} instead`);
  }
  return path.posix.join(base, name);
}

export function skillTemplate(name: string, description: string): string {
  return `---
name: ${name}
description: ${description}
---

# ${name
    .split('-')
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ')}

<!-- The description above is what an agent sees in its skill index (keep it
     under 200 characters and say WHEN to load the skill). This body arrives
     only when the agent calls load-skill; keep it under 20000 characters. -->

## When to use this

## The procedure

1.

## What refuses a mistake
`;
}

async function askScope(): Promise<SkillScope> {
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  try {
    console.log('Who should get this skill?');
    console.log('  1) package — every agent, shipped with this package   (content/functions/skills/<name>)');
    console.log('  2) local   — every agent, this installation only       (content/functions/local/skills/<name>)');
    console.log("  3) agent   — only agents that list it in their skills: (you choose the folder)");
    const answer = (await rl.question('Choose 1-3 [1]: ')).trim();
    return answer === '2' ? 'local' : answer === '3' ? 'agent' : 'package';
  } finally {
    rl.close();
  }
}

async function askPath(): Promise<string> {
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  try {
    return (await rl.question('Folder under content/functions (e.g. lib/acme/skills): ')).trim();
  } finally {
    rl.close();
  }
}

export async function createSkill(name: string, options: CreateSkillOptions = {}): Promise<string> {
  if (!SKILL_NAME.test(name) || name.length > SKILL_NAME_MAX) {
    throw new Error(`"${name}" is not a valid skill name: lowercase letters and digits joined by single hyphens, at most ${SKILL_NAME_MAX} characters`);
  }
  const root = findPackageRoot(options.dir || process.cwd());
  if (!root) throw new Error('no manifest.yaml found — run inside a package or pass --dir');

  const interactive = !!process.stdin.isTTY && !options.scope;
  let scope = (options.scope || (interactive ? await askScope() : 'package')) as SkillScope;
  if (!['package', 'local', 'agent'].includes(scope)) throw new Error(`--scope must be package, local or agent (got "${scope}")`);
  let agentPath = options.path;
  if (scope === 'agent' && !agentPath && interactive) agentPath = await askPath();

  const rel = skillDir(scope, name, agentPath);
  const dir = path.join(root, 'content', 'functions', ...rel.split('/'));
  for (const existing of ['SKILL.md', '.node.yaml']) {
    if (fs.existsSync(path.join(dir, existing))) throw new Error(`${path.join(dir, existing)} already exists`);
  }
  if (fs.existsSync(`${dir}.yaml`)) throw new Error(`${dir}.yaml already defines this node`);

  const description = options.description || `TODO: say what ${name} does and WHEN an agent should load it.`;
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, 'SKILL.md');
  fs.writeFileSync(file, skillTemplate(name, description));

  console.log(`Created ${path.relative(process.cwd(), file)}  (functions:/${rel})`);
  if (scope === 'agent') {
    console.log('\nGrant it to an agent by adding to the agent\'s .node.yaml:');
    console.log(`  skills:\n    - raisin:ref: /${rel}\n      raisin:workspace: functions`);
  } else {
    console.log(`Every agent is offered it${scope === 'local' ? ' on this installation' : ''}, unless the agent sets global_skills: false.`);
  }
  console.log('Push it: raisindb deploy <package> --install, or raisindb sync <package> --push');
  return file;
}
