// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! `SKILL.md` → `raisin:Skill`
//!
//! A package may ship an Agent Skills directory as-is:
//!
//! ```text
//! content/functions/skills/pdf-forms/SKILL.md
//! content/functions/skills/pdf-forms/references/fields.md
//! ```
//!
//! A file named exactly `SKILL.md` is `---` YAML frontmatter plus a Markdown
//! body. It becomes a `raisin:Skill` node named after its DIRECTORY
//! (`/skills/pdf-forms`), the same rule a `.node.yaml` in that directory would
//! follow — it is handed on under the synthetic path `<dir>/.node.yaml`, so
//! [`ContentNodeDef::derive_name`], the path derivation and the translation
//! lookup (`<dir>/.node.de.yaml`) all see an ordinary folder definition.
//! Sibling files are untouched here and install as the skill's children
//! through the existing asset path.
//!
//! Frontmatter mapping: `name`, `description`, `license`, `metadata`; the
//! rest of the file is `body`. The Agent Skills `allowed-tools` key is dropped
//! ON PURPOSE — a skill is text and must not be able to widen an agent's tool
//! grant. Any other unknown key is ignored and reported back to the caller.
//!
//! Refused, as a validation error and never as a silent `raisin:Asset`:
//! - a malformed file (no frontmatter, bad YAML, missing name/description);
//! - a frontmatter `name` that differs from the directory name, or that breaks
//!   the Agent Skills grammar (the runtime would treat the skill as unusable);
//! - a `SKILL.md` directly under `content/<ws>/` (it has no directory to name
//!   it after);
//! - a directory whose node is ALSO defined by a YAML file (`.node.yaml` in
//!   it, or a flat `<dir>.yaml` beside it) — two definitions of one node.
//!
//! Both the real collector (`zip_collector.rs`) and the dry-run simulator
//! (`dry_run/content_simulation.rs`) call into this module, so a `--check`
//! reports exactly what an install would do.

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

use crate::jobs::handlers::package_install::content_types::{derive_content_path, ContentNodeDef};

/// The exact file name that marks an Agent Skills directory.
pub(in crate::jobs::handlers::package_install) const SKILL_MD_FILENAME: &str = "SKILL.md";

/// The node type a `SKILL.md` becomes.
pub(in crate::jobs::handlers::package_install) const SKILL_NODE_TYPE: &str = "raisin:Skill";

/// Agent Skills name limit (mirrors `SKILL_NAME_MAX_CHARS` in the runtime).
const SKILL_NAME_MAX_CHARS: usize = 64;

/// Frontmatter keys that are dropped on purpose rather than ignored.
const DROPPED_KEYS: &[&str] = &["allowed-tools"];

/// Whether a content file is an Agent Skills definition. Exact, case-sensitive
/// match — `skill.md` or `README.md` stay ordinary assets.
pub(in crate::jobs::handlers::package_install) fn is_skill_md(filename: &str) -> bool {
    filename == SKILL_MD_FILENAME
}

/// A parsed `SKILL.md`.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::jobs::handlers::package_install) struct ParsedSkillMd {
    pub name: String,
    pub description: String,
    pub body: String,
    pub license: Option<String>,
    pub metadata: Option<HashMap<String, PropertyValue>>,
    /// Frontmatter keys that were not mapped: `allowed-tools` (on purpose)
    /// and anything unknown. For the caller to log.
    pub unmapped_keys: Vec<String>,
}

