//! `SearchService`: searching, typeahead, trending and the caller's history.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use serde::{Deserialize, Serialize};
use tokuly_core::{analytics::AnalyticsEvent, keys::sha256_hex, page::encode_cursor, prelude::*};
use tokuly_proto::{
    common::{PageInfo, PageRequestView},
    search::*,
};

use crate::{
    Search,
    history::RecentRow,
    indexes::MAX_TOTAL_HITS,
    kinds::{self, Kind},
    query::{EngineQuery, Scope, owner_filter},
    text::{MAX_QUERY_CHARS, normalize_query, query_key},
};

const DEFAULT_PAGE_SIZE: i64 = 20;
const MAX_PAGE_SIZE: i64 = 50;
const DEFAULT_SUGGEST_HITS: i32 = 6;
const DEFAULT_COMPLETIONS: i32 = 8;
const MAX_SUGGEST_ITEMS: i32 = 10;
/// The caller's own history offered in a typeahead, at most.
const MAX_RECENT_COMPLETIONS: i64 = 3;
const DEFAULT_TRENDING: i32 = 10;
const MAX_TRENDING: i32 = 20;
const MINUTE: Duration = Duration::from_secs(60);

const SORT_NEWEST: &[&str] = &["created_at:desc"];
const SORT_POPULAR: &[&str] = &["score:desc"];

/// Where the next page starts, bound to the query it belongs to.
#[derive(Serialize, Deserialize)]
struct Cursor {
    /// Offset of the next page.
    o: usize,
    /// Fingerprint of the query, tab, order and owner filter.
    f: String,
}

/// A query after validation.
struct ParsedQuery {
    /// Trimmed and Unicode-normalized, as shown back and remembered.
    display: String,
    /// What the engine is asked: `display` without a leading `#`.
    text: String,
    scope: Scope,
}

/// Validates and normalizes a query. `None` when it is empty.
fn parse_query(input: &str) -> AppResult<Option<ParsedQuery>> {
    let display = normalize_query(input);
    ensure!(display.chars().count() <= MAX_QUERY_CHARS, invalid, "QUERY_TOO_LONG", "a query is at most {MAX_QUERY_CHARS} characters");
    if display.is_empty() {
        return Ok(None);
    }
    Ok(Some(match display.strip_prefix('#') {
        Some(tag) => ParsedQuery { text: tag.trim_start_matches('#').trim().to_owned(), scope: Scope::Tags, display },
        None => ParsedQuery { text: display.clone(), scope: Scope::Everything, display },
    }))
}

/// The tab a request asks for: `None` for all kinds.
fn parse_kind(kind: buffa::EnumValue<SearchKind>) -> AppResult<Option<&'static Kind>> {
    match kind.as_known() {
        Some(SearchKind::SEARCH_KIND_UNSPECIFIED) => Ok(None),
        Some(known) => Ok(kinds::by_proto(known)),
        None => bail!(invalid, "KIND_UNKNOWN", "unknown search kind {}", kind.to_i32()),
    }
}

fn parse_sort(sort: buffa::EnumValue<SearchSort>) -> AppResult<Option<&'static [&'static str]>> {
    match sort.as_known() {
        Some(SearchSort::SEARCH_SORT_UNSPECIFIED | SearchSort::SEARCH_SORT_RELEVANCE) => Ok(None),
        Some(SearchSort::SEARCH_SORT_NEWEST) => Ok(Some(SORT_NEWEST)),
        Some(SearchSort::SEARCH_SORT_POPULAR) => Ok(Some(SORT_POPULAR)),
        None => bail!(invalid, "SORT_UNKNOWN", "unknown sort order {}", sort.to_i32()),
    }
}

/// A page request with this RPC's default and maximum size.
pub(crate) fn page_of(page: Option<&PageRequestView<'_>>, default: i64, max: i64) -> Page {
    match page {
        Some(page) => Page::new(page.limit, page.cursor, default, max),
        None => Page::new(0, "", default, max),
    }
}

fn kind_name(kind: Option<&Kind>) -> &'static str {
    kind.map(|k| k.name).unwrap_or("")
}

