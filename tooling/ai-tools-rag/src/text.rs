//! Turning a node into passage text, and a passage into a snippet. All pure.
//!
//! The engine hands back chunk text only for hits its VECTOR leg found: a hit
//! found by the full-text leg alone has no chunk, and its `chunk_text` is NULL.
//! The JavaScript `search-documents` dropped every such row ("a passage with no
//! text is dropped"), which quietly made the hybrid search a vector search for
//! answering purposes — exactly the "Geschäftsführer" news the lexical leg
//! finds and the vector leg does not. Those hits now get a passage cut from the
//! node's own text, around the words that matched.

use serde_json::Value;

/// Longest passage built from a node's own text (characters). Roughly one
/// engine chunk (512 tokens), so a lexical passage costs the prompt what a
/// vector passage does.
pub const WINDOW_CHARS: usize = 1500;

/// Snippet length (characters).
pub const SNIPPET_CHARS: usize = 240;

/// Property keys whose values are never prose: identifiers, references,
/// presentation settings, links.
const SKIP_KEYS: [&str; 34] = [
    "id",
    "uuid",
    "key",
    "ref",
    "type",
    "element_type",
    "node_type",
    "archetype",
    "variant",
    "style",
    "layout",
    "theme",
    "color",
    "colour",
    "icon",
    "size",
    "alignment",
    "align",
    "href",
    "url",
    "src",
    "link",
    "target",
    "slug",
    "path",
    "workspace",
    "mime_type",
    "file_type",
    "content_hash",
    "locale",
    "language",
    "status",
    "format",
    "file",
];

/// The prose of a node, one entry per text field, in document order: the
/// title first, then every string leaf that reads as text.
///
/// Engine-owned keys (`__…`) are skipped except `__extracted_text` (an uploaded
/// document's body), which goes LAST: it is the longest, and the fields before
/// it are the ones a page author wrote.
pub fn node_text(properties: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(t) = properties.get("title").and_then(Value::as_str) {
        push_text(&mut out, t);
    }
    if let Value::Object(map) = properties {
        for (k, child) in map {
            // The title is already first; the body goes last.
            if k != "title" && !skipped_key(k) {
                collect(child, &mut out);
            }
        }
    }
    if let Some(t) = properties.get("__extracted_text").and_then(Value::as_str) {
        push_text(&mut out, t);
    }
    out
}

fn skipped_key(k: &str) -> bool {
    k.starts_with("__") || SKIP_KEYS.contains(&k) || k.ends_with("_id") || k.ends_with("_url")
}

fn push_text(out: &mut Vec<String>, s: &str) {
    let plain = strip_html(s);
    let t = plain.trim();
    if t.is_empty() || !reads_as_text(t) || out.iter().any(|o| o == t) {
        return;
    }
    out.push(t.to_string());
}

fn collect(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => push_text(out, s),
        Value::Array(items) => {
            // A table's grid is rendered by `tables`, whole rows at a time; its
            // cells one by one ("2,00 €") are neither prose nor searchable.
            if is_grid(items) {
                return;
            }
            for i in items {
                collect(i, out);
            }
        }
        Value::Object(map) => {
            // A reference object is a pointer, not content.
            if map.contains_key("raisin:ref") {
                return;
            }
            for (k, child) in map {
                if !skipped_key(k) {
                    collect(child, out);
                }
            }
        }
        _ => {}
    }
}

/// Prose, as opposed to an identifier, a URL or a colour.
fn reads_as_text(s: &str) -> bool {
    if s.starts_with('/')
        || s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with('#')
    {
        return false;
    }
    let letters = s.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 2 {
        return false;
    }
    // "3f2c9a1e-…", "studio:Hero", "btn-primary": one token, no spaces, and
    // either digits or punctuation mixed in.
    if !s.contains(char::is_whitespace) {
        let odd = s
            .chars()
            .any(|c| c.is_ascii_digit() || c == ':' || c == '_' || c == '-');
        if odd && s.len() > 12 || s.contains(':') {
            return false;
        }
    }
    true
}

