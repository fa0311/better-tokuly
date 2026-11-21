//! `SearchAdminService`: index operations and the moderators' word lists.
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{events, prelude::*};
use tokuly_proto::search::*;

use crate::{
    Search,
    events::{REINDEX_REQUESTED_TOPIC, ReindexRequested},
    kinds::{self, KINDS, Kind},
    maintenance::{APPLY_SYNONYMS_JOB, ApplySynonymsJob},
    search_svc::page_of,
    text::{clean, normalize_term},
    users::{REINDEX_USERS_JOB, ReindexUsersJob},
};

const ROLE: &str = "admin";
const MAX_TERM_CHARS: usize = 64;
const MAX_NOTE_CHARS: usize = 200;
const MIN_SYNONYMS: usize = 2;
const MAX_SYNONYMS: usize = 20;
const MAX_SYNONYM_GROUPS: i64 = 500;

/// The kind an index operation is about; it has to name one.
fn required_kind(kind: buffa::EnumValue<SearchKind>) -> AppResult<&'static Kind> {
    match kind.as_known() {
        Some(SearchKind::SEARCH_KIND_UNSPECIFIED) => bail!(invalid, "KIND_REQUIRED", "choose the kind of document"),
        Some(known) => kinds::by_proto(known).ok_or_else(|| AppError::invalid("KIND_UNKNOWN", "unknown search kind")),
        None => bail!(invalid, "KIND_UNKNOWN", "unknown search kind {}", kind.to_i32()),
    }
}

/// A moderator-entered word in the form queries are compared in.
fn valid_term(input: &str) -> Option<String> {
    let term = normalize_term(input);
    (1..=MAX_TERM_CHARS).contains(&term.chars().count()).then_some(term)
}

/// The words of a synonym group: normalized, distinct, 2-20 of them.
fn valid_synonyms(words: &[impl AsRef<str>]) -> AppResult<Vec<String>> {
    let mut normalized: Vec<String> = Vec::new();
    for word in words {
        let Some(word) = valid_term(word.as_ref()) else {
            bail!(invalid, "SYNONYMS_INVALID", "a synonym is 1-{MAX_TERM_CHARS} characters")
        };
        if !normalized.contains(&word) {
            normalized.push(word);
        }
    }
    ensure!(
        (MIN_SYNONYMS..=MAX_SYNONYMS).contains(&normalized.len()),
        invalid,
        "SYNONYMS_INVALID",
        "a synonym group has {MIN_SYNONYMS}-{MAX_SYNONYMS} different words"
    );
    Ok(normalized)
}

