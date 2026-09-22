import { describe, it, expect } from 'vitest';
import yaml from 'yaml';
import { skillMdToNodeYaml } from './skill-md.js';
import { mapChangeToNode } from './mapping.js';

const SKILL = `---
name: pdf-forms
description: Fill PDF forms.
allowed-tools: [bash]
license: MIT
---
# Filling forms

Step one.
`;

describe('SKILL.md sync', () => {
  it('maps SKILL.md to its directory node, like the installer', () => {
    expect(mapChangeToNode('functions/skills/pdf-forms/SKILL.md')).toEqual({
      kind: 'node-yaml', workspace: 'functions', nodePath: 'skills/pdf-forms', skillMd: true,
    });
  });

  it('a SKILL.md directly under a workspace is not a skill (no directory to name it)', () => {
    expect(mapChangeToNode('functions/SKILL.md').skillMd).toBeUndefined();
  });

  it('builds the raisin:Skill node: frontmatter → properties, the rest → body, allowed-tools dropped', () => {
    const node = yaml.parse(skillMdToNodeYaml(SKILL));
    expect(node.node_type).toBe('raisin:Skill');
    expect(node.properties).toEqual({
      name: 'pdf-forms', description: 'Fill PDF forms.', license: 'MIT', body: '# Filling forms\n\nStep one.\n',
    });
  });

  it('refuses a file without frontmatter, name or description', () => {
    expect(() => skillMdToNodeYaml('# no frontmatter')).toThrow(/frontmatter/);
    expect(() => skillMdToNodeYaml('---\ndescription: x\n---\nbody')).toThrow(/name/);
    expect(() => skillMdToNodeYaml('---\nname: x\n---\nbody')).toThrow(/description/);
  });
});
