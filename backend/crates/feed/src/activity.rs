//! Activity entries: storage, ingestion from the event stream and conversion
//! to API messages.
//!
//! An entry is stored once, when its `activity.recorded` event arrives, and
//! every list of the API (the owner's log, a profile, the friends' feed, map
//! stories, the home feed, recaps) is a read of the `activities` table that
//! decides *at read time* what the reader may see:
//!
//! - the entry's audience against the reader's relationship to the author;
//! - the author's hidden kinds ([`kind_shown`]);
//! - blocks: an entry that involves someone hidden from the reader (as the
//!   person it is about or as one of the people it names) is left out
//!   ([`push_not_involving`]), because its summary carries that person's name.
//!
//! Nothing is fanned out, so friendships, blocks and preferences that change
//! later need no fix-up.
use std::collections::{HashMap, HashSet};

use connectrpc::RequestContext;
use sqlx::{Postgres, QueryBuilder};
use tokuly_core::{
    events::Event,
    outbound::{ActivityAudience as EntryAudience, ActivityEntry, Localized},
    prelude::*,
};
use tokuly_identity::maintenance::UserCreated;
use tokuly_proto::{
    common::{LatLng, SubjectRef, SubjectType, UserSummary},
    feed::{Activity, ActivityAudience},
};

use crate::{Feed, kinds};

/// Columns of [`ActivityRow`], read from a table or subquery aliased `a`.
macro_rules! activity_columns {
    () => {
        "a.id, a.user_id, a.kind, a.audience, a.subject_type, a.subject_id, a.with_user_ids, a.summary_ja, a.summary_en, \
         a.path, a.image_url, a.lat, a.lng, a.data, a.occurred_at"
    };
}
pub(crate) use activity_columns;

/// SQL condition: the author of the entry aliased `$a` did not hide its kind
/// from other people.
macro_rules! kind_shown {
    ($a:literal) => {
        concat!(
            "NOT EXISTS (SELECT 1 FROM activity_hidden_kinds hk WHERE hk.user_id = ",
            $a,
            ".user_id AND (",
            $a,
            ".kind = hk.kind OR starts_with(",
            $a,
            ".kind, hk.kind || '.')))"
        )
    };
}
pub(crate) use kind_shown;

/// SQL condition: the author of the entry aliased `$a` is an active account.
macro_rules! author_active {
    ($a:literal) => {
        concat!("EXISTS (SELECT 1 FROM users au WHERE au.id = ", $a, ".user_id AND au.deleted_at IS NULL AND au.banned_at IS NULL)")
    };
}
pub(crate) use author_active;

/// `tokuly.feed.v1.ActivityAudience` numbers as stored in `activities.audience`.
pub(crate) const AUDIENCE_PRIVATE: i16 = 1;
pub(crate) const AUDIENCE_FRIENDS: i16 = 2;
pub(crate) const AUDIENCE_PUBLIC: i16 = 3;

const MAX_SUMMARY_CHARS: usize = 300;
const MAX_PATH_LEN: usize = 512;
const MAX_IMAGE_URL_LEN: usize = 2048;
const MAX_WITH_USERS: usize = 20;
const MAX_DATA_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ActivityRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub kind: String,
    pub audience: i16,
    pub subject_type: Option<i32>,
    pub subject_id: Option<Uuid>,
    pub with_user_ids: Vec<Uuid>,
    pub summary_ja: String,
    pub summary_en: String,
    pub path: String,
    pub image_url: Option<String>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub data: serde_json::Value,
    pub occurred_at: Time,
}

impl ActivityRow {
    /// The keyset every list sorts by, newest first.
    pub fn key(&self) -> (Time, Uuid) {
        (self.occurred_at, self.id)
    }

    pub fn summary(&self) -> Localized {
        Localized::new(&self.summary_ja, &self.summary_en)
    }

    /// Everyone the entry shows: the author and the people it names.
    pub fn people(&self) -> impl Iterator<Item = Uuid> + '_ {
        std::iter::once(self.user_id).chain(self.with_user_ids.iter().copied())
    }
}

