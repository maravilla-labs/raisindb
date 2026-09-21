// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Agent skills (`raisin:Skill`) for the workflow path.
//!
//! This is the Rust half of ONE rule with two implementations. The other half
//! is `builtin-packages/ai-tools/content/functions/lib/raisin/ai/agent-shared/
//! skills.js`, which the chat loop (agent-handler, agent-continue-handler) and
//! the `load-skill` tool call. The parity test at the bottom of this file
//! executes that JS in QuickJS and asserts both produce identical JSON for
//! every case, so the two cannot drift the way the agent's `rules` did (chat
//! applied them, workflows silently dropped them).
//!
//! A skill follows the Agent Skills / SKILL.md shape: a `name`, a one-sentence
//! `description`, and a Markdown `body`. The prompt carries only a capped
//! INDEX — one line per skill — and the body is loaded on demand through the
//! builtin `load-skill` tool, which refuses a skill the caller was not given.
//!
//! The rule is pure: [`select_skills`] decides the GRANT from what was read,
//! [`skill_index_text`] renders the index, [`compose_instruction_tail`] the
//! exact text appended to the system prompt. Only the loader in
//! `ai_callback.rs` does IO, and it decides nothing.
//!
//! Where a grant comes from, in order (the first occurrence of a name wins):
//!
//! 1. the agent's `skills:` references, in declared order;
//! 2. a workflow step's own `skills:`, added for that step only;
//! 3. installation globals — children of `functions:/local/skills`, by name;
//! 4. package globals — children of `functions:/skills`, by name.
//!
//! A local skill REPLACES a package one of the same name; a local skill with
//! `enabled: false` MASKS it. An agent opts out of globals with
//! `global_skills: false`. A disabled or unusable explicit reference is simply
//! absent and masks nothing.
//!
//! Absent means absent: no skills and no rules produce an empty tail, and the
//! system prompt is untouched.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// The node type a usable skill must have.
pub const SKILL_NODE_TYPE: &str = "raisin:Skill";

/// Workspace holding the global skill folders and the `load-skill` function.
pub const SKILL_WORKSPACE: &str = "functions";

/// Package-shipped globals.
pub const PACKAGE_SKILLS_PATH: &str = "/skills";

/// The installation's own globals; a package must never ship `/local`.
pub const INSTALLATION_SKILLS_PATH: &str = "/local/skills";

/// The builtin tool that loads a skill's body.
pub const LOAD_SKILL_TOOL: &str = "load-skill";

/// The function node behind [`LOAD_SKILL_TOOL`], in [`SKILL_WORKSPACE`].
pub const LOAD_SKILL_PATH: &str = "/lib/raisin/ai/load-skill";

/// At most this many index lines.
pub const SKILL_INDEX_MAX_LINES: usize = 40;

/// At most this many characters in the whole index block, header included.
pub const SKILL_INDEX_MAX_CHARS: usize = 6000;

/// A description longer than this is cut to `MAX - 1` characters plus `…`.
pub const SKILL_DESCRIPTION_MAX_CHARS: usize = 200;

/// `load-skill` returns at most this many characters of a body.
pub const SKILL_BODY_MAX_CHARS: usize = 20000;

/// The Agent Skills limit on a name.
pub const SKILL_NAME_MAX_CHARS: usize = 64;

/// The index header, verbatim.
pub const SKILL_INDEX_HEADER: &str = "\n\n## Skills\nLoad a skill with the load-skill tool before doing work its description covers; its instructions then apply.";

/// The only characters that count as blank. Deliberately NOT
/// `char::is_whitespace` / `trim()`, whose Unicode set differs from JS.
const BLANK: [char; 4] = [' ', '\t', '\r', '\n'];

/// What the loader read at a place (only what the rule looks at).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SkillNode {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub node_type: String,
    /// `{name, description, enabled}`; anything else is ignored.
    #[serde(default)]
    pub properties: Value,
}

/// One explicit reference — the agent's or the step's — and what it resolved
/// to. `node: None` means the read found nothing (or failed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeclaredSkill {
    /// `"agent"` or `"step"`.
    pub source: String,
    pub workspace: String,
    pub path: String,
    #[serde(default)]
    pub node: Option<SkillNode>,
}

