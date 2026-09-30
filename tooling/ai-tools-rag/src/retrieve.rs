//! Retrieval: the search legs, their fusion, the filters, and the passage text.
//!
//! ```text
//! HYBRID_SEARCH(query)            vector + full text, fused by the engine
//! HYBRID_SEARCH(expansions)       full text only   (when there are expansions)
//! HYBRID_SEARCH(query, lang)      full text only   (per extra fulltext language)
//!        │  every leg: granularity 'chunk', the scope, and the WHERE residual
//!        ▼
//! reciprocal-rank fusion → exact filters → per-document cap
//!        ▼
//! passage text: the engine's chunk, or a window of the node's own text
//! (lexical-only hits), or of its locale overlay (when `locale` is given)
//! ```
//!
//! Every leg is one SQL statement through `raisin.sql.query`, so row-level
//! security applies to each exactly as it did to the JavaScript function: the
//! caller sees what the caller may read.

use crate::backend::Backend;
use crate::options::{is_workspace_name, kind_of, Kind, SearchOptions, ASSET_TYPE};
use crate::text;
use serde_json::{json, Value};
use std::collections::HashMap;

/// The Reciprocal Rank Fusion constant — the engine's own (`RRF_K`), so a
/// fused score here reads like one from `HYBRID_SEARCH`.
const RRF_K: f64 = 60.0;

/// How much table text one passage may carry, in characters. A tariff page's
/// tables run to a few hundred characters each; this keeps two or three.
const TABLE_CHARS: usize = 2000;

/// How much of an uploaded document's text a lexical passage is cut from.
const DOCUMENT_TEXT_CHARS: usize = 60_000;

/// One passage.
#[derive(Debug, Clone)]
pub struct Passage {
    pub path: String,
    pub node_id: String,
    pub workspace: String,
    pub node_type: String,
    pub kind: Kind,
    pub title: String,
    /// `None` for a passage cut from the node's text rather than an engine
    /// chunk. Reported as `0`, as the JavaScript function did for NULL.
    pub chunk_index: Option<i64>,
    pub text: String,
    pub text_is_exact: bool,
    pub score: f64,
    /// Did the words match, the meaning, or both?
    pub lexical: bool,
    pub semantic: bool,
    /// Set when `text` and `title` come from this locale's overlay.
    pub locale: Option<String>,
    pub url_hint: Option<String>,
    /// `chunk` (the engine's) or `excerpt` (cut from the node's text).
    pub source: &'static str,
    /// The base title as the search row carried it, to tell an overlay apart.
    base_title: String,
}

impl Passage {
    pub fn matched(&self) -> &'static str {
        match (self.lexical, self.semantic) {
            (true, true) => "both",
            (true, false) => "text",
            _ => "meaning",
        }
    }

    pub fn to_json(&self, terms: &[String]) -> Value {
        json!({
            "path": self.path,
            "node_id": self.node_id,
            "title": self.title,
            "chunk_index": self.chunk_index.unwrap_or(0),
            "text": self.text,
            "text_is_exact": self.text_is_exact,
            "score": self.score,
            "workspace": self.workspace,
            "node_type": self.node_type,
            "kind": self.kind.as_str(),
            "snippet": text::snippet(&self.text, terms),
            "matched": self.matched(),
            "source": self.source,
            "locale": self.locale,
            "url_hint": self.url_hint,
        })
    }
}

/// What a retrieval found, and how.
#[derive(Debug, Clone)]
pub struct Retrieval {
    pub passages: Vec<Passage>,
    /// `hybrid`, or `fulltext` when the tenant has no embedder.
    pub mode: &'static str,
    /// Words the passages were cut around and snippets centred on.
    pub terms: Vec<String>,
    /// Where the time went: every SQL leg, the passage reads, the total.
    pub timings: Value,
}