/// Words worth looking for in a text: the query's content words (four letters
/// or more, or an acronym such as "CEO") and every expansion term, longest
/// first. Lower-cased.
pub fn terms(query: &str, expansions: &[String]) -> Vec<String> {
    const STOP: [&str; 40] = [
        "what", "which", "where", "when", "with", "from", "that", "this", "have", "does", "about",
        "there", "their", "your", "would", "could", "should", "sind", "eine", "einer", "einen",
        "wird", "werden", "nicht", "oder", "gibt", "haben", "welche", "welcher", "wann", "warum",
        "wieso", "dans", "pour", "avec", "sont", "quel", "quelle", "comment", "est-ce",
    ];
    let mut out: Vec<String> = Vec::new();
    let add = |w: &str, out: &mut Vec<String>| {
        let lw = w.to_lowercase();
        if !lw.is_empty() && !out.contains(&lw) {
            out.push(lw);
        }
    };
    for e in expansions {
        add(e.trim(), &mut out);
    }
    for raw in query.split(|c: char| !(c.is_alphanumeric() || c == '-')) {
        let w = raw.trim_matches('-');
        let n = w.chars().count();
        let acronym =
            (2..=6).contains(&n) && w.chars().all(|c| c.is_uppercase() || c.is_ascii_digit());
        if (n >= 4 || acronym) && !STOP.contains(&w.to_lowercase().as_str()) {
            add(w, &mut out);
        }
    }
    for e in expansions {
        for w in e.split_whitespace() {
            if w.chars().count() >= 4 {
                add(w, &mut out);
            }
        }
    }
    out.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
    out
}

/// Char index of the first place any term occurs in `text`, case-insensitive.
fn first_match(text: &str, terms: &[String]) -> Option<usize> {
    if terms.is_empty() {
        return None;
    }
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    // `to_lowercase` can change the char count (e.g. 'İ'); fall back to the
    // start rather than mis-slice when it does.
    if lower.len() != text.chars().count() {
        return None;
    }
    let hay: String = lower.iter().collect();
    let mut best: Option<usize> = None;
    for t in terms {
        if let Some(byte) = hay.find(t.as_str()) {
            let at = hay[..byte].chars().count();
            best = Some(best.map_or(at, |b: usize| b.min(at)));
        }
    }
    best
}

/// Cut `chars[from..]` to at most `max` chars, ending on whitespace when one is
/// near the end.
fn cut(chars: &[char], from: usize, max: usize) -> (String, bool) {
    let end = (from + max).min(chars.len());
    let mut stop = end;
    if end < chars.len() {
        if let Some(ws) = chars[from..end].iter().rposition(|c| c.is_whitespace()) {
            if ws > max / 2 {
                stop = from + ws;
            }
        }
    }
    (
        chars[from..stop]
            .iter()
            .collect::<String>()
            .trim()
            .to_string(),
        stop < chars.len(),
    )
}

/// Start of the sentence or line containing char index `at`, at most `back`
/// chars before it.
fn start_before(chars: &[char], at: usize, back: usize) -> usize {
    let floor = at.saturating_sub(back);
    let mut i = at;
    while i > floor {
        let prev = chars[i - 1];
        if prev == '\n' || ((prev == ' ') && i >= 2 && matches!(chars[i - 2], '.' | '!' | '?')) {
            return i;
        }
        i -= 1;
    }
    // No boundary in reach: start on a word.
    let mut i = floor;
    while i < at && i > 0 && !chars[i - 1].is_whitespace() {
        i += 1;
    }
    i
}

/// A passage of at most `max` chars from `segments`, starting shortly before
/// the first matched term; the opening of the text when nothing matches. Pure.
pub fn window(segments: &[String], terms: &[String], max: usize) -> String {
    let joined = segments.join("\n");
    let chars: Vec<char> = joined.chars().collect();
    if chars.len() <= max {
        return joined.trim().to_string();
    }
    let from = match first_match(&joined, terms) {
        Some(at) => start_before(&chars, at, max / 4),
        None => 0,
    };
    let (text, more) = cut(&chars, from, max);
    let text = if from > 0 { format!("…{text}") } else { text };
    if more {
        format!("{text}…")
    } else {
        text
    }
}