/// An entry ready to be stored: an [`ActivityEntry`] with everything a
/// producer could get wrong cut down to what clients can rely on.
#[derive(Debug, Clone)]
pub struct NewActivity {
    pub user_id: Uuid,
    pub kind: String,
    pub audience: i16,
    pub subject: Option<(i32, Uuid)>,
    pub with_user_ids: Vec<Uuid>,
    pub summary: Localized,
    pub path: String,
    pub image_url: Option<String>,
    pub location: Option<(f64, f64)>,
    pub data: serde_json::Value,
    pub occurred_at: Time,
}

fn audience_number(audience: EntryAudience) -> i16 {
    match audience {
        EntryAudience::Private => AUDIENCE_PRIVATE,
        EntryAudience::Friends => AUDIENCE_FRIENDS,
        EntryAudience::Public => AUDIENCE_PUBLIC,
    }
}

/// One line of display text: no control characters, single spaces, at most
/// `max` characters.
fn clean_line(text: &str, max: usize) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned: String = collapsed.chars().filter(|c| !c.is_control()).collect();
    match cleaned.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", cleaned[..cut].trim_end()),
        None => cleaned,
    }
}

/// An in-app path: starts with a single `/`, no control characters.
fn clean_path(path: &str) -> String {
    let path = path.trim();
    let ok = path.starts_with('/') && !path.starts_with("//") && path.len() <= MAX_PATH_LEN && !path.chars().any(|c| c.is_control() || c.is_whitespace());
    if ok { path.to_owned() } else { String::new() }
}

/// An image reference clients may load: an http(s) URL or a root-relative path.
fn clean_image_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.len() > MAX_IMAGE_URL_LEN || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let absolute = url::Url::parse(url).ok().is_some_and(|parsed| matches!(parsed.scheme(), "http" | "https") && parsed.host().is_some());
    let relative = url.starts_with('/') && !url.starts_with("//");
    (absolute || relative).then(|| url.to_owned())
}

/// Turns a producer's entry into what is stored, or `None` when it is
/// unusable (malformed kind). `occurred_at` is the event's time.
pub fn sanitize(entry: ActivityEntry, occurred_at: Time, now: Time) -> Option<NewActivity> {
    if !kinds::is_valid(&entry.kind) {
        return None;
    }
    let (mut ja, mut en) = (clean_line(&entry.summary.ja, MAX_SUMMARY_CHARS), clean_line(&entry.summary.en, MAX_SUMMARY_CHARS));
    if ja.is_empty() && en.is_empty() {
        // Better a generic line than an empty card.
        let label = kinds::label(&entry.kind);
        (ja, en) = (label.ja, label.en);
    } else if ja.is_empty() {
        ja = en.clone();
    } else if en.is_empty() {
        en = ja.clone();
    }

    let mut seen = HashSet::new();
    let with_user_ids: Vec<Uuid> =
        entry.with_user_ids.into_iter().filter(|id| *id != entry.user_id && !id.is_nil() && seen.insert(*id)).take(MAX_WITH_USERS).collect();
    let location = entry.location.filter(|(lat, lng)| lat.is_finite() && lng.is_finite() && (-90.0..=90.0).contains(lat) && (-180.0..=180.0).contains(lng));
    let data = if entry.data.is_null() || entry.data.to_string().len() <= MAX_DATA_BYTES { entry.data } else { serde_json::Value::Null };

    Some(NewActivity {
        user_id: entry.user_id,
        kind: entry.kind,
        audience: audience_number(entry.audience),
        subject: entry.subject.filter(|(subject_type, id)| *subject_type > 0 && !id.is_nil()),
        with_user_ids,
        summary: Localized::new(ja, en),
        path: clean_path(&entry.path),
        image_url: entry.image_url.as_deref().and_then(clean_image_url),
        location,
        data,
        // A clock that runs ahead must not pin an entry to the top of every list.
        occurred_at: occurred_at.min(now),
    })
}

/// What became of one event that carries an activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    /// Stored as the entry with this ID.
    Stored(Uuid),
    /// The event had been handled before (a redelivery).
    Duplicate,
    /// Unusable entry, or its user does not exist (anymore).
    Dropped,
}