/// Everything [`select_skills`] decides from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SkillSelection {
    #[serde(default)]
    pub declared: Vec<DeclaredSkill>,
    /// Children of `functions:/local/skills`.
    #[serde(default)]
    pub installation: Vec<SkillNode>,
    /// Children of `functions:/skills`.
    #[serde(default)]
    pub pkg: Vec<SkillNode>,
    #[serde(default = "default_true", rename = "globalsEnabled")]
    pub globals_enabled: bool,
}

fn default_true() -> bool {
    true
}

/// A granted skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedSkill {
    pub name: String,
    pub description: String,
    pub workspace: String,
    pub path: String,
    /// `"agent"`, `"step"`, `"installation"` or `"package"`.
    pub source: String,
}

/// The Agent Skills name grammar: `^[a-z0-9]+(-[a-z0-9]+)*$`, 64 at most.
pub fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= SKILL_NAME_MAX_CHARS
        && name.split('-').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

fn prop_str<'a>(node: &'a SkillNode, key: &str) -> Option<&'a str> {
    node.properties.get(key).and_then(Value::as_str)
}

fn is_disabled(node: &SkillNode) -> bool {
    node.properties.get("enabled") == Some(&Value::Bool(false))
}

/// A skill node with a valid name, regardless of whether it is enabled.
fn named_skill(node: &SkillNode) -> Option<&str> {
    if node.node_type != SKILL_NODE_TYPE {
        return None;
    }
    prop_str(node, "name").filter(|n| is_valid_skill_name(n))
}

/// Collapse each run of ASCII whitespace to one space, then trim it.
fn collapse_blank(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if BLANK.contains(&ch) {
            pending_space = true;
            continue;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        out.push(ch);
    }
    out
}

/// `(name, description)` of a usable skill: the right type, a valid name,
/// not disabled, and a description that is not blank.
fn usable(node: &SkillNode) -> Option<(String, String)> {
    let name = named_skill(node)?;
    if is_disabled(node) {
        return None;
    }
    let description = prop_str(node, "description")?;
    if collapse_blank(description).is_empty() {
        return None;
    }
    Some((name.to_string(), description.to_string()))
}

/// Decide the grant. See the module docs for the order and collision rule.
pub fn select_skills(selection: &SkillSelection) -> Vec<SelectedSkill> {
    let mut out: Vec<SelectedSkill> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |out: &mut Vec<SelectedSkill>, skill: SelectedSkill| {
        if seen.insert(skill.name.clone()) {
            out.push(skill);
        }
    };

    for declared in &selection.declared {
        let Some(node) = declared.node.as_ref() else {
            continue;
        };
        let Some((name, description)) = usable(node) else {
            continue;
        };
        push(
            &mut out,
            SelectedSkill {
                name,
                description,
                workspace: declared.workspace.clone(),
                path: declared.path.clone(),
                source: declared.source.clone(),
            },
        );
    }

    if !selection.globals_enabled {
        return out;
    }

    // A disabled installation skill with a valid name masks the package one.
    let masked: HashSet<&str> = selection
        .installation
        .iter()
        .filter(|n| is_disabled(n))
        .filter_map(named_skill)
        .collect();

    let by_name = |nodes: &[SkillNode]| -> Vec<(String, String, String)> {
        let mut usable_nodes: Vec<(String, String, String)> = nodes
            .iter()
            .filter_map(|n| usable(n).map(|(name, d)| (name, d, n.path.clone())))
            .collect();
        usable_nodes.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));
        usable_nodes
    };

    for (name, description, path) in by_name(&selection.installation) {
        push(
            &mut out,
            SelectedSkill {
                name,
                description,
                workspace: SKILL_WORKSPACE.to_string(),
                path,
                source: "installation".to_string(),
            },
        );
    }
    for (name, description, path) in by_name(&selection.pkg) {
        if masked.contains(name.as_str()) {
            continue;
        }
        push(
            &mut out,
            SelectedSkill {
                name,
                description,
                workspace: SKILL_WORKSPACE.to_string(),
                path,
                source: "package".to_string(),
            },
        );
    }
    out
}

