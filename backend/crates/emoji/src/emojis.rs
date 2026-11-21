//! The life of an emoji: creation, the moderators' decisions, edits and
//! deletion.
//!
//! Every function that changes something runs inside the caller's
//! transaction and notes in [`Effects`] what is left to do once that
//! transaction has committed. [`Emoji::commit`] is the one way to end such a
//! transaction: it gives the catalogue changes their version, commits, and
//! carries out the effects.
//!
//! ```text
//!            submit                    approve
//!  (nothing) ───────▶ PENDING ──────────────────────▶ APPROVED ◀──┐
//!      │                 │  ▲                           │   ▲     │ enable
//!      │ trusted / staff │  │ author changes it         │   │     │
//!      └─────────────────┼──┼───────────────────────────┘   │  DISABLED
//!                 reject ▼  │                    disable /  │     ▲
//!                      REJECTED ─── approve ────────────────┘     │
//!                                                takedown ────────┘
//! ```
use std::collections::HashMap;

use sqlx::{PgExecutor, Postgres, Transaction};
use tokuly_core::{
    events,
    outbound::{self, ActivityAudience, ActivityEntry, Localized, NotificationRequest, SearchDoc},
    prelude::*,
};
use tokuly_media::assets;
use tokuly_proto::{
    common::SubjectType,
    emoji::{EmojiDetail, EmojiEvent, ReviewRecord},
    realtime::server_frame::Body,
};

use crate::{
    Emoji,
    catalog::{self, Pending},
    events::{
        AddedVia, EMOJI_ADDED, EMOJI_REJECTED, EMOJI_REMOVED, EMOJI_SUBMITTED, EMOJI_UPDATED, EmojiAdded,
        EmojiDecision, EmojiRef, EmojiRemoved, EmojiUpdated, RemovedBecause,
    },
    model::{EmojiRow, Picture, Viewer, action, emoji_columns, holder, shortcode, state},
    validate,
};

pub type Tx<'a> = Transaction<'a, Postgres>;

/// Search index kind of emoji documents.
pub const SEARCH_KIND: &str = "emoji";

/// What a change leaves to do once its transaction has committed.
#[derive(Debug, Default)]
pub struct Effects {
    /// Changes to the catalogue, waiting for their version.
    pub catalog: Pending,
    /// Emoji whose author is shown the new state right away (realtime).
    pub author_updates: Vec<Uuid>,
    /// Uploads nothing uses any more, to delete instead of leaving them to
    /// the media janitor.
    pub discarded_assets: Vec<Uuid>,
}

impl Emoji {
    /// Ends a transaction that changed emoji: seals the catalogue changes,
    /// commits, then publishes the new version and pushes to authors.
    pub(crate) async fn commit(&self, mut tx: Tx<'_>, fx: Effects) -> AppResult<()> {
        let version = fx.catalog.seal(&mut tx).await?;
        tx.commit().await?;
        if let Some(version) = version {
            catalog::published(&self.core, version).await;
        }
        self.push_author_updates(&fx.author_updates).await;
        for asset_id in fx.discarded_assets {
            // If this fails nothing is lost: the upload has no holder, so
            // the media janitor collects it.
            if let Err(e) = assets::delete(&self.core, asset_id, None).await {
                tracing::debug!(%asset_id, error = %e, "unused emoji upload left to the media janitor");
            }
        }
        Ok(())
    }

    /// Shows authors the current state of their emoji on their open clients.
    async fn push_author_updates(&self, emoji_ids: &[Uuid]) {
        if emoji_ids.is_empty() {
            return;
        }
        let pushed: AppResult<()> = async {
            let rows = find_many(&self.core.db, emoji_ids).await?;
            for row in rows {
                let Some(author_id) = row.author_id else { continue };
                let detail = self.detail(&row, Viewer::user(author_id)).await?;
                let event = EmojiEvent { event: Some(detail.into()), ..Default::default() };
                self.core.rt.notify_user(author_id, Body::Emoji(Box::new(event))).await;
            }
            Ok(())
        }
        .await;
        if let Err(e) = pushed {
            tracing::warn!(error = %e, "emoji update was not pushed to its author");
        }
    }

