import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { createSkill, skillDir } from './create-skill.js';
import { skillMdToNodeYaml } from '../sync/skill-md.js';

let pkg: string;
beforeEach(() => {
  pkg = fs.mkdtempSync(path.join(os.tmpdir(), 'skill-'));
  fs.writeFileSync(path.join(pkg, 'manifest.yaml'), 'name: t\nversion: 0.1.0\n');
});
afterEach(() => fs.rmSync(pkg, { recursive: true, force: true }));

describe('raisindb create skill', () => {
  it('puts each scope where the runtime looks for it', () => {
    expect(skillDir('package', 'pdf-forms')).toBe('skills/pdf-forms');
    expect(skillDir('local', 'pdf-forms')).toBe('local/skills/pdf-forms');
    expect(skillDir('agent', 'pdf-forms', '/lib/acme/skills/')).toBe('lib/acme/skills/pdf-forms');
    expect(() => skillDir('agent', 'x')).toThrow(/--path/);
    expect(() => skillDir('agent', 'x', 'skills')).toThrow(/GLOBAL/);
  });

  it('writes a SKILL.md that sync can turn into a raisin:Skill node', async () => {
    const file = await createSkill('pdf-forms', { dir: pkg, scope: 'package', description: 'Fill PDF forms.' });
    expect(file).toBe(path.join(pkg, 'content/functions/skills/pdf-forms/SKILL.md'));
    expect(skillMdToNodeYaml(fs.readFileSync(file, 'utf8'))).toMatch(/name: pdf-forms/);
  });

  it('refuses a bad name and an existing definition', async () => {
    await expect(createSkill('PDF_Forms', { dir: pkg, scope: 'package' })).rejects.toThrow(/not a valid skill name/);
    await createSkill('pdf-forms', { dir: pkg, scope: 'package', description: 'x' });
    await expect(createSkill('pdf-forms', { dir: pkg, scope: 'package' })).rejects.toThrow(/already exists/);
  });
});
