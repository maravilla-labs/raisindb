//! `ask` — expand, retrieve, grade, retry, then answer with citations.
//!
//! # Why this is a function and not engine code
//!
//! Everything below the seam is deterministic — same document, same chunks,
//! same vectors — and belongs in the engine. Everything here is a judgement
//! call: how many passages to show, how to say "I don't know", when a retrieval
//! is good enough, which model to spend. That is policy, it changes per
//! product, and it is what a function is for: a tenant can still copy this
//! function node and replace it.
//!
//! # The loop
//!
//! ```text
//! (expand) → retrieve → grade → (rewrite → retrieve again) → answer → check
//! ```
//!
//! **Expansion** runs once, before the first retrieval, for short or
//! abbreviated questions: "wer ist der CEO?" becomes an extra lexical search
//! for "Geschäftsführer", and "Wem gehört der Flughafen?" one for "Anteile,
//! Gesellschafter, Beteiligung" — the words in which a document STATES the
//! answer. It only WIDENS retrieval: the terms are searched on the full-text
//! leg, never shown to the answering model as facts, and a failed expansion
//! changes nothing.
//!
//! **The claim check** runs after the answer: every sentence of the draft is
//! judged against the passages, and a sentence they do not state is dropped
//! (`dropped_claims`). When nothing that cites a passage survives, the result
//! is `grounded: false`. See [`verify`].
//!
//! **The grader** sees the question and the passages and answers with JSON:
//! does this answer it, and if not, what would a better search be? It is
//! bounded at [`MAX_ATTEMPTS`], and a grader that cannot be parsed counts as
//! "good enough" — a broken grader must degrade to plain RAG, never to a loop
//! that keeps paying for retrievals.
//!
//! # Grounding is enforced here, not hoped for
//!
//! 1. Retrieval returns nothing → `grounded: false` and a plain "not found"
//!    WITHOUT calling the answering model.
//! 2. Passages are numbered and the numbering is returned, so a caller can
//!    resolve `[2]` to a path rather than trusting prose.
//! 3. A passage the engine could only give as a PREVIEW is labelled as one.
//! 4. The answer prompt forbids relationships no passage states, and the
//!    claim check removes the ones the model states anyway.

use crate::backend::Backend;
use crate::options::SearchOptions;
use crate::retrieve::{retrieve, Passage, Retrieval};
use serde_json::{json, Value};

const GRAPH_FUNCTION: &str = "/lib/raisin/ai/graph-context";

/// Retrievals per question, including the first. Two: the second attempt is
/// where nearly all the benefit is, and a third is paid on every question that
/// genuinely has no answer.
pub const MAX_ATTEMPTS: usize = 2;

/// Passages per document handed to the model. One long PDF must not fill the
/// whole context while the page that answers sits at rank nine.
const MAX_PER_DOCUMENT: usize = 3;

pub const SYSTEM_PROMPT: &str = "You answer questions using only the numbered passages provided.

Rules:
- Use only what the passages say. Do not add facts from your own knowledge,
  even when you are confident they are correct.
- Cite the passages you used as [1], [2], and so on, immediately after the
  claim they support.
- If the passages do not answer the question, say plainly that the available
  documents do not cover it. That is a correct and useful answer, not a
  failure.
- Never infer a relationship the passages do not state. Two facts from
  different passages do not make a third, and a place, person or
  organisation that merely appears in a passage is not thereby the answer.
  If no passage states the answer itself, say the documents do not cover it.
- A passage marked (preview only) is truncated. You may use it to point the
  reader at the document, but never quote it as if it were complete.
- Answer in the language the question was asked in.";

pub const VERIFIER_PROMPT: &str =
    "You check a drafted answer, sentence by sentence, against the passages it was drafted from.