/// Milliseconds since `t`, to a tenth.
pub fn ms_since(t: std::time::Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

fn s(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// One leg.
#[derive(Clone, Copy, PartialEq)]
enum Leg<'a> {
    /// Vector + full text.
    Hybrid,
    /// Full text only, optionally with another analyzer.
    Lexical(Option<&'a str>),
}

/// The SQL of one leg and its parameters. Pure.
fn leg_sql(o: &SearchOptions, leg: Leg) -> (String, Vec<Value>) {
    // Only numbers this function produced and validated language codes are
    // interpolated; query, scope and every filter value are bound.
    let language = match leg {
        Leg::Lexical(Some(l)) => Some(l),
        _ => o.base_language.as_deref(),
    };
    let mut named = String::from("workspaces => $2, granularity => 'chunk'");
    if let Some(l) = language {
        named.push_str(&format!(", language => '{l}'"));
    }
    match leg {
        Leg::Hybrid => {
            if let Some(d) = o.max_distance {
                named.push_str(&format!(", max_distance => {d}"));
            }
        }
        Leg::Lexical(_) => named.push_str(", vector_weight => 0"),
    }
    let (filter, params) = o.where_clause(3);
    let sql = format!(
        "SELECT node_id, path, name, node_type, workspace_id, score, fulltext_rank, vector_rank, \
         chunk_index, chunk_text, chunk_text_source, \
         properties->>'title' AS title, properties->>'file_type' AS file_type, properties->>'url' AS url \
         FROM HYBRID_SEARCH($1, {}, {named}){}",
        o.candidates,
        filter.map(|f| format!(" WHERE {f}")).unwrap_or_default()
    );
    (sql, params)
}

fn run_leg(
    b: &dyn Backend,
    o: &SearchOptions,
    query: &str,
    leg: Leg,
) -> Result<Vec<Value>, String> {
    let (sql, mut filter_params) = leg_sql(o, leg);
    let mut params = vec![json!(query), json!(o.scope)];
    params.append(&mut filter_params);
    b.sql(&sql, &params)
}

/// The engine refuses a vector leg on a tenant with no embedder, and says so.
fn is_no_embedder(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("requires an embedding provider") || m.contains("cannot embed the query")
}

/// Fused candidate.
struct Candidate {
    passage: Passage,
    first_seen: usize,
}

/// Reciprocal-rank fusion of the legs' rows, keyed by passage. Pure.
fn fuse(lists: &[Vec<Value>]) -> Vec<Candidate> {
    let mut order: Vec<(String, String, Option<i64>)> = Vec::new();
    let mut acc: HashMap<(String, String, Option<i64>), Candidate> = HashMap::new();
    for list in lists {
        let mut rank = 0usize;
        for row in list {
            let workspace = s(row.get("workspace_id"));
            let node_id = s(row.get("node_id"));
            if node_id.is_empty() {
                continue;
            }
            let chunk_index = row.get("chunk_index").and_then(Value::as_i64);
            let key = (workspace.to_ascii_lowercase(), node_id.clone(), chunk_index);
            rank += 1;
            let add = 1.0 / (RRF_K + rank as f64);
            let lexical = row
                .get("fulltext_rank")
                .map(|v| !v.is_null())
                .unwrap_or(false);
            let semantic = row
                .get("vector_rank")
                .map(|v| !v.is_null())
                .unwrap_or(false);
            if let Some(c) = acc.get_mut(&key) {
                c.passage.score += add;
                c.passage.lexical |= lexical;
                c.passage.semantic |= semantic;
                continue;
            }
            let path = s(row.get("path"));
            let node_type = s(row.get("node_type"));
            let file_type = s(row.get("file_type"));
            let title = Some(s(row.get("title")))
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| s(row.get("name")));
            let chunk_text = s(row.get("chunk_text"));
            let source = s(row.get("chunk_text_source"));
            order.push(key.clone());
            acc.insert(
                key,
                Candidate {
                    first_seen: order.len(),
                    passage: Passage {
                        kind: kind_of(&node_type, &file_type, &path),
                        path,
                        node_id,
                        workspace,
                        node_type,
                        base_title: title.clone(),
                        title,
                        chunk_index,
                        text_is_exact: source == "exact",
                        text: text::strip_html(&chunk_text),
                        score: add,
                        lexical,
                        semantic,
                        locale: None,
                        url_hint: Some(s(row.get("url")))
                            .filter(|u| u.starts_with('/') || u.starts_with("http")),
                        source: "chunk",
                    },
                },
            );
        }
    }
    let mut out: Vec<Candidate> = order.into_iter().filter_map(|k| acc.remove(&k)).collect();

    // A node found by the lexical leg alone arrives as a chunk-less row. When
    // the same node also has chunk rows, its lexical evidence belongs to its
    // best chunk: fold it in rather than citing the document twice.
    let mut folded: Vec<Candidate> = Vec::with_capacity(out.len());
    let mut lexical_only: Vec<Candidate> = Vec::new();
    for c in out.drain(..) {
        if c.passage.chunk_index.is_none() {
            lexical_only.push(c);
        } else {
            folded.push(c);
        }
    }
    for lo in lexical_only {
        let target = folded
            .iter_mut()
            .filter(|c| {
                c.passage.node_id == lo.passage.node_id
                    && c.passage
                        .workspace
                        .eq_ignore_ascii_case(&lo.passage.workspace)
            })
            .max_by(|a, b| {
                a.passage
                    .score
                    .partial_cmp(&b.passage.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        match target {
            Some(t) => {
                t.passage.score += lo.passage.score;
                t.passage.lexical = true;
            }
            None => folded.push(lo),
        }
    }
    folded.sort_by(|a, b| {
        b.passage
            .score
            .partial_cmp(&a.passage.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.first_seen.cmp(&b.first_seen))
    });
    folded
}

/// The ranked rows of every search leg run so far, before fusion.
#[derive(Debug, Clone)]
pub struct Legs {
    lists: Vec<Vec<Value>>,
    timings: Vec<Value>,
    mode: &'static str,
    started: std::time::Instant,
}

fn timed_leg(
    b: &dyn Backend,
    o: &SearchOptions,
    legs: &mut Legs,
    name: &str,
    query: &str,
    leg: Leg,
) -> Result<(), String> {
    let t = std::time::Instant::now();
    let out = run_leg(b, o, query, leg);
    legs.timings.push(json!({
        "leg": name,
        "ms": ms_since(t),
        "rows": out.as_ref().map(|r| r.len()).unwrap_or(0),
    }));
    legs.lists.push(out?);
    Ok(())
}

/// Run the search legs: hybrid, the expansion terms (lexical), and one
/// lexical leg per extra full-text language.
pub fn run_legs(b: &dyn Backend, o: &SearchOptions) -> Result<Legs, String> {
    let mut legs = Legs {
        lists: Vec::new(),
        timings: Vec::new(),
        mode: "hybrid",
        started: std::time::Instant::now(),
    };
    match timed_leg(b, o, &mut legs, "hybrid", &o.query, Leg::Hybrid) {
        Ok(()) => {}
        // No embedder: the engine refuses rather than silently running half a
        // hybrid query. For answering, keyword search is far better than no
        // answer, so it runs deliberately and the result says so.
        Err(e) if is_no_embedder(&e) => {
            b.log(&format!("[search] no embedder, full text only: {e}"));
            legs.mode = "fulltext";
            timed_leg(b, o, &mut legs, "fulltext", &o.query, Leg::Lexical(None))?;
        }
        Err(e) => return Err(e),
    }
    if !o.expansions.is_empty() {
        add_expansion_leg(b, o, &mut legs, &o.expansions)?;
    }
    for lang in &o.fulltext_languages {
        let q = if o.expansions.is_empty() {
            o.query.clone()
        } else {
            format!("{} {}", o.query, o.expansions.join(" "))
        };
        timed_leg(
            b,
            o,
            &mut legs,
            &format!("fulltext:{lang}"),
            &q,
            Leg::Lexical(Some(lang.as_str())),
        )?;
    }
    Ok(legs)
}

/// Add the lexical leg for expansion terms to legs already run — so an
/// expansion decided AFTER a first look costs one full-text query, not a
/// second hybrid one (and no second query embedding).
pub fn add_expansion_leg(
    b: &dyn Backend,
    o: &SearchOptions,
    legs: &mut Legs,
    expansions: &[String],
) -> Result<(), String> {
    if expansions.is_empty() {
        return Ok(());
    }
    timed_leg(
        b,
        o,
        legs,
        "expansions",
        &expansions.join(" "),
        Leg::Lexical(None),
    )
}

/// Run every leg, fuse, filter, cap and fill in text.
pub fn retrieve(b: &dyn Backend, o: &SearchOptions) -> Result<Retrieval, String> {
    let legs = run_legs(b, o)?;
    assemble(b, o, &legs)
}

/// Weight of the term-coverage rerank, in RRF units: at full coverage a
/// passage gains about what two top ranks in one leg are worth.
const COVERAGE_WEIGHT: f64 = 0.03;

/// Fuse the legs, filter, cap per document, read passage text, rerank by
/// term coverage and cut to `limit`.
pub fn assemble(b: &dyn Backend, o: &SearchOptions, legs: &Legs) -> Result<Retrieval, String> {
    let mut per_doc: HashMap<(String, String), usize> = HashMap::new();
    let mut picked: Vec<Passage> = Vec::new();
    for c in fuse(&legs.lists) {
        let p = c.passage;
        if !o.admits(&p.workspace, &p.path, &p.node_type, p.kind) {
            continue;
        }
        if o.max_per_document > 0 {
            let n = per_doc
                .entry((p.workspace.to_ascii_lowercase(), p.node_id.clone()))
                .or_insert(0);
            if *n >= o.max_per_document {
                continue;
            }
            *n += 1;
        }
        picked.push(p);
    }

    let terms = text::terms(&o.query, &o.expansions);

    // Text is filled for a margin beyond `limit`, because a candidate whose
    // text turns out to be empty is dropped and the next one takes its place
    // — and because the rerank below can lift a candidate from the margin.
    let reach = (o.limit * 2 + 4).min(picked.len());
    let mut head: Vec<Passage> = picked.drain(..reach).collect();
    let t = std::time::Instant::now();
    let reads = fill_text(b, o, &terms, &mut head)?;
    let fill_ms = ms_since(t);
    head.retain(|p| !p.text.trim().is_empty());
    rerank_by_coverage(&mut head, &terms);
    head.truncate(o.limit);

    Ok(Retrieval {
        passages: head,
        mode: legs.mode,
        terms,
        timings: json!({
            "legs": legs.timings,
            "reads": reads,
            "reads_ms": fill_ms,
            "total_ms": ms_since(legs.started),
        }),
    })
}

/// Lift passages that contain the question's distinctive words, now that
/// their full text — tables included — is known. Pure.
///
/// Fusion ranks what the INDEXES saw. A page's tariff table is not in them,
/// so "Was kostet Parken für eine Woche?" ranked two AGB PDFs (which say
/// "Parken" everywhere) above the Parken page whose table has the
/// "Wochentarif" rows. Here every term is weighted by how FEW of the
/// candidates contain it (a word all of them share decides nothing), and a
/// passage gains up to [`COVERAGE_WEIGHT`] for the share of that weight it
/// covers. Terms match as substrings, so "woche" finds "Wochentarif".
pub fn rerank_by_coverage(passages: &mut [Passage], terms: &[String]) {
    if passages.len() < 2 || terms.is_empty() {
        return;
    }
    let texts: Vec<String> = passages
        .iter()
        .map(|p| format!("{}\n{}", p.title, p.text).to_lowercase())
        .collect();
    let n = texts.len() as f64;
    let weights: Vec<(usize, f64)> = terms
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let df = texts.iter().filter(|x| x.contains(t.as_str())).count();
            (df > 0).then(|| (i, (1.0 + n / (1.0 + df as f64)).ln()))
        })
        .collect();
    let total: f64 = weights.iter().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return;
    }
    for (p, text) in passages.iter_mut().zip(&texts) {
        let got: f64 = weights
            .iter()
            .filter(|(i, _)| text.contains(terms[*i].as_str()))
            .map(|(_, w)| w)
            .sum();
        p.score += COVERAGE_WEIGHT * got / total;
    }
    passages.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Is this retrieval good enough to answer without asking a model? Pure.
///
/// The question's own content words must mostly (two thirds) appear in the
/// top three passages, and the best passage must have matched on its words,
/// not only on meaning. Used by `ask` to skip expansion and grading — each a
/// model call — when the first look already found the vocabulary asked about.
pub fn confident(passages: &[Passage], question: &str) -> bool {
    let terms = text::terms(question, &[]);
    if terms.is_empty() || passages.is_empty() || !passages[0].lexical {
        return false;
    }
    let top: Vec<String> = passages
        .iter()
        .take(3)
        .map(|p| format!("{}\n{}", p.title, p.text).to_lowercase())
        .collect();
    let covered = terms
        .iter()
        .filter(|t| top.iter().any(|x| x.contains(t.as_str())))
        .count();
    covered * 3 >= terms.len() * 2
}

/// Give every passage text, and overlay text when a locale was asked for.
///
/// One batched read per (workspace, asset-or-not) group: a document's body is
/// selected as a bounded SUBSTRING, never as the whole property map, because an
/// uploaded PDF's `__extracted_text` can be megabytes.
fn fill_text(
    b: &dyn Backend,
    o: &SearchOptions,
    terms: &[String],
    passages: &mut [Passage],
) -> Result<usize, String> {
    let overlay = o.overlay_locale();
    // (workspace, is_asset) -> indexes of passages that need a read
    let mut groups: Vec<((String, bool), Vec<usize>)> = Vec::new();
    for (i, p) in passages.iter().enumerate() {
        let needs_text = p.text.trim().is_empty();
        // Every page is read: its tables and lists are structured block
        // content that is not in the chunk the vector leg matched.
        let is_page = p.kind == Kind::Page;
        if !(needs_text || is_page) || !is_workspace_name(&p.workspace) {
            continue;
        }
        let key = (p.workspace.clone(), p.node_type == ASSET_TYPE);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, idx)) => idx.push(i),
            None => groups.push((key, vec![i])),
        }
    }

    let reads = groups.len();
    let mut tabled: Vec<(String, String)> = Vec::new();
    for ((workspace, is_asset), idx) in groups {
        let mut paths: Vec<String> = Vec::new();
        for &i in &idx {
            if !paths.contains(&passages[i].path) {
                paths.push(passages[i].path.clone());
            }
        }
        let rows = read_nodes(b, &workspace, is_asset, &paths, overlay)?;
        let by_path: HashMap<String, &Value> = rows.iter().map(|r| (s(r.get("path")), r)).collect();
        for &i in &idx {
            let p = &mut passages[i];
            let Some(row) = by_path.get(&p.path) else {
                continue;
            };
            let (props, title) = if is_asset {
                let props = json!({
                    "title": row.get("title"),
                    "description": row.get("description"),
                    "caption": row.get("caption"),
                    "alt_text": row.get("alt_text"),
                    "__extracted_text": row.get("body"),
                });
                (props, s(row.get("title")))
            } else {
                let props = row.get("properties").cloned().unwrap_or(Value::Null);
                let title = s(props.get("title"));
                (props, title)
            };
            let translated = overlay.is_some()
                && !title.trim().is_empty()
                && title.trim() != p.base_title.trim();
            let segments = text::node_text(&props);
            if p.text.trim().is_empty() || (translated && p.kind == Kind::Page) {
                let w = text::window(&segments, terms, text::WINDOW_CHARS);
                if !w.trim().is_empty() {
                    p.text = w;
                    p.text_is_exact = true;
                    p.source = "excerpt";
                }
            }
            if translated {
                p.title = title;
                p.locale = overlay.map(str::to_string);
            }
            if let Some(u) = text::url_hint(&props, overlay) {
                p.url_hint = Some(u);
            }
            // Tables go with the first passage of their page only: three
            // passages of one page must not carry its price list three times.
            let key = (p.workspace.to_ascii_lowercase(), p.node_id.clone());
            if p.kind == Kind::Page && !p.text.trim().is_empty() && !tabled.contains(&key) {
                let picked = text::pick_tables(&text::tables(&props), &p.text, terms, TABLE_CHARS);
                if !picked.is_empty() {
                    p.text = format!("{}\n\n{}", p.text, picked.join("\n\n"));
                    tabled.push(key);
                }
            }
        }
    }
    Ok(reads)
}

