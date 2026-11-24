//! Antennas (アンテナ): saved searches that collect matching new notes into a
//! timeline of their own, as the notes are posted.
//!
//! A note matches an antenna when its text contains every word of at least
//! one keyword group and none of the excluded words (both compared folded:
//! NFKC, lowercase, so `ＴＯＫＵＬＹ` finds `tokuly`), it comes from the
//! antenna's source, it is a reply only if the antenna takes replies, it has
//! pictures or a video if the antenna asks for media, and the owner may see
//! it (the visibility rules of [`crate::visibility`], blocks both ways).
//!
//! # Matching
//!
//! Every new note queues `notes.antenna_match` ([`crate::posting::JOB_ANTENNA_MATCH`]);
//! the job runs [`Notes::match_antennas`]. Its cost does not grow with the
//! number of antennas:
//!
//! - one Aho-Corasick automaton over the folded words of **all** antennas
//!   ([`Matcher`]) finds every word in the note's text in one pass;
//! - EVERYONE antennas are candidates only through a word hit;
//! - FOLLOWING, LIST and USERS antennas are found from the author: their
//!   followers and friends, the lists they are on (an index on list
//!   members), and the antennas naming them (an in-memory index);
//! - only then is each candidate's owner checked against the note
//!   (`Viewer::can_see_note`), once per owner.
//!
//! The automaton lives in-process ([`AntennaIndex`]) and is rebuilt when the
//! version counter `notes:antennas:version` in Valkey moves (every change to
//! an antenna bumps it) or when it is older than [`MAX_INDEX_AGE`] (a bump
//! that got lost).
//!
//! Collected notes go into `antenna_notes`; each antenna keeps its latest
//! [`MAX_COLLECTED`]. The job is idempotent: a note already collected is
//! not collected, nor announced, again.
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use aho_corasick::{AhoCorasick, MatchKind};
use chrono::{DateTime, Utc};
use redis::AsyncCommands as _;
use tokio::sync::RwLock;
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_identity::users::pick;
use tokuly_proto::{
    common::{PageInfo, UserSummary},
    notes::{Antenna, AntennaInput, AntennaSource, KeywordGroup, TimelineEntry},
};
use tokuly_social::graph;
use unicode_normalization::UnicodeNormalization as _;

use crate::{
    Notes,
    model::{NOTE_COLUMNS, NoteRow, antenna_path, to_json},
    text,
    visibility::{Viewer, sees_note},
};

/// Antennas one person may have.
pub const MAX_ANTENNAS: i64 = 5;
pub const MAX_NAME_CHARS: usize = 30;
pub const MAX_GROUPS: usize = 10;
pub const MAX_WORDS_PER_GROUP: usize = 5;
pub const MAX_EXCLUDE_WORDS: usize = 10;
/// Longest keyword, in characters.
pub const MAX_WORD_CHARS: usize = 50;
pub const MAX_USERS: usize = 50;
/// Notes each antenna keeps.
pub const MAX_COLLECTED: i64 = 1000;

/// Antennas created or changed, per person and hour.
const CHANGES_PER_HOUR: u32 = 60;
/// An antenna that notifies tells its owner at most once in this time (the
/// inbox entry is collapsed per antenna anyway).
const NOTICE_INTERVAL: Duration = Duration::from_secs(600);
/// The automaton is rebuilt at least this often, whatever the version says.
pub const MAX_INDEX_AGE: Duration = Duration::from_secs(300);
/// Without Valkey, the automaton is trusted for this long.
const UNVERSIONED_INDEX_AGE: Duration = Duration::from_secs(30);
/// Valkey key of the antenna version counter.
const VERSION_KEY: &str = "notes:antennas:version";