Return ONLY JSON, no prose and no code fence:
{\"sentences\": [{\"n\": 1, \"supported\": true|false}]}

Rules:
- A sentence is supported only when one passage, or passages together,
  STATE what it says. Paraphrase and translation are fine.
- A relationship no passage states is NOT supported, even when each fact it
  combines appears somewhere: a town named on a directions page does not own
  the airport, a person named in a caption does not lead the company.
- A sentence that makes no factual claim (it says the documents do not
  cover something, or points the reader to a document) is supported.
- Judge every numbered sentence exactly once.";

pub const GRADER_PROMPT: &str = "You judge whether retrieved passages can answer a question.

Return ONLY JSON, no prose and no code fence:
{\"sufficient\": true|false, \"rewrite\": \"a better search query, or empty\"}

Rules:
- `sufficient` is true when the passages contain the facts needed. They do
  not have to be well written or complete documents.
- When false, `rewrite` must be a query phrased in the vocabulary the
  documents themselves appear to use, not a rephrasing of the question.
- If the passages are simply about another subject, say false and rewrite.";

pub const EXPANDER_PROMPT: &str =
    "You widen a search query so a keyword search finds the documents that answer it.

Return ONLY JSON, no prose and no code fence:
{\"terms\": [\"...\"]}

Rules:
- Give at most 6 short search terms a document would use to STATE the
  answer, which are often not the query's own words: the spelled-out form of
  an abbreviation, the official title or name, and the nouns in which the
  relationship asked about is written down. A question about who owns
  something is answered in terms of shareholders, shares and stakes; one
  about who leads something in terms of management and managing directors.
- Write the terms in the documents' language given below, translating from
  the query's language when they differ.
- Terms only. Never an answer, a fact, a person's name, a date or a number
  you are guessing.
- If the query needs no widening, return {\"terms\": []}.";

/// A language code's English name, for prompts.
fn language_name(code: &str) -> &str {
    match &code[..2.min(code.len())] {
        "de" => "German",
        "en" => "English",
        "fr" => "French",
        "it" => "Italian",
        "es" => "Spanish",
        "pt" => "Portuguese",
        "nl" => "Dutch",
        "pl" => "Polish",
        "cs" => "Czech",
        "da" => "Danish",
        "sv" => "Swedish",
        "fi" => "Finnish",
        "no" | "nb" => "Norwegian",
        "tr" => "Turkish",
        "ja" => "Japanese",
        "zh" => "Chinese",
        _ => code,
    }
}

/// Short or abbreviated: at most eight words (a chat question, not a
/// paragraph), or an acronym anywhere. Pure.
pub fn needs_expansion(question: &str) -> bool {
    let words: Vec<&str> = question.split_whitespace().collect();
    let acronym = words.iter().any(|w| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        let n = w.chars().count();
        (2..=6).contains(&n)
            && w.chars().all(|c| c.is_uppercase() || c.is_ascii_digit())
            && w.chars().any(char::is_alphabetic)
    });
    words.len() <= 8 || acronym
}

/// The first `{...}` in a model reply, parsed. Tolerates prose and code fences
/// around it. Pure.
fn json_in(content: &str) -> Option<Value> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&content[start..=end]).ok()
}

fn content_of(response: &Value) -> String {
    response
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Extra search terms for the question, or none. Never fails the answer.
pub fn expand(
    b: &dyn Backend,
    model: &str,
    question: &str,
    base_language: Option<&str>,
) -> Vec<String> {
    let lang = match base_language {
        Some(l) => format!("The documents are written in {}.", language_name(l)),
        None => "The documents' language is not known: write the terms in the query's language."
            .to_string(),
    };
    let request = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": EXPANDER_PROMPT },
            { "role": "user", "content": format!("{lang}\n\nQuery: {question}") },
        ],
    });
    let response = match b.completion(&request) {
        Ok(r) => r,
        Err(e) => {
            b.log(&format!("[ask] expansion unavailable, proceeding: {e}"));
            return Vec::new();
        }
    };
    let terms: Vec<String> = json_in(&content_of(&response))
        .and_then(|v| v.get("terms").cloned())
        .and_then(|t| t.as_array().cloned())
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .take(6)
                .collect()
        })
        .unwrap_or_default();
    crate::options::sanitize_terms(&terms, question)
}

/// The grader's verdict. Fails to "sufficient".
struct Verdict {
    sufficient: bool,
    rewrite: String,
}

fn grade(b: &dyn Backend, model: &str, question: &str, passages: &[Passage]) -> Verdict {
    let good = Verdict {
        sufficient: true,
        rewrite: String::new(),
    };

    // An EMPTY retrieval still goes to the grader: it is the case where a
    // rewrite is worth the most, and the only case an early return would get
    // wrong (no query to retry WITH).
    let summary = if passages.is_empty() {
        "(no passages matched this query at all)".to_string()
    } else {
        passages
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let head = if p.title.is_empty() {
                    &p.path
                } else {
                    &p.title
                };
                format!(
                    "[{}] {}\n{}",
                    i + 1,
                    head,
                    p.text.chars().take(400).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let request = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": GRADER_PROMPT },
            { "role": "user", "content": format!("Question: {question}\n\nPassages:\n\n{summary}") },
        ],
    });
    let response = match b.completion(&request) {
        Ok(r) => r,
        Err(e) => {
            b.log(&format!("[ask] grader unavailable, proceeding: {e}"));
            return good;
        }
    };
    match json_in(&content_of(&response)) {
        Some(v) => Verdict {
            sufficient: v.get("sufficient") != Some(&Value::Bool(false)),
            rewrite: v
                .get("rewrite")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
        },
        None => good,
    }
}

/// The sentences of an answer, one per claim, keeping list items and lines
/// apart. Pure.
///
/// A split happens after `.`, `!` or `?` followed by whitespace and an
/// uppercase letter, a digit or a citation bracket — not after a short
/// capitalised token ("Dr.", "Nr.") or a lone letter ("z. B."), where a naive
/// split would hand the verifier a fragment it cannot judge.
pub fn sentences(answer: &str) -> Vec<Vec<String>> {
    let mut lines: Vec<Vec<String>> = Vec::new();
    for line in answer.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut out: Vec<String> = Vec::new();
        let mut start = 0;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let ends = matches!(c, '.' | '!' | '?')
                && i + 2 < chars.len()
                && chars[i + 1].is_whitespace()
                && {
                    let next = chars[i + 2..].iter().find(|c| !c.is_whitespace());
                    next.map(|n| n.is_uppercase() || n.is_ascii_digit() || *n == '[')
                        .unwrap_or(false)
                }
                && {
                    // the token the period closes
                    let token: String = chars[start..i]
                        .iter()
                        .rev()
                        .take_while(|c| !c.is_whitespace())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    let n = token.chars().count();
                    !(c == '.'
                        && (n == 1
                            || (n <= 3
                                && token.chars().next().is_some_and(char::is_uppercase)
                                && token.chars().all(char::is_alphabetic))))
                };
            if ends {
                let s: String = chars[start..=i].iter().collect();
                if !s.trim().is_empty() {
                    out.push(s.trim().to_string());
                }
                start = i + 1;
            }
            i += 1;
        }
        let rest: String = chars[start..].iter().collect();
        if !rest.trim().is_empty() {
            out.push(rest.trim().to_string());
        }
        lines.push(out);
    }
    lines
}