/// The entry recorded for a new account ("Tokulyに参加しました！").
fn joined_entry(user_id: Uuid, handle: &str) -> ActivityEntry {
    let entry = ActivityEntry::new(user_id, kinds::ACCOUNT_JOINED, EntryAudience::Public, Localized::new("Tokulyに参加しました！", "Joined Tokuly!"));
    if handle.is_empty() { entry } else { entry.path(format!("/u/{handle}")) }
}

impl Feed {
    /// Handles `activity.recorded`: stores the entry, once per event.
    pub async fn handle_activity_recorded(&self, event: &Event) -> AppResult<IngestOutcome> {
        let entry: ActivityEntry = event.data()?;
        self.ingest(event, entry).await
    }

    /// Handles `identity.user.created`: the account's first log entry.
    pub async fn handle_user_created(&self, event: &Event) -> AppResult<IngestOutcome> {
        let created: UserCreated = event.data()?;
        self.ingest(event, joined_entry(created.user_id, &created.handle)).await
    }

    /// Stores `entry` unless `event` was handled before. The event ID is
    /// recorded in the same transaction, so a redelivery changes nothing and
    /// an entry its owner deleted does not come back.
    async fn ingest(&self, event: &Event, entry: ActivityEntry) -> AppResult<IngestOutcome> {
        let Some(new) = sanitize(entry, event.occurred_at, now()) else {
            tracing::warn!(event = %event.id, topic = %event.topic, "dropping a malformed activity entry");
            return Ok(IngestOutcome::Dropped);
        };
        let mut tx = self.core.db.begin().await?;
        let claimed = sqlx::query("INSERT INTO feed_processed_events (event_id) VALUES ($1) ON CONFLICT DO NOTHING").bind(event.id).execute(&mut *tx).await?;
        if claimed.rows_affected() == 0 {
            return Ok(IngestOutcome::Duplicate);
        }
        // The SELECT keeps entries of unknown or deleted accounts out without
        // tripping the foreign key (which would abort the transaction).
        let id: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO activities (user_id, kind, audience, subject_type, subject_id, with_user_ids, summary_ja, summary_en,
                                     path, image_url, lat, lng, data, occurred_at)
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14
             WHERE EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)
             RETURNING id",
        )
        .bind(new.user_id)
        .bind(&new.kind)
        .bind(new.audience)
        .bind(new.subject.map(|(subject_type, _)| subject_type))
        .bind(new.subject.map(|(_, id)| id))
        .bind(&new.with_user_ids)
        .bind(&new.summary.ja)
        .bind(&new.summary.en)
        .bind(&new.path)
        .bind(&new.image_url)
        .bind(new.location.map(|(lat, _)| lat))
        .bind(new.location.map(|(_, lng)| lng))
        .bind(&new.data)
        .bind(new.occurred_at)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id.map_or(IngestOutcome::Dropped, IngestOutcome::Stored))
    }
}

/// Who is reading, and what that means for what they are shown.
#[derive(Debug, Clone)]
pub(crate) struct Reader {
    pub viewer: Option<Uuid>,
    /// Language summaries and labels are returned in.
    pub locale: String,
    /// Everyone hidden from the viewer by a block, in either direction.
    pub hidden: Vec<Uuid>,
}

impl Reader {
    pub fn is_hidden(&self, user_id: Uuid) -> bool {
        self.hidden.contains(&user_id)
    }
}

/// Adds `AND <the entry aliased `alias` involves nobody in `hidden`>` to a
/// query: neither the person it is about nor the people it names.
pub(crate) fn push_not_involving(query: &mut QueryBuilder<Postgres>, alias: &str, hidden: &[Uuid]) {
    if hidden.is_empty() {
        return;
    }
    let user_subject = SubjectType::SUBJECT_TYPE_USER as i32;
    query.push(format!(" AND NOT ({alias}.with_user_ids && "));
    query.push_bind(hidden.to_vec());
    query.push(format!(") AND NOT (COALESCE({alias}.subject_type, 0) = {user_subject} AND {alias}.subject_id = ANY("));
    query.push_bind(hidden.to_vec());
    query.push("))");
}