/// Words as antennas compare them: compatibility forms folded, lowercase,
/// runs of blank space as one space, no blank space around.
pub fn fold(input: &str) -> String {
    let folded: String = input.nfkc().collect::<String>().to_lowercase();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where an antenna looks, as stored (`antennas.source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Everyone = 1,
    Following = 2,
    List = 3,
    Users = 4,
}

impl Source {
    pub fn from_stored(value: i32) -> Self {
        match value {
            2 => Self::Following,
            3 => Self::List,
            4 => Self::Users,
            _ => Self::Everyone,
        }
    }

    fn from_api(value: buffa::EnumValue<AntennaSource>) -> AppResult<Self> {
        match value.as_known() {
            Some(AntennaSource::ANTENNA_SOURCE_UNSPECIFIED | AntennaSource::ANTENNA_SOURCE_EVERYONE) => {
                Ok(Self::Everyone)
            }
            Some(AntennaSource::ANTENNA_SOURCE_FOLLOWING) => Ok(Self::Following),
            Some(AntennaSource::ANTENNA_SOURCE_LIST) => Ok(Self::List),
            Some(AntennaSource::ANTENNA_SOURCE_USERS) => Ok(Self::Users),
            None => Err(AppError::invalid("INVALID_SOURCE", "unknown antenna source")),
        }
    }

    /// Whether the source alone narrows the notes enough to go without keywords.
    fn names_accounts(self) -> bool {
        matches!(self, Self::List | Self::Users)
    }
}

/// An antenna as asked for, checked and folded (the accounts and the list
/// are checked against the database separately).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AntennaSpec {
    pub name: String,
    pub keywords: Vec<Vec<String>>,
    pub exclude_words: Vec<String>,
    pub source: Source,
    pub list_id: Option<Uuid>,
    pub user_ids: Vec<Uuid>,
    pub with_replies: bool,
    pub media_only: bool,
    pub notify: bool,
}

/// Folds and checks one list of words: blank ones dropped, duplicates once.
fn words(input: &[String], max: usize) -> AppResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for word in input {
        let word = fold(word);
        if word.is_empty() || out.contains(&word) {
            continue;
        }
        ensure!(
            text::fits(&word, MAX_WORD_CHARS),
            invalid,
            "INVALID_KEYWORDS",
            "a keyword is at most {MAX_WORD_CHARS} characters"
        );
        out.push(word);
    }
    ensure!(out.len() <= max, invalid, "INVALID_KEYWORDS", "too many words");
    Ok(out)
}

impl AntennaSpec {
    pub fn parse(input: &AntennaInput) -> AppResult<Self> {
        let name = text::clean(&input.name).replace('\n', " ");
        ensure!(
            !name.is_empty() && text::fits(&name, MAX_NAME_CHARS),
            invalid,
            "INVALID_NAME",
            "an antenna name is 1–{MAX_NAME_CHARS} characters"
        );
        ensure!(input.keywords.len() <= MAX_GROUPS, invalid, "INVALID_KEYWORDS", "at most {MAX_GROUPS} keyword groups");
        let mut keywords: Vec<Vec<String>> = Vec::new();
        for group in &input.keywords {
            let group = words(&group.words, MAX_WORDS_PER_GROUP)?;
            if !group.is_empty() && !keywords.contains(&group) {
                keywords.push(group);
            }
        }
        let exclude_words = words(&input.exclude_words, MAX_EXCLUDE_WORDS)?;
        let source = Source::from_api(input.source)?;
        ensure!(
            !keywords.is_empty() || source.names_accounts(),
            invalid,
            "INVALID_KEYWORDS",
            "an antenna needs at least one keyword"
        );
        let list_id = match source {
            Source::List => Some(
                parse_opt_id(&input.list_id)?.ok_or_else(|| AppError::not_found("LIST_NOT_FOUND", "list not found"))?,
            ),
            _ => None,
        };
        let user_ids = match source {
            Source::Users => {
                let mut ids = parse_ids(&input.user_ids)?;
                ids.sort_unstable();
                ids.dedup();
                ensure!(
                    (1..=MAX_USERS).contains(&ids.len()),
                    invalid,
                    "INVALID_USERS",
                    "an antenna of accounts names 1–{MAX_USERS} of them"
                );
                ids
            }
            _ => Vec::new(),
        };
        Ok(Self {
            name,
            keywords,
            exclude_words,
            source,
            list_id,
            user_ids,
            with_replies: input.with_replies,
            media_only: input.media_only,
            notify: input.notify,
        })
    }

    fn keyword_groups(&self) -> Vec<KeywordGroup> {
        self.keywords.iter().map(|words| KeywordGroup { words: words.clone(), ..Default::default() }).collect()
    }
}

/// A row of `antennas`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct AntennaRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub keywords: serde_json::Value,
    pub exclude_words: Vec<String>,
    pub source: i32,
    pub list_id: Option<Uuid>,
    pub user_ids: Vec<Uuid>,
    pub with_replies: bool,
    pub media_only: bool,
    pub notify: bool,
    pub last_matched_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const ANTENNA_COLUMNS: &str = "a.id, a.owner_id, a.name, a.keywords, a.exclude_words, a.source, a.list_id, a.user_ids, \
     a.with_replies, a.media_only, a.notify, a.last_matched_at, a.created_at, a.updated_at";

impl AntennaRow {
    fn keyword_groups(&self) -> Vec<KeywordGroup> {
        serde_json::from_value(self.keywords.clone()).unwrap_or_default()
    }
}

// ───────────────────────────── The matcher ─────────────────────────────

/// One antenna as the matcher holds it: its words as pattern numbers.
#[derive(Debug, Clone)]
pub struct Watch {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub source: Source,
    pub list_id: Option<Uuid>,
    pub user_ids: Vec<Uuid>,
    pub with_replies: bool,
    pub media_only: bool,
    pub notify: bool,
    pub name: String,
    pub created_at: DateTime<Utc>,
    groups: Vec<Vec<usize>>,
    exclude: Vec<usize>,
}

/// What the matcher needs to know of an antenna.
#[derive(Debug, Clone)]
pub struct WatchSpec {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub keywords: Vec<Vec<String>>,
    pub exclude_words: Vec<String>,
    pub source: Source,
    pub list_id: Option<Uuid>,
    pub user_ids: Vec<Uuid>,
    pub with_replies: bool,
    pub media_only: bool,
    pub notify: bool,
    pub created_at: DateTime<Utc>,
}

/// What a new note looks like to the matcher.
#[derive(Debug, Clone, Default)]
pub struct Candidate<'a> {
    pub text: &'a str,
    pub is_reply: bool,
    pub has_media: bool,
    pub created_at: Option<DateTime<Utc>>,
}

/// Every antenna, with one automaton over all their words and indexes from
/// an account (the note's author) to the antennas that look at it.
#[derive(Debug, Default)]
pub struct Matcher {
    automaton: Option<AhoCorasick>,
    watches: Vec<Watch>,
    /// Pattern → EVERYONE antennas with the pattern in one of their groups.
    everyone_by_pattern: Vec<Vec<usize>>,
    /// Owner → their FOLLOWING antennas.
    following_by_owner: HashMap<Uuid, Vec<usize>>,
    list_watches: HashMap<Uuid, Vec<usize>>,
    user_watches: HashMap<Uuid, Vec<usize>>,
}

impl Matcher {
    pub fn build(specs: Vec<WatchSpec>) -> Self {
        let mut patterns: Vec<String> = Vec::new();
        let mut numbers: HashMap<String, usize> = HashMap::new();
        let mut number = |word: &str| -> usize {
            let word = fold(word);
            if let Some(number) = numbers.get(&word) {
                return *number;
            }
            patterns.push(word.clone());
            numbers.insert(word, patterns.len() - 1);
            patterns.len() - 1
        };
        let mut watches = Vec::with_capacity(specs.len());
        for spec in specs {
            let groups: Vec<Vec<usize>> = spec
                .keywords
                .iter()
                .map(|group| group.iter().filter(|word| !fold(word).is_empty()).map(|word| number(word)).collect())
                .filter(|group: &Vec<usize>| !group.is_empty())
                .collect();
            let exclude = spec.exclude_words.iter().filter(|word| !fold(word).is_empty()).map(|w| number(w)).collect();
            watches.push(Watch {
                id: spec.id,
                owner_id: spec.owner_id,
                source: spec.source,
                list_id: spec.list_id,
                user_ids: spec.user_ids,
                with_replies: spec.with_replies,
                media_only: spec.media_only,
                notify: spec.notify,
                name: spec.name,
                created_at: spec.created_at,
                groups,
                exclude,
            });
        }
        let mut everyone_by_pattern = vec![Vec::new(); patterns.len()];
        let mut following_by_owner: HashMap<Uuid, Vec<usize>> = HashMap::new();
        let mut list_watches: HashMap<Uuid, Vec<usize>> = HashMap::new();
        let mut user_watches: HashMap<Uuid, Vec<usize>> = HashMap::new();
        for (index, watch) in watches.iter().enumerate() {
            match watch.source {
                Source::Everyone => {
                    let mut used: Vec<usize> = watch.groups.iter().flatten().copied().collect();
                    used.sort_unstable();
                    used.dedup();
                    for pattern in used {
                        everyone_by_pattern[pattern].push(index);
                    }
                }
                Source::Following => following_by_owner.entry(watch.owner_id).or_default().push(index),
                Source::List => {
                    if let Some(list_id) = watch.list_id {
                        list_watches.entry(list_id).or_default().push(index);
                    }
                }
                Source::Users => {
                    for user_id in &watch.user_ids {
                        user_watches.entry(*user_id).or_default().push(index);
                    }
                }
            }
        }
        let automaton = (!patterns.is_empty()).then(|| {
            AhoCorasick::builder()
                .match_kind(MatchKind::Standard)
                .build(&patterns)
                .expect("an automaton over plain words builds")
        });
        Self { automaton, watches, everyone_by_pattern, following_by_owner, list_watches, user_watches }
    }

    pub fn len(&self) -> usize {
        self.watches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.watches.is_empty()
    }

    pub fn watch(&self, index: usize) -> &Watch {
        &self.watches[index]
    }

    /// The patterns found in `text` (folded first): one pass, overlaps included.
    pub fn hits(&self, text: &str) -> HashSet<usize> {
        let Some(automaton) = &self.automaton else { return HashSet::new() };
        let folded = fold(text);
        automaton.find_overlapping_iter(&folded).map(|found| found.pattern().as_usize()).collect()
    }

    /// Whether the note is one for this antenna, as far as its words and
    /// kind go (the source and the owner's view are checked elsewhere).
    fn accepts(watch: &Watch, hits: &HashSet<usize>, note: &Candidate<'_>) -> bool {
        if note.is_reply && !watch.with_replies {
            return false;
        }
        if watch.media_only && !note.has_media {
            return false;
        }
        // Notes from before the antenna existed are not collected late.
        if note.created_at.is_some_and(|at| at < watch.created_at) {
            return false;
        }
        if watch.exclude.iter().any(|pattern| hits.contains(pattern)) {
            return false;
        }
        watch.groups.is_empty() || watch.groups.iter().any(|group| group.iter().all(|pattern| hits.contains(pattern)))
    }

    /// The antennas a note of `author` matches, by index. `followers` are
    /// the accounts that follow the author or are their friend; `lists` the
    /// lists the author is on.
    pub fn matching(
        &self,
        author: Uuid,
        note: &Candidate<'_>,
        followers: &HashSet<Uuid>,
        lists: &[Uuid],
    ) -> Vec<usize> {
        let hits = self.hits(note.text);
        let mut candidates: HashSet<usize> =
            hits.iter().flat_map(|pattern| &self.everyone_by_pattern[*pattern]).copied().collect();
        // Whichever side is smaller is walked.
        if self.following_by_owner.len() < followers.len() {
            for (owner, watches) in &self.following_by_owner {
                if followers.contains(owner) {
                    candidates.extend(watches);
                }
            }
        } else {
            for follower in followers {
                if let Some(watches) = self.following_by_owner.get(follower) {
                    candidates.extend(watches);
                }
            }
        }
        for list_id in lists {
            if let Some(watches) = self.list_watches.get(list_id) {
                candidates.extend(watches);
            }
        }
        if let Some(watches) = self.user_watches.get(&author) {
            candidates.extend(watches);
        }
        let mut matched: Vec<usize> =
            candidates.into_iter().filter(|index| Self::accepts(&self.watches[*index], &hits, note)).collect();
        matched.sort_unstable();
        matched
    }
}

struct Built {
    version: Option<i64>,
    at: Instant,
    matcher: Arc<Matcher>,
}

/// The in-process matcher, rebuilt when antennas change.
#[derive(Default)]
pub struct AntennaIndex {
    built: RwLock<Option<Built>>,
}

impl AntennaIndex {
    fn fresh(built: &Built, version: Option<i64>) -> bool {
        match version {
            Some(_) => built.version == version && built.at.elapsed() < MAX_INDEX_AGE,
            None => built.at.elapsed() < UNVERSIONED_INDEX_AGE,
        }
    }
}

/// `notes.antenna`: new notes arrived in one of the recipient's antennas.
/// Collapsed per antenna.
fn antenna_notice(watch: &Watch, note: &NoteRow) -> NotificationRequest {
    let title = Localized::new(
        format!("アンテナ「{}」に新しいノートが届いたよ!", watch.name),
        format!("New notes in your antenna \"{}\"", watch.name),
    );
    let excerpt = text::excerpt(&note.text, 80);
    let body = if note.content_warning {
        Localized::new("注意書きのついたノート", "A note with a content warning")
    } else if excerpt.is_empty() {
        Localized::new("📷 写真・動画", "📷 Photo or video")
    } else {
        Localized::same(excerpt)
    };
    NotificationRequest::new(watch.owner_id, "notes.antenna", title, body, antenna_path(watch.id))
        .actor(note.author_id)
        .collapse(format!("notes.antenna:{}", watch.id))
        .data(serde_json::json!({ "antenna_id": watch.id, "note_id": note.id }))
}

impl Notes {
    /// Tells every process that the antennas changed (their matchers are
    /// rebuilt before the next note). A failure only delays that until
    /// [`MAX_INDEX_AGE`].
    pub(crate) async fn antennas_changed(&self) {
        let mut conn = self.core.cache.conn();
        if let Err(e) = conn.incr::<_, _, i64>(VERSION_KEY, 1).await {
            tracing::warn!(error = %e, "bumping the antenna version failed");
        }
    }

    /// The matcher over the antennas as they are now.
    async fn matcher(&self) -> AppResult<Arc<Matcher>> {
        let version = match self.core.cache.get_string(VERSION_KEY).await {
            Ok(value) => Some(value.and_then(|value| value.parse().ok()).unwrap_or(0)),
            Err(e) => {
                tracing::warn!(error = %e, "reading the antenna version failed");
                None
            }
        };
        if let Some(built) = self.antenna_index.built.read().await.as_ref()
            && AntennaIndex::fresh(built, version)
        {
            return Ok(built.matcher.clone());
        }
        let mut slot = self.antenna_index.built.write().await;
        if let Some(built) = slot.as_ref()
            && AntennaIndex::fresh(built, version)
        {
            return Ok(built.matcher.clone());
        }
        // The version was read first: a change from now on bumps it again.
        let sql = format!("SELECT {ANTENNA_COLUMNS} FROM antennas a ORDER BY a.id");
        let rows: Vec<AntennaRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).fetch_all(&self.core.db).await?;
        let specs = rows
            .into_iter()
            .map(|row| WatchSpec {
                keywords: row.keyword_groups().into_iter().map(|group| group.words).collect(),
                id: row.id,
                owner_id: row.owner_id,
                name: row.name,
                exclude_words: row.exclude_words,
                source: Source::from_stored(row.source),
                list_id: row.list_id,
                user_ids: row.user_ids,
                with_replies: row.with_replies,
                media_only: row.media_only,
                notify: row.notify,
                created_at: row.created_at,
            })
            .collect();
        let matcher = Arc::new(Matcher::build(specs));
        *slot = Some(Built { version, at: Instant::now(), matcher: matcher.clone() });
        Ok(matcher)
    }

    /// The `notes.antenna_match` job: puts a new note into the antennas it
    /// matches and returns into how many it went. Running it again for the
    /// same note changes nothing.
    pub async fn match_antennas(&self, note_id: Uuid) -> AppResult<usize> {
        let core = &self.core;
        let sql = format!("SELECT {NOTE_COLUMNS} FROM notes n WHERE n.id = $1");
        let note: Option<NoteRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(note_id).fetch_optional(&core.db).await?;
        let Some(note) = note else { return Ok(0) };
        let matcher = self.matcher().await?;
        if matcher.is_empty() {
            return Ok(0);
        }

        let author = note.author_id;
        let (followers, friends) =
            tokio::try_join!(graph::follower_ids(core, author), graph::friend_ids(core, author))?;
        let followers: HashSet<Uuid> = followers.into_iter().chain(friends).collect();
        let lists: Vec<Uuid> = sqlx::query_scalar("SELECT list_id FROM note_list_members WHERE user_id = $1")
            .bind(author)
            .fetch_all(&core.db)
            .await?;
        let text = format!("{}\n{}", note.text, note.content_warning_text);
        let candidate = Candidate {
            text: &text,
            is_reply: note.is_reply(),
            has_media: !note.asset_ids.is_empty(),
            created_at: Some(note.created_at),
        };
        let matched = matcher.matching(author, &candidate, &followers, &lists);
        if matched.is_empty() {
            return Ok(0);
        }

        // Each owner is asked once whether they may see the note.
        let mut owners: Vec<Uuid> = matched.iter().map(|index| matcher.watch(*index).owner_id).collect();
        owners.sort_unstable();
        owners.dedup();
        let active: HashSet<Uuid> = tokuly_identity::users::filter_active(core, &owners).await?.into_iter().collect();
        let mut seeing = HashSet::new();
        for owner in owners.into_iter().filter(|owner| active.contains(owner)) {
            let viewer = Viewer::load(core, Some(owner)).await?;
            if viewer.hides(author) || !viewer.can_see_note(&core.db, note.id).await? {
                continue;
            }
            if viewer.newly_locked(core, &[&note]).await?.contains(&author) && !viewer.is_tied_to(author) {
                continue;
            }
            seeing.insert(owner);
        }
        let watches: Vec<&Watch> = matched
            .iter()
            .map(|index| matcher.watch(*index))
            .filter(|watch| seeing.contains(&watch.owner_id))
            .collect();
        if watches.is_empty() {
            return Ok(0);
        }

        let ids: Vec<Uuid> = watches.iter().map(|watch| watch.id).collect();
        let mut tx = core.db.begin().await?;
        // Antennas deleted since the matcher was built are skipped by the join.
        let collected: Vec<Uuid> = sqlx::query_scalar(
            "INSERT INTO antenna_notes (antenna_id, note_id)
             SELECT a.id, $2 FROM antennas a WHERE a.id = ANY($1)
             ON CONFLICT DO NOTHING RETURNING antenna_id",
        )
        .bind(&ids)
        .bind(note.id)
        .fetch_all(&mut *tx)
        .await?;
        if collected.is_empty() {
            tx.commit().await?;
            return Ok(0);
        }
        sqlx::query("UPDATE antennas SET last_matched_at = now() WHERE id = ANY($1)")
            .bind(&collected)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM antenna_notes a
             USING (SELECT t.id, (SELECT x.note_id FROM antenna_notes x WHERE x.antenna_id = t.id
                                  ORDER BY x.note_id DESC OFFSET $2 LIMIT 1) AS oldest_kept
                    FROM unnest($1::uuid[]) AS t(id)) f
             WHERE a.antenna_id = f.id AND a.note_id < f.oldest_kept",
        )
        .bind(&collected)
        .bind(MAX_COLLECTED - 1)
        .execute(&mut *tx)
        .await?;
        for watch in watches.iter().filter(|watch| watch.notify && collected.contains(&watch.id)) {
            // Valkey only: no second database connection while the transaction is open.
            let quiet = !core
                .cache
                .set_nx(&format!("notes:notified:antenna:{}", watch.id), "1", NOTICE_INTERVAL)
                .await
                .unwrap_or(true);
            if !quiet {
                outbound::notify(&mut *tx, &antenna_notice(watch, &note)).await?;
            }
        }
        tx.commit().await?;
        Ok(collected.len())
    }

    /// Checks what an antenna names against the database: the list is one
    /// of the owner's, the accounts are in good standing and not across a
    /// block with the owner.
    async fn check_antenna_sources(&self, owner: Uuid, spec: &AntennaSpec) -> AppResult<()> {
        if let Some(list_id) = spec.list_id {
            self.require_own_list(owner, list_id).await?;
        }
        if !spec.user_ids.is_empty() {
            let active = tokuly_identity::users::filter_active(&self.core, &spec.user_ids).await?;
            let hidden = graph::blocked_ids_either(&self.core, owner).await?;
            ensure!(
                active.len() == spec.user_ids.len() && spec.user_ids.iter().all(|id| !hidden.contains(id)),
                not_found,
                "USER_NOT_FOUND",
                "user not found"
            );
        }
        Ok(())
    }

    pub(crate) async fn create_antenna(&self, owner: Uuid, spec: AntennaSpec) -> AppResult<AntennaRow> {
        self.core
            .cache
            .rate_limit(&format!("notes:antenna-change:{owner}"), CHANGES_PER_HOUR, Duration::from_secs(3600))
            .await?;
        self.check_antenna_sources(owner, &spec).await?;
        let mut tx = self.core.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("notes:antennas:{owner}"))
            .execute(&mut *tx)
            .await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM antennas WHERE owner_id = $1")
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(count < MAX_ANTENNAS, precondition, "TOO_MANY_ANTENNAS", "at most {MAX_ANTENNAS} antennas");
        let sql = format!(
            "INSERT INTO antennas AS a (owner_id, name, keywords, exclude_words, source, list_id, user_ids, with_replies, media_only, notify)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING {ANTENNA_COLUMNS}"
        );
        let row: AntennaRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(owner)
            .bind(&spec.name)
            .bind(to_json(&spec.keyword_groups())?)
            .bind(&spec.exclude_words)
            .bind(spec.source as i32)
            .bind(spec.list_id)
            .bind(&spec.user_ids)
            .bind(spec.with_replies)
            .bind(spec.media_only)
            .bind(spec.notify)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        self.antennas_changed().await;
        Ok(row)
    }

    /// Changes what an antenna collects from now on; what it collected stays.
    pub(crate) async fn update_antenna(
        &self,
        owner: Uuid,
        antenna_id: Uuid,
        spec: AntennaSpec,
    ) -> AppResult<AntennaRow> {
        self.core
            .cache
            .rate_limit(&format!("notes:antenna-change:{owner}"), CHANGES_PER_HOUR, Duration::from_secs(3600))
            .await?;
        self.require_antenna(owner, antenna_id).await?;
        self.check_antenna_sources(owner, &spec).await?;
        let sql = format!(
            "UPDATE antennas AS a SET name = $3, keywords = $4, exclude_words = $5, source = $6, list_id = $7, user_ids = $8,
                    with_replies = $9, media_only = $10, notify = $11
             WHERE a.id = $1 AND a.owner_id = $2 RETURNING {ANTENNA_COLUMNS}"
        );
        let row: Option<AntennaRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(antenna_id)
            .bind(owner)
            .bind(&spec.name)
            .bind(to_json(&spec.keyword_groups())?)
            .bind(&spec.exclude_words)
            .bind(spec.source as i32)
            .bind(spec.list_id)
            .bind(&spec.user_ids)
            .bind(spec.with_replies)
            .bind(spec.media_only)
            .bind(spec.notify)
            .fetch_optional(&self.core.db)
            .await?;
        let row = row.or_not_found("ANTENNA_NOT_FOUND", "antenna not found")?;
        self.antennas_changed().await;
        Ok(row)
    }

    pub(crate) async fn delete_antenna(&self, owner: Uuid, antenna_id: Uuid) -> AppResult<()> {
        let deleted = sqlx::query("DELETE FROM antennas WHERE id = $1 AND owner_id = $2")
            .bind(antenna_id)
            .bind(owner)
            .execute(&self.core.db)
            .await?
            .rows_affected();
        ensure!(deleted == 1, not_found, "ANTENNA_NOT_FOUND", "antenna not found");
        self.antennas_changed().await;
        Ok(())
    }

    /// One of `owner`'s antennas, or `ANTENNA_NOT_FOUND`.
    pub(crate) async fn require_antenna(&self, owner: Uuid, antenna_id: Uuid) -> AppResult<AntennaRow> {
        let sql = format!("SELECT {ANTENNA_COLUMNS} FROM antennas a WHERE a.id = $1 AND a.owner_id = $2");
        let row: Option<AntennaRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(antenna_id).bind(owner).fetch_optional(&self.core.db).await?;
        row.or_not_found("ANTENNA_NOT_FOUND", "antenna not found")
    }

    pub(crate) async fn antennas_of(&self, owner: Uuid) -> AppResult<Vec<AntennaRow>> {
        let sql = format!("SELECT {ANTENNA_COLUMNS} FROM antennas a WHERE a.owner_id = $1 ORDER BY a.id");
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(owner).fetch_all(&self.core.db).await?)
    }

    /// Antennas as their owner (`viewer`) gets them.
    pub(crate) async fn antennas_to_proto(&self, viewer: &Viewer, rows: &[AntennaRow]) -> AppResult<Vec<Antenna>> {
        let mut user_ids: Vec<Uuid> = rows.iter().flat_map(|row| row.user_ids.iter().copied()).collect();
        user_ids.sort_unstable();
        user_ids.dedup();
        let users = tokuly_identity::summaries(&self.core, &user_ids).await?;
        Ok(rows
            .iter()
            .map(|row| Antenna {
                id: row.id.to_string(),
                name: row.name.clone(),
                keywords: row.keyword_groups(),
                exclude_words: row.exclude_words.clone(),
                source: match Source::from_stored(row.source) {
                    Source::Everyone => AntennaSource::ANTENNA_SOURCE_EVERYONE,
                    Source::Following => AntennaSource::ANTENNA_SOURCE_FOLLOWING,
                    Source::List => AntennaSource::ANTENNA_SOURCE_LIST,
                    Source::Users => AntennaSource::ANTENNA_SOURCE_USERS,
                }
                .into(),
                list_id: row.list_id.map(|id| id.to_string()).unwrap_or_default(),
                users: row
                    .user_ids
                    .iter()
                    .filter(|id| !viewer.hides(**id))
                    .map(|id| pick(&users, *id))
                    .collect::<Vec<UserSummary>>(),
                with_replies: row.with_replies,
                media_only: row.media_only,
                notify: row.notify,
                last_matched_at: opt_ts(row.last_matched_at),
                created_at: ts(row.created_at),
                updated_at: ts(row.updated_at),
                ..Default::default()
            })
            .collect())
    }

    /// The notes an antenna collected that its owner may still see, newest first.
    pub(crate) async fn antenna_timeline(
        &self,
        viewer: &Viewer,
        owner: Uuid,
        antenna_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<TimelineEntry>, PageInfo)> {
        self.require_antenna(owner, antenna_id).await?;
        let sql = format!(
            "SELECT a.note_id AS entry_id, NULL::uuid AS renoter_id, NULL::timestamptz AS renoted_at, {NOTE_COLUMNS}
             FROM antenna_notes a JOIN notes n ON n.id = a.note_id
             WHERE a.antenna_id = $7 AND ($8::uuid IS NULL OR a.note_id < $8) AND {}
             ORDER BY a.note_id DESC LIMIT $9",
            sees_note("n")
        );
        self.entries(viewer, page, sql, antenna_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(keywords: &[&[&str]], exclude: &[&str], source: Source) -> WatchSpec {
        WatchSpec {
            id: Uuid::now_v7(),
            owner_id: Uuid::now_v7(),
            name: "test".into(),
            keywords: keywords.iter().map(|group| group.iter().map(|word| word.to_string()).collect()).collect(),
            exclude_words: exclude.iter().map(|word| word.to_string()).collect(),
            source,
            list_id: None,
            user_ids: Vec::new(),
            with_replies: false,
            media_only: false,
            notify: false,
            created_at: DateTime::<Utc>::MIN_UTC,
        }
    }

    fn note(text: &str) -> Candidate<'_> {
        Candidate { text, ..Default::default() }
    }

    fn matches(matcher: &Matcher, text: &str) -> Vec<usize> {
        matcher.matching(Uuid::now_v7(), &note(text), &HashSet::new(), &[])
    }

    #[test]
    fn words_are_folded_for_width_and_case() {
        assert_eq!(fold("  ＴＯＫＵＬＹ\u{3000} ｶﾞｯｺｳ "), "tokuly ガッコウ");
        let matcher = Matcher::build(vec![spec(&[&["Tokuly"]], &[], Source::Everyone)]);
        assert_eq!(matches(&matcher, "ｔｏｋｕｌｙ最高!"), vec![0]);
        assert_eq!(matches(&matcher, "TOKULY"), vec![0]);
        assert!(matches(&matcher, "toku ly").is_empty());
    }

    #[test]
    fn groups_are_ands_joined_by_or_and_exclusions_win() {
        let matcher = Matcher::build(vec![spec(&[&["ラーメン", "渋谷"], &["つけ麺"]], &["閉店"], Source::Everyone)]);
        assert_eq!(matches(&matcher, "渋谷でラーメン食べた"), vec![0]);
        assert_eq!(matches(&matcher, "つけ麺うまい"), vec![0]);
        assert!(matches(&matcher, "ラーメン食べた").is_empty(), "one word of a group is not enough");
        assert!(matches(&matcher, "渋谷のラーメン屋が閉店").is_empty(), "an excluded word vetoes");
    }

    #[test]
    fn overlapping_words_are_all_found_in_one_pass() {
        let matcher = Matcher::build(vec![
            spec(&[&["abc"]], &[], Source::Everyone),
            spec(&[&["bcd"]], &[], Source::Everyone),
            spec(&[&["abcd", "x"]], &[], Source::Everyone),
        ]);
        assert_eq!(matches(&matcher, "abcd"), vec![0, 1]);
        assert_eq!(matches(&matcher, "x abcd"), vec![0, 1, 2]);
    }

    #[test]
    fn sources_find_their_antennas_from_the_author() {
        let (author, follower, list) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let mut following = spec(&[&["猫"]], &[], Source::Following);
        following.owner_id = follower;
        let mut on_list = spec(&[], &[], Source::List);
        on_list.list_id = Some(list);
        let mut named = spec(&[], &[], Source::Users);
        named.user_ids = vec![author];
        let matcher = Matcher::build(vec![following, on_list, named, spec(&[&["猫"]], &[], Source::Everyone)]);
        let followers: HashSet<Uuid> = [follower].into();
        let text = note("猫かわいい");
        assert_eq!(matcher.matching(author, &text, &followers, &[list]), vec![0, 1, 2, 3]);
        assert_eq!(matcher.matching(author, &text, &HashSet::new(), &[]), vec![2, 3]);
        assert_eq!(matcher.matching(Uuid::now_v7(), &note("犬"), &followers, &[]), Vec::<usize>::new());
    }

    #[test]
    fn replies_media_and_age_are_respected() {
        let mut replies = spec(&[&["a"]], &[], Source::Everyone);
        replies.with_replies = true;
        let mut media = spec(&[&["a"]], &[], Source::Everyone);
        media.media_only = true;
        let mut young = spec(&[&["a"]], &[], Source::Everyone);
        young.created_at = Utc::now();
        let matcher = Matcher::build(vec![spec(&[&["a"]], &[], Source::Everyone), replies, media, young]);
        let author = Uuid::now_v7();
        let none = HashSet::new();
        let reply = Candidate { text: "a", is_reply: true, ..Default::default() };
        assert_eq!(matcher.matching(author, &reply, &none, &[]), vec![1]);
        let photo = Candidate {
            text: "a",
            has_media: true,
            created_at: Some(Utc::now() - chrono::Duration::hours(1)),
            ..Default::default()
        };
        assert_eq!(
            matcher.matching(author, &photo, &none, &[]),
            vec![0, 1, 2],
            "the antenna younger than the note skips it"
        );
    }

    #[test]
    fn an_empty_matcher_matches_nothing() {
        let matcher = Matcher::build(Vec::new());
        assert!(matcher.is_empty() && matcher.hits("anything").is_empty());
    }

    fn input(fill: impl FnOnce(&mut AntennaInput)) -> AntennaInput {
        let mut input = AntennaInput {
            name: "猫".into(),
            keywords: vec![KeywordGroup { words: vec!["ネコ".into()], ..Default::default() }],
            ..Default::default()
        };
        fill(&mut input);
        input
    }

    fn reason(input: AntennaInput) -> String {
        AntennaSpec::parse(&input).unwrap_err().reason.into_owned()
    }

    #[test]
    fn antenna_input_is_checked() {
        let parsed = AntennaSpec::parse(&input(|input| {
            input
                .keywords
                .push(KeywordGroup {
                    words: vec!["ＣＡＴ".into(), " cat ".into(), "".into()], ..Default::default()
                });
            input.exclude_words = vec!["犬".into()];
        }))
        .unwrap();
        assert_eq!(parsed.keywords, vec![vec!["ネコ".to_owned()], vec!["cat".to_owned()]]);
        assert_eq!(parsed.source, Source::Everyone);
        assert_eq!(reason(input(|input| input.name = " ".into())), "INVALID_NAME");
        assert_eq!(reason(input(|input| input.name = "あ".repeat(31))), "INVALID_NAME");
        assert_eq!(reason(input(|input| input.keywords.clear())), "INVALID_KEYWORDS");
        assert_eq!(
            reason(input(|input| input.keywords = vec![KeywordGroup::default(); MAX_GROUPS + 1])),
            "INVALID_KEYWORDS"
        );
        assert_eq!(
            reason(input(|input| input.keywords[0].words = (0..6).map(|i| i.to_string()).collect())),
            "INVALID_KEYWORDS"
        );
        assert_eq!(
            reason(input(|input| input.exclude_words = (0..11).map(|i| i.to_string()).collect())),
            "INVALID_KEYWORDS"
        );
        assert_eq!(reason(input(|input| input.keywords[0].words = vec!["あ".repeat(51)])), "INVALID_KEYWORDS");
        // A list or accounts are enough without keywords.
        let list = AntennaSpec::parse(&input(|input| {
            input.keywords.clear();
            input.source = AntennaSource::ANTENNA_SOURCE_LIST.into();
            input.list_id = Uuid::now_v7().to_string();
        }))
        .unwrap();
        assert!(list.keywords.is_empty() && list.list_id.is_some());
        assert_eq!(reason(input(|input| input.source = AntennaSource::ANTENNA_SOURCE_LIST.into())), "LIST_NOT_FOUND");
        assert_eq!(reason(input(|input| input.source = AntennaSource::ANTENNA_SOURCE_USERS.into())), "INVALID_USERS");
        assert_eq!(
            reason(input(|input| {
                input.source = AntennaSource::ANTENNA_SOURCE_USERS.into();
                input.user_ids = (0..51).map(|_| Uuid::now_v7().to_string()).collect();
            })),
            "INVALID_USERS"
        );
    }
}
