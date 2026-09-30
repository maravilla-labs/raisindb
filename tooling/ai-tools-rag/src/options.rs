//! The retrieval options both handlers share, parsed and validated once.
//!
//! Every option added here is OPTIONAL, and its absence reproduces the
//! behaviour of the JavaScript functions this replaces — with one deliberate
//! exception, documented on [`Kind`]: image assets are no longer answer
//! passages unless asked for.

use serde_json::Value;

/// Passages to return when the caller doesn't say.
pub const DEFAULT_LIMIT: usize = 8;

/// Hard ceiling on returned passages. Not politeness: every leg over-fetches
/// 20x its limit before row-level security filters.
pub const MAX_LIMIT: usize = 50;

/// How many passages each search leg draws before fusion, filtering and
/// per-document capping cut it down to `limit`.
///
/// Forty, because the failure this exists for was measured at eight: on a real
/// site, "wer ist der CEO?" put three images and a garbled PDF above the right
/// page, so a top-8 draw handed the model four useless passages of eight. A
/// wider draw costs one bigger index walk, not more round trips.
pub const DEFAULT_CANDIDATES: usize = 40;

/// Ceiling on the candidate window. The engine draws 20x the window per leg
/// (capped at 2000), so past 100 the leg cap, not this number, decides.
pub const MAX_CANDIDATES: usize = 100;

/// The only breadth spelling the engine accepts. `'*'` and `'ALL'` are
/// rejected on purpose, so don't "helpfully" translate them here.
pub const DEFAULT_SCOPE: &str = "ALL READABLE";

/// What a hit IS, for the purpose of answering from it.
///
/// Derived from columns every node has — `node_type` and, for
/// `raisin:Asset`, `file_type` (with the file extension as a fallback) — so it
/// needs no schema cooperation from the site.
///
/// **The default excludes [`Kind::Image`].** An image asset carries no answer
/// text: it reaches a text search through its caption, alt text or OCR, and a
/// short caption embeds closer to a short question than any real page does.
/// Measured on a production-like site: "wer ist der CEO?" returned three
/// portraits above the page that names the CEO. That is the one behaviour
/// change for callers that pass nothing new; `include_kinds: ["all"]` (or a
/// list naming `image`) restores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Anything that is not a `raisin:Asset`: pages, records, notes, entries.
    Page,
    /// A `raisin:Asset` that is neither an image, a video nor audio: PDFs,
    /// office files, text files, and assets with no recorded type.
    Document,
    /// A `raisin:Asset` whose type is `image/*` (SVG included).
    Image,
    /// A `raisin:Asset` whose type is `video/*` or `audio/*`.
    Media,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Page, Kind::Document, Kind::Image, Kind::Media];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Page => "page",
            Kind::Document => "document",
            Kind::Image => "image",
            Kind::Media => "media",
        }
    }

    fn parse(raw: &str) -> Result<Vec<Kind>, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "page" | "pages" => Ok(vec![Kind::Page]),
            "document" | "documents" => Ok(vec![Kind::Document]),
            "image" | "images" => Ok(vec![Kind::Image]),
            "media" => Ok(vec![Kind::Media]),
            "all" => Ok(Kind::ALL.to_vec()),
            other => Err(format!(
                "include_kinds: unknown kind '{other}'; use page, document, image, media or all"
            )),
        }
    }
}

/// The node type of an uploaded file.
pub const ASSET_TYPE: &str = "raisin:Asset";

/// The kind of a hit, from its node type, file type and path. Pure.
pub fn kind_of(node_type: &str, file_type: &str, path: &str) -> Kind {
    if node_type != ASSET_TYPE {
        return Kind::Page;
    }
    let mime = file_type.trim().to_ascii_lowercase();
    if mime.starts_with("image/") {
        return Kind::Image;
    }
    if mime.starts_with("video/") || mime.starts_with("audio/") {
        return Kind::Media;
    }
    if mime.is_empty() {
        let last = path.rsplit('/').next().unwrap_or("");
        let ext = last
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        const IMAGES: [&str; 9] = [
            "jpg", "jpeg", "png", "webp", "gif", "avif", "svg", "tif", "tiff",
        ];
        const MEDIA: [&str; 7] = ["mp4", "mov", "webm", "mp3", "wav", "m4a", "ogg"];
        if IMAGES.contains(&ext.as_str()) {
            return Kind::Image;
        }
        if MEDIA.contains(&ext.as_str()) {
            return Kind::Media;
        }
    }
    Kind::Document
}