/// A snippet of at most [`SNIPPET_CHARS`] around the first matched term. Pure.
pub fn snippet(text: &str, terms: &[String]) -> String {
    let flat: String = strip_html(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let chars: Vec<char> = flat.chars().collect();
    if chars.len() <= SNIPPET_CHARS {
        return flat;
    }
    let from = match first_match(&flat, terms) {
        Some(at) => start_before(&chars, at, 80),
        None => 0,
    };
    let (body, more) = cut(&chars, from, SNIPPET_CHARS - 2);
    let body = body.trim_start_matches('…').to_string();
    let body = if from > 0 { format!("…{body}") } else { body };
    if more {
        format!("{body}…")
    } else {
        body
    }
}

/// Plain text from a rich-text value: tags removed (block-level ones become
/// line breaks), the common entities decoded, `**bold**` markers dropped. Pure.
///
/// Rich-text fields hold HTML, and so did the snippets built from them — a
/// site rendering a snippet as text showed `<p>` to the visitor, and one
/// rendering it as HTML trusted markup that came out of a search.
pub fn strip_html(s: &str) -> String {
    if !s.contains('<') && !s.contains('&') && !s.contains("**") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            let mut closed = false;
            for t in chars.by_ref() {
                if t == '>' {
                    closed = true;
                    break;
                }
                tag.push(t);
            }
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            if !closed || name.is_empty() {
                // A lone '<' ("a < b") is text.
                out.push('<');
                out.push_str(&tag);
                if closed {
                    out.push('>');
                }
                continue;
            }
            match name.as_str() {
                "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                | "ul" | "ol" | "table" => {
                    if !out.ends_with('\n') && !out.is_empty() {
                        out.push('\n');
                    }
                }
                "td" | "th" => out.push(' '),
                _ => {}
            }
        } else if c == '&' {
            let mut ent = String::new();
            while let Some(&n) = chars.peek() {
                if n == ';' || ent.len() > 8 || n.is_whitespace() || n == '&' {
                    break;
                }
                ent.push(n);
                chars.next();
            }
            let semi = chars.peek() == Some(&';');
            let decoded = match ent.as_str() {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" | "#160" => Some(' '),
                "shy" | "#173" => Some('\u{0}'),
                _ => ent
                    .strip_prefix('#')
                    .and_then(|n| n.parse::<u32>().ok())
                    .and_then(char::from_u32),
            };
            match (decoded, semi) {
                (Some(d), true) => {
                    chars.next();
                    if d != '\u{0}' {
                        out.push(d);
                    }
                }
                _ => {
                    out.push('&');
                    out.push_str(&ent);
                }
            }
        } else {
            out.push(c);
        }
    }
    out.replace("**", "")
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// An array of rows, each an array of scalars: a table's grid. Pure.
fn is_grid(items: &[Value]) -> bool {
    items.len() >= 2
        && items.iter().all(|r| {
            r.as_array().is_some_and(|cells| {
                !cells.is_empty()
                    && cells
                        .iter()
                        .all(|c| c.is_string() || c.is_number() || c.is_null())
            })
        })
}

fn cell(v: &Value) -> String {
    match v {
        Value::String(s) => strip_html(s).trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// One table as lines a model can read: every row restates its column
/// headers, so a passage cut anywhere still says which column a price is. Pure.
fn render_grid(rows: &[Value], header_row: bool, header_col: bool) -> Vec<String> {
    let grid: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            r.as_array()
                .map(|c| c.iter().map(cell).collect())
                .unwrap_or_default()
        })
        .collect();
    let (head, body) = if header_row && grid.len() > 1 {
        (Some(&grid[0]), &grid[1..])
    } else {
        (None, &grid[..])
    };
    let mut out = Vec::new();
    for row in body {
        let mut parts: Vec<String> = Vec::new();
        let mut label = String::new();
        for (j, c) in row.iter().enumerate() {
            if header_col && j == 0 {
                label = c.clone();
                continue;
            }
            if c.is_empty() || c == "–" || c == "-" {
                continue;
            }
            match head.and_then(|h| h.get(j)).filter(|h| !h.is_empty()) {
                Some(h) => parts.push(format!("{h}: {c}")),
                None => parts.push(c.clone()),
            }
        }
        if parts.is_empty() {
            continue;
        }
        out.push(if label.is_empty() {
            parts.join("; ")
        } else {
            format!("{label} — {}", parts.join("; "))
        });
    }
    out
}

/// The tables of a node — any `{rows: [[…]]}` object or bare grid, anywhere in
/// its properties — each rendered as one text block, its caption fields
/// (`title`, `subtitle`, `note`) first. Pure.
///
/// A page's indexed chunk carries its prose; its tariff tables are structured
/// block content the chunker never saw. Asked for the parking price, `ask`
/// answered "no concrete price" because the price only exists here.
pub fn tables(properties: &Value) -> Vec<String> {
    let mut out = Vec::new();
    find_tables(properties, &mut Vec::new(), &mut out);
    out
}

