/**
 * The skills rule: select (collision order, masking, opt-out, usability),
 * the index (description collapse and cut, both caps at their boundaries),
 * the tail, and the IO loader.
 *
 * The same rule runs in Rust for workflows (flow_callbacks_factory/skills.rs)
 * and a parity test there executes this module. These tests pin what the JS
 * says; the parity test pins that Rust says the same.
 */
import assert from 'node:assert/strict';
import test from 'node:test';

import {
  SKILL_NODE_TYPE,
  SKILL_INDEX_MAX_LINES,
  SKILL_INDEX_MAX_CHARS,
  SKILL_DESCRIPTION_MAX_CHARS,
  SKILL_BODY_MAX_CHARS,
  GLOBAL_SKILL_ROOTS,
  LOAD_SKILL_TOOL_NAME,
  LOAD_SKILL_FUNCTION,
  selectSkills,
  skillIndexText,
  skillIndexOmitted,
  skillIndexDescription,
  composeInstructionTail,
  appendTail,
  loadSkillGrant,
  grantEntries,
  skillBodyText,
  isValidSkillName,
} from '../content/functions/lib/raisin/ai/agent-shared/skills.js';

const HEADER = '\n\n## Skills\nLoad a skill with the load-skill tool before doing work its description covers; its instructions then apply.';
const len = (s) => Array.from(s).length;

function skill(name, description = `Does ${name}.`, extra = {}, path = `/lib-skills/${name}`) {
  return { path, node_type: 'raisin:Skill', properties: { name, description, ...extra } };
}
const declared = (node, source = 'agent') => ({ source, workspace: 'functions', path: node ? node.path : '/missing', node });
const local = (name, description, extra) => skill(name, description, extra, `/local/skills/${name}`);
const pkg = (name, description, extra) => skill(name, description, extra, `/skills/${name}`);
const names = (skills) => skills.map((s) => s.name);

test('contract constants', () => {
  assert.equal(SKILL_NODE_TYPE, 'raisin:Skill');
  assert.equal(SKILL_INDEX_MAX_LINES, 40);
  assert.equal(SKILL_INDEX_MAX_CHARS, 6000);
  assert.equal(SKILL_DESCRIPTION_MAX_CHARS, 200);
  assert.equal(SKILL_BODY_MAX_CHARS, 20000);
  assert.deepEqual(GLOBAL_SKILL_ROOTS, {
    installation: { workspace: 'functions', path: '/local/skills' },
    package: { workspace: 'functions', path: '/skills' },
  });
  assert.equal(LOAD_SKILL_TOOL_NAME, 'load-skill');
  assert.deepEqual(LOAD_SKILL_FUNCTION, { workspace: 'functions', path: '/lib/raisin/ai/load-skill' });
});

test('select: nothing at all → []', () => {
  assert.deepEqual(selectSkills({}), []);
  assert.deepEqual(selectSkills({ declared: [], installation: [], pkg: [], globalsEnabled: true }), []);
  assert.equal(skillIndexText([]), '');
});

test('select: agent, then step, then installation by name, then package by name', () => {
  const got = selectSkills({
    declared: [declared(skill('zeta')), declared(skill('alpha')), declared(skill('step-one'), 'step')],
    installation: [local('mike'), local('bravo')],
    pkg: [pkg('yankee'), pkg('charlie')],
    globalsEnabled: true,
  });
  assert.deepEqual(names(got), ['zeta', 'alpha', 'step-one', 'bravo', 'mike', 'charlie', 'yankee']);
  assert.deepEqual(got.map((s) => s.source), ['agent', 'agent', 'step', 'installation', 'installation', 'package', 'package']);
  assert.deepEqual(got[3], { name: 'bravo', description: 'Does bravo.', workspace: 'functions', path: '/local/skills/bravo', source: 'installation' });
});

test('select: the first occurrence of a name wins', () => {
  const got = selectSkills({
    declared: [declared(skill('shared', 'From the agent.')), declared(skill('shared', 'From the step.', {}, '/s2'), 'step')],
    installation: [local('shared', 'Local.')],
    pkg: [pkg('shared', 'Package.')],
    globalsEnabled: true,
  });
  assert.deepEqual(got.map((s) => [s.name, s.description, s.source]), [['shared', 'From the agent.', 'agent']]);
});

