//! Writing notes: posting (notes, replies, quote notes), deleting, changing
//! who may see a note, and pinning.
//!
//! A note is immutable once posted. Everything that follows from a post —
//! hashtags, mentions, counters on the notes it answers or quotes, the
//! search document, the activity entry and the notifications — is written
//! in the transaction that stores the note.
use std::{
    collections::{HashMap, hash_map::Entry},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    analytics::AnalyticsEvent,
    events,
    outbound::{self, SearchDoc},
    prelude::*,
};
use tokuly_identity::settings;
use tokuly_media::assets;
use tokuly_proto::{
    common::{UserSummary, Visibility},
    media::{AssetKind, AssetPurpose, Attachment},
    notes::{Mention, NoteCounters, PlaceTag, notes_event::Event as Push},
};
use tokuly_social::{InteractionKind, graph};
use tokuly_stamp::SendContext;

use crate::{
    Notes, copy,
    events::{
        NOTE_CREATED, NOTE_DELETED, NOTE_VISIBILITY_CHANGED, NoteCreated, NoteDeleted, NoteVisibilityChanged,
        RemovalReason,
    },
    hashtags,
    model::{NOTE_COLUMNS, NoteRow, note_holder, note_path, note_topic, to_json},
    rules, text,
    visibility::{self, Viewer},
};

/// Longest note, in characters.
pub const MAX_NOTE_CHARS: usize = 300;
/// A note carries up to four images, or one video.
pub const MAX_ATTACHMENTS: usize = 4;
pub const MAX_PLACE_NAME_CHARS: usize = 100;
/// Notes one user may post per day, whatever the pace.
const MAX_POSTS_PER_DAY: u32 = 1000;
/// Longest idempotency key accepted.
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 64;
/// How long a retried request finds the note it created the first time.
const IDEMPOTENCY_WINDOW: Duration = Duration::from_secs(600);

/// Name of the background job that fetches a note's link preview.
pub const JOB_LINK_PREVIEW: &str = "notes.link_preview";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkPreviewJob {
    pub note_id: Uuid,
}

/// What a user asks to post.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewNote {
    pub text: String,
    pub asset_ids: Vec<Uuid>,
    pub stamp_id: Option<Uuid>,
    pub place: Option<PlaceTag>,
    /// `None` = the author's default.
    pub visibility: Option<i32>,
    pub reply_to_id: Option<Uuid>,
    pub quote_of_id: Option<Uuid>,
    pub content_warning: bool,
}

/// A place tag as stored.
struct Place {
    name: String,
    lat: f64,
    lng: f64,
    place_id: Option<Uuid>,
}

/// Counters of notes that changed as a side effect, to announce once the
/// transaction has committed.
#[derive(Debug, Default)]
pub(crate) struct Removed {
    pub note_ids: Vec<Uuid>,
    pub touched: Vec<NoteCounters>,
}

#[derive(sqlx::FromRow)]
struct CounterRow {
    id: Uuid,
    reply_count: i64,
    renote_count: i64,
    quote_count: i64,
    like_count: i64,
    view_count: i64,
}

impl From<CounterRow> for NoteCounters {
    fn from(row: CounterRow) -> Self {
        Self {
            note_id: row.id.to_string(),
            reply_count: row.reply_count,
            renote_count: row.renote_count,
            quote_count: row.quote_count,
            like_count: row.like_count,
            view_count: row.view_count,
            ..Default::default()
        }
    }
}

/// The visibility a client asked for: `None` when left unspecified.
pub(crate) fn requested_visibility(value: buffa::EnumValue<Visibility>) -> AppResult<Option<i32>> {
    match value.as_known() {
        Some(Visibility::VISIBILITY_UNSPECIFIED) => Ok(None),
        Some(Visibility::VISIBILITY_PUBLIC) => Ok(Some(visibility::PUBLIC)),
        Some(Visibility::VISIBILITY_FRIENDS) => Ok(Some(visibility::FRIENDS)),
        Some(Visibility::VISIBILITY_PRIVATE) => Ok(Some(visibility::PRIVATE)),
        _ => Err(AppError::invalid("INVALID_VISIBILITY", "a note is public, for friends, or private")),
    }
}

/// The narrower of two visibilities (public ⊃ friends ⊃ private).
pub(crate) fn narrower(a: i32, b: i32) -> i32 {
    let rank = |visibility: i32| match visibility {
        visibility::PUBLIC => 0,
        visibility::FRIENDS => 1,
        _ => 2,
    };
    if rank(a) >= rank(b) { a } else { b }
}

