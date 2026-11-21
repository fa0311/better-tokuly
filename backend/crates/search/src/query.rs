//! The read path: asking the engine and turning its hits into API messages.
//!
//! - All kinds at once is one **federated** multi-search over every index:
//!   the engine merges the hits (by relevance, then the shared `rank`) and a
//!   merged facet on `kind` yields the per-tab counts in the same request.
//! - One kind is a plain multi-search: the tab's query plus, on the first
//!   page, a zero-hit query per other kind for their counts.
//!
//! A document matches when it contains every word of the query (the last
//! word may be a prefix, and longer words may be misspelt).
//!
//! Two filters are applied on our side of the engine, whatever it returns:
//! content of users hidden from the caller by a block, and content of
//! suspended or deleted owners. The first is also part of the engine filter
//! (so counts and pages are right); the second normally never matches,
//! because the indexer keeps such documents out, and exists for the moments
//! between a suspension and the indexer catching up.
use std::collections::{HashMap, HashSet};

use meilisearch_sdk::{
    errors::{Error as EngineError, ErrorCode},
    indexes::Index,
    search::{FederationOptions, MatchingStrategies, MergeFacets, SearchQuery, SearchResult, Selectors},
};
use serde::Deserialize;
use tokuly_core::{prelude::*, time::from_unix};
use tokuly_proto::search::{KindCount, SearchHit};

use crate::{
    Search,
    indexes::{LOCALES, MAX_TOTAL_HITS, REQUEST_TIMEOUT, engine_error, engine_timeout, is_code},
    kinds::{self, KINDS, Kind},
    text::{HIGHLIGHT_END, HIGHLIGHT_START, parse_highlight, plain_segments},
};

/// Document fields returned with a hit (the body comes back as a snippet).
const RETRIEVE: &[&str] = &["id", "kind", "title", "tags", "owner_id", "image_url", "path", "created_at", "extra"];
const COUNT_ONLY: &[&str] = &["id"];
const HIGHLIGHT_TITLE: &[&str] = &["title"];
const HIGHLIGHT_TITLE_AND_TEXT: &[&str] = &["title", "text"];
const CROP_TEXT: &[(&str, Option<usize>)] = &[("text", None)];
/// Snippet length in engine words (for Japanese: morphemes).
const SNIPPET_WORDS: usize = 30;
const CROP_MARKER: &str = "…";
const ON_TAGS: &[&str] = &["tags"];
const ON_NAMES: &[&str] = &["title", "handle", "tags"];
const START_TAG: &str = "\u{E000}";
const END_TAG: &str = "\u{E001}";
const FACET_KIND: &str = "kind";

const _: () = assert!(HIGHLIGHT_START == '\u{E000}' && HIGHLIGHT_END == '\u{E001}');

/// Which attributes the query is matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Title, handle, tags and body.
    Everything,
    /// Title, handle and tags: what typeahead looks at.
    Names,
    /// Tags only (`#tag` queries).
    Tags,
}

/// One question for the engine.
#[derive(Debug, Clone)]
pub(crate) struct EngineQuery {
    pub text: String,
    pub scope: Scope,
    /// The tab; `None` searches every kind.
    pub kind: Option<&'static Kind>,
    /// Engine filter expression (see [`owner_filter`]).
    pub filter: Option<String>,
    /// Explicit order, e.g. `["created_at:desc"]`; `None` = relevance.
    pub sort: Option<&'static [&'static str]>,
    pub offset: usize,
    pub limit: usize,
    /// Return an excerpt of the body with each hit.
    pub snippets: bool,
    /// Count the matches of every kind.
    pub counts: bool,
    /// Favour names (users, channels, places, hashtags) when merging kinds.
    pub typeahead: bool,
}

/// A hit as the engine returned it.
#[derive(Debug, Deserialize)]
pub(crate) struct HitDoc {
    id: Uuid,
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    owner_id: Option<Uuid>,
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    path: String,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    extra: serde_json::Value,
}

pub(crate) struct RawHit {
    doc: HitDoc,
    /// The title and the body excerpt with highlight markers.
    title: Option<String>,
    text: Option<String>,
}