fn caption_of(map: &serde_json::Map<String, Value>) -> Vec<String> {
    ["title", "subtitle", "caption", "heading", "note"]
        .iter()
        .filter_map(|k| map.get(*k).and_then(Value::as_str))
        .map(strip_html)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn find_tables(v: &Value, captions: &mut Vec<Vec<String>>, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            if map.contains_key("raisin:ref") {
                return;
            }
            // `{rows: [[…]], header_row, header_col}` — the table control's shape.
            if let Some(rows) = map
                .get("rows")
                .and_then(Value::as_array)
                .filter(|r| is_grid(r))
            {
                let header_row = map
                    .get("header_row")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let header_col = map
                    .get("header_col")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut block: Vec<String> = captions.last().cloned().unwrap_or_default();
                block.extend(caption_of(map));
                block.extend(render_grid(rows, header_row, header_col));
                out.push(block.join("\n"));
                return;
            }
            captions.push(caption_of(map));
            for (k, child) in map {
                if !k.starts_with("__") {
                    find_tables(child, captions, out);
                }
            }
            captions.pop();
        }
        Value::Array(items) => {
            if is_grid(items) {
                let mut block: Vec<String> = captions.last().cloned().unwrap_or_default();
                block.extend(render_grid(items, true, false));
                out.push(block.join("\n"));
                return;
            }
            for i in items {
                find_tables(i, captions, out);
            }
        }
        _ => {}
    }
}

/// Tables worth adding to a passage: those mentioning a term first, then the
/// rest, while they fit in `budget` characters — and never one whose rows the
/// passage already carries. Pure.
pub fn pick_tables(all: &[String], passage: &str, terms: &[String], budget: usize) -> Vec<String> {
    let lower_passage = passage.to_lowercase();
    let mut ranked: Vec<(bool, &String)> = all
        .iter()
        .filter(|t| {
            let probe = t.lines().last().unwrap_or("").to_lowercase();
            probe.is_empty() || !lower_passage.contains(&probe)
        })
        .map(|t| {
            let lt = t.to_lowercase();
            (terms.iter().any(|term| lt.contains(term.as_str())), t)
        })
        .collect();
    ranked.sort_by_key(|(hit, _)| !*hit);
    let mut used = 0;
    let mut out = Vec::new();
    for (_, t) in ranked {
        let n = t.chars().count();
        if used + n > budget {
            if out.is_empty() {
                out.push(t.chars().take(budget).collect::<String>() + "…");
            }
            break;
        }
        used += n;
        out.push(t.clone());
    }
    out
}