fn recent_message(row: RecentRow) -> RecentSearch {
    RecentSearch {
        id: row.id.to_string(),
        query: row.query,
        kind: kinds::by_name(&row.kind).map(|k| k.proto).unwrap_or_default().into(),
        searched_at: ts(row.searched_at),
        ..Default::default()
    }
}

impl Search {
    /// Applies the rate limit of an RPC that works with and without signing
    /// in — per account when signed in, per client address otherwise — and
    /// returns the caller.
    async fn limited_caller(&self, ctx: &RequestContext, action: &str, per_user: u32, anonymous: u32) -> AppResult<Option<Uuid>> {
        let caller = ctx.maybe_user_id()?;
        match caller {
            Some(user_id) => self.core.cache.rate_limit(&format!("search:{action}:u:{user_id}"), per_user, MINUTE).await?,
            None => {
                let ip = ctx.client().ip;
                let address = if ip.is_empty() { "unknown" } else { ip.as_str() };
                self.core.cache.rate_limit(&format!("search:{action}:ip:{address}"), anonymous, MINUTE).await?
            }
        }
        Ok(caller)
    }

    /// Everyone hidden from the caller by a block, in either direction.
    async fn hidden_from(&self, caller: Option<Uuid>) -> AppResult<Vec<Uuid>> {
        match caller {
            Some(user_id) => tokuly_social::graph::blocked_ids_either(&self.core, user_id).await,
            None => Ok(Vec::new()),
        }
    }

    /// Remembers a first-page search in the caller's history and counts it
    /// towards trending. Failures are logged: bookkeeping never fails a
    /// search that already has its results.
    async fn note_search(&self, user_id: Uuid, parsed: &ParsedQuery, kind: Option<&Kind>, found: bool) {
        let key = query_key(&parsed.display);
        match self.remember_search(user_id, &parsed.display, &key, kind_name(kind)).await {
            // A paused history also keeps the search out of trending.
            Ok(true) if found => self.record_trend(user_id, &key).await,
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "search not remembered"),
        }
    }
}

impl SearchService for Search {
    async fn search(&self, ctx: RequestContext, request: ServiceRequest<'_, SearchRequest>) -> ServiceResult<SearchResponse> {
        handle(async {
            let caller =
                self.limited_caller(&ctx, "query", self.cfg.rate_per_minute, self.cfg.anonymous_rate_per_minute).await?;
            let parsed = parse_query(request.query)?.filter(|q| !q.text.is_empty());
            let Some(parsed) = parsed else { bail!(invalid, "QUERY_REQUIRED", "enter something to search for") };
            let kind = parse_kind(request.kind)?;
            let sort = parse_sort(request.sort)?;
            let owner = parse_opt_id(request.owner_id)?;
            let page = page_of(request.page.as_option(), DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE);

            // A cursor only continues the query it was issued for.
            let fingerprint = sha256_hex(
                format!("{}\u{1f}{}\u{1f}{}\u{1f}{}", query_key(&parsed.display), kind_name(kind), request.sort.to_i32(), owner.unwrap_or_default())
                    .as_bytes(),
            )[..16]
                .to_owned();
            let offset = match page.decode::<Cursor>()? {
                Some(cursor) => {
                    ensure!(cursor.f == fingerprint && cursor.o <= MAX_TOTAL_HITS, invalid, "INVALID_CURSOR", "invalid page cursor");
                    cursor.o
                }
                None => 0,
            };
            let first_page = offset == 0;
            let limit = page.limit as usize;

            let hidden = self.hidden_from(caller).await?;
            let answer = self
                .ask_engine(&EngineQuery {
                    text: parsed.text.clone(),
                    scope: parsed.scope,
                    kind,
                    filter: owner_filter(&hidden, owner),
                    sort,
                    offset,
                    limit,
                    snippets: true,
                    counts: first_page,
                    typeahead: false,
                })
                .await?;
            let next_offset = offset + limit;
            let has_more = answer.has_more && next_offset < MAX_TOTAL_HITS;
            let hits = self.present_hits(&hidden, answer.hits).await?;

            if first_page {
                if let Some(user_id) = caller {
                    if !request.skip_history {
                        self.note_search(user_id, &parsed, kind, answer.total > 0).await;
                    }
                }
                self.core
                    .analytics
                    .track(
                        AnalyticsEvent::new("search.query")
                            .user(caller)
                            .value(answer.total as f64)
                            .props(serde_json::json!({ "kind": kind_name(kind), "tags_only": parsed.scope == Scope::Tags })),
                    )
                    .await;
            }

            ok(SearchResponse {
                hits,
                page: PageInfo {
                    next_cursor: if has_more { encode_cursor(&Cursor { o: next_offset, f: fingerprint }) } else { String::new() },
                    has_more,
                    ..Default::default()
                }
                .into(),
                counts: answer.counts,
                total: answer.total,
                query: parsed.display,
                ..Default::default()
            })
        })
        .await
    }