/// One path prefix, optionally bound to one workspace (`"assets:/bap"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathScope {
    pub workspace: Option<String>,
    /// Normalised: leading `/`, no trailing `/` (except the root itself).
    pub prefix: String,
}

impl PathScope {
    fn parse(raw: &str) -> Result<PathScope, String> {
        let raw = raw.trim();
        let (workspace, path) = match raw.find(":/") {
            Some(i) if i > 0 => (Some(raw[..i].trim().to_string()), &raw[i + 1..]),
            _ => (None, raw),
        };
        if !path.starts_with('/') {
            return Err(format!(
                "paths: '{raw}' is not a path prefix; write it as '/site' or 'workspace:/site'"
            ));
        }
        if let Some(ws) = &workspace {
            if !is_workspace_name(ws) {
                return Err(format!("paths: '{ws}' is not a workspace name"));
            }
        }
        let trimmed = path.trim_end_matches('/');
        let prefix = if trimmed.is_empty() {
            "/".to_string()
        } else {
            trimmed.to_string()
        };
        Ok(PathScope { workspace, prefix })
    }

    /// Does a hit at `path` in `workspace` fall under this prefix? Pure.
    pub fn admits(&self, workspace: &str, path: &str) -> bool {
        if let Some(ws) = &self.workspace {
            if !ws.eq_ignore_ascii_case(workspace) {
                return false;
            }
        }
        self.prefix == "/" || path == self.prefix || path.starts_with(&format!("{}/", self.prefix))
    }
}

/// A workspace name we are willing to put in a `FROM` clause.
pub fn is_workspace_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}

/// `de`, `fr`, `de-CH`: what a locale column and a language argument accept.
fn is_locale(s: &str) -> bool {
    let b = s.as_bytes();
    let lang = b.len() >= 2 && b[..2].iter().all(u8::is_ascii_lowercase);
    match b.len() {
        2 => lang,
        5 => lang && (b[2] == b'-' || b[2] == b'_') && b[3..].iter().all(u8::is_ascii_alphabetic),
        _ => false,
    }
}

/// Everything retrieval needs.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub query: String,
    /// The `workspaces =>` scope, in the engine's grammar.
    pub scope: String,
    pub limit: usize,
    pub candidates: usize,
    pub max_distance: Option<f64>,
    /// Empty = no path restriction.
    pub paths: Vec<PathScope>,
    pub kinds: Vec<Kind>,
    pub node_types: Vec<String>,
    pub exclude_node_types: Vec<String>,
    /// Return titles, passages and URL hints from this locale's translation
    /// overlays where one exists.
    pub locale: Option<String>,
    /// The language base content is written (and full-text indexed) in. Unset:
    /// the repository default, which is what the engine uses anyway.
    pub base_language: Option<String>,
    /// Extra full-text analyzers to search the base content with.
    pub fulltext_languages: Vec<String>,
    /// Extra terms searched on the lexical leg only (see `ask`'s expansion).
    pub expansions: Vec<String>,
    /// At most this many passages per document; 0 = no cap.
    pub max_per_document: usize,
}

fn strings(v: Option<&Value>, field: &str) -> Result<Vec<String>, String> {
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| match i {
                Value::String(s) => Ok(s.trim().to_string()),
                _ => Err(format!("{field}: every entry must be a string")),
            })
            .filter(|r| r.as_ref().map(|s| !s.is_empty()).unwrap_or(true))
            .collect(),
        Some(_) => Err(format!("{field}: expected a string or a list of strings")),
    }
}

fn int(v: Option<&Value>) -> Option<i64> {
    v.and_then(|v| v.as_f64())
        .filter(|f| f.is_finite())
        .map(|f| f.floor() as i64)
}

