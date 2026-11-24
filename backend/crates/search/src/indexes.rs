//! The Meilisearch indexes: names, settings, synonyms and engine helpers.
//!
//! Every kind has its own index, `<prefix>_<kind>`, and every index has the
//! same settings ([`index_settings`]). They must stay identical: the "all"
//! tab merges the indexes with a federated search, which compares documents
//! of different indexes rule by rule and refuses indexes whose ranking rules
//! differ.
use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use meilisearch_sdk::{
    errors::{Error as EngineError, ErrorCode},
    indexes::Index,
    settings::{LocalizedAttributes, MinWordSizeForTypos, PaginationSetting, Settings, TypoToleranceSettings},
    task_info::TaskInfo,
    tasks::{Task, TasksSearchQuery},
};
use tokuly_core::prelude::*;

use crate::{
    Search,
    kinds::{KINDS, Kind},
};

/// Attributes matched against the query, most important first.
pub const SEARCHABLE: &[&str] = &["title", "handle", "tags", "text"];
/// Languages documents and queries are tokenized as. Without this, short
/// kanji-only text ("渋谷") is easily taken for Chinese and segmented
/// differently from the Japanese query that looks for it.
pub const LOCALES: &[&str] = &["jpn", "eng"];
/// How far into a result list a query can page.
pub const MAX_TOTAL_HITS: usize = 1000;

/// How long an engine request (not a task) may take.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long index creation and settings changes may take. Changing settings
/// re-indexes the whole index, which on a large one is not quick.
const SETUP_TASK_TIMEOUT: Duration = Duration::from_secs(600);
const TASK_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// A failed conversation with the engine, as an `unavailable` error whose
/// cause is kept for the logs.
pub(crate) fn engine_error(context: &'static str, error: EngineError) -> AppError {
    AppError {
        source: Some(anyhow::anyhow!("{context}: {error}")),
        ..AppError::unavailable("SEARCH_UNAVAILABLE", "search is temporarily unavailable")
    }
}

pub(crate) fn engine_timeout(context: &'static str) -> AppError {
    AppError {
        source: Some(anyhow::anyhow!("{context}: Meilisearch did not answer within {REQUEST_TIMEOUT:?}")),
        ..AppError::unavailable("SEARCH_UNAVAILABLE", "search is temporarily unavailable")
    }
}

/// Runs an engine request with the request timeout.
pub(crate) async fn engine<T>(
    context: &'static str,
    request: impl Future<Output = Result<T, EngineError>>,
) -> AppResult<T> {
    match tokio::time::timeout(REQUEST_TIMEOUT, request).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(engine_error(context, error)),
        Err(_) => Err(engine_timeout(context)),
    }
}

pub(crate) fn is_code(error: &EngineError, code: ErrorCode) -> bool {
    matches!(error, EngineError::Meilisearch(e) if e.error_code == code)
}

/// How an engine task ended.
#[derive(Debug)]
pub(crate) enum TaskEnd {
    Succeeded,
    Failed(meilisearch_sdk::errors::MeilisearchError),
    /// Still queued or running when the wait ran out.
    Unfinished,
}

/// Waits for a task through the engine's task API.
pub(crate) async fn wait_for_task(search: &Search, task: &TaskInfo, timeout: Duration) -> AppResult<TaskEnd> {
    match search.core.search.wait_for_task(task, Some(TASK_POLL_INTERVAL), Some(timeout)).await {
        Ok(Task::Succeeded { .. }) => Ok(TaskEnd::Succeeded),
        Ok(Task::Failed { content }) => Ok(TaskEnd::Failed(content.error)),
        Ok(Task::Enqueued { .. } | Task::Processing { .. }) | Err(EngineError::Timeout) => Ok(TaskEnd::Unfinished),
        Err(error) => Err(engine_error("waiting for an engine task", error)),
    }
}