    async fn suggest(&self, ctx: RequestContext, request: ServiceRequest<'_, SuggestRequest>) -> ServiceResult<SuggestResponse> {
        handle(async {
            let per_user = self.cfg.suggest_rate_per_minute;
            let caller = self.limited_caller(&ctx, "suggest", per_user, (per_user / 2).max(1)).await?;
            let kind = parse_kind(request.kind)?;
            let Some(parsed) = parse_query(request.query)? else { return ok(SuggestResponse::default()) };
            let hit_limit = if request.hit_limit <= 0 { DEFAULT_SUGGEST_HITS } else { request.hit_limit.min(MAX_SUGGEST_ITEMS) };
            let completion_limit =
                if request.completion_limit <= 0 { DEFAULT_COMPLETIONS } else { request.completion_limit.min(MAX_SUGGEST_ITEMS) };
            let prefix = query_key(&parsed.display);

            let completions = async {
                let recent = match caller {
                    Some(user_id) => self.recent_completions(user_id, &prefix, MAX_RECENT_COMPLETIONS).await?,
                    None => Vec::new(),
                };
                let popular = self.popular_completions(&prefix, completion_limit as i64).await?;
                let mut seen: Vec<String> = Vec::new();
                let mut completions: Vec<Completion> = Vec::new();
                for row in recent {
                    seen.push(query_key(&row.query));
                    completions.push(Completion {
                        query: row.query,
                        source: CompletionSource::COMPLETION_SOURCE_RECENT.into(),
                        recent_search_id: row.id.to_string(),
                        ..Default::default()
                    });
                }
                for query in popular {
                    if !seen.contains(&query) {
                        completions.push(Completion {
                            query,
                            source: CompletionSource::COMPLETION_SOURCE_POPULAR.into(),
                            ..Default::default()
                        });
                    }
                }
                completions.truncate(completion_limit as usize);
                AppResult::Ok(completions)
            };
            let hits = async {
                // "#" alone is a tag search without a tag yet.
                if parsed.text.is_empty() {
                    return AppResult::Ok(Vec::new());
                }
                let hidden = self.hidden_from(caller).await?;
                let answer = self
                    .ask_engine(&EngineQuery {
                        text: parsed.text.clone(),
                        scope: if parsed.scope == Scope::Tags { Scope::Tags } else { Scope::Names },
                        kind,
                        filter: owner_filter(&hidden, None),
                        sort: None,
                        offset: 0,
                        limit: hit_limit as usize,
                        snippets: false,
                        counts: false,
                        typeahead: true,
                    })
                    .await?;
                self.present_hits(&hidden, answer.hits).await
            };
            let (completions, hits) = tokio::join!(completions, hits);
            ok(SuggestResponse { completions: completions?, hits: hits?, ..Default::default() })
        })
        .await
    }