/// Does this sentence cite a passage (`[3]`)? Pure.
fn cites(sentence: &str) -> bool {
    let b = sentence.as_bytes();
    b.iter()
        .enumerate()
        .any(|(i, c)| *c == b'[' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
}

/// The outcome of the claim check.
struct Checked {
    answer: String,
    dropped: Vec<String>,
    /// `passed`, `trimmed`, `rejected` (nothing supported survived) or
    /// `unavailable` (the check itself failed; the draft is returned as is).
    verification: &'static str,
}

/// Check the draft sentence by sentence against the passages, and drop what
/// they do not state.
///
/// The failure this exists for was real: asked "Wem gehört der Flughafen?",
/// retrieval missed the one PDF passage that states the shareholders, and the
/// model answered anyway by combining the town names on a directions page into
/// an owner. The answer prompt forbids that; this makes it checkable. It costs
/// one more model call, and `verify: false` skips it.
fn verify(b: &dyn Backend, model: &str, question: &str, context: &str, draft: &str) -> Checked {
    let lines = sentences(draft);
    let numbered: Vec<&String> = lines.iter().flatten().collect();
    if numbered.is_empty() {
        return Checked {
            answer: draft.to_string(),
            dropped: Vec::new(),
            verification: "passed",
        };
    }
    let listing = numbered
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {s}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let unavailable = || Checked {
        answer: draft.to_string(),
        dropped: Vec::new(),
        verification: "unavailable",
    };
    let response = match b.completion(&json!({
        "model": model,
        "messages": [
            { "role": "system", "content": VERIFIER_PROMPT },
            { "role": "user", "content": format!("Question: {question}\n\nPassages:\n\n{context}\n\nSentences:\n{listing}") },
        ],
    })) {
        Ok(r) => r,
        Err(e) => {
            b.log(&format!("[ask] claim check unavailable, returning the draft unchecked: {e}"));
            return unavailable();
        }
    };
    let Some(verdicts) = json_in(&content_of(&response))
        .and_then(|v| v.get("sentences").and_then(Value::as_array).cloned())
    else {
        return unavailable();
    };
    // Only an explicit `false` drops a sentence: a verdict the checker left out
    // is not evidence against the claim.
    let unsupported: Vec<usize> = verdicts
        .iter()
        .filter(|v| v.get("supported") == Some(&Value::Bool(false)))
        .filter_map(|v| v.get("n").and_then(Value::as_u64))
        .map(|n| n as usize)
        .collect();
    if unsupported.is_empty() {
        return Checked {
            answer: draft.to_string(),
            dropped: Vec::new(),
            verification: "passed",
        };
    }

    let mut n = 0;
    let mut dropped: Vec<String> = Vec::new();
    let mut kept_lines: Vec<String> = Vec::new();
    for line in &lines {
        let mut kept: Vec<&str> = Vec::new();
        for s in line {
            n += 1;
            if unsupported.contains(&n) {
                dropped.push(s.clone());
            } else {
                kept.push(s);
            }
        }
        if !kept.is_empty() {
            kept_lines.push(kept.join(" "));
        }
    }
    let answer = kept_lines.join("\n");
    // What is left must still cite something: "More on the website." after the
    // claims were dropped is not an answer.
    let verification = if kept_lines.iter().any(|l| cites(l)) {
        "trimmed"
    } else {
        "rejected"
    };
    Checked {
        answer,
        dropped,
        verification,
    }
}

/// The "nothing found" answer, in the visitor's language when we know it.
fn not_found(locale: Option<&str>) -> &'static str {
    match locale.map(|l| &l[..2]) {
        Some("de") => {
            "Ich konnte in den verfügbaren Dokumenten nichts finden, das diese Frage beantwortet."
        }
        Some("fr") => {
            "Je n'ai rien trouvé dans les documents disponibles qui réponde à cette question."
        }
        Some("it") => {
            "Non ho trovato nulla nei documenti disponibili che risponda a questa domanda."
        }
        Some("es") => {
            "No encontré nada en los documentos disponibles que responda a esta pregunta."
        }
        _ => "I could not find anything in the available documents that answers this question.",
    }
}