fn validate_place(place: &PlaceTag) -> AppResult<Place> {
    let invalid = |message: &'static str| AppError::invalid("INVALID_PLACE", message);
    let name = text::clean(&place.name).split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || !text::fits(&name, MAX_PLACE_NAME_CHARS) {
        return Err(invalid("a place tag needs a name of 1-100 characters"));
    }
    let location = place.location.as_option().ok_or_else(|| invalid("a place tag needs coordinates"))?;
    let (lat, lng) = (location.lat, location.lng);
    if !(lat.is_finite() && lng.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lng)) {
        return Err(invalid("the coordinates of the place tag are out of range"));
    }
    let place_id = if place.place_id.is_empty() {
        None
    } else {
        Some(Uuid::parse_str(&place.place_id).map_err(|_| invalid("place_id is not an ID"))?)
    };
    Ok(Place { name, lat, lng, place_id })
}

/// The search document of a note (`kind: "note"`). Only public notes are
/// indexed; passing a non-public one removes the document.
pub(crate) fn search_doc(note: &NoteRow, author: &UserSummary) -> SearchDoc {
    SearchDoc {
        kind: "note".to_owned(),
        id: note.id,
        title: note.title(),
        text: note.text.clone(),
        tags: note.hashtags.clone(),
        owner_id: Some(note.author_id),
        public: note.is_public(),
        image_url: note.image_url().filter(|_| !note.content_warning),
        path: note_path(note.id),
        created_at: note.created_at.timestamp(),
        score: (note.like_count + 2 * note.renote_count + note.reply_count + note.quote_count) as f64,
        extra: serde_json::json!({
            "author_handle": author.handle,
            "author_name": author.display_name,
            "reply_to_id": note.reply_to_id,
            "content_warning": note.content_warning,
            "attachment_count": note.asset_ids.len(),
        }),
    }
}

impl Notes {
    /// Resolves the uploads of a new note: up to four images (animations
    /// count as images) or exactly one video, uploaded by the author for notes.
    async fn resolve_attachments(&self, author: Uuid, asset_ids: &[Uuid]) -> AppResult<Vec<Attachment>> {
        let mut unique = Vec::with_capacity(asset_ids.len());
        for id in asset_ids {
            if !unique.contains(id) {
                unique.push(*id);
            }
        }
        ensure!(
            unique.len() <= MAX_ATTACHMENTS,
            invalid,
            "TOO_MANY_ATTACHMENTS",
            "a note carries at most {MAX_ATTACHMENTS} images"
        );
        let purposes = [AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, AssetPurpose::ASSET_PURPOSE_NOTE_VIDEO];
        let mut attachments = Vec::with_capacity(unique.len());
        let mut videos = 0;
        for asset_id in unique {
            let asset = assets::require_ready(&self.core, asset_id, author, &purposes).await?;
            match asset.asset_kind() {
                AssetKind::ASSET_KIND_IMAGE | AssetKind::ASSET_KIND_ANIMATION => {}
                AssetKind::ASSET_KIND_VIDEO => videos += 1,
                _ => bail!(invalid, "ATTACHMENT_UNSUPPORTED", "notes carry images or a video"),
            }
            attachments.push(asset.to_attachment());
        }
        ensure!(
            videos == 0 || attachments.len() == 1,
            invalid,
            "ATTACHMENTS_MIXED",
            "a video cannot be combined with other attachments"
        );
        Ok(attachments)
    }

    /// The users behind the `@handles` of a note, in order of appearance.
    /// Handles that name nobody, a suspended account, or someone in a block
    /// with the author stay plain text.
    async fn resolve_mentions(&self, viewer: &Viewer, handles: &[String]) -> AppResult<Vec<(Uuid, String)>> {
        if handles.is_empty() {
            return Ok(Vec::new());
        }
        let found: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id, handle::text FROM users WHERE handle = ANY($1::citext[]) AND deleted_at IS NULL AND banned_at IS NULL")
                .bind(handles)
                .fetch_all(&self.core.db)
                .await?;
        let by_handle: HashMap<String, Uuid> =
            found.into_iter().map(|(id, handle)| (handle.to_ascii_lowercase(), id)).collect();
        Ok(handles
            .iter()
            .filter_map(|handle| by_handle.get(handle).map(|id| (*id, handle.clone())))
            .filter(|(id, _)| !viewer.hides(*id))
            .collect())
    }

    /// What stands between a user and the post button: the rules must be
    /// agreed to, and the posting limits apply.
    async fn admit_post(&self, author: Uuid) -> AppResult<()> {
        rules::ensure_agreed(&self.core, author).await?;
        self.core
            .cache
            .rate_limit(&format!("notes:post:{author}"), self.cfg.posts_per_minute, Duration::from_secs(60))
            .await?;
        self.core
            .cache
            .rate_limit(&format!("notes:post-day:{author}"), MAX_POSTS_PER_DAY, Duration::from_secs(24 * 3600))
            .await
    }