    /// Emoji as `viewer` sees them, in the order of `rows`. The moderators'
    /// notes go to the author and to moderators, the history to moderators.
    pub(crate) async fn details(&self, rows: &[EmojiRow], viewer: Viewer) -> AppResult<Vec<EmojiDetail>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let core = &self.core;
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let author_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.author_id).collect();
        let authors = tokuly_identity::summaries(core, &author_ids).await?;
        let use_counts: HashMap<Uuid, i64> =
            sqlx::query_as::<_, (Uuid, i64)>("SELECT emoji_id, use_count FROM emoji_stats WHERE emoji_id = ANY($1)")
                .bind(&ids)
                .fetch_all(&core.db)
                .await?
                .into_iter()
                .collect();
        let mut history = if viewer.moderator { self.history(&ids).await? } else { HashMap::new() };

        Ok(rows
            .iter()
            .map(|row| {
                let sees_review = viewer.sees_review(row);
                EmojiDetail {
                    emoji: Some(row.to_proto()).into(),
                    state: i32::from(row.state).into(),
                    author: row.author_id.and_then(|id| authors.get(&id).cloned()).into(),
                    license: row.license.clone(),
                    use_count: use_counts.get(&row.id).copied().unwrap_or(0),
                    created_at: ts(row.created_at),
                    updated_at: ts(row.updated_at),
                    review_note: if sees_review { row.review_note.clone() } else { String::new() },
                    reviewed_at: opt_ts(row.reviewed_at.filter(|_| sees_review)),
                    history: history.remove(&row.id).unwrap_or_default(),
                    can_edit: viewer.can_edit(row),
                    ..Default::default()
                }
            })
            .collect())
    }

    pub(crate) async fn detail(&self, row: &EmojiRow, viewer: Viewer) -> AppResult<EmojiDetail> {
        let mut details = self.details(std::slice::from_ref(row), viewer).await?;
        Ok(details.pop().unwrap_or_default())
    }

    /// What happened to each emoji, newest first.
    async fn history(&self, emoji_ids: &[Uuid]) -> AppResult<HashMap<Uuid, Vec<ReviewRecord>>> {
        let rows: Vec<(Uuid, Option<Uuid>, i16, String, Time)> = sqlx::query_as(
            "SELECT emoji_id, actor_id, action, note, created_at FROM emoji_reviews WHERE emoji_id = ANY($1) ORDER BY id DESC",
        )
        .bind(emoji_ids)
        .fetch_all(&self.core.db)
        .await?;
        let actor_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.1).collect();
        let actors = tokuly_identity::summaries(&self.core, &actor_ids).await?;
        let mut history: HashMap<Uuid, Vec<ReviewRecord>> = HashMap::new();
        for (emoji_id, actor_id, action, note, created_at) in rows {
            history.entry(emoji_id).or_default().push(ReviewRecord {
                action: i32::from(action).into(),
                note,
                actor: actor_id.and_then(|id| actors.get(&id).cloned()).into(),
                created_at: ts(created_at),
                ..Default::default()
            });
        }
        Ok(history)
    }
}

// ───────────────────────────── Reading ─────────────────────────────

pub async fn find_by_id<'e, E: PgExecutor<'e>>(executor: E, emoji_id: Uuid) -> AppResult<Option<EmojiRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", emoji_columns!(), " FROM emojis e WHERE e.id = $1"))
        .bind(emoji_id)
        .fetch_optional(executor)
        .await?)
}

/// The emoji holding `name` (a rejected emoji holds none).
pub async fn find_by_name<'e, E: PgExecutor<'e>>(executor: E, name: &str) -> AppResult<Option<EmojiRow>> {
    Ok(sqlx::query_as(concat!("SELECT ", emoji_columns!(), " FROM emojis e WHERE e.name = $1 AND e.state <> 3"))
        .bind(name)
        .fetch_optional(executor)
        .await?)
}