/// A link the site can use, when the node carries one: the locale's own URL
/// field (`url_fr`), then `url`, then `href`. Only absolute paths and
/// http(s) URLs qualify.
pub fn url_hint(properties: &Value, locale: Option<&str>) -> Option<String> {
    let mut keys: Vec<String> = Vec::new();
    if let Some(l) = locale {
        keys.push(format!("url_{}", &l[..2]));
        keys.push(format!("url_{}", l.replace('-', "_").to_lowercase()));
    }
    keys.push("url".into());
    keys.push("href".into());
    keys.iter()
        .filter_map(|k| properties.get(k.as_str()).and_then(Value::as_str))
        .map(str::trim)
        .find(|u| u.starts_with('/') || u.starts_with("http://") || u.starts_with("https://"))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_page_reads_as_its_prose_not_its_settings() {
        let props = json!({
            "title": "Geschäftsleitung",
            "slug": "geschaeftsleitung",
            "content": [
                { "element_type": "studio:Hero", "id": "3f2c9a1e-11aa-4bb2-9c1d-000000000001", "heading": "Unser Team" },
                { "element_type": "studio:Text", "body": "Max Muster ist Geschäftsführer der Flughafen AG.", "style": "wide",
                  "image": { "raisin:ref": "/bap/max.jpg" } },
            ],
            "url_fr": "/fr/direction",
            "__search_vector": "x",
        });
        assert_eq!(
            node_text(&props),
            vec![
                "Geschäftsleitung",
                "Unser Team",
                "Max Muster ist Geschäftsführer der Flughafen AG."
            ]
        );
        assert_eq!(
            url_hint(&props, Some("fr")).as_deref(),
            Some("/fr/direction")
        );
        assert_eq!(url_hint(&props, None), None);
    }

    #[test]
    fn an_uploaded_document_reads_as_its_body_last() {
        let props = json!({ "title": "Jahresbericht", "__extracted_text": "Der Geschäftsführer berichtet.", "file_type": "application/pdf" });
        assert_eq!(
            node_text(&props),
            vec!["Jahresbericht", "Der Geschäftsführer berichtet."]
        );
    }

    #[test]
    fn terms_keep_acronyms_and_drop_filler() {
        let t = terms("wer ist der CEO?", &["Geschäftsführer".to_string()]);
        assert_eq!(t, vec!["geschäftsführer".to_string(), "ceo".to_string()]);
    }

    #[test]
    fn a_window_starts_near_the_match() {
        let filler = "Lorem ipsum dolor sit amet. ".repeat(200);
        let segs = vec![format!(
            "{filler}Max Muster ist Geschäftsführer der Flughafen AG. {filler}"
        )];
        let w = window(&segs, &["geschäftsführer".to_string()], 300);
        assert!(w.contains("Geschäftsführer"), "{w}");
        assert!(w.chars().count() <= 302);
        assert!(w.starts_with('…') && w.ends_with('…'));
        let head = window(&segs, &["nowhere".to_string()], 300);
        assert!(head.starts_with("Lorem"));
    }

    #[test]
    fn a_snippet_is_short_and_centred() {
        let text = format!(
            "{} Max Muster leitet als CEO die Flughafen AG.",
            "Einleitung. ".repeat(60)
        );
        let s = snippet(&text, &["ceo".to_string()]);
        assert!(s.contains("CEO"), "{s}");
        assert!(s.chars().count() <= SNIPPET_CHARS + 2);
    }

    /// The bap:Table block's shape: a composite of tables, each with a caption
    /// and a `{header_row, header_col, rows}` grid.
    fn parking_page() -> Value {
        json!({
            "title": "Parken am Flughafen",
            "content": [
                { "element_type": "bap:Text", "body": "<p>Parken direkt am <strong>Terminal</strong>.</p>" },
                { "element_type": "bap:Table", "tables": [{
                    "title": "Parktarife Kurzzeitparken",
                    "subtitle": "gültig ab 01.02.2026",
                    "note": "<p>P3 kann <em>nicht</em> online reserviert werden.</p>",
                    "data": { "header_row": true, "header_col": true,
                              "rows": [["", "P3", "P11"], ["bis 30 Minuten", "–", "2,00 €"], ["1 Tag", "**19,00 €**", "25,00 €"]] }
                }]}
            ]
        })
    }

    #[test]
    fn a_table_renders_as_rows_that_name_their_columns() {
        let t = tables(&parking_page());
        assert_eq!(t.len(), 1);
        assert_eq!(
            t[0],
            "Parktarife Kurzzeitparken\ngültig ab 01.02.2026\nP3 kann nicht online reserviert werden.\n\
             bis 30 Minuten — P11: 2,00 €\n1 Tag — P3: 19,00 €; P11: 25,00 €"
        );
    }

    #[test]
    fn table_cells_are_not_scattered_through_the_prose() {
        let text = node_text(&parking_page()).join("\n");
        assert!(!text.contains("25,00"), "{text}");
        assert!(text.contains("Parken direkt am Terminal."), "{text}");
    }

    #[test]
    fn tables_that_mention_the_question_go_first_within_the_budget() {
        let all = vec![
            "Frachttarife\nKilo — 1 €".to_string(),
            "Parktarife\n1 Tag — P3: 19 €".to_string(),
        ];
        let picked = pick_tables(&all, "Parken am Flughafen", &["parktarife".to_string()], 30);
        assert_eq!(picked, vec!["Parktarife\n1 Tag — P3: 19 €".to_string()]);
        assert!(
            pick_tables(&all, "Parktarife\n1 Tag — P3: 19 €", &[], 500)
                .iter()
                .all(|t| !t.starts_with("Parktarife")),
            "a table the passage already carries is not added twice"
        );
    }

    #[test]
    fn html_becomes_plain_text() {
        assert_eq!(
            strip_html("<p>Parken &amp; Reisen</p><ul><li>P3</li><li>P11&nbsp;Nord</li></ul>"),
            "Parken & Reisen\nP3\nP11 Nord"
        );
        assert_eq!(strip_html("a < b &unknown; c"), "a < b &unknown; c");
        assert_eq!(strip_html("Preis: **19,00 €**"), "Preis: 19,00 €");
        assert_eq!(
            snippet("<p>Max Muster ist <b>CEO</b>.</p>", &[]),
            "Max Muster ist CEO."
        );
    }
}