    /// Posts a note for `author` as asked through the API.
    pub(crate) async fn create_note(&self, author: Uuid, input: NewNote) -> AppResult<NoteRow> {
        self.admit_post(author).await?;
        self.store_note(author, new_id(), input).await
    }

    /// [`Self::create_note`] made safe to retry: a request repeating
    /// `idempotency_key` within ten minutes returns the note the first one
    /// created.
    pub(crate) async fn create_note_once(
        &self,
        author: Uuid,
        idempotency_key: &str,
        input: NewNote,
    ) -> AppResult<NoteRow> {
        if idempotency_key.is_empty() {
            return self.create_note(author, input).await;
        }
        ensure!(
            idempotency_key.len() <= MAX_IDEMPOTENCY_KEY_BYTES,
            invalid,
            "INVALID_IDEMPOTENCY_KEY",
            "the idempotency key is too long"
        );
        let key = format!("notes:idem:note:{author}:{idempotency_key}");
        let note_id = new_id();
        if !self.core.cache.set_nx(&key, &note_id.to_string(), IDEMPOTENCY_WINDOW).await? {
            // Someone got there first: the earlier request, or one still running.
            let earlier = self.core.cache.get_string(&key).await?.and_then(|id| Uuid::parse_str(&id).ok());
            let viewer = Viewer::load(&self.core, Some(author)).await?;
            return match earlier {
                Some(id) => match self.find_note(&viewer, id).await? {
                    Some(note) => Ok(note),
                    None => Err(AppError::unavailable(
                        "REQUEST_IN_PROGRESS",
                        "the first request with this key has not finished; retry shortly",
                    )),
                },
                None => Err(AppError::unavailable("REQUEST_IN_PROGRESS", "retry shortly")),
            };
        }
        let result = async {
            self.admit_post(author).await?;
            self.store_note(author, note_id, input).await
        }
        .await;
        if result.is_err() {
            // Nothing was posted: the key is free for the corrected request.
            let _ = self.core.cache.del(&key).await;
        }
        result
    }