    async fn get_trending(&self, ctx: RequestContext, request: ServiceRequest<'_, GetTrendingRequest>) -> ServiceResult<GetTrendingResponse> {
        handle(async {
            let per_user = self.cfg.suggest_rate_per_minute;
            self.limited_caller(&ctx, "trending", per_user, (per_user / 2).max(1)).await?;
            let limit = if request.limit <= 0 { DEFAULT_TRENDING } else { request.limit.min(MAX_TRENDING) };
            let (queries, updated_at) = self.trending(limit as i64).await?;
            ok(GetTrendingResponse {
                queries: queries
                    .into_iter()
                    .enumerate()
                    .map(|(position, query)| TrendingQuery { query, rank: position as i32 + 1, ..Default::default() })
                    .collect(),
                updated_at: opt_ts(updated_at),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_recent_searches(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListRecentSearchesRequest>,
    ) -> ServiceResult<ListRecentSearchesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = page_of(request.page.as_option(), DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE);
            let mut rows = self.list_recent(user_id, &page).await?;
            let info = page.finish(&mut rows, |row| (row.searched_at, row.id));
            ok(ListRecentSearchesResponse {
                searches: rows.into_iter().map(recent_message).collect(),
                page: info.into(),
                history_paused: self.history_paused(user_id).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_recent_search(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteRecentSearchRequest>,
    ) -> ServiceResult<DeleteRecentSearchResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let id = parse_id(request.id)?;
            ensure!(self.delete_recent(user_id, id).await?, not_found, "RECENT_SEARCH_NOT_FOUND", "recent search not found");
            ok(DeleteRecentSearchResponse::default())
        })
        .await
    }

    async fn clear_recent_searches(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ClearRecentSearchesRequest>,
    ) -> ServiceResult<ClearRecentSearchesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let deleted = self.clear_recent(user_id).await?;
            ok(ClearRecentSearchesResponse { deleted_count: deleted.min(i32::MAX as u64) as i32, ..Default::default() })
        })
        .await
    }

    async fn set_history_paused(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetHistoryPausedRequest>,
    ) -> ServiceResult<SetHistoryPausedResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.set_history_paused(user_id, request.paused).await?;
            ok(SetHistoryPausedResponse { paused: request.paused, ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_normalized_and_tag_searches_recognized() {
        let plain = parse_query("  渋谷　カフェ ").unwrap().unwrap();
        assert_eq!((plain.display.as_str(), plain.text.as_str(), plain.scope), ("渋谷 カフェ", "渋谷 カフェ", Scope::Everything));

        let tag = parse_query("＃散歩").unwrap().unwrap();
        assert_eq!((tag.display.as_str(), tag.text.as_str(), tag.scope), ("#散歩", "散歩", Scope::Tags));

        let bare = parse_query("#").unwrap().unwrap();
        assert!(bare.text.is_empty() && bare.scope == Scope::Tags);

        assert!(parse_query("   ").unwrap().is_none());
        assert!(parse_query("\u{0}\n").unwrap().is_none());
    }

    #[test]
    fn overlong_queries_are_refused() {
        assert!(parse_query(&"あ".repeat(MAX_QUERY_CHARS)).unwrap().is_some());
        let error = parse_query(&"あ".repeat(MAX_QUERY_CHARS + 1)).err().unwrap();
        assert_eq!(error.reason, "QUERY_TOO_LONG");
    }

    #[test]
    fn kinds_and_sorts_are_validated() {
        assert!(parse_kind(SearchKind::SEARCH_KIND_UNSPECIFIED.into()).unwrap().is_none());
        assert_eq!(parse_kind(SearchKind::SEARCH_KIND_STAMP_PACK.into()).unwrap().unwrap().name, "stamp_pack");
        assert_eq!(parse_kind(buffa::EnumValue::from(99)).err().unwrap().reason, "KIND_UNKNOWN");

        assert_eq!(parse_sort(SearchSort::SEARCH_SORT_UNSPECIFIED.into()).unwrap(), None);
        assert_eq!(parse_sort(SearchSort::SEARCH_SORT_RELEVANCE.into()).unwrap(), None);
        assert_eq!(parse_sort(SearchSort::SEARCH_SORT_NEWEST.into()).unwrap(), Some(SORT_NEWEST));
        assert_eq!(parse_sort(SearchSort::SEARCH_SORT_POPULAR.into()).unwrap(), Some(SORT_POPULAR));
        assert_eq!(parse_sort(buffa::EnumValue::from(42)).err().unwrap().reason, "SORT_UNKNOWN");
    }
}