/// A compact rendering of the graph neighbourhood, or `""`.
fn neighbourhood(b: &dyn Backend, question: &str, scope: &str) -> String {
    let graph = match b.call_function(
        GRAPH_FUNCTION,
        &json!({ "query": question, "workspaces": scope, "hops": 1 }),
    ) {
        Ok(g) => g,
        Err(_) => return String::new(),
    };
    // An ENRICHMENT: failing the answer because the relation index had nothing
    // to say would trade a good answer for no answer.
    if graph.get("error").map(|e| !e.is_null()).unwrap_or(false) {
        return String::new();
    }
    let Some(nodes) = graph.get("nodes").and_then(Value::as_array) else {
        return String::new();
    };
    nodes
        .iter()
        .take(20)
        .map(|n| {
            let name = n
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or_else(|| n.get("path").and_then(Value::as_str))
                .unwrap_or("");
            match n
                .get("via")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                Some(via) => format!("- {name} ({via})"),
                None => format!("- {name}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn ask(b: &dyn Backend, input: &Value) -> Result<Value, String> {
    let question = input
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if question.is_empty() {
        return Err("A non-empty `question` is required".into());
    }
    let mut opts = SearchOptions::parse(input, &question)?;
    if input.get("max_per_document").is_none() {
        opts.max_per_document = MAX_PER_DOCUMENT;
    }

    let model = match input
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.trim().is_empty())
    {
        Some(m) => m.to_string(),
        None => b.default_chat_model().ok_or_else(|| {
            "No chat model is configured for this tenant, and none was passed as `model`. \
             Configure one in the AI settings."
                .to_string()
        })?,
    };

    // Expansion: "auto" (default) for short or abbreviated questions, or
    // forced with true / off with false.
    let wants_expansion = match input.get("expand") {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) if s == "always" => true,
        Some(Value::String(s)) if s == "never" || s == "off" => false,
        _ => needs_expansion(&question),
    };
    if wants_expansion && opts.expansions.is_empty() {
        opts.expansions = expand(b, &model, &question, opts.base_language.as_deref());
    }

    let mut search_for = question.clone();
    let mut found = Retrieval {
        passages: Vec::new(),
        mode: "hybrid",
        terms: Vec::new(),
    };
    let mut attempts: Vec<Value> = Vec::new();

    for attempt in 1..=MAX_ATTEMPTS {
        let mut o = opts.clone();
        o.query = search_for.clone();
        found = retrieve(b, &o).map_err(|e| format!("Retrieval failed: {e}"))?;
        let mut entry = json!({ "query": search_for, "passages": found.passages.len() });
        if attempt == 1 && !opts.expansions.is_empty() {
            entry["expansions"] = json!(opts.expansions);
        }
        attempts.push(entry);

        if attempt == MAX_ATTEMPTS {
            break;
        }
        let verdict = grade(b, &model, &question, &found.passages);
        if verdict.sufficient || verdict.rewrite.is_empty() {
            break;
        }
        b.log(&format!(
            "[ask] rewriting \"{search_for}\" → \"{}\"",
            verdict.rewrite
        ));
        search_for = verdict.rewrite;
    }

    if found.passages.is_empty() {
        b.log(&format!(
            "[ask] no passages for \"{}\"",
            question.chars().take(80).collect::<String>()
        ));
        return Ok(json!({
            "answer": not_found(opts.locale.as_deref()),
            "grounded": false,
            "citations": [],
            "attempts": attempts,
            "model": "",
        }));
    }

    let terms = &found.terms;
    let citations: Vec<Value> = found
        .passages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut c = p.to_json(terms);
            let o = c.as_object_mut().expect("a passage is an object");
            o.remove("text");
            o.remove("score");
            o.insert("marker".into(), json!(i + 1));
            c
        })
        .collect();

    let mut context = found
        .passages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let label = if p.text_is_exact {
                ""
            } else {
                " (preview only)"
            };
            let head = if p.title.is_empty() {
                p.path.clone()
            } else {
                format!("{} — {}", p.title, p.path)
            };
            format!("[{}] {head}{label}\n{}", i + 1, p.text)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    // The graph leg, off by default: it answers a different question — how
    // things relate — and costs a walk.
    if input
        .get("use_graph")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let related = neighbourhood(b, &question, &opts.scope);
        if !related.is_empty() {
            context.push_str(&format!("\n\nRelated entities:\n{related}"));
        }
    }

    let response = b.completion(&json!({
        "model": model,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": format!("Question: {question}\n\nPassages:\n\n{context}") },
        ],
    }))?;
    let draft = content_of(&response);
    let used_model = response
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .unwrap_or(&model)
        .to_string();

    let checked = if input.get("verify").and_then(Value::as_bool).unwrap_or(true) {
        verify(b, &model, &question, &context, &draft)
    } else {
        Checked {
            answer: draft.clone(),
            dropped: Vec::new(),
            verification: "skipped",
        }
    };
    b.log(&format!(
        "[ask] \"{}\" → {} passage(s) in {} attempt(s), {} chars from {model}, claim check {} ({} dropped)",
        question.chars().take(80).collect::<String>(),
        found.passages.len(),
        attempts.len(),
        draft.chars().count(),
        checked.verification,
        checked.dropped.len()
    ));

    if checked.verification == "rejected" {
        // Nothing the passages state survived: this is "not found", and it is
        // reported as such rather than as a trimmed husk of an answer.
        return Ok(json!({
            "answer": not_found(opts.locale.as_deref()),
            "grounded": false,
            "citations": [],
            "attempts": attempts,
            "model": used_model,
            "verification": "rejected",
            "dropped_claims": checked.dropped,
        }));
    }

    let mut out = json!({
        "answer": checked.answer,
        "grounded": true,
        "citations": citations,
        "attempts": attempts,
        "model": used_model,
        "verification": checked.verification,
    });
    if !checked.dropped.is_empty() {
        out["dropped_claims"] = json!(checked.dropped);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    //! The JavaScript suite (`ask/index.test.mjs`), ported case for case, plus
    //! the new behaviour.

    use super::*;
    use crate::backend::fake::Fake;
    use serde_json::json;
    use std::collections::HashMap;

    fn passage_row(path: &str, id: &str, idx: i64, text: &str, exact: bool) -> Value {
        json!({ "node_id": id, "path": path, "name": "MSA", "node_type": "studio:Page", "workspace_id": "docs",
                "score": 0.03, "fulltext_rank": 1, "vector_rank": 1, "chunk_index": idx,
                "chunk_text": text, "chunk_text_source": if exact { "exact" } else { "excerpt" },
                "title": "MSA", "file_type": null })
    }

    fn msa() -> Value {
        passage_row(
            "/contracts/msa",
            "n1",
            3,
            "Either party may terminate on thirty days written notice.",
            true,
        )
    }

    /// Retrieval keyed by the query ($1) of the main hybrid leg; the grader,
    /// expander and answer distinguished by their system prompt.
    fn stub(search_for: HashMap<&'static str, Result<Vec<Value>, String>>, grader: Value) -> Fake {
        Fake {
            sql_fn: Box::new(move |sql, params| {
                if !sql.contains("HYBRID_SEARCH") || sql.contains("vector_weight => 0") {
                    return Ok(vec![]);
                }
                let q = params[0].as_str().unwrap_or("");
                search_for.get(q).cloned().unwrap_or(Ok(vec![]))
            }),
            completion_fn: Box::new(move |req| {
                let system = req["messages"][0]["content"].as_str().unwrap_or("");
                if system.contains("You judge whether") {
                    Ok(json!({ "content": grader.to_string() }))
                } else if system.contains("You widen a search query") {
                    Ok(json!({ "content": "{\"terms\": []}" }))
                } else if system.contains("You check a drafted answer") {
                    Ok(json!({ "content": "{\"sentences\": [{\"n\": 1, \"supported\": true}]}" }))
                } else {
                    Ok(
                        json!({ "content": "The notice period is 30 days [1].", "model": req["model"] }),
                    )
                }
            }),
            ..Fake::default()
        }
    }

    fn answers(f: &Fake) -> Vec<Value> {
        f.completions
            .borrow()
            .iter()
            .filter(|c| {
                let s = c["messages"][0]["content"].as_str().unwrap_or("");
                !s.contains("You judge") && !s.contains("You widen") && !s.contains("You check")
            })
            .cloned()
            .collect()
    }

    fn sufficient() -> Value {
        json!({ "sufficient": true, "rewrite": "" })
    }

    const LONG_Q: &str = "how long is the notice period for this contract?";

    #[test]
    fn nothing_retrieved_means_no_answer_call_and_it_says_so() {
        let f = stub(HashMap::new(), sufficient());
        let out = ask(&f, &json!({ "question": LONG_Q })).unwrap();
        assert_eq!(out["grounded"], json!(false));
        assert_eq!(out["citations"], json!([]));
        assert_eq!(
            answers(&f).len(),
            0,
            "an empty context answers from the model's own weights"
        );
    }

    #[test]
    fn a_good_first_retrieval_does_not_trigger_a_second_search() {
        let f = stub(HashMap::from([(LONG_Q, Ok(vec![msa()]))]), sufficient());
        let out = ask(&f, &json!({ "question": LONG_Q })).unwrap();
        assert_eq!(
            f.sqls()
                .iter()
                .filter(|s| s.contains("HYBRID_SEARCH"))
                .count(),
            1
        );
        assert_eq!(out["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(out["grounded"], json!(true));
    }

    #[test]
    fn an_insufficient_retrieval_is_rewritten_and_retried_once() {
        let q = "how much warning before we get kicked out?";
        let f = stub(
            HashMap::from([
                (q, Ok(vec![])),
                ("termination written notice period", Ok(vec![msa()])),
            ]),
            json!({ "sufficient": false, "rewrite": "termination written notice period" }),
        );
        let out = ask(&f, &json!({ "question": q })).unwrap();
        let searched: Vec<String> = f
            .sql_log
            .borrow()
            .iter()
            .filter(|(s, _)| s.contains("HYBRID_SEARCH"))
            .map(|(_, p)| p[0].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            searched,
            vec![
                q.to_string(),
                "termination written notice period".to_string()
            ]
        );
        assert_eq!(out["grounded"], json!(true));
        let trail: Vec<u64> = out["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["passages"].as_u64().unwrap())
            .collect();
        assert_eq!(trail, vec![0, 1], "the trail must show the rewrite worked");
    }

    #[test]
    fn the_loop_is_bounded() {
        let f = stub(
            HashMap::new(),
            json!({ "sufficient": false, "rewrite": "another phrasing" }),
        );
        ask(
            &f,
            &json!({ "question": "a question nobody can answer here" }),
        )
        .unwrap();
        assert_eq!(
            f.sqls()
                .iter()
                .filter(|s| s.contains("HYBRID_SEARCH"))
                .count(),
            2
        );
    }

    #[test]
    fn an_unparseable_grader_degrades_to_one_shot_rag() {
        let mut f = stub(HashMap::from([(LONG_Q, Ok(vec![msa()]))]), sufficient());
        f.completion_fn = Box::new(|req| {
            if req["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("You judge")
            {
                Ok(json!({ "content": "I think it is fine?" }))
            } else {
                Ok(json!({ "content": "answer [1]", "model": req["model"] }))
            }
        });
        let out = ask(&f, &json!({ "question": LONG_Q })).unwrap();
        assert_eq!(
            f.sqls()
                .iter()
                .filter(|s| s.contains("HYBRID_SEARCH"))
                .count(),
            1
        );
        assert_eq!(out["grounded"], json!(true));
    }

    #[test]
    fn a_grader_that_throws_does_not_fail_the_answer() {
        let mut f = stub(HashMap::from([(LONG_Q, Ok(vec![msa()]))]), sufficient());
        f.completion_fn = Box::new(|req| {
            if req["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("You judge")
            {
                Err("grader model unavailable".into())
            } else {
                Ok(json!({ "content": "answer [1]", "model": req["model"] }))
            }
        });
        let out = ask(&f, &json!({ "question": LONG_Q })).unwrap();
        assert_eq!(out["grounded"], json!(true));
        assert!(out["answer"].as_str().unwrap().contains("answer"));
    }

    #[test]
    fn a_failed_retrieval_is_an_error_not_an_empty_answer() {
        let f = stub(
            HashMap::from([(LONG_Q, Err("workspace not readable".to_string()))]),
            sufficient(),
        );
        let err = ask(&f, &json!({ "question": LONG_Q })).unwrap_err();
        assert_eq!(err, "Retrieval failed: workspace not readable");
    }

    #[test]
    fn passages_are_numbered_and_the_numbering_is_returned() {
        let mut nda = msa();
        nda["node_id"] = json!("n2");
        nda["path"] = json!("/contracts/nda");
        let f = stub(
            HashMap::from([(LONG_Q, Ok(vec![msa(), nda]))]),
            sufficient(),
        );
        let out = ask(&f, &json!({ "question": LONG_Q })).unwrap();
        let prompt = answers(&f)[0]["messages"][1]["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(prompt.contains("[1] MSA — /contracts/msa"), "{prompt}");
        assert!(prompt.contains("[2] MSA — /contracts/nda"), "{prompt}");
        let markers: Vec<(u64, String)> = out["citations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["marker"].as_u64().unwrap(),
                    c["node_id"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(markers, vec![(1, "n1".to_string()), (2, "n2".to_string())]);
    }

    #[test]
    fn a_citation_carries_what_a_site_needs_to_link_it() {
        let f = stub(HashMap::from([(LONG_Q, Ok(vec![msa()]))]), sufficient());
        let out = ask(&f, &json!({ "question": LONG_Q })).unwrap();
        let c = &out["citations"][0];
        for field in [
            "path",
            "workspace",
            "node_type",
            "title",
            "snippet",
            "kind",
            "chunk_index",
            "text_is_exact",
            "node_id",
        ] {
            assert!(!c[field].is_null(), "citation lacks {field}: {c}");
        }
        assert_eq!(c["workspace"], json!("docs"));
        assert!(
            c.get("text").is_none(),
            "the passage text stays in the prompt; the snippet travels"
        );
    }

    #[test]
    fn a_preview_passage_is_labelled_as_one() {
        let row = passage_row("/contracts/msa", "n1", 3, "Either party may…", false);
        let f = stub(HashMap::from([(LONG_Q, Ok(vec![row]))]), sufficient());
        ask(&f, &json!({ "question": LONG_Q })).unwrap();
        assert!(answers(&f)[0]["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("(preview only)"));
    }

    #[test]
    fn the_graph_leg_is_opt_in_and_never_fails_the_answer() {
        let mut f = stub(HashMap::from([(LONG_Q, Ok(vec![msa()]))]), sufficient());
        f.call_fn = Box::new(|_, _| Ok(json!({ "error": "relation index unavailable" })));
        let out = ask(&f, &json!({ "question": LONG_Q, "use_graph": true })).unwrap();
        assert_eq!(out["grounded"], json!(true));
        assert!(!answers(&f)[0]["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("Related entities"));
    }

    #[test]
    fn graph_context_is_appended_when_the_walk_finds_something() {
        let mut f = stub(HashMap::from([(LONG_Q, Ok(vec![msa()]))]), sufficient());
        f.call_fn = Box::new(|_, _| {
            Ok(
                json!({ "nodes": [{ "name": "Acme Ltd", "path": "/entities/acme-ltd", "via": "mentioned_in" }] }),
            )
        });
        ask(&f, &json!({ "question": LONG_Q, "use_graph": true })).unwrap();
        let prompt = answers(&f)[0]["messages"][1]["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            prompt.contains("Related entities") && prompt.contains("Acme Ltd (mentioned_in)"),
            "{prompt}"
        );
        assert_eq!(f.calls.borrow()[0].0, "/lib/raisin/ai/graph-context");
    }

    #[test]
    fn no_configured_model_is_a_clear_error() {
        let mut f = stub(HashMap::new(), sufficient());
        f.model = None;
        assert!(ask(&f, &json!({ "question": "q" }))
            .unwrap_err()
            .contains("No chat model is configured"));
    }

    #[test]
    fn an_empty_question_is_rejected() {
        let f = stub(HashMap::new(), sufficient());
        assert!(ask(&f, &json!({ "question": "  " }))
            .unwrap_err()
            .contains("non-empty"));
    }

    // ---- new behaviour -------------------------------------------------

    #[test]
    fn a_short_question_is_expanded_into_the_documents_language_and_searched_lexically() {
        let mut f = stub(HashMap::new(), sufficient());
        f.completion_fn = Box::new(|req| {
            let system = req["messages"][0]["content"].as_str().unwrap();
            if system.contains("You widen") {
                let user = req["messages"][1]["content"].as_str().unwrap();
                assert!(
                    user.contains("The documents are written in German."),
                    "{user}"
                );
                Ok(
                    json!({ "content": "```json\n{\"terms\": [\"Geschäftsführer\", \"Geschäftsleitung\"]}\n```" }),
                )
            } else if system.contains("You judge") {
                Ok(json!({ "content": "{\"sufficient\": true}" }))
            } else {
                Ok(json!({ "content": "Max Muster [1].", "model": "m" }))
            }
        });
        f.sql_fn = Box::new(|sql, params| {
            if sql.contains("vector_weight => 0") {
                assert_eq!(params[0], json!("Geschäftsführer Geschäftsleitung"));
                return Ok(vec![
                    json!({ "node_id": "t", "path": "/bap/team", "name": "team", "node_type": "studio:Page",
                    "workspace_id": "stories", "fulltext_rank": 1, "vector_rank": null, "chunk_index": 0,
                    "chunk_text": "Max Muster ist Geschäftsführer.", "chunk_text_source": "exact", "title": "Team" }),
                ]);
            }
            Ok(vec![])
        });
        let out = ask(
            &f,
            &json!({ "question": "wer ist der CEO?", "base_language": "de", "paths": ["/bap"] }),
        )
        .unwrap();
        assert_eq!(out["grounded"], json!(true));
        assert_eq!(
            out["attempts"][0]["expansions"],
            json!(["Geschäftsführer", "Geschäftsleitung"])
        );
        assert_eq!(out["citations"][0]["path"], json!("/bap/team"));
    }

    #[test]
    fn expansion_is_skipped_for_long_questions_and_can_be_turned_off() {
        let f = stub(HashMap::from([(LONG_Q, Ok(vec![msa()]))]), sufficient());
        ask(&f, &json!({ "question": LONG_Q })).unwrap();
        assert!(!f
            .completions
            .borrow()
            .iter()
            .any(|c| c["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("You widen")));

        let f = stub(HashMap::from([("CEO?", Ok(vec![msa()]))]), sufficient());
        ask(&f, &json!({ "question": "CEO?", "expand": false })).unwrap();
        assert!(!f
            .completions
            .borrow()
            .iter()
            .any(|c| c["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("You widen")));
    }

    #[test]
    fn a_failed_expansion_changes_nothing() {
        let mut f = stub(HashMap::from([("CEO?", Ok(vec![msa()]))]), sufficient());
        f.completion_fn = Box::new(|req| {
            let system = req["messages"][0]["content"].as_str().unwrap();
            if system.contains("You widen") {
                Err("rate limited".into())
            } else {
                Ok(json!({ "content": "{\"sufficient\": true}", "model": "m" }))
            }
        });
        let out = ask(&f, &json!({ "question": "CEO?" })).unwrap();
        assert_eq!(out["grounded"], json!(true));
        assert!(out["attempts"][0].get("expansions").is_none());
    }

    #[test]
    fn nothing_found_is_said_in_the_visitors_language() {
        let f = stub(HashMap::new(), sufficient());
        let out = ask(&f, &json!({ "question": LONG_Q, "locale": "de" })).unwrap();
        assert!(out["answer"].as_str().unwrap().starts_with("Ich konnte"));
    }

    #[test]
    fn needs_expansion_is_about_shortness_and_acronyms() {
        assert!(needs_expansion("wer ist der CEO?"));
        assert!(needs_expansion("Parken"));
        assert!(needs_expansion(
            "what does the AGB say about cancelling a booking?"
        ));
        assert!(!needs_expansion(
            "how long is the notice period for this contract?"
        ));
    }

    // ---- claim-level grounding ------------------------------------------

    const DIRECTIONS: &str =
        "Anfahrt: Der Flughafen liegt zwischen Rheinmünster und Hügelsheim, direkt an der A5.";
    const SHARES: &str = "Die Anteile an der Gesellschaft liegen zu 66% bei der Flughafen Stuttgart GmbH und zu 34% bei der Baden-Airpark Beteiligungsgesellschaft.";

    /// The real case: "Wem gehört der Flughafen?". The vector leg finds the
    /// directions page (town names); the fact is only in a PDF, phrased with
    /// "Anteile" and "Gesellschaft", which only the expanded lexical leg reaches.
    fn ownership_site(draft: &'static str) -> Fake {
        Fake {
            sql_fn: Box::new(|sql, params| {
                if sql.contains("HYBRID_SEARCH") && sql.contains("vector_weight => 0") {
                    let terms = params[0].as_str().unwrap_or("");
                    if terms.contains("Anteile") {
                        return Ok(vec![
                            json!({ "node_id": "pdf", "path": "/bap/downloads/unternehmen.pdf", "name": "unternehmen.pdf",
                            "node_type": "raisin:Asset", "workspace_id": "assets", "fulltext_rank": 1, "vector_rank": null,
                            "chunk_index": null, "chunk_text": null, "chunk_text_source": "unavailable",
                            "title": "Unternehmensprofil", "file_type": "application/pdf" }),
                        ]);
                    }
                    return Ok(vec![]);
                }
                if sql.contains("HYBRID_SEARCH") {
                    return Ok(vec![
                        json!({ "node_id": "anfahrt", "path": "/bap/anfahrt", "name": "anfahrt",
                        "node_type": "studio:Page", "workspace_id": "stories", "fulltext_rank": null, "vector_rank": 1,
                        "chunk_index": 0, "chunk_text": DIRECTIONS, "chunk_text_source": "exact", "title": "Anfahrt" }),
                    ]);
                }
                assert!(
                    sql.contains("SUBSTRING(properties->>'__extracted_text'"),
                    "{sql}"
                );
                Ok(vec![
                    json!({ "path": "/bap/downloads/unternehmen.pdf", "title": "Unternehmensprofil",
                    "body": format!("Seite 1\nUnternehmensprofil\n{SHARES}\nSeite 2") }),
                ])
            }),
            completion_fn: Box::new(move |req| {
                let system = req["messages"][0]["content"].as_str().unwrap_or("");
                let user = req["messages"][1]["content"].as_str().unwrap_or("");
                let content = if system.contains("You widen") {
                    assert!(user.contains("German"), "{user}");
                    "{\"terms\": [\"Gesellschafter\", \"Anteile\", \"Beteiligung\"]}".to_string()
                } else if system.contains("You judge") {
                    "{\"sufficient\": true}".to_string()
                } else if system.contains("You check a drafted answer") {
                    // A faithful checker: a sentence is supported when its
                    // claim is in a passage — here, only the shares sentence.
                    let listing = user.split("Sentences:\n").nth(1).unwrap_or("");
                    let verdicts: Vec<String> = listing
                        .lines()
                        .enumerate()
                        .map(|(i, l)| {
                            format!(
                                "{{\"n\": {}, \"supported\": {}}}",
                                i + 1,
                                l.contains("66%") && user.contains(SHARES)
                            )
                        })
                        .collect();
                    format!("{{\"sentences\": [{}]}}", verdicts.join(", "))
                } else {
                    draft.to_string()
                };
                Ok(json!({ "content": content, "model": "m" }))
            }),
            ..Fake::default()
        }
    }

    const OWNERSHIP: &str = "Wem gehört der Flughafen?";
    const MIXED_DRAFT: &str = "Der Flughafen gehört zu 66% der Flughafen Stuttgart GmbH und zu 34% der Baden-Airpark Beteiligungsgesellschaft [2]. Eigentümer sind außerdem die Gemeinden Rheinmünster und Hügelsheim [1].";
    const INVENTED_DRAFT: &str =
        "Der Flughafen gehört den Gemeinden Rheinmünster und Hügelsheim [1].";

    #[test]
    fn ownership_is_found_through_paraphrase_expansion_and_the_invented_claim_is_dropped() {
        let f = ownership_site(MIXED_DRAFT);
        let out = ask(
            &f,
            &json!({ "question": OWNERSHIP, "workspaces": ["stories", "assets"], "paths": ["/bap"], "base_language": "de" }),
        )
        .unwrap();

        assert_eq!(out["grounded"], json!(true));
        assert_eq!(out["verification"], json!("trimmed"));
        let answer = out["answer"].as_str().unwrap();
        assert!(
            answer.contains("66%") && !answer.contains("Gemeinden"),
            "{answer}"
        );
        assert_eq!(
            out["dropped_claims"][0].as_str().unwrap(),
            "Eigentümer sind außerdem die Gemeinden Rheinmünster und Hügelsheim [1]."
        );

        let pdf = out["citations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["path"] == "/bap/downloads/unternehmen.pdf")
            .expect("the PDF is cited");
        assert_eq!(pdf["kind"], json!("document"));
        assert_eq!(pdf["workspace"], json!("assets"));
        assert!(
            pdf["snippet"].as_str().unwrap().contains("66%"),
            "{}",
            pdf["snippet"]
        );

        let prompt = answers(&f)[0]["messages"][1]["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            prompt.contains(SHARES),
            "the shares passage reaches the model: {prompt}"
        );
    }

    #[test]
    fn without_the_fact_an_invented_owner_is_rejected_not_answered() {
        let f = ownership_site(INVENTED_DRAFT);
        let out = ask(
            &f,
            &json!({ "question": OWNERSHIP, "expand": false, "base_language": "de", "locale": "de" }),
        )
        .unwrap();
        assert_eq!(out["grounded"], json!(false));
        assert_eq!(out["verification"], json!("rejected"));
        assert_eq!(out["citations"], json!([]));
        assert!(
            out["answer"].as_str().unwrap().starts_with("Ich konnte"),
            "{}",
            out["answer"]
        );
        assert!(out["dropped_claims"][0]
            .as_str()
            .unwrap()
            .contains("Gemeinden"));
    }

    #[test]
    fn the_answer_prompt_forbids_inferring_unstated_relationships() {
        assert!(SYSTEM_PROMPT.contains("Never infer a relationship the passages do not state"));
    }

    #[test]
    fn a_claim_check_that_fails_returns_the_draft_flagged() {
        let mut f = ownership_site(MIXED_DRAFT);
        let inner = std::mem::replace(&mut f.completion_fn, Box::new(|_| Ok(json!({}))));
        f.completion_fn = Box::new(move |req| {
            if req["messages"][0]["content"]
                .as_str()
                .unwrap_or("")
                .contains("You check a drafted answer")
            {
                Err("verifier timed out".into())
            } else {
                inner(req)
            }
        });
        let out = ask(&f, &json!({ "question": OWNERSHIP, "base_language": "de" })).unwrap();
        assert_eq!(out["verification"], json!("unavailable"));
        assert_eq!(out["answer"], json!(MIXED_DRAFT));
    }

    #[test]
    fn verify_false_skips_the_check() {
        let f = ownership_site(MIXED_DRAFT);
        let out = ask(
            &f,
            &json!({ "question": OWNERSHIP, "base_language": "de", "verify": false }),
        )
        .unwrap();
        assert_eq!(out["verification"], json!("skipped"));
        assert!(!f
            .completions
            .borrow()
            .iter()
            .any(|c| c["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("You check")));
    }

    #[test]
    fn sentences_split_on_claims_not_on_abbreviations() {
        let s = sentences(
            "Dr. Max Muster leitet z. B. den Betrieb [1]. Die Anteile liegen zu 66% bei der FSG [2].\n- Parken: 3 Parkhäuser [3]",
        );
        assert_eq!(
            s,
            vec![
                vec![
                    "Dr. Max Muster leitet z. B. den Betrieb [1].".to_string(),
                    "Die Anteile liegen zu 66% bei der FSG [2].".to_string()
                ],
                vec!["- Parken: 3 Parkhäuser [3]".to_string()],
            ]
        );
    }
}