    /// Validates and stores a note with everything that follows from it.
    pub(crate) async fn store_note(&self, author: Uuid, note_id: Uuid, input: NewNote) -> AppResult<NoteRow> {
        let core = &self.core;
        let body = text::clean(&input.text);
        ensure!(
            text::fits(&body, MAX_NOTE_CHARS),
            invalid,
            "NOTE_TOO_LONG",
            "a note has at most {MAX_NOTE_CHARS} characters"
        );
        let viewer = Viewer::load(core, Some(author)).await?;

        // What the note answers and what it quotes must be notes the author can see.
        let parent = match input.reply_to_id {
            Some(id) => Some(self.require_note(&viewer, id).await?),
            None => None,
        };
        let quoted = match input.quote_of_id {
            Some(id) => {
                let quoted = self.require_note(&viewer, id).await?;
                ensure!(quoted.is_public(), precondition, "NOTE_NOT_PUBLIC", "only public notes can be quoted");
                Some(quoted)
            }
            None => None,
        };

        let attachments = self.resolve_attachments(author, &input.asset_ids).await?;
        let asset_ids: Vec<Uuid> =
            attachments.iter().filter_map(|attachment| Uuid::parse_str(&attachment.asset_id).ok()).collect();
        if let Some(stamp_id) = input.stamp_id {
            tokuly_stamp::ensure_usable(core, author, stamp_id).await?;
        }
        ensure!(
            !body.is_empty() || !attachments.is_empty() || input.stamp_id.is_some(),
            invalid,
            "NOTE_EMPTY",
            "a note needs text, media or a stamp"
        );
        let place = input.place.as_ref().map(validate_place).transpose()?;

        let wanted = match input.visibility {
            Some(visibility) => visibility,
            // The author's default; a setting this build does not know reads as public, the default of defaults.
            None => requested_visibility(settings::privacy_of(core, author).await?.default_note_visibility)
                .ok()
                .flatten()
                .unwrap_or(visibility::PUBLIC),
        };
        // A reply is never shown more widely than the note it answers.
        let applied = parent.as_ref().map_or(wanted, |parent| narrower(wanted, parent.visibility));

        let parsed = text::parse(&body);
        let mentioned = self.resolve_mentions(&viewer, &parsed.handles).await?;
        let mentions: Vec<Mention> = mentioned
            .iter()
            .map(|(user_id, handle)| Mention {
                handle: handle.clone(),
                user_id: user_id.to_string(),
                ..Default::default()
            })
            .collect();
        let link_url = parsed
            .first_url
            .as_deref()
            .and_then(|url| tokuly_media::link_preview::normalize_url(url).ok())
            .map(String::from);
        let actor = tokuly_identity::summary(core, author).await?;
        // The people the note may be announced to, as viewers. Everything the
        // transaction needs from elsewhere is fetched before it opens: while
        // it holds its connection, nothing here waits for a second one.
        let mut audience: HashMap<Uuid, Viewer> = HashMap::new();
        let candidates = parent
            .iter()
            .chain(&quoted)
            .map(|note| note.author_id)
            .chain(mentioned.iter().map(|(user_id, _)| *user_id));
        for user_id in candidates.filter(|user_id| *user_id != author) {
            if let Entry::Vacant(slot) = audience.entry(user_id) {
                slot.insert(Viewer::load(core, Some(user_id)).await?);
            }
        }

        let mut tx = core.db.begin().await?;
        let sql = format!(
            "INSERT INTO notes AS n (id, author_id, text, attachments, asset_ids, stamp_id, place_name, place_lat, place_lng, place_id,
                                     visibility, content_warning, reply_to_id, reply_to_author_id, quote_of_id, hashtags, mentions, link_url)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
             RETURNING {NOTE_COLUMNS}"
        );
        let note: NoteRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(note_id)
            .bind(author)
            .bind(&body)
            .bind(to_json(&attachments)?)
            .bind(&asset_ids)
            .bind(input.stamp_id)
            .bind(place.as_ref().map(|place| place.name.clone()))
            .bind(place.as_ref().map(|place| place.lat))
            .bind(place.as_ref().map(|place| place.lng))
            .bind(place.as_ref().and_then(|place| place.place_id))
            .bind(applied)
            .bind(input.content_warning)
            .bind(parent.as_ref().map(|parent| parent.id))
            .bind(parent.as_ref().map(|parent| parent.author_id))
            .bind(quoted.as_ref().map(|quoted| quoted.id))
            .bind(&parsed.hashtags)
            .bind(to_json(&mentions)?)
            .bind(&link_url)
            .fetch_one(&mut *tx)
            .await?;
        for asset_id in &asset_ids {
            assets::attach(&mut *tx, *asset_id, &note_holder(note.id)).await?;
        }
        if !mentioned.is_empty() {
            let (user_ids, handles): (Vec<Uuid>, Vec<String>) = mentioned.iter().cloned().unzip();
            sqlx::query("INSERT INTO note_mentions (note_id, user_id, handle) SELECT $1, * FROM unnest($2::uuid[], $3::text[]) ON CONFLICT DO NOTHING")
                .bind(note.id)
                .bind(&user_ids)
                .bind(&handles)
                .execute(&mut *tx)
                .await?;
        }
        // Rows are always locked notes first, hashtags second (deleting a
        // note does the same), so a reply racing the deletion of the note it
        // answers waits instead of deadlocking.
        let touched =
            bump_counters(&mut tx, parent.as_ref().map(|parent| parent.id), quoted.as_ref().map(|quoted| quoted.id))
                .await?;
        hashtags::link(&mut tx, note.id, &note.hashtags, note.is_public()).await?;

        events::emit(
            &mut *tx,
            NOTE_CREATED,
            Some(author),
            &NoteCreated {
                note_id: note.id,
                author_id: author,
                visibility: note.visibility,
                reply_to_id: note.reply_to_id,
                quote_of_id: note.quote_of_id,
            },
        )
        .await?;
        if note.is_public() {
            outbound::index(&mut *tx, search_doc(&note, &actor)).await?;
        }
        outbound::activity(&mut *tx, &copy::note_activity(&note)).await?;