/// The settings of every index. `synonyms` maps each word to the words that
/// should match in its place (see [`synonym_map`]).
pub fn index_settings(synonyms: HashMap<String, Vec<String>>) -> Settings {
    Settings {
        searchable_attributes: Some(SEARCHABLE.iter().map(|a| a.to_string()).collect()),
        filterable_attributes: Some(
            ["kind", "owner_id", "tags", "created_at", crate::query::CHANNEL_CLIP].into_iter().map(Into::into).collect(),
        ),
        sortable_attributes: Some(["created_at", "score", "rank"].into_iter().map(str::to_owned).collect()),
        // Queries only match documents containing all their words, so
        // `words` separates nothing; it leads because the engine wants it
        // before `typo`. An explicit order (`sort`: newest, most popular) is
        // strict. Without one, relevance decides in tiers — few typos, words
        // close together, matched in the title or handle rather than the
        // body, matched exactly — and inside a tier `rank`, popularity
        // blended with recency (`document::rank`), orders the documents.
        // Position of the match inside the attribute only breaks what is
        // left.
        ranking_rules: Some(
            ["words", "sort", "typo", "proximity", "attributeRank", "exactness", "rank:desc", "wordPosition"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        ),
        typo_tolerance: Some(TypoToleranceSettings {
            enabled: Some(true),
            // A handle is an identifier: "taro" must not find "tarou".
            disable_on_attributes: Some(vec!["handle".to_owned()]),
            disable_on_words: Some(Vec::new()),
            // Short tokens are too easily turned into other words; the
            // engine's defaults (5 and 9) are stated so they cannot drift.
            min_word_size_for_typos: Some(MinWordSizeForTypos { one_typo: Some(5), two_typos: Some(9) }),
            disable_on_numbers: Some(true),
        }),
        localized_attributes: Some(vec![LocalizedAttributes {
            locales: LOCALES.iter().map(|l| l.to_string()).collect(),
            attribute_patterns: vec!["title".to_owned(), "text".to_owned(), "tags".to_owned()],
        }]),
        pagination: Some(PaginationSetting { max_total_hits: MAX_TOTAL_HITS }),
        synonyms: Some(synonyms),
        ..Settings::new()
    }
}

fn as_set(values: &Option<Vec<String>>) -> BTreeSet<&str> {
    values.iter().flatten().map(String::as_str).collect()
}

fn sorted_synonyms(synonyms: &Option<HashMap<String, Vec<String>>>) -> Vec<(&str, BTreeSet<&str>)> {
    let mut entries: Vec<(&str, BTreeSet<&str>)> = synonyms
        .iter()
        .flatten()
        .map(|(word, others)| (word.as_str(), others.iter().map(String::as_str).collect()))
        .collect();
    entries.sort();
    entries
}

/// Whether an index already has the settings we manage, so that applying
/// them (which re-indexes) can be skipped.
pub fn settings_match(current: &Settings, desired: &Settings) -> bool {
    let filterable = |settings: &Settings| -> BTreeSet<String> {
        settings
            .filterable_attributes
            .iter()
            .flatten()
            .map(|attribute| serde_json::to_string(attribute).unwrap_or_default())
            .collect()
    };
    let typo = |settings: &Settings| {
        settings.typo_tolerance.as_ref().map(|t| {
            (
                t.enabled.unwrap_or(true),
                as_set(&t.disable_on_attributes).into_iter().map(str::to_owned).collect::<BTreeSet<_>>(),
                as_set(&t.disable_on_words).into_iter().map(str::to_owned).collect::<BTreeSet<_>>(),
                t.min_word_size_for_typos.clone(),
                t.disable_on_numbers.unwrap_or(false),
            )
        })
    };
    current.searchable_attributes == desired.searchable_attributes
        && current.ranking_rules == desired.ranking_rules
        && filterable(current) == filterable(desired)
        && as_set(&current.sortable_attributes) == as_set(&desired.sortable_attributes)
        && typo(current) == typo(desired)
        && current.localized_attributes == desired.localized_attributes
        && current.pagination == desired.pagination
        && sorted_synonyms(&current.synonyms) == sorted_synonyms(&desired.synonyms)
}

/// Meilisearch synonyms for groups of equivalent words: every word of a
/// group maps to the other words of all the groups it appears in.
pub fn synonym_map(groups: &[Vec<String>]) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, BTreeSet<String>> = HashMap::new();
    for group in groups {
        for word in group {
            let others = map.entry(word.clone()).or_default();
            others.extend(group.iter().filter(|other| *other != word).cloned());
        }
    }
    map.into_iter()
        .filter(|(_, others)| !others.is_empty())
        .map(|(word, others)| (word, others.into_iter().collect()))
        .collect()
}