fn opt_locale(v: Option<&Value>, field: &str) -> Result<Option<String>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) if is_locale(s.trim()) => Ok(Some(s.trim().to_string())),
        Some(other) => Err(format!(
            "{field}: expected a language code such as 'de' or 'fr', got {other}"
        )),
    }
}

fn node_type_list(v: Option<&Value>, field: &str) -> Result<Vec<String>, String> {
    let list = strings(v, field)?;
    for t in &list {
        if !t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '.'))
        {
            return Err(format!("{field}: '{t}' is not a node type name"));
        }
    }
    Ok(list)
}

impl SearchOptions {
    /// Parse the shared options from a handler input, with `query` supplied by
    /// the caller (it is `query` for search-documents and `question` for ask).
    pub fn parse(input: &Value, query: &str) -> Result<SearchOptions, String> {
        let get = |k: &str| input.get(k);

        // `workspaces`: the tool schema says string, and a list is accepted
        // too — a site writing `['stories', 'assets']` used to get an
        // UNSCOPED search, because anything that was not a string silently fell
        // back to 'ALL READABLE'.
        let scope = match get("workspaces") {
            None | Some(Value::Null) => DEFAULT_SCOPE.to_string(),
            Some(Value::String(s)) if s.trim().is_empty() => DEFAULT_SCOPE.to_string(),
            Some(Value::String(s)) => s.trim().to_string(),
            Some(Value::Array(_)) => {
                let list = strings(get("workspaces"), "workspaces")?;
                if list.is_empty() {
                    return Err("workspaces: the list is empty; name at least one workspace".into());
                }
                list.join(", ")
            }
            Some(_) => return Err("workspaces: expected a string or a list of strings".into()),
        };

        let limit = int(get("limit"))
            .map(|l| l.clamp(1, MAX_LIMIT as i64) as usize)
            .unwrap_or(DEFAULT_LIMIT);
        let candidates = int(get("candidates"))
            .map(|c| c.clamp(1, MAX_CANDIDATES as i64) as usize)
            .unwrap_or(DEFAULT_CANDIDATES)
            .max(limit);

        let max_distance = get("max_distance")
            .and_then(Value::as_f64)
            .filter(|d| d.is_finite());

        let mut paths = Vec::new();
        for p in strings(get("paths"), "paths")? {
            paths.push(PathScope::parse(&p)?);
        }
        // An unqualified root admits everything: the filter would be a no-op.
        if paths
            .iter()
            .any(|p| p.workspace.is_none() && p.prefix == "/")
        {
            paths.clear();
        }

        let kinds = match get("include_kinds") {
            None | Some(Value::Null) => vec![Kind::Page, Kind::Document, Kind::Media],
            v => {
                let mut out: Vec<Kind> = Vec::new();
                for k in strings(v, "include_kinds")? {
                    for kind in Kind::parse(&k)? {
                        if !out.contains(&kind) {
                            out.push(kind);
                        }
                    }
                }
                if out.is_empty() {
                    return Err("include_kinds: the list is empty; name at least one kind".into());
                }
                out
            }
        };

        let locale = opt_locale(get("locale"), "locale")?;
        let base_language =
            opt_locale(get("base_language"), "base_language")?.map(|l| l[..2].to_string());
        let mut fulltext_languages: Vec<String> = Vec::new();
        for l in strings(get("fulltext_languages"), "fulltext_languages")? {
            if !is_locale(&l) {
                return Err(format!("fulltext_languages: '{l}' is not a language code"));
            }
            let l = l[..2].to_string();
            if Some(&l) != base_language.as_ref() && !fulltext_languages.contains(&l) {
                fulltext_languages.push(l);
            }
        }

        let expansions = strings(get("expansions"), "expansions")?;
        let max_per_document = int(get("max_per_document"))
            .map(|n| n.max(0) as usize)
            .unwrap_or(0);

        Ok(SearchOptions {
            query: query.trim().to_string(),
            scope,
            limit,
            candidates,
            max_distance,
            paths,
            kinds,
            node_types: node_type_list(get("node_types"), "node_types")?,
            exclude_node_types: node_type_list(get("exclude_node_types"), "exclude_node_types")?,
            locale,
            base_language,
            fulltext_languages,
            expansions: sanitize_terms(&expansions, query),
            max_per_document,
        })
    }