/// Emoji by ID, in the order of `emoji_ids`; unknown IDs are skipped.
pub async fn find_many<'e, E: PgExecutor<'e>>(executor: E, emoji_ids: &[Uuid]) -> AppResult<Vec<EmojiRow>> {
    let mut rows: Vec<EmojiRow> =
        sqlx::query_as(concat!("SELECT ", emoji_columns!(), " FROM emojis e WHERE e.id = ANY($1)"))
            .bind(emoji_ids)
            .fetch_all(executor)
            .await?;
    rows.sort_by_key(|row| emoji_ids.iter().position(|id| *id == row.id).unwrap_or(usize::MAX));
    Ok(rows)
}

/// Resolves what a client sent to name an emoji: its ID, or the name of an
/// emoji that holds one (with or without colons).
pub async fn find(core: &Core, reference: &str) -> AppResult<Option<EmojiRow>> {
    let reference = reference.trim();
    if let Ok(emoji_id) = Uuid::parse_str(reference) {
        return find_by_id(&core.db, emoji_id).await;
    }
    match validate::normalize_name(reference) {
        Some(name) => find_by_name(&core.db, &name).await,
        None => Ok(None),
    }
}

fn not_found() -> AppError {
    AppError::not_found("EMOJI_NOT_FOUND", "emoji not found")
}

/// Like [`find`], failing with `EMOJI_NOT_FOUND`.
pub async fn require(core: &Core, reference: &str) -> AppResult<EmojiRow> {
    find(core, reference).await?.ok_or_else(not_found)
}

/// Reads an emoji and locks its row until the transaction ends. Every
/// change starts here, so two changes to one emoji never interleave.
pub async fn lock(tx: &mut Tx<'_>, emoji_id: Uuid) -> AppResult<EmojiRow> {
    sqlx::query_as(concat!("SELECT ", emoji_columns!(), " FROM emojis e WHERE e.id = $1 FOR UPDATE"))
        .bind(emoji_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(not_found)
}

/// Locks several emoji, in ID order so that two bulk changes cannot wait
/// on each other. Unknown IDs are skipped.
pub async fn lock_many(tx: &mut Tx<'_>, emoji_ids: &[Uuid]) -> AppResult<Vec<EmojiRow>> {
    Ok(sqlx::query_as(concat!(
        "SELECT ",
        emoji_columns!(),
        " FROM emojis e WHERE e.id = ANY($1) ORDER BY e.id FOR UPDATE"
    ))
    .bind(emoji_ids)
    .fetch_all(&mut **tx)
    .await?)
}

/// Emoji of `user_id` waiting for review.
pub async fn pending_count<'e, E: PgExecutor<'e>>(executor: E, user_id: Uuid) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM emojis WHERE author_id = $1 AND state = 1")
        .bind(user_id)
        .fetch_one(executor)
        .await?)
}

/// Emoji of `user_id` in the catalogue.
pub async fn approved_count<'e, E: PgExecutor<'e>>(executor: E, user_id: Uuid) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM emojis WHERE author_id = $1 AND state = 2")
        .bind(user_id)
        .fetch_one(executor)
        .await?)
}