/// The Agent Skills name grammar: `^[a-z0-9]+(-[a-z0-9]+)*$`, 64 at most.
fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= SKILL_NAME_MAX_CHARS
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Free-form frontmatter YAML → a property value, LITERALLY.
///
/// Deliberately not the untagged `PropertyValue` deserializer a `.node.yaml`
/// goes through: that ladder turns the Agent Skills string `version: "1.0"`
/// into a `Decimal` and an object carrying `raisin:ref` into a reference. A
/// string here stays a string.
fn yaml_to_property(value: serde_yaml::Value) -> std::result::Result<PropertyValue, String> {
    Ok(match value {
        serde_yaml::Value::Null => PropertyValue::Null,
        serde_yaml::Value::Bool(b) => PropertyValue::Boolean(b),
        serde_yaml::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => PropertyValue::Integer(i),
            (None, Some(f)) => PropertyValue::Float(f),
            (None, None) => return Err(format!("holds an unrepresentable number {n}")),
        },
        serde_yaml::Value::String(s) => PropertyValue::String(s),
        serde_yaml::Value::Sequence(items) => PropertyValue::Array(
            items
                .into_iter()
                .map(yaml_to_property)
                .collect::<std::result::Result<_, _>>()?,
        ),
        serde_yaml::Value::Mapping(map) => {
            let mut out = HashMap::with_capacity(map.len());
            for (k, v) in map {
                let serde_yaml::Value::String(key) = k else {
                    return Err(format!("has a non-string key {k:?}"));
                };
                out.insert(key, yaml_to_property(v)?);
            }
            PropertyValue::Object(out)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_property(tagged.value)?,
    })
}

fn is_delimiter(line: &str) -> bool {
    line.trim_end() == "---"
}