impl From<SearchResult<HitDoc>> for RawHit {
    fn from(hit: SearchResult<HitDoc>) -> Self {
        let formatted = |field: &str| {
            hit.formatted_result.as_ref().and_then(|f| f.get(field)).and_then(|v| v.as_str()).map(str::to_owned)
        };
        Self { title: formatted("title"), text: formatted("text"), doc: hit.result }
    }
}

pub(crate) struct EngineAnswer {
    /// At most `limit` hits.
    pub hits: Vec<RawHit>,
    /// Whether the engine has hits beyond this page.
    pub has_more: bool,
    /// Estimated matches for the query in the selected tab.
    pub total: i64,
    /// Matches per kind (kinds without matches left out); empty unless asked for.
    pub counts: Vec<KindCount>,
}

/// The engine filter for a caller: nothing owned by `hidden` users, and only
/// documents of `owner` when given. IDs are UUIDs, so quoting them is all
/// the escaping there is to do.
pub(crate) fn owner_filter(hidden: &[Uuid], owner: Option<Uuid>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(owner) = owner {
        parts.push(format!("owner_id = \"{owner}\""));
    }
    if !hidden.is_empty() {
        let ids: Vec<String> = hidden.iter().map(|id| format!("\"{id}\"")).collect();
        parts.push(format!("owner_id NOT IN [{}]", ids.join(", ")));
    }
    (!parts.is_empty()).then(|| parts.join(" AND "))
}

fn count(kind: &Kind, matches: usize) -> Option<KindCount> {
    (matches > 0).then(|| KindCount { kind: kind.proto.into(), count: matches as i64, ..Default::default() })
}

/// The engine query for one index.
fn index_query<'a>(index: &'a Index, query: &'a EngineQuery) -> SearchQuery<'a, meilisearch_sdk::DefaultHttpClient> {
    let mut built = SearchQuery::new(index);
    built
        .with_query(&query.text)
        .with_locales(LOCALES)
        .with_attributes_to_retrieve(Selectors::Some(RETRIEVE))
        .with_attributes_to_highlight(Selectors::Some(if query.snippets {
            HIGHLIGHT_TITLE_AND_TEXT
        } else {
            HIGHLIGHT_TITLE
        }))
        .with_highlight_pre_tag(START_TAG)
        .with_highlight_post_tag(END_TAG);
    if query.snippets {
        built
            .with_attributes_to_crop(Selectors::Some(CROP_TEXT))
            .with_crop_length(SNIPPET_WORDS)
            .with_crop_marker(CROP_MARKER);
    }
    scope_and_filter(&mut built, query);
    if let Some(sort) = query.sort {
        built.with_sort(sort);
    }
    built
}

/// The engine query that only counts an index's matches.
fn count_query<'a>(index: &'a Index, query: &'a EngineQuery) -> SearchQuery<'a, meilisearch_sdk::DefaultHttpClient> {
    let mut built = SearchQuery::new(index);
    built
        .with_query(&query.text)
        .with_locales(LOCALES)
        .with_attributes_to_retrieve(Selectors::Some(COUNT_ONLY))
        .with_limit(0);
    scope_and_filter(&mut built, query);
    built
}

fn scope_and_filter<'a>(built: &mut SearchQuery<'a, meilisearch_sdk::DefaultHttpClient>, query: &'a EngineQuery) {
    // Every word of the query has to match. The engine's default drops words
    // from the end until something matches, which suits space-separated
    // languages; Japanese is segmented into morphemes, and 「おさんぽ」 would
    // then also "match" every text that merely contains an 「お」.
    built.with_matching_strategy(MatchingStrategies::ALL);
    match query.scope {
        Scope::Everything => {}
        Scope::Names => {
            built.with_attributes_to_search_on(ON_NAMES);
        }
        Scope::Tags => {
            built.with_attributes_to_search_on(ON_TAGS);
        }
    }
    if let Some(filter) = &query.filter {
        built.with_filter(filter);
    }
}

/// How many hits to ask for: one more than the page to learn whether there
/// is another, without reaching past what the engine pages into.
fn fetch_limit(query: &EngineQuery) -> usize {
    (query.limit + 1).min(MAX_TOTAL_HITS.saturating_sub(query.offset))
}