        // Notifications: the author of the note answered, then of the note
        // quoted, then everyone mentioned — each person once, and only
        // people who may see the new note.
        // (A private reply, say, is a note to oneself: the person answered
        // is not told about something they cannot open.)
        let mut readers: Vec<Uuid> = Vec::new();
        for (user_id, viewer) in &audience {
            if viewer.can_see_note(&mut *tx, note.id).await? {
                readers.push(*user_id);
            }
        }
        // Each of them is told (below), so each received the note: they can
        // report it even after it is hidden from them (see `subjects`).
        if !readers.is_empty() {
            sqlx::query("INSERT INTO note_recipients (note_id, user_id) SELECT $1, * FROM unnest($2::uuid[]) ON CONFLICT DO NOTHING")
                .bind(note.id)
                .bind(&readers)
                .execute(&mut *tx)
                .await?;
        }
        let mut tell = |user_id: Uuid| {
            let position = readers.iter().position(|reader| *reader == user_id)?;
            Some(readers.swap_remove(position))
        };
        let mut answered = false;
        if let Some(recipient) = parent.as_ref().and_then(|parent| tell(parent.author_id)) {
            answered = true;
            outbound::notify(&mut *tx, &copy::reply(&actor, recipient, &note)).await?;
        }
        if let Some(recipient) = quoted.as_ref().and_then(|quoted| tell(quoted.author_id)) {
            outbound::notify(&mut *tx, &copy::quote(&actor, recipient, &note)).await?;
        }
        for (user_id, _) in &mentioned {
            if let Some(recipient) = tell(*user_id) {
                outbound::notify(&mut *tx, &copy::mention(&actor, recipient, &note)).await?;
            }
        }
        tx.commit().await?;