impl Search {
    /// Name of a kind's index in Meilisearch.
    pub fn index_uid(&self, kind: &Kind) -> String {
        format!("{}_{}", self.cfg.index_prefix, kind.name)
    }

    pub(crate) fn index(&self, kind: &Kind) -> Index {
        self.core.search.index(self.index_uid(kind))
    }

    /// Creates the missing indexes and brings every index's settings in line
    /// with [`index_settings`] and the stored synonyms. Idempotent: an index
    /// that is already as it should be is left alone. Returns once the
    /// engine has applied everything.
    ///
    /// Safe to run from several processes at once: runs are serialized (a
    /// PostgreSQL advisory lock held for the run) and each reads the
    /// synonyms after it got its turn, so the run that finishes last is
    /// never one that started from older synonyms.
    pub async fn ensure_indexes(&self) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("search:setup:{}", self.cfg.index_prefix))
            .execute(&mut *tx)
            .await?;
        let groups: Vec<Vec<String>> =
            sqlx::query_scalar("SELECT words FROM search_synonym_groups ORDER BY id").fetch_all(&mut *tx).await?;
        let desired = index_settings(synonym_map(&groups));
        let results = futures::future::join_all(KINDS.iter().map(|kind| self.ensure_index(kind, &desired))).await;
        results.into_iter().collect::<AppResult<()>>()?;
        tx.commit().await?;
        Ok(())
    }

    async fn ensure_index(&self, kind: &Kind, desired: &Settings) -> AppResult<()> {
        let client = &self.core.search;
        let uid = self.index_uid(kind);
        match tokio::time::timeout(REQUEST_TIMEOUT, client.get_index(&uid)).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) if is_code(&error, ErrorCode::IndexNotFound) => {
                let task = engine("creating an index", client.create_index(&uid, Some("id"))).await?;
                match wait_for_task(self, &task, SETUP_TASK_TIMEOUT).await? {
                    TaskEnd::Succeeded => tracing::info!(index = %uid, "search index created"),
                    // Another process got there first.
                    TaskEnd::Failed(error) if error.error_code == ErrorCode::IndexAlreadyExists => {}
                    TaskEnd::Failed(error) => return Err(engine_error("creating an index", error.into())),
                    TaskEnd::Unfinished => return Err(engine_timeout("creating an index")),
                }
            }
            Ok(Err(error)) => return Err(engine_error("looking up an index", error)),
            Err(_) => return Err(engine_timeout("looking up an index")),
        }
        let index = client.index(&uid);
        let current = engine("reading index settings", index.get_settings()).await?;
        if settings_match(&current, desired) {
            return Ok(());
        }
        let task = engine("updating index settings", index.set_settings(desired)).await?;
        match wait_for_task(self, &task, SETUP_TASK_TIMEOUT).await? {
            TaskEnd::Succeeded => {
                tracing::info!(index = %uid, "search index settings applied");
                Ok(())
            }
            TaskEnd::Failed(error) => Err(engine_error("updating index settings", error.into())),
            TaskEnd::Unfinished => Err(engine_timeout("updating index settings")),
        }
    }

    /// Deletes every index of this deployment's prefix and waits for the
    /// engine to finish. For tests and for tearing an environment down; the
    /// registry is untouched, so `ensure_indexes` plus a reindex restores
    /// everything.
    pub async fn delete_indexes(&self) -> AppResult<()> {
        let mut tasks = Vec::new();
        for kind in KINDS {
            match tokio::time::timeout(REQUEST_TIMEOUT, self.core.search.delete_index(self.index_uid(kind))).await {
                Ok(Ok(task)) => tasks.push(task),
                Ok(Err(error)) if is_code(&error, ErrorCode::IndexNotFound) => {}
                Ok(Err(error)) => return Err(engine_error("deleting an index", error)),
                Err(_) => return Err(engine_timeout("deleting an index")),
            }
        }
        for task in &tasks {
            match wait_for_task(self, task, SETUP_TASK_TIMEOUT).await? {
                TaskEnd::Succeeded => {}
                TaskEnd::Failed(error) if error.error_code == ErrorCode::IndexNotFound => {}
                TaskEnd::Failed(error) => return Err(engine_error("deleting an index", error.into())),
                TaskEnd::Unfinished => return Err(engine_timeout("deleting an index")),
            }
        }
        Ok(())
    }

    /// Waits until the engine has no queued or running task left for this
    /// deployment's indexes (through the task API). Settings changes and
    /// writes requested without waiting are in effect once this returns.
    pub async fn wait_until_idle(&self, timeout: Duration) -> AppResult<()> {
        let uids: Vec<String> = KINDS.iter().map(|kind| self.index_uid(kind)).collect();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let mut query = TasksSearchQuery::new(&self.core.search);
            query
                .with_index_uids(uids.iter().map(String::as_str))
                .with_statuses(["enqueued", "processing"])
                .with_limit(1);
            let unfinished = engine("listing engine tasks", query.execute()).await?;
            if unfinished.results.is_empty() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(engine_timeout("waiting for the engine to become idle"));
            }
            tokio::time::sleep(TASK_POLL_INTERVAL).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn synonym_groups_expand_to_mutual_mappings() {
        let map = synonym_map(&[
            group(&["スタンプ", "ステッカー", "sticker"]),
            group(&["sticker", "seal"]),
            group(&["alone"]),
        ]);
        assert_eq!(map["スタンプ"], vec!["sticker", "ステッカー"]);
        assert_eq!(map["sticker"], vec!["seal", "スタンプ", "ステッカー"]);
        assert_eq!(map["seal"], vec!["sticker"]);
        assert!(!map.contains_key("alone"));
    }

    #[test]
    fn identical_settings_match_regardless_of_set_order() {
        let desired = index_settings(synonym_map(&[group(&["a", "b"])]));
        let mut current = desired.clone();
        current.sortable_attributes.as_mut().unwrap().reverse();
        current.filterable_attributes.as_mut().unwrap().reverse();
        // The engine also reports settings we do not manage.
        current.stop_words = Some(Vec::new());
        current.proximity_precision = Some("byWord".to_owned());
        assert!(settings_match(&current, &desired));
    }

    #[test]
    fn any_managed_difference_is_noticed() {
        let desired = index_settings(HashMap::new());
        assert!(!settings_match(&Settings::new(), &desired));

        let mut rules = desired.clone();
        rules.ranking_rules.as_mut().unwrap().swap(0, 1);
        assert!(!settings_match(&rules, &desired));

        let mut searchable = desired.clone();
        searchable.searchable_attributes.as_mut().unwrap().swap(0, 1);
        assert!(!settings_match(&searchable, &desired), "searchable order is priority");

        let mut typo = desired.clone();
        typo.typo_tolerance.as_mut().unwrap().disable_on_attributes = Some(Vec::new());
        assert!(!settings_match(&typo, &desired));

        let mut locales = desired.clone();
        locales.localized_attributes = None;
        assert!(!settings_match(&locales, &desired));

        let with_synonyms = index_settings(synonym_map(&[group(&["a", "b"])]));
        assert!(!settings_match(&desired, &with_synonyms));
    }

    #[test]
    fn every_index_shares_ranking_rules_that_end_in_the_rank_tie_breakers() {
        let settings = index_settings(HashMap::new());
        let rules = settings.ranking_rules.unwrap();
        let position = |rule: &str| rules.iter().position(|r| r == rule).unwrap_or_else(|| panic!("{rule} missing"));
        assert!(position("words") < position("rank:desc"));
        assert!(position("exactness") < position("rank:desc"));
        assert!(position("sort") < position("typo"), "an explicit sort is strict among full matches");
        assert!(settings.sortable_attributes.unwrap().contains(&"rank".to_owned()));
    }
}
