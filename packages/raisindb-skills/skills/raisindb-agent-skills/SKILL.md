---
name: raisindb-agent-skills
description: "Write agent skills for RaisinDB agents (raisin:AIAgent): a SKILL.md an agent loads on demand with load-skill. Covers where a skill lives and who gets it (package, installation-local, or one agent), scaffolding with `raisindb create skill`, granting it through an agent's `skills:`, pushing with deploy/sync, and the limits the runtime enforces. Use when an agent needs a procedure, checklist or domain know-how it should only read when relevant, or when a skill does not show up in an agent's index."
---

# Agent skills

A skill is text an agent reads **on demand**. The agent always sees an index —
one line per skill: its `name` and `description` — and pulls the full body with
the built-in `load-skill` tool only when the task needs it. That keeps long
procedures out of every prompt.

A skill is a `raisin:Skill` node. Write it as a `SKILL.md` (the Agent Skills
format) in its own folder; the installer and `raisindb sync --push` both turn it
into the node, named after the folder.

```markdown
---
name: pdf-forms
description: Fill PDF forms from a record. Load before generating any form PDF.
---

# Filling PDF forms

1. …
```

## Where it lives decides who gets it

| Scope | Package folder | Who gets it |
|-------|----------------|-------------|
| package | `content/functions/skills/<name>/SKILL.md` | every agent, shipped with the package |
| local | `content/functions/local/skills/<name>/SKILL.md` | every agent, this installation only (the operator's layer) |
| agent | `content/functions/<any folder>/<name>/SKILL.md` | only agents that list it in `skills:` |

Scaffold it in the right place:

```bash
raisindb create skill pdf-forms                     # asks which scope
raisindb create skill pdf-forms --scope agent --path lib/acme/skills
```

Grant an agent-only skill in the agent's `.node.yaml`:

```yaml
properties:
  skills:
    - raisin:ref: /lib/acme/skills/pdf-forms
      raisin:workspace: functions
```

An agent that must NOT see the global layers sets `global_skills: false`. A
workflow step can add skills for that step only with its own `skills:`.

## Rules the runtime enforces

- **Name:** lowercase letters and digits joined by single hyphens, at most 64
  characters, and equal to the folder name. An invalid name makes the skill
  silently unusable.
- **Description:** what it is for AND when to load it — it is the only thing
  the agent sees before loading. Keep it under 200 characters; the index line
  is cut there.
- **Body:** at most 20000 characters (what `load-skill` returns).
- `allowed-tools` in the frontmatter is dropped on purpose: a skill is text and
  cannot widen an agent's tools. Grant tools on the agent.
- One folder, one definition: a `SKILL.md` next to a `.node.yaml` (or a flat
  `<name>.yaml`) for the same node is refused at install.

## Pushing changes

`raisindb deploy <package> --install` installs new skills. To update an
existing one in place use `raisindb sync <package> --push` — whether a redeploy
replaces existing content depends on the package's `.raisin-sync.yaml`.

## When a skill does not show up

- It is agent-only and the agent's `skills:` does not reference it.
- It is global but the agent has `global_skills: false`.
- The name breaks the grammar above, or differs from the folder name.
- The index is full: it lists skills up to a size cap and says how many more
  exist — make descriptions shorter or scope skills to the agents that need them.