/// The description as the index shows it: blank runs collapsed, trimmed, and
/// cut to [`SKILL_DESCRIPTION_MAX_CHARS`] with `…` when longer.
pub fn index_description(description: &str) -> String {
    let collapsed = collapse_blank(description);
    if collapsed.chars().count() <= SKILL_DESCRIPTION_MAX_CHARS {
        return collapsed;
    }
    let mut head: String = collapsed
        .chars()
        .take(SKILL_DESCRIPTION_MAX_CHARS - 1)
        .collect();
    head.push('…');
    head
}

/// The index block without its truncation note, and how many skills it left
/// out.
fn index_parts(skills: &[SelectedSkill]) -> (String, usize) {
    if skills.is_empty() {
        return (String::new(), 0);
    }
    let mut out = SKILL_INDEX_HEADER.to_string();
    let mut chars = SKILL_INDEX_HEADER.chars().count();
    for (i, skill) in skills.iter().enumerate() {
        let line = format!(
            "\n- {} \u{2014} {}",
            skill.name,
            index_description(&skill.description)
        );
        let n = line.chars().count();
        if i >= SKILL_INDEX_MAX_LINES || chars + n > SKILL_INDEX_MAX_CHARS {
            return (out, skills.len() - i);
        }
        out.push_str(&line);
        chars += n;
    }
    (out, 0)
}

/// How many granted skills the index does not list.
pub fn skill_index_omitted(skills: &[SelectedSkill]) -> usize {
    index_parts(skills).1
}

/// The index text appended to the system prompt; empty for no skills.
pub fn skill_index_text(skills: &[SelectedSkill]) -> String {
    let (mut text, omitted) = index_parts(skills);
    if omitted > 0 {
        text.push_str(&format!(
            "\n[index truncated: {} more skills not listed]",
            omitted
        ));
    }
    text
}

