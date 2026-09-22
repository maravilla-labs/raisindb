/**
 * `SKILL.md` → the `raisin:Skill` node definition the server installer builds.
 *
 * Mirrors crates/raisin-rocksdb/.../package_install/install_content/skill_md.rs:
 * a file named exactly `SKILL.md` is `---` YAML frontmatter plus a Markdown
 * body, and it defines the node of its DIRECTORY. Frontmatter `name`,
 * `description`, `license` and `metadata` become properties; the rest of the
 * file is `body`. `allowed-tools` is dropped on purpose — a skill is text and
 * must not widen an agent's tool grant — and any other key is ignored.
 *
 * Without this, `sync --push` treated SKILL.md as a plain binary asset, so a
 * skill that installs correctly could never be updated in place.
 */
import yaml from 'yaml';

export const SKILL_MD_FILENAME = 'SKILL.md';

const MAPPED = ['name', 'description', 'license', 'metadata'] as const;

/** The node YAML a SKILL.md stands for. Throws on a malformed file. */
export function skillMdToNodeYaml(text: string): string {
  const fm = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text);
  if (!fm) throw new Error('SKILL.md has no --- frontmatter');
  const meta = (yaml.parse(fm[1]) || {}) as Record<string, unknown>;
  if (typeof meta.name !== 'string' || !meta.name) throw new Error('SKILL.md frontmatter has no name');
  if (typeof meta.description !== 'string' || !meta.description) {
    throw new Error('SKILL.md frontmatter has no description');
  }
  const properties: Record<string, unknown> = {};
  for (const k of MAPPED) if (meta[k] !== undefined) properties[k] = meta[k];
  properties.body = text.slice(fm[0].length);
  return yaml.stringify({ node_type: 'raisin:Skill', properties });
}