/// Whether a moderator flagged `user_id` as a trusted submitter.
pub async fn is_trusted<'e, E: PgExecutor<'e>>(executor: E, user_id: Uuid) -> AppResult<bool> {
    let trusted: Option<bool> = sqlx::query_scalar("SELECT trusted FROM emoji_submitters WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(executor)
        .await?;
    Ok(trusted.unwrap_or(false))
}

/// Makes the submissions of one user queue up for the rest of the
/// transaction, so the limit on waiting submissions holds under concurrency.
pub async fn serialize_submissions(tx: &mut Tx<'_>, user_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('emoji-submit:' || $1::text, 0))")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

// ───────────────────────────── Changing ─────────────────────────────

/// A unique violation on `emojis` can only be the name.
fn name_taken(error: AppError) -> AppError {
    if error.is_already_exists() {
        AppError::already_exists("NAME_TAKEN", "another emoji has this name")
    } else {
        error
    }
}

async fn record(tx: &mut Tx<'_>, emoji_id: Uuid, actor_id: Option<Uuid>, action: i16, note: &str) -> AppResult<()> {
    sqlx::query("INSERT INTO emoji_reviews (emoji_id, actor_id, action, note) VALUES ($1, $2, $3, $4)")
        .bind(emoji_id)
        .bind(actor_id)
        .bind(action)
        .bind(note)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Notes a moderator's edit in the emoji's history.
pub async fn record_edit(tx: &mut Tx<'_>, emoji_id: Uuid, moderator_id: Uuid, changed: &[&str]) -> AppResult<()> {
    record(tx, emoji_id, Some(moderator_id), action::EDITED, &changed.join(", ")).await
}

fn search_doc(emoji: &EmojiRow) -> SearchDoc {
    SearchDoc {
        kind: SEARCH_KIND.to_owned(),
        id: emoji.id,
        title: emoji.name.clone(),
        text: emoji.category.clone().unwrap_or_default(),
        tags: emoji.aliases.clone(),
        owner_id: emoji.author_id,
        public: emoji.is_approved(),
        image_url: Some(emoji.image().url).filter(|url| !url.is_empty()),
        path: emoji.path(),
        created_at: emoji.approved_at.unwrap_or(emoji.created_at).timestamp(),
        score: 0.0,
        extra: serde_json::json!({
            "name": emoji.name,
            "category": emoji.category,
            "animated": emoji.animated,
            "sensitive": emoji.sensitive,
        }),
    }
}

/// A notification to the emoji's author about the emoji. Nobody is told
/// which moderator decided, so there is no actor.
async fn notify_author(
    tx: &mut Tx<'_>,
    emoji: &EmojiRow,
    kind: &str,
    title: Localized,
    body: Localized,
    path: String,
) -> AppResult<()> {
    let Some(author_id) = emoji.author_id else { return Ok(()) };
    let mut notice = NotificationRequest::new(author_id, kind, title, body, path)
        .subject(SubjectType::SUBJECT_TYPE_CUSTOM_EMOJI, emoji.id)
        .data(serde_json::json!({ "emoji_id": emoji.id, "name": emoji.name }));
    let image = emoji.image();
    if !image.url.is_empty() {
        notice = notice.image(image.url);
    }
    outbound::notify(&mut **tx, &notice).await
}

/// The emoji is in the catalogue from now on.
async fn joined_catalog(
    tx: &mut Tx<'_>,
    fx: &mut Effects,
    emoji: &EmojiRow,
    via: AddedVia,
    actor_id: Option<Uuid>,
) -> AppResult<()> {
    fx.catalog.upsert(emoji.id);
    let added = EmojiAdded { emoji_id: emoji.id, name: emoji.name.clone(), author_id: emoji.author_id, via };
    events::emit(&mut **tx, EMOJI_ADDED, actor_id, &added).await?;
    outbound::index(&mut **tx, search_doc(emoji)).await?;
    // Friends hear about an emoji when it first becomes usable, not when a
    // moderator switches it back on.
    if via != AddedVia::Enabled
        && let Some(author_id) = emoji.author_id
    {
        let summary = Localized::new(
            format!("絵文字 {} を作ったよ!", emoji.shortcode()),
            format!("made the emoji {}", emoji.shortcode()),
        );
        let mut entry = ActivityEntry::new(author_id, "emoji.added", ActivityAudience::Public, summary)
            .subject(SubjectType::SUBJECT_TYPE_CUSTOM_EMOJI, emoji.id)
            .path(emoji.path())
            .data(serde_json::json!({ "name": emoji.name }));
        let image = emoji.image();
        if !image.url.is_empty() {
            entry = entry.image(image.url);
        }
        outbound::activity(&mut **tx, &entry).await?;
    }
    Ok(())
}

/// The emoji is out of the catalogue from now on.
async fn left_catalog(
    tx: &mut Tx<'_>,
    fx: &mut Effects,
    emoji: &EmojiRow,
    because: RemovedBecause,
    actor_id: Option<Uuid>,
) -> AppResult<()> {
    fx.catalog.remove(emoji.id);
    outbound::unindex(&mut **tx, SEARCH_KIND, emoji.id).await?;
    let removed = EmojiRemoved { emoji_id: emoji.id, name: emoji.name.clone(), author_id: emoji.author_id, because };
    events::emit(&mut **tx, EMOJI_REMOVED, actor_id, &removed).await
}

/// Moves an emoji to another state. `decided` marks a moderator's decision.
async fn set_state(tx: &mut Tx<'_>, emoji_id: Uuid, to: i16, note: &str, decided: bool) -> AppResult<EmojiRow> {
    sqlx::query_as(concat!(
        "UPDATE emojis e SET
             state = $2,
             review_note = $3,
             reviewed_at = CASE WHEN $4 THEN now() ELSE e.reviewed_at END,
             approved_at = CASE WHEN $2 = 2 THEN COALESCE(e.approved_at, now()) ELSE e.approved_at END
         WHERE e.id = $1
         RETURNING ",
        emoji_columns!()
    ))
    .bind(emoji_id)
    .bind(to)
    .bind(note)
    .bind(decided)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| name_taken(e.into()))
}

/// What a new emoji is made of.
#[derive(Debug, Clone)]
pub struct NewEmoji {
    pub name: String,
    pub picture: Picture,
    pub category: Option<String>,
    pub aliases: Vec<String>,
    pub license: String,
    pub sensitive: bool,
    pub local_only: bool,
    pub author_id: Option<Uuid>,
}

/// How a new emoji enters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// It waits for a moderator.
    Review,
    /// It goes straight into the catalogue.
    Catalog(AddedVia),
}

/// Stores a new emoji and keeps its upload in use. Returns `None` when the
/// name is held by another emoji; nothing is stored then.
pub async fn create(
    tx: &mut Tx<'_>,
    fx: &mut Effects,
    new: &NewEmoji,
    admission: Admission,
    actor_id: Option<Uuid>,
) -> AppResult<Option<EmojiRow>> {
    let initial = match admission {
        Admission::Review => state::PENDING,
        Admission::Catalog(_) => state::APPROVED,
    };
    let created: Option<EmojiRow> = sqlx::query_as(concat!(
        "INSERT INTO emojis AS e
             (name, state, asset_id, image, animated, category, aliases, license, sensitive, local_only, author_id, approved_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, CASE WHEN $2 = 2 THEN now() END)
         ON CONFLICT (name) WHERE state <> 3 DO NOTHING
         RETURNING ",
        emoji_columns!()
    ))
    .bind(&new.name)
    .bind(initial)
    .bind(new.picture.asset_id)
    .bind(&new.picture.image)
    .bind(new.picture.animated)
    .bind(&new.category)
    .bind(&new.aliases)
    .bind(&new.license)
    .bind(new.sensitive)
    .bind(new.local_only)
    .bind(new.author_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(emoji) = created else { return Ok(None) };
    assets::attach(&mut **tx, emoji.asset_id, &holder(emoji.id)).await?;

    match admission {
        Admission::Review => {
            record(tx, emoji.id, actor_id, action::SUBMITTED, "").await?;
            let submitted = EmojiRef { emoji_id: emoji.id, name: emoji.name.clone(), author_id: emoji.author_id };
            events::emit(&mut **tx, EMOJI_SUBMITTED, actor_id, &submitted).await?;
        }
        Admission::Catalog(via) => {
            let entered = match via {
                AddedVia::Import => action::IMPORTED,
                AddedVia::Trusted => action::SUBMITTED,
                _ => action::ADDED,
            };
            record(tx, emoji.id, actor_id, entered, "").await?;
            if via == AddedVia::Trusted {
                // Approved by the rule, not by a person.
                record(tx, emoji.id, None, action::APPROVED, "").await?;
            }
            joined_catalog(tx, fx, &emoji, via, actor_id).await?;
        }
    }
    Ok(Some(emoji))
}

/// A moderator lets an emoji into the catalogue and its author is told.
/// Errors: `EMOJI_NOT_REVIEWABLE`, `NAME_TAKEN` (a rejected emoji whose name
/// was taken in the meantime).
pub async fn approve(tx: &mut Tx<'_>, fx: &mut Effects, emoji: &EmojiRow, moderator_id: Uuid) -> AppResult<EmojiRow> {
    ensure!(
        matches!(emoji.state, state::PENDING | state::REJECTED),
        precondition,
        "EMOJI_NOT_REVIEWABLE",
        "only an emoji that is waiting or was rejected can be approved"
    );
    let approved = set_state(tx, emoji.id, state::APPROVED, "", true).await?;
    record(tx, approved.id, Some(moderator_id), action::APPROVED, "").await?;
    joined_catalog(tx, fx, &approved, AddedVia::Review, Some(moderator_id)).await?;
    notify_author(
        tx,
        &approved,
        "emoji.approved",
        Localized::new("絵文字が承認されたよ!", "Your emoji was approved!"),
        Localized::new(
            format!("{} がみんなに使えるようになったよ!", approved.shortcode()),
            format!("{} is now available to everyone.", approved.shortcode()),
        ),
        approved.path(),
    )
    .await?;
    fx.author_updates.push(approved.id);
    Ok(approved)
}

/// A moderator declines a waiting emoji and its author is told why.
/// Errors: `EMOJI_NOT_REVIEWABLE`.
pub async fn reject(
    tx: &mut Tx<'_>,
    fx: &mut Effects,
    emoji: &EmojiRow,
    moderator_id: Uuid,
    reason: &str,
) -> AppResult<EmojiRow> {
    ensure!(
        emoji.state == state::PENDING,
        precondition,
        "EMOJI_NOT_REVIEWABLE",
        "only an emoji waiting for review can be rejected"
    );
    let rejected = set_state(tx, emoji.id, state::REJECTED, reason, true).await?;
    record(tx, rejected.id, Some(moderator_id), action::REJECTED, reason).await?;
    let decision = EmojiDecision {
        emoji_id: rejected.id,
        name: rejected.name.clone(),
        author_id: rejected.author_id,
        moderator_id,
        reason: reason.to_owned(),
    };
    events::emit(&mut **tx, EMOJI_REJECTED, Some(moderator_id), &decision).await?;
    notify_author(
        tx,
        &rejected,
        "emoji.rejected",
        Localized::new("絵文字が承認されなかったよ", "Your emoji was not approved"),
        Localized::new(
            format!("{} は承認されなかったよ。理由: {reason}", rejected.shortcode()),
            format!("{} was declined. Reason: {reason}", rejected.shortcode()),
        ),
        "/emoji/submissions".to_owned(),
    )
    .await?;
    fx.author_updates.push(rejected.id);
    Ok(rejected)
}

/// Takes an approved emoji out of the catalogue; it keeps its name. A
/// takedown (`taken_down`) is the outcome of a report: the admin domain
/// tells the author itself, so no second notification goes out.
/// Errors: `EMOJI_NOT_APPROVED`.
pub async fn disable(
    tx: &mut Tx<'_>,
    fx: &mut Effects,
    emoji: &EmojiRow,
    moderator_id: Option<Uuid>,
    reason: &str,
    taken_down: bool,
) -> AppResult<EmojiRow> {
    ensure!(emoji.is_approved(), precondition, "EMOJI_NOT_APPROVED", "only an approved emoji can be disabled");
    let disabled = set_state(tx, emoji.id, state::DISABLED, reason, true).await?;
    let (noted, because) = if taken_down {
        (action::TAKEN_DOWN, RemovedBecause::TakenDown)
    } else {
        (action::DISABLED, RemovedBecause::Disabled)
    };
    record(tx, disabled.id, moderator_id, noted, reason).await?;
    left_catalog(tx, fx, &disabled, because, moderator_id).await?;
    if !taken_down {
        notify_author(
            tx,
            &disabled,
            "emoji.disabled",
            Localized::new("絵文字が使えなくなったよ", "Your emoji was disabled"),
            Localized::new(
                format!("{} はカタログから外されたよ。理由: {reason}", disabled.shortcode()),
                format!("{} was taken out of the catalogue. Reason: {reason}", disabled.shortcode()),
            ),
            "/emoji/submissions".to_owned(),
        )
        .await?;
    }
    fx.author_updates.push(disabled.id);
    Ok(disabled)
}

/// Puts a disabled emoji back into the catalogue. Errors: `EMOJI_NOT_DISABLED`.
pub async fn enable(tx: &mut Tx<'_>, fx: &mut Effects, emoji: &EmojiRow, moderator_id: Uuid) -> AppResult<EmojiRow> {
    ensure!(emoji.state == state::DISABLED, precondition, "EMOJI_NOT_DISABLED", "the emoji is not disabled");
    let enabled = set_state(tx, emoji.id, state::APPROVED, "", true).await?;
    record(tx, enabled.id, Some(moderator_id), action::ENABLED, "").await?;
    joined_catalog(tx, fx, &enabled, AddedVia::Enabled, Some(moderator_id)).await?;
    fx.author_updates.push(enabled.id);
    Ok(enabled)
}

/// The author sends a rejected emoji to review again; a trusted author's
/// goes straight into the catalogue. Errors: `NAME_TAKEN`.
pub async fn resubmit(
    tx: &mut Tx<'_>,
    fx: &mut Effects,
    emoji: &EmojiRow,
    author_id: Uuid,
    trusted: bool,
) -> AppResult<EmojiRow> {
    record(tx, emoji.id, Some(author_id), action::SUBMITTED, "").await?;
    if trusted {
        let approved = set_state(tx, emoji.id, state::APPROVED, "", false).await?;
        record(tx, approved.id, None, action::APPROVED, "").await?;
        joined_catalog(tx, fx, &approved, AddedVia::Trusted, Some(author_id)).await?;
        return Ok(approved);
    }
    let pending = set_state(tx, emoji.id, state::PENDING, "", false).await?;
    let submitted = EmojiRef { emoji_id: pending.id, name: pending.name.clone(), author_id: pending.author_id };
    events::emit(&mut **tx, EMOJI_SUBMITTED, Some(author_id), &submitted).await?;
    Ok(pending)
}

/// Deletes an emoji for good and releases its upload.
pub async fn delete(
    tx: &mut Tx<'_>,
    fx: &mut Effects,
    emoji: &EmojiRow,
    because: RemovedBecause,
    actor_id: Option<Uuid>,
) -> AppResult<()> {
    sqlx::query("DELETE FROM emojis WHERE id = $1").bind(emoji.id).execute(&mut **tx).await?;
    assets::detach_holder(&mut **tx, &holder(emoji.id)).await?;
    if emoji.is_approved() {
        left_catalog(tx, fx, emoji, because, actor_id).await?;
    }
    Ok(())
}

/// Changes to an emoji; fields left `None` stay as they are.
#[derive(Debug, Default)]
pub struct Edit {
    pub name: Option<String>,
    pub picture: Option<Picture>,
    /// `Some(None)` takes the emoji out of its category.
    pub category: Option<Option<String>>,
    pub aliases: Option<Vec<String>>,
    pub license: Option<String>,
    pub sensitive: Option<bool>,
    pub local_only: Option<bool>,
    /// `Some(None)` removes the credit.
    pub author_id: Option<Option<Uuid>>,
}

/// Applies an edit and returns the emoji with the names of the fields that
/// really changed (none: nothing was written). An approved emoji gets a new
/// catalogue version; a rename is carried into decks and recently used
/// lists. Errors: `NAME_TAKEN`.
pub async fn edit(
    tx: &mut Tx<'_>,
    fx: &mut Effects,
    emoji: &EmojiRow,
    edit: Edit,
) -> AppResult<(EmojiRow, Vec<&'static str>)> {
    let name = edit.name.unwrap_or_else(|| emoji.name.clone());
    let picture = edit.picture.filter(|picture| picture.asset_id != emoji.asset_id);
    let category = edit.category.unwrap_or_else(|| emoji.category.clone());
    let aliases = edit.aliases.unwrap_or_else(|| emoji.aliases.clone());
    let license = edit.license.unwrap_or_else(|| emoji.license.clone());
    let sensitive = edit.sensitive.unwrap_or(emoji.sensitive);
    let local_only = edit.local_only.unwrap_or(emoji.local_only);
    let author_id = edit.author_id.unwrap_or(emoji.author_id);

    let mut changed = Vec::new();
    for (field, differs) in [
        ("name", name != emoji.name),
        ("picture", picture.is_some()),
        ("category", category != emoji.category),
        ("aliases", aliases != emoji.aliases),
        ("license", license != emoji.license),
        ("sensitive", sensitive != emoji.sensitive),
        ("local_only", local_only != emoji.local_only),
        ("author", author_id != emoji.author_id),
    ] {
        if differs {
            changed.push(field);
        }
    }
    if changed.is_empty() {
        return Ok((emoji.clone(), changed));
    }

    let (asset_id, image, animated) = match &picture {
        Some(picture) => (picture.asset_id, &picture.image, picture.animated),
        None => (emoji.asset_id, &emoji.image, emoji.animated),
    };
    let updated: EmojiRow = sqlx::query_as(concat!(
        "UPDATE emojis e SET name = $2, asset_id = $3, image = $4, animated = $5, category = $6, aliases = $7,
             license = $8, sensitive = $9, local_only = $10, author_id = $11
         WHERE e.id = $1
         RETURNING ",
        emoji_columns!()
    ))
    .bind(emoji.id)
    .bind(&name)
    .bind(asset_id)
    .bind(image)
    .bind(animated)
    .bind(&category)
    .bind(&aliases)
    .bind(&license)
    .bind(sensitive)
    .bind(local_only)
    .bind(author_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| name_taken(e.into()))?;

    if picture.is_some() {
        let holder = holder(emoji.id);
        assets::detach(&mut **tx, emoji.asset_id, &holder).await?;
        assets::attach(&mut **tx, updated.asset_id, &holder).await?;
    }
    let renamed = updated.name != emoji.name;
    if renamed && matches!(updated.state, state::APPROVED | state::DISABLED) {
        rename_in_lists(tx, &emoji.name, &updated.name).await?;
    }
    if updated.is_approved() {
        fx.catalog.upsert(updated.id);
        outbound::index(&mut **tx, search_doc(&updated)).await?;
        let change = EmojiUpdated {
            emoji_id: updated.id,
            name: updated.name.clone(),
            previous_name: renamed.then(|| emoji.name.clone()),
        };
        events::emit(&mut **tx, EMOJI_UPDATED, None, &change).await?;
    }
    Ok((updated, changed))
}

/// Carries a rename into every deck and recently used list, so pinned
/// emoji stay pinned. A list that somehow holds both names keeps the new one.
async fn rename_in_lists(tx: &mut Tx<'_>, from: &str, to: &str) -> AppResult<()> {
    let (from, to) = (shortcode(from), shortcode(to));
    sqlx::query(
        "UPDATE emoji_decks
         SET items = CASE WHEN items @> ARRAY[$2] THEN array_remove(items, $1) ELSE array_replace(items, $1, $2) END
         WHERE items @> ARRAY[$1]",
    )
    .bind(&from)
    .bind(&to)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM emoji_recents r
         WHERE r.item = $2 AND EXISTS (SELECT 1 FROM emoji_recents o WHERE o.user_id = r.user_id AND o.item = $1)",
    )
    .bind(&from)
    .bind(&to)
    .execute(&mut **tx)
    .await?;
    sqlx::query("UPDATE emoji_recents SET item = $2 WHERE item = $1").bind(&from).bind(&to).execute(&mut **tx).await?;
    Ok(())
}

/// Rewrites the search documents of approved emoji whose data changed
/// without an [`edit`] (their category was renamed, emptied or deleted, or
/// their author's account is gone).
pub async fn reindex(tx: &mut Tx<'_>, emoji_ids: &[Uuid]) -> AppResult<()> {
    if emoji_ids.is_empty() {
        return Ok(());
    }
    let rows: Vec<EmojiRow> =
        sqlx::query_as(concat!("SELECT ", emoji_columns!(), " FROM emojis e WHERE e.id = ANY($1) AND e.state = 2"))
            .bind(emoji_ids)
            .fetch_all(&mut **tx)
            .await?;
    for row in &rows {
        outbound::index(&mut **tx, search_doc(row)).await?;
    }
    Ok(())
}