/// The agent's `rules`, as the tail renders them: the string entries only.
pub fn rules_from_json(rules: &Value) -> Vec<String> {
    match rules {
        Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

/// The exact text appended to the system prompt: the skills index (when any
/// skill is granted), then the agent's `rules` in the chat loop's historical
/// `## Rules` form, which stays last. Empty when there is nothing to add.
pub fn compose_instruction_tail(rules: &[String], skills: &[SelectedSkill]) -> String {
    let mut out = skill_index_text(skills);
    if !rules.is_empty() {
        out.push_str("\n\n## Rules\n");
        out.push_str(
            &rules
                .iter()
                .map(|r| format!("- {}", r))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    out
}

/// Append the tail. An empty tail returns the input AS GIVEN — `None` stays
/// `None`, so an agent without a system prompt, skills or rules still sends
/// no system message at all.
pub fn apply_tail(system_prompt: Option<String>, tail: &str) -> Option<String> {
    if tail.is_empty() {
        return system_prompt;
    }
    match system_prompt {
        Some(sp) => Some(sp + tail),
        None => Some(tail.to_string()),
    }
}

/// The grant as the runtime hands it to `load-skill`: `[{name, workspace, path}]`.
pub fn grant_json(skills: &[SelectedSkill]) -> Value {
    Value::Array(
        skills
            .iter()
            .map(
                |s| serde_json::json!({ "name": s.name, "workspace": s.workspace, "path": s.path }),
            )
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn skill(name: &str, description: &str) -> Value {
        json!({
            "path": format!("/skills/{name}"),
            "node_type": SKILL_NODE_TYPE,
            "properties": { "name": name, "description": description },
        })
    }

    fn node(v: Value) -> SkillNode {
        serde_json::from_value(v).unwrap()
    }

    fn declared(source: &str, path: &str, n: Option<Value>) -> DeclaredSkill {
        DeclaredSkill {
            source: source.into(),
            workspace: "functions".into(),
            path: path.into(),
            node: n.map(node),
        }
    }

    // ── Absent means absent ────────────────────────────────────────────────

    #[test]
    fn nothing_composes_to_nothing() {
        assert_eq!(compose_instruction_tail(&[], &[]), "");
        assert!(select_skills(&SkillSelection {
            globals_enabled: true,
            ..Default::default()
        })
        .is_empty());
    }

    #[test]
    fn an_empty_tail_leaves_the_prompt_byte_for_byte() {
        assert_eq!(apply_tail(None, ""), None);
        assert_eq!(apply_tail(Some(String::new()), ""), Some(String::new()));
        assert_eq!(
            apply_tail(Some("abc\n".to_string()), ""),
            Some("abc\n".to_string())
        );
    }

    #[test]
    fn rules_only_is_the_chat_loops_rules_text() {
        let tail = compose_instruction_tail(&["a".to_string(), "b".to_string()], &[]);
        assert_eq!(tail, "\n\n## Rules\n- a\n- b");
        assert_eq!(
            apply_tail(Some("sp".to_string()), &tail),
            Some("sp\n\n## Rules\n- a\n- b".to_string())
        );
        assert_eq!(apply_tail(None, &tail), Some(tail.clone()));
    }

    // ── Selection ──────────────────────────────────────────────────────────

    #[test]
    fn a_step_skill_is_added_to_the_agents() {
        let sel = SkillSelection {
            declared: vec![
                declared("agent", "/s/a", Some(skill("alpha", "A."))),
                declared("step", "/s/b", Some(skill("beta", "B."))),
            ],
            globals_enabled: true,
            ..Default::default()
        };
        let names: Vec<_> = select_skills(&sel)
            .into_iter()
            .map(|s| (s.name, s.source))
            .collect();
        assert_eq!(
            names,
            vec![
                ("alpha".to_string(), "agent".to_string()),
                ("beta".to_string(), "step".to_string())
            ]
        );
    }

    #[test]
    fn local_masks_and_replaces_package() {
        let mut off = skill("gone", "x");
        off["properties"]["enabled"] = json!(false);
        let sel = SkillSelection {
            installation: vec![node(skill("shared", "local")), node(off)],
            pkg: vec![
                node(skill("shared", "package")),
                node(skill("gone", "package")),
                node(skill("kept", "package")),
            ],
            globals_enabled: true,
            ..Default::default()
        };
        let got: Vec<_> = select_skills(&sel)
            .into_iter()
            .map(|s| (s.name, s.description))
            .collect();
        assert_eq!(
            got,
            vec![
                ("shared".to_string(), "local".to_string()),
                ("kept".to_string(), "package".to_string())
            ]
        );
    }

    #[test]
    fn names_follow_the_agent_skills_grammar() {
        assert!(is_valid_skill_name("pdf"));
        assert!(is_valid_skill_name("a-1-b"));
        assert!(!is_valid_skill_name("-a"));
        assert!(!is_valid_skill_name("a--b"));
        assert!(!is_valid_skill_name("A"));
        assert!(!is_valid_skill_name("a_b"));
        assert!(is_valid_skill_name(&"a".repeat(64)));
        assert!(!is_valid_skill_name(&"a".repeat(65)));
    }

    #[test]
    fn truncation_says_it_truncated() {
        let skills: Vec<SelectedSkill> = (0..41)
            .map(|i| SelectedSkill {
                name: format!("s{i}"),
                description: "d".into(),
                workspace: "functions".into(),
                path: format!("/skills/s{i}"),
                source: "package".into(),
            })
            .collect();
        assert_eq!(skill_index_omitted(&skills), 1);
        assert!(
            skill_index_text(&skills).ends_with("\n[index truncated: 1 more skills not listed]")
        );
        assert_eq!(skill_index_omitted(&skills[..40]), 0);
    }

    // ── Parity with the JS resolver, executed in QuickJS ──────────────────

    const JS: &str = include_str!(
        "../../../../../builtin-packages/ai-tools/content/functions/lib/raisin/ai/agent-shared/skills.js"
    );

    /// A package global with a description of exactly `total` index-block
    /// characters when it is the only skill.
    fn filler(name: &str, description_chars: usize) -> Value {
        skill(name, &"d".repeat(description_chars))
    }

    /// `n` skills whose lines are short.
    fn many(n: usize) -> Vec<Value> {
        (0..n)
            .map(|i| skill(&format!("s{i:02}"), "Short."))
            .collect()
    }

    /// A list of package globals whose index block totals `total` characters
    /// (header included) — one long description, padded to hit it exactly.
    fn block_of(total: usize) -> Vec<Value> {
        let header = SKILL_INDEX_HEADER.chars().count();
        // "\n- " + name + " — " + description, per line; descriptions <= 200.
        let mut out = Vec::new();
        let mut used = header;
        let mut i = 0;
        while used < total {
            let name = format!("f{i:02}");
            let fixed = 3 + name.len() + 3;
            let room = total - used;
            let d = (room.saturating_sub(fixed)).min(SKILL_DESCRIPTION_MAX_CHARS);
            if d == 0 {
                break;
            }
            out.push(filler(&name, d));
            used += fixed + d;
            i += 1;
        }
        out
    }

    /// Every parity case: (name, selection, rules, system_prompt).
    fn cases() -> Vec<(String, Value, Value, Value)> {
        let mut off = skill("off", "Disabled.");
        off["properties"]["enabled"] = json!(false);
        let mut local_off = skill("shared", "Masked.");
        local_off["properties"]["enabled"] = json!(false);
        let mut wrong_type = skill("wrong", "Wrong type.");
        wrong_type["node_type"] = json!("raisin:Asset");
        let astral_199 = format!("{}\u{1F600}tail", "x".repeat(198));
        let astral_200 = format!("{}\u{1F600}tail", "x".repeat(199));
        let decl = |source: &str, v: Value| json!({ "source": source, "workspace": "functions", "path": v["path"].clone(), "node": v });
        let base = |declared: Vec<Value>, inst: Vec<Value>, pkg: Vec<Value>, globals: bool| json!({ "declared": declared, "installation": inst, "pkg": pkg, "globalsEnabled": globals });
        let empty = base(vec![], vec![], vec![], true);

        let mut out: Vec<(String, Value, Value, Value)> = vec![
            (
                "nothing at all".into(),
                empty.clone(),
                json!([]),
                json!("sp"),
            ),
            (
                "nothing, null prompt".into(),
                empty.clone(),
                json!([]),
                Value::Null,
            ),
            (
                "rules only".into(),
                empty.clone(),
                json!(["a", "b"]),
                json!("sp"),
            ),
            (
                "rules only, empty prompt".into(),
                empty.clone(),
                json!(["a"]),
                json!(""),
            ),
            (
                "non-string rules".into(),
                empty.clone(),
                json!(["a", 1, null, true, {"x": 1}, "b"]),
                json!("sp"),
            ),
            (
                "agent skills".into(),
                base(
                    vec![
                        decl("agent", skill("beta", "B does b.")),
                        decl("agent", skill("alpha", "A.")),
                    ],
                    vec![],
                    vec![],
                    true,
                ),
                json!(["r"]),
                json!("sp"),
            ),
            (
                "unresolved reference".into(),
                base(
                    vec![
                        json!({ "source": "agent", "workspace": "functions", "path": "/nope", "node": null }),
                    ],
                    vec![],
                    vec![],
                    true,
                ),
                json!([]),
                json!("sp"),
            ),
            (
                "globals only".into(),
                base(
                    vec![],
                    vec![skill("zed", "Z.")],
                    vec![skill("bee", "B."), skill("ay", "A.")],
                    true,
                ),
                json!([]),
                json!("sp"),
            ),
            (
                "local replacing package".into(),
                base(
                    vec![],
                    vec![skill("shared", "Local.")],
                    vec![skill("shared", "Package.")],
                    true,
                ),
                json!([]),
                json!("sp"),
            ),
            (
                "disabled local masking package".into(),
                base(
                    vec![],
                    vec![local_off.clone()],
                    vec![skill("shared", "Package."), skill("other", "O.")],
                    true,
                ),
                json!([]),
                json!("sp"),
            ),
            (
                "global_skills false".into(),
                base(
                    vec![decl("agent", skill("mine", "M."))],
                    vec![skill("loc", "L.")],
                    vec![skill("pk", "P.")],
                    false,
                ),
                json!([]),
                json!("sp"),
            ),
            (
                "a step skill added".into(),
                base(
                    vec![
                        decl("agent", skill("mine", "M.")),
                        decl("step", skill("step-only", "S.")),
                    ],
                    vec![],
                    vec![skill("pk", "P.")],
                    true,
                ),
                json!(["r"]),
                json!("sp"),
            ),
            (
                "a name collision".into(),
                base(
                    vec![
                        decl("agent", skill("dup", "Agent.")),
                        decl("step", skill("dup", "Step.")),
                    ],
                    vec![skill("dup", "Local.")],
                    vec![skill("dup", "Package.")],
                    true,
                ),
                json!([]),
                json!("sp"),
            ),
            (
                "disabled explicit masks nothing".into(),
                base(
                    vec![decl("agent", off.clone())],
                    vec![],
                    vec![skill("off", "Global.")],
                    true,
                ),
                json!([]),
                json!("sp"),
            ),
            (
                "an invalid name".into(),
                base(
                    vec![
                        decl("agent", skill("Bad_Name", "X.")),
                        decl("agent", skill("a--b", "X.")),
                    ],
                    vec![skill("-lead", "X.")],
                    vec![skill(&"a".repeat(65), "X."), skill(&"b".repeat(64), "Ok.")],
                    true,
                ),
                json!([]),
                json!("sp"),
            ),
            (
                "the wrong node_type".into(),
                base(
                    vec![decl("agent", wrong_type.clone())],
                    vec![],
                    vec![wrong_type.clone()],
                    true,
                ),
                json!([]),
                json!("sp"),
            ),
            (
                "blank descriptions".into(),
                base(
                    vec![decl("agent", skill("blank", " \t\r\n "))],
                    vec![skill("empty", "")],
                    vec![
                        json!({ "path": "/skills/none", "node_type": SKILL_NODE_TYPE, "properties": { "name": "none" } }),
                    ],
                    true,
                ),
                json!([]),
                json!("sp"),
            ),
            (
                "astral at 199".into(),
                base(vec![], vec![], vec![skill("astral", &astral_199)], true),
                json!([]),
                json!("sp"),
            ),
            (
                "astral at 200".into(),
                base(vec![], vec![], vec![skill("astral", &astral_200)], true),
                json!([]),
                json!("sp"),
            ),
            (
                "description exactly 200".into(),
                base(vec![], vec![], vec![skill("exact", &"e".repeat(200))], true),
                json!([]),
                json!("sp"),
            ),
            (
                "crlf and trailing whitespace".into(),
                base(
                    vec![],
                    vec![],
                    vec![skill("ws", "  line one\r\nline two \t\r\n\n  ")],
                    true,
                ),
                json!(["Grüße"]),
                json!("sp"),
            ),
        ];

        for n in [39usize, 40, 41] {
            out.push((
                format!("{n} lines"),
                base(vec![], vec![], many(n), true),
                json!([]),
                json!("sp"),
            ));
        }
        for total in [5999usize, 6000, 6001] {
            out.push((
                format!("{total} characters"),
                base(vec![], vec![], block_of(total), true),
                json!([]),
                json!("sp"),
            ));
        }
        out
    }

    fn rust_side(selection: &Value, rules: &Value, system_prompt: &Value) -> Value {
        let selection: SkillSelection = serde_json::from_value(selection.clone()).unwrap();
        let skills = select_skills(&selection);
        let index = skill_index_text(&skills);
        let tail = compose_instruction_tail(&rules_from_json(rules), &skills);
        let applied = apply_tail(system_prompt.as_str().map(String::from), &tail);
        json!({ "skills": skills, "index": index, "tail": tail, "applied": applied })
    }

    #[test]
    fn block_of_hits_its_target() {
        for total in [5999usize, 6000, 6001] {
            let sel = SkillSelection {
                pkg: block_of(total).into_iter().map(node).collect(),
                globals_enabled: true,
                ..Default::default()
            };
            let skills = select_skills(&sel);
            let mut all = SKILL_INDEX_HEADER.chars().count();
            for s in &skills {
                all += format!("\n- {} \u{2014} {}", s.name, s.description)
                    .chars()
                    .count();
            }
            assert_eq!(all, total, "fixture for {total}");
        }
    }

    /// The parity cases are only worth something if they reach the edges.
    #[test]
    fn parity_cases_exercise_the_edges() {
        let by_name: std::collections::HashMap<String, Value> = cases()
            .into_iter()
            .map(|(name, sel, rules, sp)| (name, rust_side(&sel, &rules, &sp)))
            .collect();
        let index = |name: &str| by_name[name]["index"].as_str().unwrap().to_string();
        let truncated = |name: &str| index(name).contains("[index truncated: ");
        assert_eq!(by_name["nothing at all"]["applied"], json!("sp"));
        assert_eq!(by_name["nothing, null prompt"]["applied"], Value::Null);
        assert!(!truncated("39 lines") && !truncated("40 lines"));
        assert!(index("41 lines").ends_with("[index truncated: 1 more skills not listed]"));
        assert!(!truncated("5999 characters") && !truncated("6000 characters"));
        assert!(truncated("6001 characters"));
        assert!(index("astral at 199").contains('\u{1F600}'));
        assert!(!index("astral at 200").contains('\u{1F600}'));
        assert!(index("astral at 200").contains('…'));
        assert_eq!(by_name["blank descriptions"]["skills"], json!([]));
        assert_eq!(
            by_name["global_skills false"]["skills"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    /// A JS value as JSON; `undefined` reads as `null`.
    fn js_to_json<'js>(ctx: &rquickjs::Ctx<'js>, v: rquickjs::Value<'js>) -> Value {
        match ctx.json_stringify(v).unwrap() {
            Some(s) => serde_json::from_str(&s.to_string().unwrap()).unwrap(),
            None => Value::Null,
        }
    }

    #[test]
    fn skills_parity_with_js() {
        use rquickjs::{Context, Function, Module, Runtime};

        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        let cases = cases();
        let mut failures = Vec::new();

        ctx.with(|ctx| {
            let module = Module::declare(ctx.clone(), "skills", JS)
                .expect("skills.js must parse as an ES module");
            let (module, promise) = module.eval().expect("skills.js must evaluate");
            promise
                .finish::<()>()
                .expect("skills.js top level must settle");
            let ns = module.namespace().unwrap();
            let select_fn: Function = ns.get("selectSkills").expect("export selectSkills");
            let index_fn: Function = ns.get("skillIndexText").expect("export skillIndexText");
            let compose_fn: Function = ns
                .get("composeInstructionTail")
                .expect("export composeInstructionTail");
            let append_fn: Function = ns.get("appendTail").expect("export appendTail");

            for (name, selection, rules, system_prompt) in &cases {
                let parse = |v: &Value| ctx.json_parse(v.to_string()).unwrap();
                let stringify = |v| js_to_json(&ctx, v);
                let skills: rquickjs::Value = select_fn
                    .call((parse(selection),))
                    .unwrap_or_else(|e| panic!("{name}: selectSkills threw: {e}"));
                let index: rquickjs::Value = index_fn
                    .call((skills.clone(),))
                    .unwrap_or_else(|e| panic!("{name}: skillIndexText threw: {e}"));
                let compose_arg = rquickjs::Object::new(ctx.clone()).unwrap();
                compose_arg.set("skills", skills.clone()).unwrap();
                compose_arg.set("rules", parse(rules)).unwrap();
                let tail: rquickjs::Value = compose_fn
                    .call((compose_arg,))
                    .unwrap_or_else(|e| panic!("{name}: composeInstructionTail threw: {e}"));
                let applied: rquickjs::Value = append_fn
                    .call((parse(system_prompt), tail.clone()))
                    .unwrap_or_else(|e| panic!("{name}: appendTail threw: {e}"));
                let js = json!({
                    "skills": stringify(skills),
                    "index": stringify(index),
                    "tail": stringify(tail),
                    "applied": stringify(applied),
                });
                let rust = rust_side(selection, rules, system_prompt);
                if js != rust {
                    failures.push(format!("case '{name}':\n  js:   {js}\n  rust: {rust}"));
                }
            }
        });

        assert!(
            failures.is_empty(),
            "JS and Rust skill resolvers diverge in {} of {} case(s):\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }
}