#[derive(sqlx::FromRow)]
struct KindRegistryStats {
    kind: String,
    live: i64,
    pending: i64,
    last_changed_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct BlockedTermRow {
    term: String,
    note: String,
    added_by: Option<Uuid>,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct SynonymGroupRow {
    id: Uuid,
    words: Vec<String>,
    updated_at: DateTime<Utc>,
}

impl From<SynonymGroupRow> for SynonymGroup {
    fn from(row: SynonymGroupRow) -> Self {
        SynonymGroup { id: row.id.to_string(), words: row.words, updated_at: ts(row.updated_at), ..Default::default() }
    }
}

impl Search {
    async fn blocked_term_messages(&self, rows: Vec<BlockedTermRow>) -> AppResult<Vec<BlockedTerm>> {
        let moderators: Vec<Uuid> = rows.iter().filter_map(|row| row.added_by).collect();
        let summaries = tokuly_identity::summaries(&self.core, &moderators).await?;
        Ok(rows
            .into_iter()
            .map(|row| BlockedTerm {
                term: row.term,
                note: row.note,
                added_by: row.added_by.and_then(|id| summaries.get(&id).cloned()).into(),
                created_at: ts(row.created_at),
                ..Default::default()
            })
            .collect())
    }

    /// Has the stored synonyms applied to the indexes by a worker.
    async fn queue_synonyms(&self) -> AppResult<()> {
        self.core.jobs.enqueue(APPLY_SYNONYMS_JOB, &ApplySynonymsJob {}).await
    }
}

impl SearchAdminService for Search {
    async fn get_index_stats(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetIndexStatsRequest>,
    ) -> ServiceResult<GetIndexStatsResponse> {
        handle(async {
            ctx.require_role(ROLE)?;
            let registry: Vec<KindRegistryStats> = sqlx::query_as(
                "SELECT kind,
                        count(*) FILTER (WHERE doc IS NOT NULL) AS live,
                        count(*) FILTER (WHERE synced_generation < generation) AS pending,
                        max(updated_at) AS last_changed_at
                 FROM search_documents GROUP BY kind",
            )
            .fetch_all(&self.core.db)
            .await?;
            let registry: HashMap<String, KindRegistryStats> = registry.into_iter().map(|row| (row.kind.clone(), row)).collect();

            // The registry numbers are worth showing even when the engine is down.
            let client = &self.core.search;
            let engine = tokio::time::timeout(crate::indexes::REQUEST_TIMEOUT, async {
                let stats = client.get_stats().await?;
                let version = client.get_version().await?;
                Ok::<_, meilisearch_sdk::errors::Error>((stats, version))
            })
            .await;
            let engine = match engine {
                Ok(Ok(engine)) => Some(engine),
                Ok(Err(error)) => {
                    tracing::warn!(%error, "search engine statistics unavailable");
                    None
                }
                Err(_) => {
                    tracing::warn!("search engine statistics timed out");
                    None
                }
            };

            let indexes = KINDS
                .iter()
                .map(|kind| {
                    let uid = self.index_uid(kind);
                    let held = registry.get(kind.name);
                    let index = engine.as_ref().and_then(|(stats, _)| stats.indexes.get(&uid));
                    IndexStats {
                        kind: kind.proto.into(),
                        exists: index.is_some(),
                        indexed_documents: index.map(|i| i.number_of_documents as i64).unwrap_or(0),
                        indexing: index.is_some_and(|i| i.is_indexing),
                        registered_documents: held.map(|h| h.live).unwrap_or(0),
                        pending_documents: held.map(|h| h.pending).unwrap_or(0),
                        last_changed_at: opt_ts(held.and_then(|h| h.last_changed_at)),
                        index_uid: uid,
                        ..Default::default()
                    }
                })
                .collect();
            ok(GetIndexStatsResponse {
                indexes,
                healthy: engine.is_some(),
                engine_version: engine.as_ref().map(|(_, version)| version.pkg_version.clone()).unwrap_or_default(),
                database_size_bytes: engine.as_ref().map(|(stats, _)| stats.database_size as i64).unwrap_or(0),
                ..Default::default()
            })
        })
        .await
    }

    async fn reindex(&self, ctx: RequestContext, request: ServiceRequest<'_, ReindexRequest>) -> ServiceResult<ReindexResponse> {
        handle(async {
            let admin = ctx.require_role(ROLE)?.user_id;
            let kind = required_kind(request.kind)?;
            let mut tx = self.core.db.begin().await?;
            let queued = Search::requeue_kind(&mut *tx, kind.name).await?;
            events::emit(
                &mut *tx,
                REINDEX_REQUESTED_TOPIC,
                Some(admin),
                &ReindexRequested { kind: kind.name.to_owned(), requested_by: Some(admin) },
            )
            .await?;
            tx.commit().await?;
            self.indexer_wake.notify_one();
            // Users have no owning domain to answer the event: they are ours.
            if kind.name == kinds::USER {
                self.core.jobs.enqueue(REINDEX_USERS_JOB, &ReindexUsersJob {}).await?;
            }
            tracing::info!(kind = kind.name, queued, %admin, "search reindex requested");
            ok(ReindexResponse { queued_documents: queued as i64, ..Default::default() })
        })
        .await
    }

    async fn delete_document(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteDocumentRequest>,
    ) -> ServiceResult<DeleteDocumentResponse> {
        handle(async {
            let admin = ctx.require_role(ROLE)?.user_id;
            let kind = required_kind(request.kind)?;
            let id = parse_id(request.id)?;
            let existed = self.force_remove(Uuid::now_v7(), kind.name, id).await?;
            tracing::info!(kind = kind.name, %id, existed, %admin, "search document deleted by an administrator");
            ok(DeleteDocumentResponse { existed, ..Default::default() })
        })
        .await
    }

    async fn list_blocked_terms(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListBlockedTermsRequest>,
    ) -> ServiceResult<ListBlockedTermsResponse> {
        handle(async {
            ctx.require_role(ROLE)?;
            let page = page_of(request.page.as_option(), Page::DEFAULT_LIMIT, 100);
            let after: Option<String> = page.decode()?;
            let mut rows: Vec<BlockedTermRow> = sqlx::query_as(
                "SELECT term::text AS term, note, added_by, created_at FROM search_blocked_terms
                 WHERE ($1::text IS NULL OR term > $1)
                 ORDER BY term
                 LIMIT $2",
            )
            .bind(after)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| row.term.clone());
            ok(ListBlockedTermsResponse { terms: self.blocked_term_messages(rows).await?, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn add_blocked_term(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddBlockedTermRequest>,
    ) -> ServiceResult<AddBlockedTermResponse> {
        handle(async {
            let admin = ctx.require_role(ROLE)?.user_id;
            let Some(term) = valid_term(request.term) else {
                bail!(invalid, "TERM_INVALID", "a blocked term is 1-{MAX_TERM_CHARS} characters")
            };
            let note = clean(request.note, MAX_NOTE_CHARS + 1);
            ensure!(note.chars().count() <= MAX_NOTE_CHARS, invalid, "NOTE_TOO_LONG", "a note is at most {MAX_NOTE_CHARS} characters");
            let mut tx = self.core.db.begin().await?;
            let row: BlockedTermRow = sqlx::query_as(
                "INSERT INTO search_blocked_terms (term, note, added_by) VALUES ($1, $2, $3)
                 ON CONFLICT (term) DO UPDATE SET note = EXCLUDED.note
                 RETURNING term::text AS term, note, added_by, created_at",
            )
            .bind(&term)
            .bind(&note)
            .bind(admin)
            .fetch_one(&mut *tx)
            .await?;
            // What is already popular or trending and now blocked goes at once.
            sqlx::query("DELETE FROM search_popular_queries WHERE position($1 in query) > 0").bind(&term).execute(&mut *tx).await?;
            tx.commit().await?;
            self.invalidate_blocklist();
            let term = self.blocked_term_messages(vec![row]).await?.pop();
            ok(AddBlockedTermResponse { term: term.into(), ..Default::default() })
        })
        .await
    }

    async fn remove_blocked_term(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveBlockedTermRequest>,
    ) -> ServiceResult<RemoveBlockedTermResponse> {
        handle(async {
            ctx.require_role(ROLE)?;
            let term = normalize_term(request.term);
            let done = sqlx::query("DELETE FROM search_blocked_terms WHERE term = $1").bind(&term).execute(&self.core.db).await?;
            ensure!(done.rows_affected() > 0, not_found, "TERM_NOT_FOUND", "this term is not blocked");
            self.invalidate_blocklist();
            ok(RemoveBlockedTermResponse::default())
        })
        .await
    }

    async fn list_synonym_groups(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListSynonymGroupsRequest>,
    ) -> ServiceResult<ListSynonymGroupsResponse> {
        handle(async {
            ctx.require_role(ROLE)?;
            let rows: Vec<SynonymGroupRow> =
                sqlx::query_as("SELECT id, words, updated_at FROM search_synonym_groups ORDER BY id").fetch_all(&self.core.db).await?;
            ok(ListSynonymGroupsResponse { groups: rows.into_iter().map(Into::into).collect(), ..Default::default() })
        })
        .await
    }

    async fn save_synonym_group(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SaveSynonymGroupRequest>,
    ) -> ServiceResult<SaveSynonymGroupResponse> {
        handle(async {
            let admin = ctx.require_role(ROLE)?.user_id;
            let id = parse_opt_id(request.id)?;
            let words = valid_synonyms(&request.words.iter().copied().collect::<Vec<&str>>())?;
            let row: Option<SynonymGroupRow> = match id {
                Some(id) => {
                    sqlx::query_as(
                        "UPDATE search_synonym_groups SET words = $2, updated_by = $3 WHERE id = $1
                         RETURNING id, words, updated_at",
                    )
                    .bind(id)
                    .bind(&words)
                    .bind(admin)
                    .fetch_optional(&self.core.db)
                    .await?
                }
                None => {
                    // The cap keeps the engine's synonym table small; it is a
                    // guard against runaway imports, not an exact quota.
                    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM search_synonym_groups").fetch_one(&self.core.db).await?;
                    ensure!(
                        existing < MAX_SYNONYM_GROUPS,
                        precondition,
                        "TOO_MANY_SYNONYM_GROUPS",
                        "at most {MAX_SYNONYM_GROUPS} synonym groups"
                    );
                    sqlx::query_as(
                        "INSERT INTO search_synonym_groups (words, updated_by) VALUES ($1, $2)
                         RETURNING id, words, updated_at",
                    )
                    .bind(&words)
                    .bind(admin)
                    .fetch_optional(&self.core.db)
                    .await?
                }
            };
            let row = row.or_not_found("SYNONYM_GROUP_NOT_FOUND", "synonym group not found")?;
            self.queue_synonyms().await?;
            ok(SaveSynonymGroupResponse { group: SynonymGroup::from(row).into(), ..Default::default() })
        })
        .await
    }

    async fn delete_synonym_group(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteSynonymGroupRequest>,
    ) -> ServiceResult<DeleteSynonymGroupResponse> {
        handle(async {
            ctx.require_role(ROLE)?;
            let id = parse_id(request.id)?;
            let done = sqlx::query("DELETE FROM search_synonym_groups WHERE id = $1").bind(id).execute(&self.core.db).await?;
            ensure!(done.rows_affected() > 0, not_found, "SYNONYM_GROUP_NOT_FOUND", "synonym group not found");
            self.queue_synonyms().await?;
            ok(DeleteSynonymGroupResponse::default())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_operations_need_a_kind() {
        assert_eq!(required_kind(SearchKind::SEARCH_KIND_UNSPECIFIED.into()).err().unwrap().reason, "KIND_REQUIRED");
        assert_eq!(required_kind(buffa::EnumValue::from(77)).err().unwrap().reason, "KIND_UNKNOWN");
        assert_eq!(required_kind(SearchKind::SEARCH_KIND_NEWS.into()).unwrap().name, "news");
    }

    #[test]
    fn terms_are_normalized_and_bounded() {
        assert_eq!(valid_term("  ＳＰＡＭ  Words ").as_deref(), Some("spam words"));
        assert_eq!(valid_term("   "), None);
        assert_eq!(valid_term(&"あ".repeat(MAX_TERM_CHARS)).map(|t| t.chars().count()), Some(MAX_TERM_CHARS));
        assert_eq!(valid_term(&"あ".repeat(MAX_TERM_CHARS + 1)), None);
    }

    #[test]
    fn synonym_groups_need_two_to_twenty_distinct_words() {
        assert_eq!(valid_synonyms(&["スタンプ", " Sticker ", "sticker"]).unwrap(), vec!["スタンプ", "sticker"]);
        assert_eq!(valid_synonyms(&["one"]).err().unwrap().reason, "SYNONYMS_INVALID");
        assert_eq!(valid_synonyms(&["same", "SAME"]).err().unwrap().reason, "SYNONYMS_INVALID");
        assert_eq!(valid_synonyms(&["ok", ""]).err().unwrap().reason, "SYNONYMS_INVALID");
        let many: Vec<String> = (0..=MAX_SYNONYMS).map(|n| format!("word{n}")).collect();
        assert_eq!(valid_synonyms(&many).err().unwrap().reason, "SYNONYMS_INVALID");
        let empty: [&str; 0] = [];
        assert_eq!(valid_synonyms(&empty).err().unwrap().reason, "SYNONYMS_INVALID");
    }
}