/// Adds `AND <the entry aliased `alias` is of a kind under `prefix`>`.
pub(crate) fn push_kind_filter(query: &mut QueryBuilder<Postgres>, alias: &str, prefix: Option<&str>) {
    if let Some(prefix) = prefix {
        query.push(format!(" AND ({alias}.kind = "));
        query.push_bind(prefix.to_owned());
        query.push(format!(" OR starts_with({alias}.kind, "));
        query.push_bind(format!("{prefix}."));
        query.push("))");
    }
}

impl Feed {
    /// The reader of a request: the caller's language (their account's, or
    /// the request's `Accept-Language` when anonymous) and block list.
    pub(crate) async fn reader(&self, ctx: &RequestContext) -> AppResult<Reader> {
        let viewer = ctx.maybe_user_id()?;
        match viewer {
            Some(user_id) => self.reader_for(user_id).await,
            None => {
                let language = ctx.client().language;
                Ok(Reader { viewer: None, locale: if language.is_empty() { "ja".to_owned() } else { language }, hidden: Vec::new() })
            }
        }
    }

    /// The reader context of a signed-in user.
    pub(crate) async fn reader_for(&self, user_id: Uuid) -> AppResult<Reader> {
        let locale: Option<String> = sqlx::query_scalar("SELECT locale FROM users WHERE id = $1").bind(user_id).fetch_optional(&self.core.db).await?;
        let hidden = tokuly_social::graph::blocked_ids_either(&self.core, user_id).await?;
        Ok(Reader { viewer: Some(user_id), locale: locale.filter(|l| !l.is_empty()).unwrap_or_else(|| "ja".to_owned()), hidden })
    }

    /// Summaries of everyone the rows show.
    pub(crate) async fn people_of(&self, rows: &[&ActivityRow]) -> AppResult<HashMap<Uuid, UserSummary>> {
        let mut ids: Vec<Uuid> = rows.iter().flat_map(|row| row.people()).collect();
        ids.sort_unstable();
        ids.dedup();
        tokuly_identity::summaries(&self.core, &ids).await
    }

    /// Entries as API messages, in the reader's language. `own_hidden_kinds`
    /// is set when the reader is looking at their own log: entries of those
    /// kinds are flagged `hidden_from_others`.
    pub(crate) async fn to_activities(&self, reader: &Reader, rows: &[ActivityRow], own_hidden_kinds: Option<&[String]>) -> AppResult<Vec<Activity>> {
        let people = self.people_of(&rows.iter().collect::<Vec<_>>()).await?;
        Ok(rows.iter().map(|row| to_activity(row, reader, &people, own_hidden_kinds)).collect())
    }
}