test('select: a local skill REPLACES the package skill of the same name', () => {
  const got = selectSkills({ installation: [local('notes', 'Local notes.')], pkg: [pkg('notes', 'Package notes.'), pkg('other')] });
  assert.deepEqual(got.map((s) => [s.name, s.description, s.path]), [
    ['notes', 'Local notes.', '/local/skills/notes'],
    ['other', 'Does other.', '/skills/other'],
  ]);
});

test('select: a disabled local skill with a valid name MASKS the package one', () => {
  const got = selectSkills({ installation: [local('notes', 'x', { enabled: false })], pkg: [pkg('notes'), pkg('other')] });
  assert.deepEqual(names(got), ['other']);
});

test('select: a disabled local skill does NOT mask an explicit reference', () => {
  const got = selectSkills({
    declared: [declared(skill('notes'))],
    installation: [local('notes', 'x', { enabled: false })],
    pkg: [pkg('notes')],
  });
  assert.deepEqual(got.map((s) => [s.name, s.source]), [['notes', 'agent']]);
});

test('select: a disabled or unusable EXPLICIT reference is absent and masks nothing', () => {
  const got = selectSkills({
    declared: [declared(skill('notes', 'x', { enabled: false })), declared(null), declared(skill('Bad Name'))],
    pkg: [pkg('notes', 'Package notes.')],
  });
  assert.deepEqual(got.map((s) => [s.name, s.source]), [['notes', 'package']]);
});

test('select: globalsEnabled false drops both global layers, never the explicit ones', () => {
  const got = selectSkills({
    declared: [declared(skill('mine')), declared(skill('step-skill'), 'step')],
    installation: [local('loc')],
    pkg: [pkg('pk')],
    globalsEnabled: false,
  });
  assert.deepEqual(names(got), ['mine', 'step-skill']);
});

test('select: enabled:true and absent enabled both count', () => {
  assert.deepEqual(names(selectSkills({ pkg: [pkg('a', 'x', { enabled: true }), pkg('b')] })), ['a', 'b']);
});

test('select: invalid names, the wrong node_type and blank descriptions are unusable', () => {
  for (const bad of ['', 'Upper', 'under_score', 'double--hyphen', '-lead', 'trail-', 'sp ace', 'ümlaut', 'a'.repeat(65), 42, null]) {
    assert.equal(isValidSkillName(bad), false, JSON.stringify(bad));
  }
  for (const good of ['a', 'a1', 'write-release-notes', '0-9', 'a'.repeat(64)]) {
    assert.equal(isValidSkillName(good), true, good);
  }
  const got = selectSkills({
    pkg: [
      pkg('ok'),
      { ...pkg('wrong-type'), node_type: 'raisin:Function' },
      pkg('blank', ''),
      pkg('spaces', ' \t\r\n '),
      pkg('nondesc', 42),
      { ...pkg('x'), properties: { ...pkg('x').properties, name: 'Not Valid' } },
      { path: '/skills/noprops', node_type: 'raisin:Skill' },
    ],
  });
  assert.deepEqual(names(got), ['ok']);
});

test('select: ties on name order by path, by code point', () => {
  const got = selectSkills({ pkg: [skill('dup', 'Second.', {}, '/skills/b'), skill('dup', 'First.', {}, '/skills/a')] });
  assert.deepEqual(got.map((s) => s.description), ['First.']);
});

test('index: one line per skill, em dash, header first', () => {
  const text = skillIndexText(selectSkills({ pkg: [pkg('alpha', 'Does alpha.'), pkg('beta', 'Does beta.')] }));
  assert.equal(text, HEADER + '\n- alpha — Does alpha.\n- beta — Does beta.');
});

test('index: descriptions collapse ASCII whitespace (CRLF, tabs, trailing) and keep the rest', () => {
  assert.equal(skillIndexDescription('  Line one\r\n\tline  two \n'), 'Line one line two');
  // A non-ASCII space is NOT whitespace to the rule: both runtimes keep it.
  assert.equal(skillIndexDescription('a b'), 'a b');
  assert.equal(skillIndexDescription('Grüße — ok'), 'Grüße — ok');
});

test('index: a description of 200 characters is whole; 201 is cut to 199 + ellipsis', () => {
  const d200 = 'x'.repeat(200);
  assert.equal(skillIndexDescription(d200), d200);
  const d201 = 'x'.repeat(201);
  assert.equal(skillIndexDescription(d201), 'x'.repeat(199) + '…');
  assert.equal(len(skillIndexDescription(d201)), 200);
});