        // From here on nothing can fail the post.
        self.push_counters(touched).await;
        if let Some(stamp_id) = note.stamp_id
            && let Err(e) = tokuly_stamp::record_send(core, author, stamp_id, SendContext::Note).await
        {
            tracing::warn!(error = %e, "recording a stamp sent in a note failed");
        }
        if let Some(parent) = parent.as_ref().filter(|_| answered) {
            self.record_interaction(author, parent.author_id, InteractionKind::Message).await;
        }
        if note.link_url.is_some()
            && let Err(e) = core
                .jobs
                .enqueue_unique(JOB_LINK_PREVIEW, &note.id.to_string(), &LinkPreviewJob { note_id: note.id })
                .await
        {
            tracing::warn!(error = %e, note_id = %note.id, "queueing the link preview failed");
        }
        core.analytics
            .track(AnalyticsEvent::new("note.post").user(Some(author)).subject("note", note.id).props(
                serde_json::json!({
                    "visibility": note.visibility,
                    "reply": note.is_reply(),
                    "quote": note.quote_of_id.is_some(),
                    "attachments": note.asset_ids.len(),
                }),
            ))
            .await;
        Ok(note)
    }

    /// Reports something one user did with another's note to the
    /// best-friend ranking (ignored there unless the two are friends).
    pub(crate) async fn record_interaction(&self, from: Uuid, to: Uuid, kind: InteractionKind) {
        if let Err(e) = graph::record_interaction(&self.core, from, to, kind, 1).await {
            tracing::warn!(error = %e, "recording a note interaction failed");
        }
    }

    /// Pushes fresh counters to the clients watching those notes.
    pub(crate) async fn push_counters(&self, counters: impl IntoIterator<Item = NoteCounters>) {
        for counters in counters {
            let Ok(note_id) = Uuid::parse_str(&counters.note_id) else { continue };
            self.push_topic(&note_topic(note_id), Push::CountersChanged(Box::new(counters))).await;
        }
    }

    /// The note for one of its author's own operations. A note of someone
    /// else is `NOT_NOTE_AUTHOR` only if the caller can see it at all.
    pub(crate) async fn own_note(&self, user_id: Uuid, note_id: Uuid) -> AppResult<NoteRow> {
        let viewer = Viewer::load(&self.core, Some(user_id)).await?;
        let note = self.require_note(&viewer, note_id).await?;
        ensure!(note.author_id == user_id, denied, "NOT_NOTE_AUTHOR", "only the author can do this");
        Ok(note)
    }

    /// Deletes notes inside `tx`, with everything that hangs off them:
    /// likes, renotes, mentions and hashtag links go with the rows; counters
    /// of the notes they answered or quoted and of their hashtags are put
    /// right; uploads are released; search documents removed; and
    /// `notes.note.deleted` is emitted for each. Replies to a deleted note
    /// stay. Call [`Self::announce_removed`] after the commit.
    pub(crate) async fn delete_notes(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        note_ids: &[Uuid],
        reason: RemovalReason,
        actor: Option<Uuid>,
    ) -> AppResult<Removed> {
        #[derive(sqlx::FromRow)]
        struct Gone {
            id: Uuid,
            author_id: Uuid,
            visibility: i32,
            reply_to_id: Option<Uuid>,
            quote_of_id: Option<Uuid>,
            hashtags: Vec<String>,
            has_assets: bool,
        }
        if note_ids.is_empty() {
            return Ok(Removed::default());
        }
        // Locked in ID order, like every other statement that takes several notes.
        let gone: Vec<Gone> = sqlx::query_as(
            "WITH doomed AS (
                 SELECT id FROM notes WHERE id = ANY($1) ORDER BY id FOR UPDATE
             )
             DELETE FROM notes WHERE id IN (SELECT id FROM doomed)
             RETURNING id, author_id, visibility, reply_to_id, quote_of_id, hashtags, (asset_ids <> '{}') AS has_assets",
        )
        .bind(note_ids)
        .fetch_all(&mut **tx)
        .await?;

        let mut deltas: HashMap<Uuid, (i32, i32)> = HashMap::new();
        let mut public_tags = Vec::new();
        for note in &gone {
            if let Some(parent) = note.reply_to_id {
                deltas.entry(parent).or_default().0 -= 1;
            }
            if let Some(quoted) = note.quote_of_id {
                deltas.entry(quoted).or_default().1 -= 1;
            }
            if note.visibility == visibility::PUBLIC {
                public_tags.extend(note.hashtags.iter().cloned());
                outbound::unindex(&mut **tx, "note", note.id).await?;
            }
            if note.has_assets {
                assets::detach_holder(&mut **tx, &note_holder(note.id)).await?;
            }
            events::emit(
                &mut **tx,
                NOTE_DELETED,
                actor,
                &NoteDeleted { note_id: note.id, author_id: note.author_id, reason },
            )
            .await?;
        }
        hashtags::adjust(tx, &public_tags, -1).await?;
        let touched = apply_counter_deltas(tx, &deltas).await?;
        Ok(Removed { note_ids: gone.into_iter().map(|note| note.id).collect(), touched })
    }

    /// Tells clients watching the deleted notes (and the notes whose
    /// counters changed with them) what happened.
    pub(crate) async fn announce_removed(&self, removed: Removed) {
        for note_id in removed.note_ids {
            self.push_topic(&note_topic(note_id), Push::NoteDeletedId(note_id.to_string())).await;
            // Then the topic ends: nothing more comes for a deleted note.
            self.core.rt.close_topic(&note_topic(note_id)).await;
        }
        self.push_counters(removed.touched).await;
    }

    /// Deletes one of `user_id`'s notes.
    pub(crate) async fn delete_own_note(&self, user_id: Uuid, note_id: Uuid) -> AppResult<()> {
        let note = self.own_note(user_id, note_id).await?;
        let mut tx = self.core.db.begin().await?;
        let removed = self.delete_notes(&mut tx, &[note.id], RemovalReason::Author, Some(user_id)).await?;
        tx.commit().await?;
        self.announce_removed(removed).await;
        Ok(())
    }

    /// Changes who may see one of `user_id`'s notes and returns the note as
    /// it is now. A reply stays within the audience of the note it answers.
    pub(crate) async fn set_visibility(&self, user_id: Uuid, note_id: Uuid, wanted: i32) -> AppResult<NoteRow> {
        let note = self.own_note(user_id, note_id).await?;
        let actor = tokuly_identity::summary(&self.core, user_id).await?;
        // The author's friends and blocks, for who may still see the note.
        let author = Viewer::load(&self.core, Some(user_id)).await?;
        let mut tx = self.core.db.begin().await?;
        // Changes of one note apply one after the other, each starting from
        // what the previous one left: two narrowings at once must not both
        // take the note's hashtags out of the public counts.
        let sql = format!("SELECT {NOTE_COLUMNS} FROM notes n WHERE n.id = $1 FOR UPDATE");
        let note: NoteRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(note.id)
            .fetch_optional(&mut *tx)
            .await?
            .or_not_found("NOTE_NOT_FOUND", "note not found")?;
        let limit: Option<i32> = match note.reply_to_id {
            Some(parent) => {
                sqlx::query_scalar("SELECT visibility FROM notes WHERE id = $1")
                    .bind(parent)
                    .fetch_optional(&mut *tx)
                    .await?
            }
            None => None,
        };
        let applied = limit.map_or(wanted, |limit| narrower(wanted, limit));
        if applied == note.visibility {
            return Ok(note);
        }
        let sql = format!("UPDATE notes AS n SET visibility = $2 WHERE n.id = $1 RETURNING {NOTE_COLUMNS}");
        let updated: NoteRow =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(note.id).bind(applied).fetch_one(&mut *tx).await?;
        // Hashtag counts and the search index only know public notes.
        if note.is_public() != updated.is_public() {
            hashtags::adjust(&mut tx, &updated.hashtags, if updated.is_public() { 1 } else { -1 }).await?;
            outbound::index(&mut *tx, search_doc(&updated, &actor)).await?;
        }
        events::emit(
            &mut *tx,
            NOTE_VISIBILITY_CHANGED,
            Some(user_id),
            &NoteVisibilityChanged {
                note_id: updated.id,
                author_id: user_id,
                visibility: updated.visibility,
                visible_to: still_visible_to(&updated, &author),
            },
        )
        .await?;
        tx.commit().await?;
        if narrower(updated.visibility, note.visibility) == updated.visibility {
            // A smaller audience: whoever follows the note's counters and
            // comments without being in it stops.
            self.core.rt.recheck_topic(&note_topic(updated.id)).await;
        }
        Ok(updated)
    }

    /// Pins one of `user_id`'s notes to their profile, replacing the note
    /// pinned before.
    pub(crate) async fn pin_note(&self, user_id: Uuid, note_id: Uuid) -> AppResult<()> {
        let note = self.own_note(user_id, note_id).await?;
        let mut tx = self.core.db.begin().await?;
        // One pin per profile: concurrent pins of the same user take turns.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("notes:pin:{user_id}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE notes SET pinned_at = NULL WHERE author_id = $1 AND pinned_at IS NOT NULL AND id <> $2")
            .bind(user_id)
            .bind(note.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE notes SET pinned_at = now() WHERE id = $1 AND author_id = $2 AND pinned_at IS NULL")
            .bind(note.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Removes `user_id`'s pinned note from their profile, if there is one.
    pub(crate) async fn unpin_note(&self, user_id: Uuid) -> AppResult<()> {
        sqlx::query("UPDATE notes SET pinned_at = NULL WHERE author_id = $1 AND pinned_at IS NOT NULL")
            .bind(user_id)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Fetches the preview of a note's first link, stores it and tells the
    /// clients showing the note (the `notes.link_preview` job).
    pub async fn resolve_link_preview(&self, note_id: Uuid) -> AppResult<()> {
        let pending: Option<(Uuid, String, i32)> =
            sqlx::query_as("SELECT author_id, link_url, visibility FROM notes WHERE id = $1 AND link_url IS NOT NULL AND link_preview IS NULL")
                .bind(note_id)
                .fetch_optional(&self.core.db)
                .await?;
        let Some((author_id, url, note_visibility)) = pending else { return Ok(()) };
        let Some(preview) = tokuly_media::link_preview(&self.core, &url).await? else { return Ok(()) };
        let stored = sqlx::query("UPDATE notes SET link_preview = $2 WHERE id = $1 AND link_preview IS NULL")
            .bind(note_id)
            .bind(to_json(&preview)?)
            .execute(&self.core.db)
            .await?;
        if stored.rows_affected() == 0 {
            return Ok(()); // deleted meanwhile, or another worker was faster
        }
        let event = || {
            Push::LinkPreviewResolved(Box::new(tokuly_proto::notes::NoteLinkPreview {
                note_id: note_id.to_string(),
                link_preview: preview.clone().into(),
                ..Default::default()
            }))
        };
        // Only a public note's preview goes to the topic: the guard admits
        // more than the note's audience for a moment after the audience
        // narrows (the gateways ask every follower again within their
        // re-check budget), and a preview is content. The author gets it
        // either way.
        self.push_user(author_id, event()).await;
        if note_visibility == visibility::PUBLIC {
            self.push_topic(&note_topic(note_id), event()).await;
        }
        Ok(())
    }
}

/// Who besides its author may see `note` (see
/// [`NoteVisibilityChanged::visible_to`]), as `sees_note` decides it:
/// `author` is the note's author as a viewer (their friends and blocks).
fn still_visible_to(note: &NoteRow, author: &Viewer) -> Option<Vec<Uuid>> {
    match note.visibility {
        visibility::PUBLIC => None,
        visibility::FRIENDS => {
            let mut people: Vec<Uuid> = author.friends().iter().copied().filter(|id| !author.hides(*id)).collect();
            if let Some(answered) = note.reply_to_author_id
                && answered != note.author_id
                && !author.hides(answered)
                && !people.contains(&answered)
            {
                people.push(answered);
            }
            Some(people)
        }
        _ => Some(Vec::new()),
    }
}

/// Adds one reply to `parent` and one quote to `quoted` and returns their
/// new counters. The rows are locked in ID order first, so two posts that
/// touch the same pair of notes cannot wait on each other.
async fn bump_counters(
    tx: &mut Transaction<'_, Postgres>,
    parent: Option<Uuid>,
    quoted: Option<Uuid>,
) -> AppResult<Vec<NoteCounters>> {
    let mut deltas: HashMap<Uuid, (i32, i32)> = HashMap::new();
    if let Some(parent) = parent {
        deltas.entry(parent).or_default().0 += 1;
    }
    if let Some(quoted) = quoted {
        deltas.entry(quoted).or_default().1 += 1;
    }
    apply_counter_deltas(tx, &deltas).await
}

/// Applies `(replies, quotes)` deltas to notes that still exist and returns
/// their new counters.
async fn apply_counter_deltas(
    tx: &mut Transaction<'_, Postgres>,
    deltas: &HashMap<Uuid, (i32, i32)>,
) -> AppResult<Vec<NoteCounters>> {
    if deltas.is_empty() {
        return Ok(Vec::new());
    }
    let mut targets: Vec<Uuid> = deltas.keys().copied().collect();
    targets.sort();
    let replies: Vec<i32> = targets.iter().map(|id| deltas[id].0).collect();
    let quotes: Vec<i32> = targets.iter().map(|id| deltas[id].1).collect();
    let rows: Vec<CounterRow> = sqlx::query_as(
        "WITH locked AS (
             SELECT id FROM notes WHERE id = ANY($1) ORDER BY id FOR UPDATE
         )
         UPDATE notes n
         SET reply_count = GREATEST(n.reply_count + d.replies, 0), quote_count = GREATEST(n.quote_count + d.quotes, 0)
         FROM unnest($1::uuid[], $2::int[], $3::int[]) AS d(id, replies, quotes)
         WHERE n.id = d.id AND n.id IN (SELECT id FROM locked)
         RETURNING n.id, n.reply_count, n.renote_count, n.quote_count, n.like_count, n.view_count",
    )
    .bind(&targets)
    .bind(&replies)
    .bind(&quotes)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(NoteCounters::from).collect())
}

#[cfg(test)]
mod tests {
    use tokuly_proto::common::LatLng;

    use super::*;

    #[test]
    fn replies_take_the_narrower_visibility() {
        use visibility::{FRIENDS, PRIVATE, PUBLIC};
        assert_eq!(narrower(PUBLIC, PUBLIC), PUBLIC);
        assert_eq!(narrower(PUBLIC, FRIENDS), FRIENDS);
        assert_eq!(narrower(FRIENDS, PUBLIC), FRIENDS);
        assert_eq!(narrower(PRIVATE, PUBLIC), PRIVATE);
        assert_eq!(narrower(FRIENDS, PRIVATE), PRIVATE);
    }

    #[test]
    fn only_three_visibilities_are_accepted() {
        assert_eq!(requested_visibility(Visibility::VISIBILITY_UNSPECIFIED.into()).unwrap(), None);
        assert_eq!(requested_visibility(Visibility::VISIBILITY_PUBLIC.into()).unwrap(), Some(visibility::PUBLIC));
        assert_eq!(requested_visibility(Visibility::VISIBILITY_FRIENDS.into()).unwrap(), Some(visibility::FRIENDS));
        assert_eq!(requested_visibility(Visibility::VISIBILITY_PRIVATE.into()).unwrap(), Some(visibility::PRIVATE));
        for other in [Visibility::VISIBILITY_UNLISTED, Visibility::VISIBILITY_NO_FEED] {
            assert_eq!(requested_visibility(other.into()).unwrap_err().reason, "INVALID_VISIBILITY");
        }
        assert_eq!(requested_visibility(buffa::EnumValue::from(99)).unwrap_err().reason, "INVALID_VISIBILITY");
    }

    #[test]
    fn place_tags_need_a_name_and_real_coordinates() {
        let place = |name: &str, lat: f64, lng: f64, place_id: &str| PlaceTag {
            name: name.to_owned(),
            location: LatLng { lat, lng, ..Default::default() }.into(),
            place_id: place_id.to_owned(),
            ..Default::default()
        };
        let ok = validate_place(&place("  渋谷\nスクランブル  ", 35.6595, 139.7005, "")).unwrap();
        assert_eq!((ok.name.as_str(), ok.lat, ok.lng, ok.place_id), ("渋谷 スクランブル", 35.6595, 139.7005, None));
        let id = Uuid::now_v7();
        assert_eq!(validate_place(&place("x", 0.0, 0.0, &id.to_string())).unwrap().place_id, Some(id));
        for bad in [
            place("", 1.0, 1.0, ""),
            place(&"あ".repeat(MAX_PLACE_NAME_CHARS + 1), 1.0, 1.0, ""),
            place("x", 91.0, 1.0, ""),
            place("x", 1.0, -181.0, ""),
            place("x", f64::NAN, 1.0, ""),
            place("x", 1.0, 1.0, "not-an-id"),
        ] {
            assert_eq!(validate_place(&bad).err().map(|e| e.reason.into_owned()), Some("INVALID_PLACE".to_owned()));
        }
        let missing = PlaceTag { name: "x".into(), ..Default::default() };
        assert!(validate_place(&missing).is_err());
    }
}