/// Read the text-bearing fields of `paths` in one workspace, in `locale` when
/// given. A locale the repository does not know is not worth failing the
/// answer over: the read is retried in the base language.
fn read_nodes(
    b: &dyn Backend,
    workspace: &str,
    is_asset: bool,
    paths: &[String],
    locale: Option<&str>,
) -> Result<Vec<Value>, String> {
    let placeholders: Vec<String> = (1..=paths.len()).map(|i| format!("${i}")).collect();
    let cols = if is_asset {
        format!(
            "path, properties->>'title' AS title, properties->>'description' AS description, \
             properties->>'caption' AS caption, properties->>'alt_text' AS alt_text, \
             SUBSTRING(properties->>'__extracted_text', 1, {DOCUMENT_TEXT_CHARS}) AS body"
        )
    } else {
        "path, properties".to_string()
    };
    let params: Vec<Value> = paths.iter().map(|p| json!(p)).collect();
    let base = format!(
        "SELECT {cols} FROM '{workspace}' WHERE path IN ({})",
        placeholders.join(", ")
    );
    if let Some(l) = locale {
        match b.sql(&format!("{base} AND locale = '{l}'"), &params) {
            Ok(rows) => return Ok(rows),
            Err(e) => b.log(&format!(
                "[search] overlay read in '{l}' failed, using base text: {e}"
            )),
        }
    }
    b.sql(&base, &params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::Fake;
    use serde_json::json;

    fn opts(v: Value) -> SearchOptions {
        SearchOptions::parse(&v, v.get("query").and_then(Value::as_str).unwrap_or("q")).unwrap()
    }

    fn chunk(path: &str, id: &str, idx: i64, text: &str) -> Value {
        json!({ "node_id": id, "path": path, "name": id, "node_type": "studio:Page", "workspace_id": "stories",
                "score": 0.03, "fulltext_rank": null, "vector_rank": 1, "chunk_index": idx,
                "chunk_text": text, "chunk_text_source": "exact", "title": format!("Title {id}"), "file_type": null })
    }

    fn lexical(path: &str, id: &str, node_type: &str) -> Value {
        json!({ "node_id": id, "path": path, "name": id, "node_type": node_type, "workspace_id": "stories",
                "score": 0.02, "fulltext_rank": 1, "vector_rank": null, "chunk_index": null,
                "chunk_text": null, "chunk_text_source": "unavailable", "title": format!("Title {id}"), "file_type": null })
    }

    #[test]
    fn the_scope_and_the_filters_are_in_the_search_itself() {
        let fake = Fake::default();
        let o = opts(
            json!({"query": "wer ist der CEO?", "workspaces": ["stories", "assets"], "paths": ["/bap"]}),
        );
        retrieve(&fake, &o).unwrap();
        let log = fake.sql_log.borrow();
        let (sql, params) = &log[0];
        assert!(
            sql.contains("FROM HYBRID_SEARCH($1, 24, workspaces => $2, granularity => 'chunk')"),
            "{sql}"
        );
        assert!(
            sql.contains("WHERE ((path = $3 OR path LIKE $4)) AND (node_type <> 'raisin:Asset'"),
            "{sql}"
        );
        assert_eq!(
            params[..4],
            [
                json!("wer ist der CEO?"),
                json!("stories, assets"),
                json!("/bap"),
                json!("/bap/%")
            ]
        );
    }

    #[test]
    fn a_lexical_only_hit_becomes_a_passage_instead_of_vanishing() {
        let fake = Fake {
            sql_fn: Box::new(|sql, _| {
                if sql.contains("HYBRID_SEARCH") {
                    Ok(vec![
                        chunk("/bap/about", "a", 0, "Über uns."),
                        lexical("/bap/news/neuer-geschaeftsfuehrer", "n", "studio:Page"),
                    ])
                } else {
                    Ok(vec![
                        json!({ "path": "/bap/news/neuer-geschaeftsfuehrer", "properties": {
                            "title": "Neuer Geschäftsführer",
                            "content": [{ "element_type": "studio:Text", "body": "Max Muster übernimmt als Geschäftsführer." }]
                        }}),
                    ])
                }
            }),
            ..Fake::default()
        };
        let r = retrieve(&fake, &opts(json!({"query": "Geschäftsführer"}))).unwrap();
        assert_eq!(r.passages.len(), 2);
        let news = r.passages.iter().find(|p| p.node_id == "n").unwrap();
        assert!(news.text.contains("Max Muster übernimmt"), "{}", news.text);
        assert_eq!(news.source, "excerpt");
        assert_eq!(news.matched(), "text");
        assert!(
            fake.sqls()[1]
                .starts_with("SELECT path, properties FROM 'stories' WHERE path IN ($1, $2)"),
            "{}",
            fake.sqls()[1]
        );
    }

    #[test]
    fn images_never_become_passages_by_default() {
        let fake = Fake {
            sql_fn: Box::new(|_, _| {
                let mut img = chunk("/bap/ceo.jpg", "i", 0, "Portrait of the CEO");
                img["node_type"] = json!("raisin:Asset");
                img["file_type"] = json!("image/jpeg");
                Ok(vec![img, chunk("/bap/team", "t", 0, "Max Muster, CEO")])
            }),
            ..Fake::default()
        };
        let r = retrieve(&fake, &opts(json!({"query": "CEO"}))).unwrap();
        assert_eq!(
            r.passages
                .iter()
                .map(|p| p.node_id.as_str())
                .collect::<Vec<_>>(),
            vec!["t"]
        );
        let r = retrieve(
            &fake,
            &opts(json!({"query": "CEO", "include_kinds": ["all"]})),
        )
        .unwrap();
        assert_eq!(r.passages.len(), 2);
    }

    #[test]
    fn a_path_outside_the_prefix_is_dropped_even_if_the_engine_returned_it() {
        let fake = Fake {
            sql_fn: Box::new(|_, _| {
                Ok(vec![
                    chunk("/demo/ceo", "d", 0, "Demo CEO"),
                    chunk("/bap/ceo", "b", 0, "Real CEO"),
                ])
            }),
            ..Fake::default()
        };
        let r = retrieve(&fake, &opts(json!({"query": "CEO", "paths": ["/bap"]}))).unwrap();
        assert_eq!(
            r.passages
                .iter()
                .map(|p| p.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/bap/ceo"]
        );
    }

    #[test]
    fn hybrid_fusion_ranks_what_both_the_query_and_the_expansion_found_first() {
        let fake = Fake {
            sql_fn: Box::new(|sql, params| {
                if !sql.contains("HYBRID_SEARCH") {
                    return Ok(vec![]);
                }
                if sql.contains("vector_weight => 0") {
                    assert_eq!(params[0], json!("Geschäftsführer"));
                    let mut hit = lexical("/bap/team", "t", "studio:Page");
                    hit["chunk_index"] = json!(null);
                    Ok(vec![lexical("/bap/news", "n", "studio:Page"), hit])
                } else {
                    Ok(vec![
                        chunk("/bap/faq", "f", 0, "FAQ"),
                        chunk("/bap/team", "t", 2, "Max Muster, CEO"),
                    ])
                }
            }),
            ..Fake::default()
        };
        let o = opts(json!({"query": "CEO", "expansions": ["Geschäftsführer"]}));
        let r = retrieve(&fake, &o).unwrap();
        assert_eq!(
            r.passages[0].node_id, "t",
            "found by the query AND the expansion"
        );
        assert_eq!(
            r.passages[0].chunk_index,
            Some(2),
            "lexical evidence is folded into the node's chunk"
        );
        assert_eq!(r.passages[0].matched(), "both");
        assert!(
            r.passages.iter().filter(|p| p.node_id == "t").count() == 1,
            "the document is cited once"
        );
        let sqls = fake.sqls();
        assert!(
            sqls[1].contains("vector_weight => 0"),
            "an expansion widens the LEXICAL leg only: no second embedding call"
        );
    }

    #[test]
    fn extra_fulltext_languages_are_extra_lexical_legs() {
        let fake = Fake::default();
        let o =
            opts(json!({"query": "parking", "base_language": "de", "fulltext_languages": ["en"]}));
        retrieve(&fake, &o).unwrap();
        let sqls = fake.sqls();
        assert!(sqls[0].contains("language => 'de'") && !sqls[0].contains("vector_weight"));
        assert!(
            sqls[1].contains("language => 'en', vector_weight => 0"),
            "{}",
            sqls[1]
        );
    }

    #[test]
    fn a_locale_returns_the_overlay_title_text_and_url() {
        let fake = Fake {
            sql_fn: Box::new(|sql, _| {
                if sql.contains("HYBRID_SEARCH") {
                    return Ok(vec![chunk(
                        "/bap/parken",
                        "p",
                        0,
                        "Parken am Flughafen: 3 Parkhäuser.",
                    )]);
                }
                assert!(sql.ends_with("AND locale = 'fr'"), "{sql}");
                Ok(vec![json!({ "path": "/bap/parken", "properties": {
                    "title": "Parking à l'aéroport", "body": "Trois parkings à l'aéroport.", "url_fr": "/fr/parking"
                }})])
            }),
            ..Fake::default()
        };
        let r = retrieve(
            &fake,
            &opts(json!({"query": "stationnement", "locale": "fr", "base_language": "de"})),
        )
        .unwrap();
        let p = &r.passages[0];
        assert_eq!(p.title, "Parking à l'aéroport");
        assert!(p.text.contains("Trois parkings"));
        assert_eq!(p.locale.as_deref(), Some("fr"));
        assert_eq!(p.url_hint.as_deref(), Some("/fr/parking"));
    }

    #[test]
    fn a_locale_without_an_overlay_keeps_the_base_passage() {
        let fake = Fake {
            sql_fn: Box::new(|sql, _| {
                if sql.contains("HYBRID_SEARCH") {
                    return Ok(vec![chunk("/bap/parken", "p", 0, "Parken am Flughafen.")]);
                }
                Ok(vec![
                    json!({ "path": "/bap/parken", "properties": { "title": "Title p", "body": "Parken am Flughafen." }}),
                ])
            }),
            ..Fake::default()
        };
        let r = retrieve(&fake, &opts(json!({"query": "parking", "locale": "fr"}))).unwrap();
        assert_eq!(r.passages[0].text, "Parken am Flughafen.");
        assert_eq!(r.passages[0].locale, None);
        assert_eq!(r.passages[0].source, "chunk");
    }

    #[test]
    fn no_embedder_degrades_to_keyword_search_and_says_so() {
        let fake = Fake {
            sql_fn: Box::new(|sql, _| {
                if sql.contains("vector_weight => 0") {
                    Ok(vec![])
                } else {
                    Err("HYBRID_SEARCH requires an embedding provider to embed the query".into())
                }
            }),
            ..Fake::default()
        };
        let r = retrieve(&fake, &opts(json!({"query": "x"}))).unwrap();
        assert_eq!(r.mode, "fulltext");
    }

    #[test]
    fn any_other_sql_failure_is_an_error() {
        let fake = Fake {
            sql_fn: Box::new(|_, _| Err("workspace not readable".into())),
            ..Fake::default()
        };
        assert_eq!(
            retrieve(&fake, &opts(json!({"query": "x"}))).unwrap_err(),
            "workspace not readable"
        );
    }

    #[test]
    fn the_per_document_cap_spreads_the_passages() {
        let fake = Fake {
            sql_fn: Box::new(|_, _| {
                Ok((0..6)
                    .map(|i| chunk("/bap/big.pdf", "big", i, "x y z"))
                    .chain([chunk("/bap/other", "o", 0, "other")])
                    .collect())
            }),
            ..Fake::default()
        };
        let r = retrieve(&fake, &opts(json!({"query": "x", "max_per_document": 2}))).unwrap();
        assert_eq!(r.passages.iter().filter(|p| p.node_id == "big").count(), 2);
        assert!(r.passages.iter().any(|p| p.node_id == "o"));
    }

    /// The SQL contract with the engine.
    ///
    /// These four statements are asserted HERE, character for character, and
    /// executed against a real RocksDB + Tantivy engine in
    /// `crates/raisin-sql-execution/tests/all/rag_retrieval_sql.rs`, which
    /// holds the same literals. Change one side and the other must follow —
    /// that pairing is what proves the residual `WHERE` really narrows the
    /// search rather than being parsed and ignored.
    pub const CONTRACT_SELECT: &str = "SELECT node_id, path, name, node_type, workspace_id, score, fulltext_rank, vector_rank, chunk_index, chunk_text, chunk_text_source, properties->>'title' AS title, properties->>'file_type' AS file_type, properties->>'url' AS url FROM HYBRID_SEARCH($1, 40, workspaces => $2, granularity => 'chunk', vector_weight => 0)";

    pub const CONTRACT: [(&str, &str); 4] = [
        (
            r#"{"candidates": 40, "workspaces": "stories, assets", "paths": ["/bap"]}"#,
            " WHERE ((path = $3 OR path LIKE $4)) AND (node_type <> 'raisin:Asset' OR properties->>'file_type' IS NULL OR NOT (properties->>'file_type' LIKE 'image/%'))",
        ),
        (
            r#"{"candidates": 40, "workspaces": "stories, assets", "paths": ["/bap"], "include_kinds": ["page", "document"], "exclude_node_types": ["studio:Blueprint"]}"#,
            " WHERE ((path = $3 OR path LIKE $4)) AND (node_type <> 'raisin:Asset' OR (node_type = 'raisin:Asset' AND (properties->>'file_type' IS NULL OR NOT (properties->>'file_type' LIKE 'image/%' OR properties->>'file_type' LIKE 'video/%' OR properties->>'file_type' LIKE 'audio/%')))) AND node_type <> $5",
        ),
        (
            r#"{"candidates": 40, "workspaces": "stories, assets", "paths": ["stories:/bap", "assets:/demo"]}"#,
            " WHERE ((workspace_id = $5 AND (path = $3 OR path LIKE $4)) OR (workspace_id = $8 AND (path = $6 OR path LIKE $7))) AND (node_type <> 'raisin:Asset' OR properties->>'file_type' IS NULL OR NOT (properties->>'file_type' LIKE 'image/%'))",
        ),
        (
            r#"{"candidates": 40, "workspaces": "stories, assets", "paths": ["/bap"], "include_kinds": ["image"]}"#,
            " WHERE ((path = $3 OR path LIKE $4)) AND ((node_type = 'raisin:Asset' AND properties->>'file_type' LIKE 'image/%'))",
        ),
    ];

    #[test]
    fn the_lexical_leg_sql_is_the_contract_the_engine_test_runs() {
        for (input, filter) in CONTRACT {
            let o = opts(serde_json::from_str(input).unwrap());
            let (sql, _) = leg_sql(&o, Leg::Lexical(None));
            assert_eq!(sql, format!("{CONTRACT_SELECT}{filter}"), "input {input}");
        }
        let (_, params) = leg_sql(
            &opts(serde_json::from_str(CONTRACT[2].0).unwrap()),
            Leg::Lexical(None),
        );
        assert_eq!(
            params,
            vec![
                json!("/bap"),
                json!("/bap/%"),
                json!("stories"),
                json!("/demo"),
                json!("/demo/%"),
                json!("assets")
            ]
        );
    }

    fn parking_rows(sql: &str) -> Result<Vec<Value>, String> {
        if sql.contains("HYBRID_SEARCH") {
            return Ok(vec![
                chunk(
                    "/bap/parken",
                    "p",
                    0,
                    "<p>Parken am Flughafen: drei Parkhäuser direkt am Terminal.</p>",
                ),
                chunk("/bap/parken", "p", 1, "Anreise mit dem Auto über die A5."),
            ]);
        }
        let de = !sql.contains("locale = 'fr'");
        Ok(vec![json!({ "path": "/bap/parken", "properties": {
            "title": if de { "Title p" } else { "Parking à l'aéroport" },
            "content": [{ "element_type": "bap:Table", "tables": [{
                "title": if de { "Parktarife" } else { "Tarifs de stationnement" },
                "data": { "header_row": true, "header_col": true,
                          "rows": [["", "P3"], [if de { "1 Tag" } else { "1 jour" }, "19,00 €"]] }
            }]}]
        }})])
    }

    #[test]
    fn a_page_passage_carries_its_tables_once() {
        let fake = Fake {
            sql_fn: Box::new(|sql, _| parking_rows(sql)),
            ..Fake::default()
        };
        let r = retrieve(&fake, &opts(json!({"query": "Was kostet Parken?"}))).unwrap();
        let with_table: Vec<&Passage> = r
            .passages
            .iter()
            .filter(|p| p.text.contains("19,00 €"))
            .collect();
        assert_eq!(
            with_table.len(),
            1,
            "the price list rides on one passage of the page"
        );
        assert!(
            with_table[0]
                .text
                .contains("Parktarife\n1 Tag — P3: 19,00 €"),
            "{}",
            with_table[0].text
        );
        assert!(
            !r.passages[0].text.contains("<p>"),
            "chunk text is plain text: {}",
            r.passages[0].text
        );
        let snippet = r.passages[0].to_json(&r.terms)["snippet"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!snippet.contains('<'), "{snippet}");
    }

    #[test]
    fn a_localized_page_carries_its_localized_tables() {
        let fake = Fake {
            sql_fn: Box::new(|sql, _| parking_rows(sql)),
            ..Fake::default()
        };
        let r = retrieve(
            &fake,
            &opts(json!({"query": "prix parking", "locale": "fr", "base_language": "de"})),
        )
        .unwrap();
        let p = &r.passages[0];
        assert!(
            p.text
                .contains("Tarifs de stationnement\n1 jour — P3: 19,00 €"),
            "{}",
            p.text
        );
        assert_eq!(p.locale.as_deref(), Some("fr"));
    }

    #[test]
    fn a_retrieval_reports_where_its_time_went() {
        let fake = Fake {
            sql_fn: Box::new(|sql, _| parking_rows(sql)),
            ..Fake::default()
        };
        let r = retrieve(
            &fake,
            &opts(json!({"query": "Parken", "expansions": ["Parktarife"]})),
        )
        .unwrap();
        let legs: Vec<&str> = r.timings["legs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["leg"].as_str().unwrap())
            .collect();
        assert_eq!(legs, vec!["hybrid", "expansions"]);
        assert_eq!(
            r.timings["reads"],
            json!(1),
            "one batched read for the page hits"
        );
        assert!(r.timings["total_ms"].is_number());
    }

    /// "Was kostet Parken für eine Woche?" on the real site: two AGB PDFs say
    /// "Parken" in every chunk and outrank the Parken page, whose week price
    /// is only in its table ("1 Wochentarif (8 Tage)").
    fn week_site() -> Fake {
        Fake {
            sql_fn: Box::new(|sql, _| {
                if sql.contains("HYBRID_SEARCH") {
                    let mut rows = Vec::new();
                    for (doc, n) in [("agb-p3", 3), ("agb-p11", 3)] {
                        for i in 0..n {
                            let mut r = chunk(&format!("/bap/downloads/{doc}.pdf"), doc, i,
                                "Allgemeine Einstellbedingungen: Das Parken erfolgt auf eigene Gefahr. Die Parkgebühr ist bei Ausfahrt zu entrichten.");
                            r["node_type"] = json!("raisin:Asset");
                            r["file_type"] = json!("application/pdf");
                            r["workspace_id"] = json!("assets");
                            r["fulltext_rank"] = json!(1);
                            rows.push(r);
                        }
                    }
                    rows.push(chunk(
                        "/bap/parken",
                        "parken",
                        0,
                        "Parken am Flughafen: drei Parkhäuser direkt am Terminal.",
                    ));
                    return Ok(rows);
                }
                if sql.contains("FROM 'stories'") {
                    return Ok(vec![json!({ "path": "/bap/parken", "properties": {
                        "title": "Title parken",
                        "content": [{ "element_type": "bap:Table", "tables": [{ "title": "Parktarife",
                            "data": { "header_row": true, "header_col": true, "rows": [
                                ["", "P3", "P11"], ["1 Tag", "19,00 €", "25,00 €"], ["1 Wochentarif (8 Tage)", "59,00 €", "79,00 €"]] } }] }]
                    }})]);
                }
                Ok(vec![])
            }),
            ..Fake::default()
        }
    }

    #[test]
    fn a_table_that_answers_lifts_its_page_above_documents_that_only_share_the_topic() {
        let fake = week_site();
        let o = opts(
            json!({ "query": "Was kostet Parken für eine Woche?", "limit": 4, "max_per_document": 2 }),
        );
        let r = retrieve(&fake, &o).unwrap();
        assert_eq!(
            r.passages[0].path,
            "/bap/parken",
            "{:?}",
            r.passages.iter().map(|p| &p.path).collect::<Vec<_>>()
        );
        assert!(
            r.passages[0]
                .text
                .contains("1 Wochentarif (8 Tage) — P3: 59,00 €"),
            "{}",
            r.passages[0].text
        );
    }

    #[test]
    fn confidence_needs_the_questions_words_in_the_top_passages() {
        let fake = week_site();
        let r = retrieve(
            &fake,
            &opts(json!({ "query": "Was kostet Parken für eine Woche?" })),
        )
        .unwrap();
        assert!(
            !confident(&r.passages, "Was kostet Parken für eine Woche?"),
            "'kostet' is nowhere"
        );
        assert!(
            confident(&r.passages[1..], "Parkgebühr bei Ausfahrt"),
            "the PDFs matched on their words"
        );
        assert!(!confident(&[], "Parken"));
    }
}