test('index: an astral character at position 199/200 is never split', () => {
  const otter = '\u{1F9A6}';
  // 200 scalar values, the last one astral: kept whole.
  const whole = 'x'.repeat(199) + otter;
  assert.equal(skillIndexDescription(whole), whole);
  // 201 scalar values with the astral at index 198: it survives the cut intact.
  const cut = 'x'.repeat(198) + otter + 'yy';
  assert.equal(skillIndexDescription(cut), 'x'.repeat(198) + otter + '…');
  // 201 with the astral at index 199: it is the first thing cut.
  const cut2 = 'x'.repeat(199) + otter + 'y';
  assert.equal(skillIndexDescription(cut2), 'x'.repeat(199) + '…');
});

function nSkills(n) {
  return Array.from({ length: n }, (_, i) => ({
    name: `s${String(i).padStart(2, '0')}`,
    description: 'Short.',
    workspace: 'functions',
    path: `/skills/s${i}`,
    source: 'package',
  }));
}

test('cap: 39 and 40 lines are listed whole; 41 truncates and says so', () => {
  for (const n of [39, 40]) {
    const text = skillIndexText(nSkills(n));
    assert.equal(text.split('\n- ').length - 1, n);
    assert.ok(!text.includes('[index truncated'), `${n} lines`);
    assert.equal(skillIndexOmitted(nSkills(n)), 0);
  }
  const text = skillIndexText(nSkills(41));
  assert.equal(text.split('\n- ').length - 1, 40);
  assert.ok(text.endsWith('\n- s39 — Short.\n[index truncated: 1 more skills not listed]'));
  assert.equal(skillIndexOmitted(nSkills(41)), 1);
});

/* A set whose index, fully listed, is exactly `total` characters. */
function skillsTotalling(total) {
  const skills = Array.from({ length: 36 }, (_, i) => ({
    name: `s${String(i).padStart(2, '0')}`,
    description: 'd'.repeat(150),
    workspace: 'functions',
    path: `/skills/s${i}`,
    source: 'package',
  }));
  const used = len(skillIndexText(skills));
  const lastLine = total - used; // "\n- zz — " is 8 characters
  const descLen = lastLine - 8;
  assert.ok(descLen > 0 && descLen <= 200, `fixture: ${descLen}`);
  skills.push({ name: 'zz', description: 'e'.repeat(descLen), workspace: 'functions', path: '/skills/zz', source: 'package' });
  return skills;
}

test('cap: 5999 and 6000 characters are listed whole; 6001 drops the line that breaks it', () => {
  for (const total of [5999, 6000]) {
    const skills = skillsTotalling(total);
    const text = skillIndexText(skills);
    assert.equal(len(text), total);
    assert.ok(!text.includes('[index truncated'));
  }
  const skills = skillsTotalling(6001);
  const text = skillIndexText(skills);
  assert.ok(!text.includes('\n- zz '), 'the line that breaks the cap is left out');
  assert.ok(text.endsWith('\n[index truncated: 1 more skills not listed]'));
  assert.ok(len(text.slice(0, text.indexOf('\n[index truncated'))) <= SKILL_INDEX_MAX_CHARS);
  assert.equal(skillIndexOmitted(skills), 1);
});

test('cap: truncation stops at the first line that does not fit (no skipping ahead)', () => {
  const skills = skillsTotalling(6001);
  skills.push({ name: 'tiny', description: 'x', workspace: 'functions', path: '/skills/tiny', source: 'package' });
  const text = skillIndexText(skills);
  assert.ok(!text.includes('\n- tiny '));
  assert.ok(text.endsWith('[index truncated: 2 more skills not listed]'));
});

test('compose: nothing → empty tail; rules only is the historical block, non-strings dropped', () => {
  assert.equal(composeInstructionTail({}), '');
  assert.equal(composeInstructionTail({ skills: [], rules: [] }), '');
  assert.equal(composeInstructionTail({ rules: ['a', 'b c'] }), '\n\n## Rules\n- a\n- b c');
  assert.equal(composeInstructionTail({ rules: ['a', 7, null, 'b'] }), '\n\n## Rules\n- a\n- b');
  assert.equal(composeInstructionTail({ rules: 'nope' }), '');
});