    /// Is `kind` admitted?
    pub fn admits_kind(&self, kind: Kind) -> bool {
        self.kinds.contains(&kind)
    }

    /// The locale to read overlays in, unless it IS the base language.
    pub fn overlay_locale(&self) -> Option<&str> {
        match (&self.locale, &self.base_language) {
            (Some(l), Some(b)) if l[..2] == b[..] => None,
            (Some(l), _) => Some(l.as_str()),
            _ => None,
        }
    }

    /// The residual `WHERE` for one search, with its parameters numbered from
    /// `first`. `None` when nothing restricts the search.
    ///
    /// This is how the new scoping reaches the ENGINE rather than a post-filter
    /// over a small top-k: the table function evaluates a `WHERE` above it as a
    /// residual inside its own fetch loop, counts rows only once they pass it,
    /// and redraws wider when too many are dropped. So `paths => ['/bap']`
    /// returns the best `/bap` passages, not whichever `/bap` passages happened
    /// to survive a global top 40. (The engine deliberately has no
    /// `path_prefix =>` or `node_types =>` argument: "the universe is an
    /// argument, everything else is WHERE".)
    ///
    /// Every caller-controlled value is a bound parameter. The constants
    /// (`raisin:Asset`, the MIME prefixes) are the only literals.
    pub fn where_clause(&self, first: usize) -> (Option<String>, Vec<Value>) {
        let mut params: Vec<Value> = Vec::new();
        let mut next = first;
        let mut bind = |v: String, params: &mut Vec<Value>| {
            params.push(Value::String(v));
            let p = format!("${next}");
            next += 1;
            p
        };
        let mut conj: Vec<String> = Vec::new();

        if !self.paths.is_empty() {
            let mut any: Vec<String> = Vec::new();
            for scope in &self.paths {
                let under = if scope.prefix == "/" {
                    None
                } else {
                    let exact = bind(scope.prefix.clone(), &mut params);
                    let below = bind(format!("{}/%", scope.prefix), &mut params);
                    Some(format!("(path = {exact} OR path LIKE {below})"))
                };
                match (&scope.workspace, under) {
                    (Some(ws), Some(under)) => {
                        let w = bind(ws.clone(), &mut params);
                        any.push(format!("(workspace_id = {w} AND {under})"));
                    }
                    (Some(ws), None) => {
                        let w = bind(ws.clone(), &mut params);
                        any.push(format!("workspace_id = {w}"));
                    }
                    (None, Some(under)) => any.push(under),
                    (None, None) => {}
                }
            }
            if !any.is_empty() {
                conj.push(format!("({})", any.join(" OR ")));
            }
        }

        if let Some(kinds) = kind_predicate(&self.kinds) {
            conj.push(kinds);
        }

        if !self.node_types.is_empty() {
            let any: Vec<String> = self
                .node_types
                .iter()
                .map(|t| format!("node_type = {}", bind(t.clone(), &mut params)))
                .collect();
            conj.push(format!("({})", any.join(" OR ")));
        }
        for t in &self.exclude_node_types {
            conj.push(format!("node_type <> {}", bind(t.clone(), &mut params)));
        }

        if conj.is_empty() {
            (None, params)
        } else {
            (Some(conj.join(" AND ")), params)
        }
    }

    /// The same restrictions, applied exactly, to one hit. Pure.
    ///
    /// The residual is the engine's filter; this is the precise one. They differ
    /// only where SQL `LIKE` over-matches (`_` in a prefix) or `file_type` is
    /// missing and the extension decides — both harmless over-inclusions the
    /// engine returns and this drops.
    pub fn admits(&self, workspace: &str, path: &str, node_type: &str, kind: Kind) -> bool {
        (self.paths.is_empty() || self.paths.iter().any(|p| p.admits(workspace, path)))
            && self.admits_kind(kind)
            && (self.node_types.is_empty() || self.node_types.iter().any(|t| t == node_type))
            && !self.exclude_node_types.iter().any(|t| t == node_type)
    }
}

const FT: &str = "properties->>'file_type'";