/// One entry as an API message.
pub(crate) fn to_activity(row: &ActivityRow, reader: &Reader, people: &HashMap<Uuid, UserSummary>, own_hidden_kinds: Option<&[String]>) -> Activity {
    let subject = match (row.subject_type, row.subject_id) {
        (Some(subject_type), Some(id)) => Some(SubjectRef { r#type: subject_type.into(), id: id.to_string(), ..Default::default() }),
        _ => None,
    };
    let location = match (row.lat, row.lng) {
        (Some(lat), Some(lng)) => Some(LatLng { lat, lng, ..Default::default() }),
        _ => None,
    };
    let with_users = row.with_user_ids.iter().filter(|id| !reader.is_hidden(**id)).filter_map(|id| people.get(id).cloned()).collect();
    Activity {
        id: row.id.to_string(),
        actor: tokuly_identity::users::pick(people, row.user_id).into(),
        kind: row.kind.clone(),
        kind_label: kinds::label(&row.kind).pick(&reader.locale).to_owned(),
        audience: audience_of(row.audience).into(),
        subject: subject.into(),
        with_users,
        summary: row.summary().pick(&reader.locale).to_owned(),
        path: row.path.clone(),
        image_url: row.image_url.clone().unwrap_or_default(),
        location: location.into(),
        data_json: if row.data.is_null() { String::new() } else { row.data.to_string() },
        occurred_at: ts(row.occurred_at),
        hidden_from_others: own_hidden_kinds.is_some_and(|hidden| hidden.iter().any(|prefix| kinds::matches(&row.kind, prefix))),
        ..Default::default()
    }
}

fn audience_of(stored: i16) -> ActivityAudience {
    match stored {
        AUDIENCE_PRIVATE => ActivityAudience::ACTIVITY_AUDIENCE_PRIVATE,
        AUDIENCE_FRIENDS => ActivityAudience::ACTIVITY_AUDIENCE_FRIENDS,
        AUDIENCE_PUBLIC => ActivityAudience::ACTIVITY_AUDIENCE_PUBLIC,
        _ => ActivityAudience::ACTIVITY_AUDIENCE_UNSPECIFIED,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn entry(kind: &str) -> ActivityEntry {
        ActivityEntry::new(Uuid::now_v7(), kind, EntryAudience::Friends, Localized::new("ノートを投稿しました", "Posted a note"))
    }

    fn clean(entry: ActivityEntry) -> NewActivity {
        let at = now();
        sanitize(entry, at, at).expect("usable entry")
    }

    #[test]
    fn well_formed_entries_pass_through_unchanged() {
        let (subject, friend) = (Uuid::now_v7(), Uuid::now_v7());
        let original = entry("note.posted")
            .subject(SubjectType::SUBJECT_TYPE_NOTE, subject)
            .with([friend])
            .path("/notes/abc")
            .image("https://cdn.example/a.webp")
            .at(35.6595, 139.7005)
            .data(serde_json::json!({ "likes": 3 }));
        let user_id = original.user_id;
        let stored = clean(original);
        assert_eq!(stored.user_id, user_id);
        assert_eq!(stored.kind, "note.posted");
        assert_eq!(stored.audience, AUDIENCE_FRIENDS);
        assert_eq!(stored.subject, Some((SubjectType::SUBJECT_TYPE_NOTE as i32, subject)));
        assert_eq!(stored.with_user_ids, vec![friend]);
        assert_eq!((stored.summary.ja.as_str(), stored.summary.en.as_str()), ("ノートを投稿しました", "Posted a note"));
        assert_eq!(stored.path, "/notes/abc");
        assert_eq!(stored.image_url.as_deref(), Some("https://cdn.example/a.webp"));
        assert_eq!(stored.location, Some((35.6595, 139.7005)));
        assert_eq!(stored.data, serde_json::json!({ "likes": 3 }));
    }

    #[test]
    fn malformed_kinds_are_dropped() {
        let at = now();
        for kind in ["", "Note.Posted", "note posted", "note..posted"] {
            assert!(sanitize(entry(kind), at, at).is_none(), "{kind:?}");
        }
    }

    #[test]
    fn audiences_map_to_their_stored_numbers() {
        for (audience, stored) in [(EntryAudience::Private, AUDIENCE_PRIVATE), (EntryAudience::Friends, AUDIENCE_FRIENDS), (EntryAudience::Public, AUDIENCE_PUBLIC)] {
            let entry = ActivityEntry::new(Uuid::now_v7(), "note.posted", audience, Localized::same("x"));
            assert_eq!(clean(entry).audience, stored);
            assert_eq!(audience_of(stored) as i32, i32::from(stored));
        }
    }

    #[test]
    fn a_missing_language_falls_back_to_the_other_and_then_to_the_kind_label() {
        let mut only_ja = entry("note.posted");
        only_ja.summary = Localized::new("こんにちは", "  ");
        assert_eq!(clean(only_ja).summary.en, "こんにちは");

        let mut only_en = entry("note.posted");
        only_en.summary = Localized::new("", "Hello");
        assert_eq!(clean(only_en).summary.ja, "Hello");

        let mut neither = entry("social.bumped");
        neither.summary = Localized::new("", "");
        let stored = clean(neither);
        assert_eq!((stored.summary.ja.as_str(), stored.summary.en.as_str()), ("バンプ", "Bumps"));
    }

    #[test]
    fn summaries_are_single_lines_of_bounded_length() {
        let mut noisy = entry("note.posted");
        noisy.summary = Localized::new("一行目\n二行目\u{0007}  です", "x".repeat(1000));
        let stored = clean(noisy);
        assert_eq!(stored.summary.ja, "一行目 二行目 です");
        assert_eq!(stored.summary.en.chars().count(), MAX_SUMMARY_CHARS + 1, "cut to the limit plus an ellipsis");
        assert!(stored.summary.en.ends_with('…'));
    }

    #[test]
    fn only_in_app_paths_and_loadable_images_are_kept() {
        for (path, kept) in [("/watch/abc", "/watch/abc"), ("watch/abc", ""), ("//evil.example/x", ""), ("https://evil.example", ""), ("/a b", ""), ("", "")] {
            assert_eq!(clean(entry("video.published").path(path)).path, kept, "{path:?}");
        }
        for (url, kept) in [
            ("https://cdn.example/a.png", true),
            ("http://cdn.example/a.png", true),
            ("/media/a.png", true),
            ("//cdn.example/a.png", false),
            ("javascript:alert(1)", false),
            ("data:image/png;base64,AAAA", false),
            ("ftp://cdn.example/a.png", false),
            ("https://cdn.example/a b.png", false),
            ("", false),
        ] {
            assert_eq!(clean(entry("video.published").image(url)).image_url.is_some(), kept, "{url:?}");
        }
    }

    #[test]
    fn people_are_deduplicated_capped_and_never_include_the_author() {
        let original = entry("social.bumped");
        let author = original.user_id;
        let friend = Uuid::now_v7();
        let crowd: Vec<Uuid> = (0..40).map(|_| Uuid::now_v7()).collect();
        let stored = clean(original.with([friend, author, friend, Uuid::nil()].into_iter().chain(crowd.iter().copied())));
        assert_eq!(stored.with_user_ids.len(), MAX_WITH_USERS);
        assert_eq!(stored.with_user_ids[0], friend);
        assert!(!stored.with_user_ids.contains(&author) && !stored.with_user_ids.contains(&Uuid::nil()));
        assert_eq!(stored.with_user_ids.iter().collect::<HashSet<_>>().len(), MAX_WITH_USERS);
    }

    #[test]
    fn impossible_locations_subjects_and_oversized_data_are_discarded() {
        for (lat, lng) in [(91.0, 0.0), (-91.0, 0.0), (0.0, 181.0), (f64::NAN, 0.0), (0.0, f64::INFINITY)] {
            assert_eq!(clean(entry("geo.visit").at(lat, lng)).location, None, "{lat},{lng}");
        }
        assert_eq!(clean(entry("geo.visit").at(-90.0, 180.0)).location, Some((-90.0, 180.0)));

        let mut no_subject = entry("note.posted");
        no_subject.subject = Some((0, Uuid::now_v7()));
        assert_eq!(clean(no_subject).subject, None);
        let mut nil_subject = entry("note.posted");
        nil_subject.subject = Some((3, Uuid::nil()));
        assert_eq!(clean(nil_subject).subject, None);

        let big = serde_json::json!({ "blob": "x".repeat(MAX_DATA_BYTES) });
        assert!(clean(entry("note.posted").data(big)).data.is_null());
    }

    #[test]
    fn entries_from_the_future_are_pulled_back_to_now() {
        let at = now();
        let stored = sanitize(entry("note.posted"), at + Duration::hours(3), at).unwrap();
        assert_eq!(stored.occurred_at, at);
        let stored = sanitize(entry("note.posted"), at - Duration::hours(3), at).unwrap();
        assert_eq!(stored.occurred_at, at - Duration::hours(3));
    }

    #[test]
    fn the_joined_entry_is_public_and_links_to_the_profile() {
        let user = Uuid::now_v7();
        let entry = joined_entry(user, "sakura");
        assert_eq!(entry.kind, kinds::ACCOUNT_JOINED);
        assert_eq!(entry.audience, EntryAudience::Public);
        assert_eq!(entry.summary.ja, "Tokulyに参加しました！");
        assert_eq!(entry.path, "/u/sakura");
        assert_eq!(joined_entry(user, "").path, "");
    }

    fn row(kind: &str) -> ActivityRow {
        ActivityRow {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            kind: kind.to_owned(),
            audience: AUDIENCE_FRIENDS,
            subject_type: None,
            subject_id: None,
            with_user_ids: Vec::new(),
            summary_ja: "バンプしました".into(),
            summary_en: "Bumped".into(),
            path: "/u/x".into(),
            image_url: None,
            lat: None,
            lng: None,
            data: serde_json::Value::Null,
            occurred_at: now(),
        }
    }

    fn reader(locale: &str, hidden: Vec<Uuid>) -> Reader {
        Reader { viewer: Some(Uuid::now_v7()), locale: locale.to_owned(), hidden }
    }

    fn summary(id: Uuid, name: &str) -> UserSummary {
        UserSummary { id: id.to_string(), handle: name.to_owned(), display_name: name.to_owned(), ..Default::default() }
    }

    #[test]
    fn messages_are_localized_and_carry_the_structured_fields() {
        let mut row = row("social.bumped");
        let partner = Uuid::now_v7();
        row.subject_type = Some(SubjectType::SUBJECT_TYPE_USER as i32);
        row.subject_id = Some(partner);
        row.with_user_ids = vec![partner];
        row.lat = Some(35.0);
        row.lng = Some(139.0);
        row.data = serde_json::json!({ "bump_id": "b1" });
        let people = HashMap::from([(row.user_id, summary(row.user_id, "alice")), (partner, summary(partner, "bob"))]);

        let ja = to_activity(&row, &reader("ja", vec![]), &people, None);
        assert_eq!((ja.summary.as_str(), ja.kind_label.as_str()), ("バンプしました", "バンプ"));
        assert_eq!(ja.actor.as_option().unwrap().handle, "alice");
        assert_eq!(ja.with_users.len(), 1);
        assert_eq!(ja.subject.as_option().unwrap().id, partner.to_string());
        assert_eq!(ja.location.as_option().unwrap().lat, 35.0);
        assert_eq!(ja.data_json, r#"{"bump_id":"b1"}"#);
        assert_eq!(ja.audience, ActivityAudience::ACTIVITY_AUDIENCE_FRIENDS);
        assert!(!ja.hidden_from_others);

        let en = to_activity(&row, &reader("en-US", vec![]), &people, None);
        assert_eq!((en.summary.as_str(), en.kind_label.as_str()), ("Bumped", "Bumps"));
    }

    #[test]
    fn blocked_people_are_left_out_of_the_named_users() {
        let mut row = row("social.bumped");
        let (partner, blocked) = (Uuid::now_v7(), Uuid::now_v7());
        row.with_user_ids = vec![partner, blocked];
        let people = HashMap::from([(partner, summary(partner, "bob")), (blocked, summary(blocked, "mallory"))]);
        let message = to_activity(&row, &reader("ja", vec![blocked]), &people, None);
        assert_eq!(message.with_users.iter().map(|u| u.handle.as_str()).collect::<Vec<_>>(), vec!["bob"]);
        // The author is unknown to the map: a placeholder, never a panic.
        assert!(message.actor.as_option().unwrap().deleted);
    }

    #[test]
    fn own_entries_of_hidden_kinds_are_flagged() {
        let hidden = vec!["social".to_owned()];
        assert!(to_activity(&row("social.bumped"), &reader("ja", vec![]), &HashMap::new(), Some(&hidden)).hidden_from_others);
        assert!(!to_activity(&row("note.posted"), &reader("ja", vec![]), &HashMap::new(), Some(&hidden)).hidden_from_others);
        assert!(!to_activity(&row("social.bumped"), &reader("ja", vec![]), &HashMap::new(), None).hidden_from_others);
        assert!(to_activity(&row("note.posted"), &reader("ja", vec![]), &HashMap::new(), None).data_json.is_empty());
    }
}