impl Search {
    /// Asks the engine. Creates the indexes and asks once more when they turn
    /// out to be missing (a process that started before setup ran).
    pub(crate) async fn ask_engine(&self, query: &EngineQuery) -> AppResult<EngineAnswer> {
        match self.ask_engine_once(query).await? {
            Err(error) if is_code(&error, ErrorCode::IndexNotFound) => {
                tracing::warn!("search index missing; creating the indexes");
                self.ensure_indexes().await?;
                self.ask_engine_once(query).await?.map_err(|e| engine_error("searching", e))
            }
            other => other.map_err(|e| engine_error("searching", e)),
        }
    }

    /// One attempt. The outer error is ours (the engine did not answer in
    /// time); the inner one is the engine's.
    async fn ask_engine_once(&self, query: &EngineQuery) -> AppResult<Result<EngineAnswer, EngineError>> {
        let request = async {
            match query.kind {
                None => self.ask_all_kinds(query).await,
                Some(kind) => self.ask_one_kind(kind, query).await,
            }
        };
        tokio::time::timeout(REQUEST_TIMEOUT, request).await.map_err(|_| engine_timeout("searching"))
    }

    /// Every kind in one federated request.
    async fn ask_all_kinds(&self, query: &EngineQuery) -> Result<EngineAnswer, EngineError> {
        let indexes: Vec<(&'static Kind, Index)> = KINDS.iter().map(|kind| (kind, self.index(kind))).collect();
        let mut request = self.core.search.multi_search();
        for (kind, index) in &indexes {
            let weight = if query.typeahead { kind.typeahead_weight } else { kind.weight };
            request.with_search_query_and_weight(index_query(index, query), weight);
        }
        let fetch = fetch_limit(query);
        let federation = FederationOptions {
            offset: Some(query.offset),
            limit: Some(fetch),
            facets_by_index: query
                .counts
                .then(|| indexes.iter().map(|(_, index)| (index.uid.clone(), vec![FACET_KIND.to_owned()])).collect()),
            merge_facets: query.counts.then(MergeFacets::default),
            ..Default::default()
        };
        let request = request.with_federation(federation);
        let response = request.execute::<HitDoc>().await?;
        let by_kind: HashMap<String, usize> =
            response.facet_distribution.and_then(|mut facets| facets.remove(FACET_KIND)).unwrap_or_default();
        let counts =
            KINDS.iter().filter_map(|kind| count(kind, by_kind.get(kind.name).copied().unwrap_or(0))).collect();
        let mut hits: Vec<RawHit> = response.hits.into_iter().map(RawHit::from).collect();
        let has_more = hits.len() > query.limit;
        hits.truncate(query.limit);
        Ok(EngineAnswer { hits, has_more, total: response.estimated_total_hits as i64, counts })
    }

    /// One kind, plus zero-hit queries on the other indexes for their counts.
    async fn ask_one_kind(&self, kind: &'static Kind, query: &EngineQuery) -> Result<EngineAnswer, EngineError> {
        let index = self.index(kind);
        let others: Vec<(&'static Kind, Index)> = if query.counts {
            KINDS.iter().filter(|other| other.name != kind.name).map(|other| (other, self.index(other))).collect()
        } else {
            Vec::new()
        };
        let mut request = self.core.search.multi_search();
        let mut main = index_query(&index, query);
        main.with_offset(query.offset).with_limit(fetch_limit(query));
        request.with_search_query(main);
        for (_, other_index) in &others {
            request.with_search_query(count_query(other_index, query));
        }
        let response = request.execute::<HitDoc>().await?;
        let mut results = response.results.into_iter();
        let Some(main) = results.next() else {
            return Ok(EngineAnswer { hits: Vec::new(), has_more: false, total: 0, counts: Vec::new() });
        };
        let total = main.estimated_total_hits.unwrap_or(main.hits.len());
        let mut counts: Vec<KindCount> = Vec::new();
        if query.counts {
            let mut by_kind: HashMap<&str, usize> = HashMap::from([(kind.name, total)]);
            for ((other, _), result) in others.iter().zip(results) {
                by_kind.insert(other.name, result.estimated_total_hits.unwrap_or(0));
            }
            counts = KINDS.iter().filter_map(|k| count(k, by_kind.get(k.name).copied().unwrap_or(0))).collect();
        }
        let mut hits: Vec<RawHit> = main.hits.into_iter().map(RawHit::from).collect();
        let has_more = hits.len() > query.limit;
        hits.truncate(query.limit);
        Ok(EngineAnswer { hits, has_more, total: total as i64, counts })
    }

    /// Turns engine hits into API hits: drops what the caller must not see
    /// and attaches owner summaries.
    pub(crate) async fn present_hits(&self, hidden: &[Uuid], hits: Vec<RawHit>) -> AppResult<Vec<SearchHit>> {
        let hidden: HashSet<Uuid> = hidden.iter().copied().collect();
        let owner_ids: Vec<Uuid> =
            hits.iter().filter_map(|hit| hit.doc.owner_id).collect::<HashSet<_>>().into_iter().collect();
        // `known` holds every owner that is an account; `active` those that
        // are neither suspended nor deleted. An owner that is not an account
        // (official content) is neither known nor filtered.
        let known = tokuly_identity::summaries(&self.core, &owner_ids).await?;
        let active: HashSet<Uuid> =
            tokuly_identity::users::filter_active(&self.core, &owner_ids).await?.into_iter().collect();
        let mut presented = Vec::with_capacity(hits.len());
        for hit in hits {
            let Some(kind) = kinds::by_name(&hit.doc.kind) else { continue };
            let owner = match hit.doc.owner_id {
                Some(owner_id) if hidden.contains(&owner_id) => continue,
                Some(owner_id) if known.contains_key(&owner_id) && !active.contains(&owner_id) => continue,
                Some(owner_id) => known.get(&owner_id).cloned(),
                None => None,
            };
            let title_segments = match &hit.title {
                Some(formatted) => parse_highlight(formatted),
                None => plain_segments(&hit.doc.title),
            };
            let extra_json = if hit.doc.extra.is_object() { hit.doc.extra.to_string() } else { "{}".to_owned() };
            presented.push(SearchHit {
                kind: kind.proto.into(),
                id: hit.doc.id.to_string(),
                title: hit.doc.title,
                title_segments,
                snippet: hit.text.as_deref().map(parse_highlight).unwrap_or_default(),
                image_url: hit.doc.image_url.unwrap_or_default(),
                path: hit.doc.path,
                owner: owner.into(),
                created_at: ts(from_unix(hit.doc.created_at, 0)),
                tags: hit.doc.tags,
                extra_json,
                ..Default::default()
            });
        }
        Ok(presented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(offset: usize, limit: usize) -> EngineQuery {
        EngineQuery {
            text: "渋谷".into(),
            scope: Scope::Everything,
            kind: None,
            filter: None,
            sort: None,
            offset,
            limit,
            snippets: true,
            counts: false,
            typeahead: false,
        }
    }

    #[test]
    fn the_filter_hides_blocked_owners_and_can_pin_one_owner() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert_eq!(owner_filter(&[], None), None);
        assert_eq!(
            owner_filter(&[a, b], None).unwrap(),
            "owner_id NOT IN [\"00000000-0000-0000-0000-000000000001\", \"00000000-0000-0000-0000-000000000002\"]"
        );
        assert_eq!(owner_filter(&[], Some(a)).unwrap(), "owner_id = \"00000000-0000-0000-0000-000000000001\"");
        let both = owner_filter(&[b], Some(a)).unwrap();
        assert!(both.starts_with("owner_id = ") && both.contains(" AND owner_id NOT IN ["));
    }

    #[test]
    fn one_extra_hit_is_fetched_but_never_past_the_engines_paging_limit() {
        assert_eq!(fetch_limit(&query(0, 20)), 21);
        assert_eq!(fetch_limit(&query(MAX_TOTAL_HITS - 20, 20)), 20);
        assert_eq!(fetch_limit(&query(MAX_TOTAL_HITS - 5, 20)), 5);
        assert_eq!(fetch_limit(&query(MAX_TOTAL_HITS, 20)), 0);
    }

    #[test]
    fn kinds_without_matches_are_left_out_of_the_counts() {
        let user = kinds::by_name("user").unwrap();
        assert!(count(user, 0).is_none());
        let counted = count(user, 3).unwrap();
        assert_eq!(counted.count, 3);
        assert_eq!(counted.kind.as_known(), Some(user.proto));
    }
}