fn required_string(map: &serde_yaml::Mapping, key: &str) -> std::result::Result<String, String> {
    match map.get(key) {
        None | Some(serde_yaml::Value::Null) => {
            Err(format!("frontmatter is missing the required '{key}'"))
        }
        Some(serde_yaml::Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        Some(serde_yaml::Value::String(_)) => Err(format!("frontmatter '{key}' is blank")),
        Some(_) => Err(format!("frontmatter '{key}' must be a string")),
    }
}

/// Parse a `SKILL.md` into frontmatter fields and body. Pure; the error is a
/// human-readable reason without the file name (the caller adds it).
pub(in crate::jobs::handlers::package_install) fn parse_skill_md(
    text: &str,
) -> std::result::Result<ParsedSkillMd, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    // split_inclusive keeps the line endings, so the body is the exact
    // remainder of the file.
    let mut lines = text.split_inclusive('\n');
    let mut consumed = match lines.next() {
        Some(first) if is_delimiter(first) => first.len(),
        _ => return Err("must begin with a '---' line opening the YAML frontmatter".to_string()),
    };

    let mut frontmatter = String::new();
    let mut closed = false;
    for line in lines {
        consumed += line.len();
        if is_delimiter(line) {
            closed = true;
            break;
        }
        frontmatter.push_str(line);
    }
    if !closed {
        return Err("frontmatter is not closed by a '---' line".to_string());
    }

    let yaml: serde_yaml::Value = serde_yaml::from_str(&frontmatter)
        .map_err(|e| format!("frontmatter is not valid YAML: {e}"))?;
    let map = match yaml {
        serde_yaml::Value::Mapping(map) => map,
        serde_yaml::Value::Null => {
            return Err("frontmatter is missing the required 'name'".to_string())
        }
        _ => return Err("frontmatter must be a YAML mapping".to_string()),
    };

    let name = required_string(&map, "name")?;
    let description = required_string(&map, "description")?;

    let license = match map.get("license") {
        None | Some(serde_yaml::Value::Null) => None,
        Some(serde_yaml::Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("frontmatter 'license' must be a string".to_string()),
    };

    let metadata = match map.get("metadata") {
        None | Some(serde_yaml::Value::Null) => None,
        Some(serde_yaml::Value::Mapping(m)) => match yaml_to_property(m.clone().into()) {
            Ok(PropertyValue::Object(parsed)) => Some(parsed),
            Ok(_) => unreachable!("a mapping converts to an object"),
            Err(e) => return Err(format!("frontmatter 'metadata' {e}")),
        },
        Some(_) => return Err("frontmatter 'metadata' must be a mapping".to_string()),
    };

    let mut unmapped_keys: Vec<String> = map
        .keys()
        .map(|k| match k {
            serde_yaml::Value::String(s) => s.clone(),
            other => format!("{other:?}"),
        })
        .filter(|k| !matches!(k.as_str(), "name" | "description" | "license" | "metadata"))
        .collect();
    unmapped_keys.sort();

    let body = text[consumed..]
        .trim_start_matches(['\r', '\n'])
        .trim_end()
        .to_string();

    Ok(ParsedSkillMd {
        name,
        description,
        body,
        license,
        metadata,
        unmapped_keys,
    })
}

/// Frontmatter keys that are dropped deliberately (as opposed to unknown).
pub(in crate::jobs::handlers::package_install) fn is_deliberately_dropped(key: &str) -> bool {
    DROPPED_KEYS.contains(&key)
}

/// A `SKILL.md` turned into the node definition an install would create.
#[derive(Debug, Clone)]
pub(in crate::jobs::handlers::package_install) struct SkillMdNode {
    /// `content/<ws>/<dir...>/.node.yaml` — the path the definition is filed
    /// under, so every path-based rule treats it as a folder definition.
    pub synthetic_yaml_path: String,
    /// Node name (the directory name).
    pub name: String,
    /// Workspace-relative node path.
    pub path: String,
    pub def: ContentNodeDef,
    pub unmapped_keys: Vec<String>,
}

/// Convert one `content/<ws>/<dir...>/SKILL.md` archive entry into its node
/// definition, or refuse it. `zip_name` is the full archive path; the refusal
/// names it and is meant for [`refuse_skill_md_errors`].
pub(in crate::jobs::handlers::package_install) fn skill_md_to_node(
    zip_name: &str,
    bytes: &[u8],
) -> std::result::Result<SkillMdNode, String> {
    let refuse = |reason: String| format!("Invalid {zip_name}: {reason}");

    let parts: Vec<&str> = zip_name.split('/').collect();
    // content / <ws> / <dir...> / SKILL.md — at least one directory.
    if parts.len() < 4 || parts.last() != Some(&SKILL_MD_FILENAME) {
        return Err(refuse(
            "a SKILL.md must sit in its own directory under content/<workspace>/, \
             which names the skill"
                .to_string(),
        ));
    }
    let dir_name = parts[parts.len() - 2];

    let text = std::str::from_utf8(bytes).map_err(|_| refuse("not valid UTF-8".to_string()))?;
    let parsed = parse_skill_md(text).map_err(refuse)?;

    if parsed.name != dir_name {
        return Err(refuse(format!(
            "frontmatter name '{}' differs from its directory '{}'; a skill is named \
             after its directory, so rename one to match",
            parsed.name, dir_name
        )));
    }
    if !is_valid_skill_name(&parsed.name) {
        return Err(refuse(format!(
            "skill name '{}' breaks the Agent Skills grammar (lowercase letters, digits \
             and single hyphens, at most {SKILL_NAME_MAX_CHARS} characters); the runtime \
             would treat it as unusable",
            parsed.name
        )));
    }

    let synthetic_yaml_path = format!("{}/.node.yaml", parts[..parts.len() - 1].join("/"));

    let mut properties: HashMap<String, PropertyValue> = HashMap::new();
    properties.insert("name".into(), PropertyValue::String(parsed.name.clone()));
    properties.insert(
        "description".into(),
        PropertyValue::String(parsed.description),
    );
    properties.insert("body".into(), PropertyValue::String(parsed.body));
    if let Some(license) = parsed.license {
        properties.insert("license".into(), PropertyValue::String(license));
    }
    if let Some(metadata) = parsed.metadata {
        properties.insert("metadata".into(), PropertyValue::Object(metadata));
    }

    let def = ContentNodeDef {
        id: None,
        node_type: SKILL_NODE_TYPE.to_string(),
        // Left to the path: `derive_name` gives the directory, which the check
        // above has already made equal to the frontmatter name.
        name: None,
        parent: None,
        archetype: None,
        properties: Some(properties),
    };

    let name = def.derive_name(&synthetic_yaml_path);
    let path = derive_content_path(&synthetic_yaml_path, &name);

    Ok(SkillMdNode {
        synthetic_yaml_path,
        name,
        path,
        def,
        unmapped_keys: parsed.unmapped_keys,
    })
}

/// Refuse every skill whose node is also defined by a YAML file.
///
/// `skills`: `(workspace, SKILL.md archive path, node path)`.
/// `yaml_defs`: `(workspace, YAML archive path, node path)` for every YAML
/// node definition in the package. Compared on the DERIVED node path, so a
/// `.node.yaml` inside the directory, a `.node.yml`, and a flat `<dir>.yaml`
/// beside it are all caught by the same rule.
pub(in crate::jobs::handlers::package_install) fn skill_md_clashes(
    skills: &[(String, String, String)],
    yaml_defs: &[(String, String, String)],
) -> Vec<String> {
    let mut errors = Vec::new();
    for (ws, skill_file, skill_path) in skills {
        for (yws, yaml_file, yaml_path) in yaml_defs {
            if yws == ws && yaml_path == skill_path {
                errors.push(format!(
                    "{skill_file} and {yaml_file} both define '{skill_path}' in workspace \
                     '{ws}'; a skill directory holds ONE definition — remove the YAML \
                     (SKILL.md becomes the raisin:Skill) or move the SKILL.md"
                ));
            }
        }
    }
    errors
}

/// Collapse collected `SKILL.md` refusals into the one error the install (and
/// the dry run) fails with, or `Ok` when there are none.
pub(in crate::jobs::handlers::package_install) fn refuse_skill_md_errors(
    errors: Vec<String>,
) -> Result<()> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(Error::Validation(
            errors.into_iter().next().unwrap_or_default(),
        )),
        n => Err(Error::Validation(format!(
            "{n} SKILL.md files were refused:\n{}",
            errors
                .iter()
                .map(|e| format!("  - {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "---\n\
name: pdf-forms\n\
description: Fill in PDF forms. Use when the user hands over a form.\n\
license: Apache-2.0\n\
allowed-tools: Bash(python:*) Read\n\
metadata:\n  author: acme\n  version: \"1.0\"\n\
---\n\
\n\
# PDF forms\n\
\n\
Read `references/fields.md` first.\n";

    fn prop<'a>(node: &'a SkillMdNode, key: &str) -> Option<&'a PropertyValue> {
        node.def.properties.as_ref().unwrap().get(key)
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let p = parse_skill_md(GOOD).unwrap();
        assert_eq!(p.name, "pdf-forms");
        assert_eq!(
            p.description,
            "Fill in PDF forms. Use when the user hands over a form."
        );
        assert_eq!(p.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(p.body, "# PDF forms\n\nRead `references/fields.md` first.");
        let md = p.metadata.unwrap();
        assert_eq!(
            md.get("author"),
            Some(&PropertyValue::String("acme".into()))
        );
        assert_eq!(
            md.get("version"),
            Some(&PropertyValue::String("1.0".into()))
        );
        assert_eq!(p.unmapped_keys, vec!["allowed-tools".to_string()]);
    }

    #[test]
    fn metadata_is_taken_literally() {
        let p = parse_skill_md(
            "---\nname: a\ndescription: b\nmetadata:\n  version: \"05\"\n  n: 3\n  \
             ok: true\n  tags: [x, y]\n  ref: {\"raisin:ref\": \"/x\"}\n---\n",
        )
        .unwrap();
        let md = p.metadata.unwrap();
        assert_eq!(md["version"], PropertyValue::String("05".into()));
        assert_eq!(md["n"], PropertyValue::Integer(3));
        assert_eq!(md["ok"], PropertyValue::Boolean(true));
        assert_eq!(
            md["tags"],
            PropertyValue::Array(vec![
                PropertyValue::String("x".into()),
                PropertyValue::String("y".into())
            ])
        );
        assert!(matches!(&md["ref"], PropertyValue::Object(o) if o.len() == 1));
        let err =
            parse_skill_md("---\nname: a\ndescription: b\nmetadata:\n  1: x\n---\n").unwrap_err();
        assert!(err.contains("non-string key"), "{err}");
    }

    #[test]
    fn crlf_and_bom_are_accepted() {
        let text = "\u{feff}---\r\nname: a\r\ndescription: B.\r\n---\r\nBody\r\n";
        let p = parse_skill_md(text).unwrap();
        assert_eq!(p.name, "a");
        assert_eq!(p.body, "Body");
    }

    #[test]
    fn empty_body_is_allowed() {
        let p = parse_skill_md("---\nname: a\ndescription: B.\n---\n").unwrap();
        assert_eq!(p.body, "");
        let p = parse_skill_md("---\nname: a\ndescription: B.\n---").unwrap();
        assert_eq!(p.body, "");
    }

    #[test]
    fn a_later_rule_line_stays_in_the_body() {
        let p = parse_skill_md("---\nname: a\ndescription: B.\n---\nx\n\n---\n\ny\n").unwrap();
        assert_eq!(p.body, "x\n\n---\n\ny");
    }

    #[test]
    fn malformed_files_are_refused() {
        for (text, needle) in [
            ("# no frontmatter\n", "must begin"),
            ("", "must begin"),
            ("---\nname: a\ndescription: b\n", "not closed"),
            ("---\n---\nbody", "missing the required 'name'"),
            ("---\nname: a\n---\n", "missing the required 'description'"),
            (
                "---\nname: a\ndescription: '  '\n---\n",
                "'description' is blank",
            ),
            (
                "---\nname: 7\ndescription: b\n---\n",
                "'name' must be a string",
            ),
            ("---\n- a\n---\n", "must be a YAML mapping"),
            ("---\nname: [a\n---\n", "not valid YAML"),
            (
                "---\nname: a\ndescription: b\nmetadata: x\n---\n",
                "'metadata' must be a mapping",
            ),
            (
                "---\nname: a\ndescription: b\nlicense: [x]\n---\n",
                "'license' must be a string",
            ),
        ] {
            let err = parse_skill_md(text).expect_err(text);
            assert!(err.contains(needle), "{text:?} → {err}");
        }
    }

    #[test]
    fn node_is_named_after_its_directory() {
        let node = skill_md_to_node(
            "content/functions/skills/pdf-forms/SKILL.md",
            GOOD.as_bytes(),
        )
        .unwrap();
        assert_eq!(
            node.synthetic_yaml_path,
            "content/functions/skills/pdf-forms/.node.yaml"
        );
        assert_eq!(node.name, "pdf-forms");
        assert_eq!(node.path, "/skills/pdf-forms");
        assert_eq!(node.def.node_type, "raisin:Skill");
        assert_eq!(node.def.name, None);
        // `properties.name` equals the path-derived name, so no legacy twin.
        assert_eq!(
            node.def.legacy_property_name(&node.synthetic_yaml_path),
            None
        );
        assert_eq!(
            prop(&node, "name"),
            Some(&PropertyValue::String("pdf-forms".into()))
        );
        assert!(
            matches!(prop(&node, "body"), Some(PropertyValue::String(b)) if b.starts_with("# PDF forms"))
        );
        assert!(matches!(
            prop(&node, "metadata"),
            Some(PropertyValue::Object(_))
        ));
        assert_eq!(
            prop(&node, "license"),
            Some(&PropertyValue::String("Apache-2.0".into()))
        );
    }

    #[test]
    fn allowed_tools_never_reaches_the_node() {
        let node = skill_md_to_node(
            "content/functions/skills/pdf-forms/SKILL.md",
            GOOD.as_bytes(),
        )
        .unwrap();
        let props = node.def.properties.as_ref().unwrap();
        assert!(!props.contains_key("allowed-tools"));
        assert!(!props.contains_key("allowed_tools"));
        let mut keys: Vec<&str> = props.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["body", "description", "license", "metadata", "name"]);
        assert!(is_deliberately_dropped("allowed-tools"));
    }

    #[test]
    fn optional_fields_are_omitted_when_absent() {
        let node = skill_md_to_node(
            "content/functions/skills/a/SKILL.md",
            b"---\nname: a\ndescription: B.\ncompatibility: any\n---\nText\n",
        )
        .unwrap();
        let props = node.def.properties.as_ref().unwrap();
        assert!(!props.contains_key("license"));
        assert!(!props.contains_key("metadata"));
        assert_eq!(node.unmapped_keys, vec!["compatibility".to_string()]);
    }

    #[test]
    fn deep_and_shallow_directories() {
        let deep = skill_md_to_node(
            "content/functions/local/skills/a/SKILL.md",
            b"---\nname: a\ndescription: B.\n---\n",
        )
        .unwrap();
        assert_eq!(deep.path, "/local/skills/a");
        let top = skill_md_to_node(
            "content/functions/a/SKILL.md",
            b"---\nname: a\ndescription: B.\n---\n",
        )
        .unwrap();
        assert_eq!(top.path, "/a");
    }

    #[test]
    fn name_differing_from_directory_is_refused() {
        let err =
            skill_md_to_node("content/functions/skills/pdf/SKILL.md", GOOD.as_bytes()).unwrap_err();
        assert!(
            err.contains("content/functions/skills/pdf/SKILL.md"),
            "{err}"
        );
        assert!(
            err.contains("'pdf-forms' differs from its directory 'pdf'"),
            "{err}"
        );
    }

    #[test]
    fn name_breaking_the_grammar_is_refused() {
        let err = skill_md_to_node(
            "content/functions/skills/Pdf_Forms/SKILL.md",
            b"---\nname: Pdf_Forms\ndescription: B.\n---\n",
        )
        .unwrap_err();
        assert!(err.contains("Agent Skills grammar"), "{err}");
        assert!(is_valid_skill_name(&"a".repeat(64)));
        assert!(!is_valid_skill_name(&"a".repeat(65)));
        assert!(!is_valid_skill_name("a--b"));
        assert!(!is_valid_skill_name("-a"));
        assert!(is_valid_skill_name("a-1-b"));
    }

    #[test]
    fn skill_md_without_a_directory_is_refused() {
        let err = skill_md_to_node("content/functions/SKILL.md", GOOD.as_bytes()).unwrap_err();
        assert!(err.contains("own directory"), "{err}");
    }

    #[test]
    fn non_utf8_is_refused() {
        let err = skill_md_to_node("content/functions/skills/a/SKILL.md", &[0xff, 0xfe, 0x00])
            .unwrap_err();
        assert!(err.contains("UTF-8"), "{err}");
    }

    #[test]
    fn only_the_exact_filename_is_a_skill() {
        assert!(is_skill_md("SKILL.md"));
        assert!(!is_skill_md("skill.md"));
        assert!(!is_skill_md("Skill.md"));
        assert!(!is_skill_md("SKILL.markdown"));
        assert!(!is_skill_md("README.md"));
    }

    #[test]
    fn a_yaml_definition_of_the_same_node_clashes() {
        let skills = vec![(
            "functions".to_string(),
            "content/functions/skills/a/SKILL.md".to_string(),
            "/skills/a".to_string(),
        )];
        let clash = |yaml: &str, ws: &str, path: &str| {
            skill_md_clashes(
                &skills,
                &[(ws.to_string(), yaml.to_string(), path.to_string())],
            )
        };
        let errors = clash(
            "content/functions/skills/a/.node.yaml",
            "functions",
            "/skills/a",
        );
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("both define '/skills/a'"),
            "{}",
            errors[0]
        );
        // A flat sibling yaml naming the same node clashes too.
        assert_eq!(
            clash("content/functions/skills/a.yaml", "functions", "/skills/a").len(),
            1
        );
        // Same path in another workspace, or a child of the skill, does not.
        assert!(clash("content/other/skills/a/.node.yaml", "other", "/skills/a").is_empty());
        assert!(clash(
            "content/functions/skills/a/x.yaml",
            "functions",
            "/skills/a/x"
        )
        .is_empty());
    }

    #[test]
    fn refusals_collapse_into_one_validation_error() {
        assert!(refuse_skill_md_errors(Vec::new()).is_ok());
        let one = refuse_skill_md_errors(vec!["x".into()]).unwrap_err();
        assert!(matches!(one, Error::Validation(ref m) if m == "x"));
        let two = refuse_skill_md_errors(vec!["x".into(), "y".into()]).unwrap_err();
        assert!(
            matches!(two, Error::Validation(ref m) if m.contains("2 SKILL.md") && m.contains("  - y"))
        );
    }
}

/// End to end through the real collector and the dry run: a --check must
/// report exactly what an install does.
#[cfg(test)]
mod install_tests {
    use std::io::{Cursor, Write};
    use std::sync::Arc;

    use raisin_models::nodes::properties::PropertyValue;
    use raisin_storage::jobs::{JobId, JobRegistry};
    use raisin_storage::{RepositoryManagementRepository, Storage};
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::{ZipArchive, ZipWriter};

    use crate::jobs::handlers::package_install::content_types::ContentEntry;
    use crate::jobs::handlers::package_install::handler::PackageInstallHandler;
    use crate::jobs::handlers::package_install::types::InstallMode;
    use crate::RocksDBStorage;

    const TENANT: &str = "default";
    const REPO: &str = "testrepo";
    const BRANCH: &str = "main";

    const SKILL: &[u8] = b"---\nname: pdf-forms\ndescription: Fill in PDF forms.\n\
allowed-tools: Bash\n---\n# PDF forms\n";

    async fn handler() -> (TempDir, PackageInstallHandler<RocksDBStorage>) {
        let dir = TempDir::new().unwrap();
        let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
        storage
            .repository_management()
            .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
            .await
            .unwrap();
        use raisin_storage::BranchRepository;
        storage
            .branches()
            .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
            .await
            .unwrap();
        raisin_core::nodetype_init::init_repository_nodetypes(
            storage.clone(),
            TENANT,
            REPO,
            BRANCH,
        )
        .await
        .unwrap();
        raisin_core::workspace_init::init_repository_workspaces(storage.clone(), TENANT, REPO)
            .await
            .unwrap();
        (
            dir,
            PackageInstallHandler::new(storage, Arc::new(JobRegistry::new())),
        )
    }

    fn package(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default();
            zip.start_file("manifest.yaml", opts).unwrap();
            zip.write_all(b"name: skill-md-test\nversion: 1.0.0\n")
                .unwrap();
            for (name, data) in files {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(data).unwrap();
            }
            zip.finish().unwrap();
        }
        buf
    }

    fn collect(
        handler: &PackageInstallHandler<RocksDBStorage>,
        zip: &Vec<u8>,
    ) -> raisin_error::Result<Vec<ContentEntry>> {
        let mut archive = ZipArchive::new(Cursor::new(zip)).unwrap();
        let job = JobId::new();
        let (collected, metadata) = handler.collect_content_entries(&mut archive, &job)?;
        let (entries, _) = handler.build_content_entries(collected, metadata, &job)?;
        Ok(entries)
    }

    #[tokio::test]
    async fn skill_md_installs_as_a_skill_and_siblings_as_children() {
        let (_dir, handler) = handler().await;
        let zip = package(&[
            ("content/functions/skills/pdf-forms/SKILL.md", SKILL),
            (
                "content/functions/skills/pdf-forms/references/fields.md",
                b"# Fields\n",
            ),
        ]);

        let entries = collect(&handler, &zip).unwrap();
        let mut skill = None;
        let mut binaries = Vec::new();
        for entry in &entries {
            match entry {
                ContentEntry::NodeDef {
                    node, yaml_path, ..
                } => {
                    assert_eq!(yaml_path, "content/functions/skills/pdf-forms/.node.yaml");
                    skill = Some(node.clone());
                }
                ContentEntry::BinaryFile {
                    parent_path,
                    filename,
                    ..
                } => binaries.push(format!("{parent_path}/{filename}")),
                _ => {}
            }
        }
        let skill = skill.expect("SKILL.md produced no node definition");
        assert_eq!(skill.node_type, "raisin:Skill");
        assert_eq!(skill.name, "pdf-forms");
        assert_eq!(skill.path, "/skills/pdf-forms");
        assert_eq!(
            skill.properties.get("body"),
            Some(&PropertyValue::String("# PDF forms".into()))
        );
        assert!(!skill.properties.contains_key("allowed-tools"));
        // SKILL.md itself is not ALSO an asset; its sibling is, under the skill.
        assert_eq!(
            binaries,
            vec!["skills/pdf-forms/references/fields.md".to_string()]
        );

        let dry = handler
            .dry_run(TENANT, REPO, BRANCH, &zip, InstallMode::Skip)
            .await
            .unwrap();
        assert_eq!(dry.summary.content_nodes.create, 1, "{:#?}", dry.logs);
        assert_eq!(dry.summary.binary_files.create, 1, "{:#?}", dry.logs);
        assert!(
            dry.logs
                .iter()
                .any(|l| l.category == "content" && l.path == "/skills/pdf-forms"),
            "{:#?}",
            dry.logs
        );
    }

    async fn assert_both_refuse(files: &[(&str, &[u8])], needle: &str) {
        let (_dir, handler) = handler().await;
        let zip = package(files);
        let install = collect(&handler, &zip).map(|_| ()).unwrap_err().to_string();
        assert!(install.contains(needle), "install: {install}");
        let check = handler
            .dry_run(TENANT, REPO, BRANCH, &zip, InstallMode::Skip)
            .await
            .map(|_| ())
            .unwrap_err()
            .to_string();
        assert_eq!(install, check, "install and --check disagree");
    }

    #[tokio::test]
    async fn name_mismatch_is_refused_by_install_and_check() {
        assert_both_refuse(
            &[("content/functions/skills/pdf/SKILL.md", SKILL)],
            "'pdf-forms' differs from its directory 'pdf'",
        )
        .await;
    }

    #[tokio::test]
    async fn skill_md_beside_node_yaml_is_refused_by_install_and_check() {
        assert_both_refuse(
            &[
                ("content/functions/skills/pdf-forms/SKILL.md", SKILL),
                (
                    "content/functions/skills/pdf-forms/.node.yaml",
                    b"node_type: raisin:Folder\n",
                ),
            ],
            "both define '/skills/pdf-forms'",
        )
        .await;
        // `.node.yml` is a folder definition to the collector — and now to the
        // dry run as well.
        assert_both_refuse(
            &[
                ("content/functions/skills/pdf-forms/SKILL.md", SKILL),
                (
                    "content/functions/skills/pdf-forms/.node.yml",
                    b"node_type: raisin:Folder\n",
                ),
            ],
            "both define '/skills/pdf-forms'",
        )
        .await;
    }

    #[tokio::test]
    async fn malformed_skill_md_is_refused_not_installed_as_an_asset() {
        assert_both_refuse(
            &[("content/functions/skills/a/SKILL.md", b"# just markdown\n")],
            "must begin with a '---' line",
        )
        .await;
    }
}