test('compose: the skills index comes first, `## Rules` last', () => {
  const skills = selectSkills({ pkg: [pkg('alpha', 'Does alpha.')] });
  assert.equal(
    composeInstructionTail({ skills, rules: ['Be brief.'] }),
    HEADER + '\n- alpha — Does alpha.\n\n## Rules\n- Be brief.',
  );
});

test('appendTail: an empty tail is identity, undefined included', () => {
  assert.equal(appendTail(undefined, ''), undefined);
  assert.equal(appendTail('', ''), '');
  assert.equal(appendTail('X', ''), 'X');
  assert.equal(appendTail(undefined, '\n\nT'), '\n\nT');
  assert.equal(appendTail('', '\n\nT'), '\n\nT');
  assert.equal(appendTail('X', '\n\nT'), 'X\n\nT');
});

test('loadSkillGrant: reads references and both global folders; a throw counts as absent', async () => {
  const nodes = {
    'functions:/lib-skills/mine': skill('mine'),
    'functions:/lib-skills/by-id': skill('by-id'),
  };
  const children = {
    'functions:/local/skills': [local('loc')],
    'functions:/skills': [pkg('pk'), pkg('loc', 'shadowed')],
  };
  const errors = [];
  const got = await loadSkillGrant({
    agentProps: {
      skills: [
        { 'raisin:ref': 'uuid-1', 'raisin:path': '/lib-skills/mine', 'raisin:workspace': 'functions' },
        { 'raisin:ref': 'uuid-2', 'raisin:workspace': 'functions' },
        { 'raisin:ref': '/lib-skills/boom', 'raisin:workspace': 'functions' },
        'not-a-path',
      ],
    },
    stepSkills: [{ 'raisin:ref': '/lib-skills/mine', 'raisin:workspace': 'functions' }],
    getNode: async (ws, path) => {
      if (path === '/lib-skills/boom') throw new Error('denied');
      return nodes[`${ws}:${path}`] ?? null;
    },
    getNodeById: async (ws, id) => (id === 'uuid-2' ? nodes['functions:/lib-skills/by-id'] : null),
    getChildren: async (ws, path) => children[`${ws}:${path}`] ?? [],
    onReadError: (e) => errors.push(e.path),
  });
  assert.deepEqual(got.map((s) => [s.name, s.source, s.path]), [
    ['mine', 'agent', '/lib-skills/mine'],
    ['by-id', 'agent', '/lib-skills/by-id'],
    ['loc', 'installation', '/local/skills/loc'],
    ['pk', 'package', '/skills/pk'],
  ]);
  assert.deepEqual(errors, ['/lib-skills/boom']);
  assert.deepEqual(grantEntries(got)[0], { name: 'mine', workspace: 'functions', path: '/lib-skills/mine' });
});

test('loadSkillGrant: global_skills:false never lists the global folders', async () => {
  const listed = [];
  const got = await loadSkillGrant({
    agentProps: { global_skills: false },
    getNode: async () => null,
    getChildren: async (ws, path) => { listed.push(path); return [pkg('pk')]; },
  });
  assert.deepEqual(got, []);
  assert.deepEqual(listed, []);
});

test('loadSkillGrant: a step skill is added after the agent skills', async () => {
  const got = await loadSkillGrant({
    agentProps: { skills: [{ 'raisin:path': '/lib-skills/a', 'raisin:workspace': 'functions' }] },
    stepSkills: [{ 'raisin:path': '/lib-skills/b', 'raisin:workspace': 'functions' }],
    getNode: async (ws, path) => skill(path.split('/').pop()),
    getChildren: async () => [],
  });
  assert.deepEqual(got.map((s) => [s.name, s.source]), [['a', 'agent'], ['b', 'step']]);
});

test('skillBodyText: 20000 characters pass whole; 20001 is cut and says so', () => {
  const whole = 'b'.repeat(20000);
  assert.equal(skillBodyText(whole), whole);
  const over = '\u{1F9A6}'.repeat(20001);
  const text = skillBodyText(over);
  assert.ok(text.startsWith('\u{1F9A6}'.repeat(20000) + '\n[truncated: first 20000 of 20001 characters shown]'));
  assert.equal(skillBodyText(undefined), '');
});