/// The SQL for "is one of these kinds", or `None` when every kind is admitted.
fn kind_predicate(kinds: &[Kind]) -> Option<String> {
    if Kind::ALL.iter().all(|k| kinds.contains(k)) {
        return None;
    }
    let image = format!("{FT} LIKE 'image/%'");
    let media = format!("({FT} LIKE 'video/%' OR {FT} LIKE 'audio/%')");
    // The default — everything but images — is its own shorter form.
    if kinds.len() == 3 && !kinds.contains(&Kind::Image) {
        return Some(format!(
            "(node_type <> '{ASSET_TYPE}' OR {FT} IS NULL OR NOT ({image}))"
        ));
    }
    let parts: Vec<String> = kinds
        .iter()
        .map(|k| match k {
            Kind::Page => format!("node_type <> '{ASSET_TYPE}'"),
            Kind::Image => format!("(node_type = '{ASSET_TYPE}' AND {image})"),
            Kind::Media => format!("(node_type = '{ASSET_TYPE}' AND {media})"),
            Kind::Document => format!(
                "(node_type = '{ASSET_TYPE}' AND ({FT} IS NULL OR NOT ({image} OR {FT} LIKE 'video/%' OR {FT} LIKE 'audio/%')))"
            ),
        })
        .collect();
    Some(format!("({})", parts.join(" OR ")))
}

/// Expansion terms made safe for a lexical query: plain words only (the
/// full-text query parser treats `:`, quotes and brackets as syntax), no
/// repeats of the query itself, at most six. Pure.
pub fn sanitize_terms(terms: &[String], query: &str) -> Vec<String> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<String> = Vec::new();
    for t in terms {
        let clean: String = t
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '\'' {
                    c
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let clean: String = clean.chars().take(60).collect();
        let key = clean.to_lowercase();
        if clean.chars().count() < 2 || key == q || out.iter().any(|o| o.to_lowercase() == key) {
            continue;
        }
        out.push(clean);
        if out.len() == 6 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: Value) -> SearchOptions {
        SearchOptions::parse(&v, "q").unwrap()
    }

    #[test]
    fn nothing_new_means_the_old_defaults() {
        let o = parse(json!({}));
        assert_eq!(o.scope, "ALL READABLE");
        assert_eq!(o.limit, 8);
        assert!(o.paths.is_empty());
        assert_eq!(o.kinds, vec![Kind::Page, Kind::Document, Kind::Media]);
        assert!(o.locale.is_none());
    }

    #[test]
    fn a_workspace_list_is_a_scope_not_a_silent_fallback() {
        assert_eq!(
            parse(json!({"workspaces": ["stories", "assets"]})).scope,
            "stories, assets"
        );
        assert_eq!(
            parse(json!({"workspaces": "docs, handbook"})).scope,
            "docs, handbook"
        );
    }

    #[test]
    fn path_prefixes_normalise_and_bind_to_a_workspace_when_qualified() {
        let o = parse(json!({"paths": ["/bap/", "assets:/bap"]}));
        assert_eq!(
            o.paths[0],
            PathScope {
                workspace: None,
                prefix: "/bap".into()
            }
        );
        assert_eq!(
            o.paths[1],
            PathScope {
                workspace: Some("assets".into()),
                prefix: "/bap".into()
            }
        );
        assert!(o.paths[0].admits("stories", "/bap"));
        assert!(o.paths[0].admits("stories", "/bap/news/ceo"));
        assert!(
            !o.paths[0].admits("stories", "/bapx/news"),
            "a prefix is a path segment, not a string prefix"
        );
        assert!(!o.paths[1].admits("stories", "/bap/x"));
        assert!(parse(json!({"paths": ["/"]})).paths.is_empty());
        assert!(SearchOptions::parse(&json!({"paths": ["bap"]}), "q").is_err());
    }

    #[test]
    fn paths_reach_the_engine_as_a_bound_residual() {
        let o = parse(json!({"paths": ["/bap", "assets:/bap"]}));
        let (sql, params) = o.where_clause(3);
        let sql = sql.unwrap();
        assert!(sql.starts_with("((path = $3 OR path LIKE $4) OR (workspace_id = $7 AND (path = $5 OR path LIKE $6)))"), "{sql}");
        assert_eq!(
            params,
            vec![
                json!("/bap"),
                json!("/bap/%"),
                json!("/bap"),
                json!("/bap/%"),
                json!("assets")
            ]
        );
    }

    #[test]
    fn images_are_excluded_by_default_and_all_brings_them_back() {
        let (sql, _) = parse(json!({})).where_clause(3);
        assert_eq!(
            sql.unwrap(),
            "(node_type <> 'raisin:Asset' OR properties->>'file_type' IS NULL OR NOT (properties->>'file_type' LIKE 'image/%'))"
        );
        let (sql, _) = parse(json!({"include_kinds": ["all"]})).where_clause(3);
        assert!(sql.is_none());
    }

    #[test]
    fn include_kinds_names_exactly_what_is_wanted() {
        let o = parse(json!({"include_kinds": ["page", "document"]}));
        assert!(o.admits_kind(Kind::Page) && o.admits_kind(Kind::Document));
        assert!(!o.admits_kind(Kind::Image) && !o.admits_kind(Kind::Media));
        let (sql, _) = o.where_clause(3);
        let sql = sql.unwrap();
        assert!(sql.contains("node_type <> 'raisin:Asset'"));
        assert!(sql.contains("NOT (properties->>'file_type' LIKE 'image/%'"));
        assert!(SearchOptions::parse(&json!({"include_kinds": ["pictures"]}), "q").is_err());
    }

    #[test]
    fn node_type_filters_are_bound() {
        let o = parse(
            json!({"include_kinds": "all", "node_types": ["studio:Page"], "exclude_node_types": "studio:Blueprint"}),
        );
        let (sql, params) = o.where_clause(3);
        assert_eq!(sql.unwrap(), "(node_type = $3) AND node_type <> $4");
        assert_eq!(
            params,
            vec![json!("studio:Page"), json!("studio:Blueprint")]
        );
        assert!(!o.admits("stories", "/x", "studio:Blueprint", Kind::Page));
    }

    #[test]
    fn the_kind_of_a_hit() {
        assert_eq!(kind_of("studio:Page", "", "/bap/team"), Kind::Page);
        assert_eq!(
            kind_of("raisin:Asset", "image/jpeg", "/bap/ceo.jpg"),
            Kind::Image
        );
        assert_eq!(kind_of("raisin:Asset", "", "/bap/ceo.JPG"), Kind::Image);
        assert_eq!(
            kind_of("raisin:Asset", "application/pdf", "/bap/report.pdf"),
            Kind::Document
        );
        assert_eq!(
            kind_of("raisin:Asset", "video/mp4", "/bap/clip"),
            Kind::Media
        );
        assert_eq!(kind_of("raisin:Asset", "", "/bap/unknown"), Kind::Document);
    }

    #[test]
    fn locale_and_language_are_validated() {
        let o = parse(
            json!({"locale": "fr", "base_language": "de", "fulltext_languages": ["de", "en"]}),
        );
        assert_eq!(o.overlay_locale(), Some("fr"));
        assert_eq!(
            o.fulltext_languages,
            vec!["en".to_string()],
            "the base language is already the hybrid leg's"
        );
        assert_eq!(
            parse(json!({"locale": "de", "base_language": "de"})).overlay_locale(),
            None
        );
        assert!(SearchOptions::parse(&json!({"locale": "de'; DROP"}), "q").is_err());
    }

    #[test]
    fn expansion_terms_are_plain_words() {
        let t = sanitize_terms(
            &[
                "Geschäftsführer".into(),
                "title:\"CEO\"".into(),
                "q".into(),
                "geschäftsführer".into(),
            ],
            "q",
        );
        assert_eq!(
            t,
            vec!["Geschäftsführer".to_string(), "title CEO".to_string()]
        );
    }

    #[test]
    fn the_window_is_never_smaller_than_the_limit() {
        let o = parse(json!({"limit": 20, "candidates": 5}));
        assert_eq!(o.candidates, 20);
        assert_eq!(parse(json!({"limit": 5000})).limit, 50);
    }
}
