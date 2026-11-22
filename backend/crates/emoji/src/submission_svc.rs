//! `EmojiSubmissionService`: submitting emoji and following one's own
//! submissions.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_media::assets;
use tokuly_proto::emoji::*;

use crate::{
    Emoji,
    catalog_svc::MODERATOR,
    categories,
    emojis::{self, Admission, Edit, Effects, NewEmoji, Tx},
    events::{AddedVia, RemovedBecause},
    model::{EmojiRow, PICTURE_PURPOSE, Picture, Viewer, emoji_columns, state},
    validate::{self, MAX_ALIASES, MIN_IMAGE_EDGE},
};

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);

fn name_taken() -> AppError {
    AppError::already_exists("NAME_TAKEN", "another emoji has this name")
}

/// Why an author may not change an emoji any more: reason and message.
const NOT_EDITABLE: (&str, &str) =
    ("EMOJI_NOT_EDITABLE", "only an emoji that is waiting for review or was rejected can be changed");
/// Why an author may not withdraw an emoji any more.
const NOT_WITHDRAWABLE: (&str, &str) =
    ("EMOJI_NOT_WITHDRAWABLE", "an emoji that was approved can only be removed by a moderator");

/// Checks that an emoji is the caller's and not (or no longer) published.
/// Somebody else's emoji does not exist for the caller (`EMOJI_NOT_FOUND`);
/// their own that a moderator let into the catalogue is refused with `closed`.
fn ensure_own_and_open(emoji: &EmojiRow, user_id: Uuid, closed: (&'static str, &'static str)) -> AppResult<()> {
    ensure!(emoji.author_id == Some(user_id), not_found, "EMOJI_NOT_FOUND", "emoji not found");
    if !Viewer::user(user_id).can_edit(emoji) {
        return Err(AppError::precondition(closed.0, closed.1));
    }
    Ok(())
}

/// The caller of a submission RPC.
struct Submitter {
    user_id: Uuid,
    /// Moderators submit like trusted users, and may use reserved names.
    moderator: bool,
}

impl Submitter {
    fn of(ctx: &RequestContext) -> AppResult<Self> {
        let auth = ctx.auth()?;
        Ok(Self { user_id: auth.user_id, moderator: auth.has_role(MODERATOR) })
    }

    fn viewer(&self) -> Viewer {
        Viewer { user_id: Some(self.user_id), moderator: self.moderator }
    }
}

impl Emoji {
    /// Checks that an upload of `owner_id` can be an emoji picture.
    /// Errors: the `ASSET_*` reasons, `IMAGE_TOO_LARGE`, `IMAGE_TOO_SMALL`.
    pub(crate) async fn picture(&self, asset_id: &str, owner_id: Uuid) -> AppResult<Picture> {
        let asset = assets::require_ready(&self.core, parse_id(asset_id)?, owner_id, &[PICTURE_PURPOSE]).await?;
        Picture::of(&asset, self.cfg.max_image_bytes())
    }

    /// Validates a name a user wants to give an emoji.
    /// Errors: `NAME_INVALID`, `NAME_RESERVED`.
    fn submitted_name(&self, input: &str, submitter: &Submitter) -> AppResult<String> {
        let name = validate::name(input)?;
        ensure!(
            submitter.moderator || !validate::is_reserved(&name, &self.cfg.reserved_names),
            invalid,
            "NAME_RESERVED",
            "the name {name} is reserved"
        );
        Ok(name)
    }

    /// Whether the submitter's emoji skip the review.
    async fn skips_review(&self, tx: &mut Tx<'_>, submitter: &Submitter) -> AppResult<bool> {
        Ok(submitter.moderator || emojis::is_trusted(&mut **tx, submitter.user_id).await?)
    }

    /// Refuses another waiting submission when the user has their fill.
    /// Call after [`emojis::serialize_submissions`].
    async fn ensure_room_in_queue(&self, tx: &mut Tx<'_>, user_id: Uuid) -> AppResult<()> {
        let waiting = emojis::pending_count(&mut **tx, user_id).await?;
        ensure!(
            waiting < self.cfg.max_pending,
            rate_limited,
            "TOO_MANY_PENDING",
            "you already have {waiting} emoji waiting for review; wait for a decision before submitting more"
        );
        Ok(())
    }
}

impl EmojiSubmissionService for Emoji {
    async fn get_submitter_profile(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetSubmitterProfileRequest>,
    ) -> ServiceResult<GetSubmitterProfileResponse> {
        handle(async {
            let submitter = Submitter::of(&ctx)?;
            let db = &self.core.db;
            let profile = SubmitterProfile {
                trusted: submitter.moderator || emojis::is_trusted(db, submitter.user_id).await?,
                pending_count: emojis::pending_count(db, submitter.user_id).await? as i32,
                max_pending: self.cfg.max_pending as i32,
                approved_count: emojis::approved_count(db, submitter.user_id).await? as i32,
                max_image_bytes: self.cfg.max_image_bytes(),
                min_image_edge: MIN_IMAGE_EDGE,
                max_aliases: MAX_ALIASES as i32,
                ..Default::default()
            };
            ok(GetSubmitterProfileResponse { profile: profile.into(), ..Default::default() })
        })
        .await
    }

    async fn check_name(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CheckNameRequest>,
    ) -> ServiceResult<CheckNameResponse> {
        handle(async {
            let submitter = Submitter::of(&ctx)?;
            self.core.cache.rate_limit(&format!("emoji-name:{}", submitter.user_id), 120, MINUTE).await?;
            let unavailable = |name: &str, reason: &str| {
                ok(CheckNameResponse {
                    available: false,
                    name: name.to_owned(),
                    reason: reason.to_owned(),
                    ..Default::default()
                })
            };
            let name = match self.submitted_name(request.name, &submitter) {
                Ok(name) => name,
                Err(e) if e.reason == "NAME_RESERVED" => {
                    return unavailable(&validate::normalize_name(request.name).unwrap_or_default(), "NAME_RESERVED");
                }
                Err(_) => return unavailable("", "NAME_INVALID"),
            };
            if emojis::find_by_name(&self.core.db, &name).await?.is_some() {
                return unavailable(&name, "NAME_TAKEN");
            }
            ok(CheckNameResponse { available: true, name, ..Default::default() })
        })
        .await
    }

    async fn submit_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SubmitEmojiRequest>,
    ) -> ServiceResult<SubmitEmojiResponse> {
        handle(async {
            let submitter = Submitter::of(&ctx)?;
            let request = request.to_owned_message();
            let name = self.submitted_name(&request.name, &submitter)?;
            let aliases = validate::aliases(&request.aliases)?;
            let license = validate::license(&request.license)?;
            self.core.cache.rate_limit(&format!("emoji-submit:{}", submitter.user_id), 20, HOUR).await?;
            let picture = self.picture(&request.asset_id, submitter.user_id).await?;

            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            emojis::serialize_submissions(&mut tx, submitter.user_id).await?;
            let category = categories::resolve(&mut tx, &request.category).await?;
            let admission = if self.skips_review(&mut tx, &submitter).await? {
                Admission::Catalog(AddedVia::Trusted)
            } else {
                self.ensure_room_in_queue(&mut tx, submitter.user_id).await?;
                Admission::Review
            };
            let new = NewEmoji {
                name,
                picture,
                category,
                aliases,
                license,
                sensitive: request.sensitive,
                local_only: false,
                author_id: Some(submitter.user_id),
            };
            let emoji = emojis::create(&mut tx, &mut fx, &new, admission, Some(submitter.user_id))
                .await?
                .ok_or_else(name_taken)?;
            self.commit(tx, fx).await?;
            ok(SubmitEmojiResponse {
                emoji: self.detail(&emoji, submitter.viewer()).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_submission(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateSubmissionRequest>,
    ) -> ServiceResult<UpdateSubmissionResponse> {
        handle(async {
            let submitter = Submitter::of(&ctx)?;
            let request = request.to_owned_message();
            let emoji_id = parse_id(&request.emoji_id)?;
            // Whose emoji it is comes first: nothing else about the request
            // is looked at for an emoji the caller may not change.
            let found = emojis::find_by_id(&self.core.db, emoji_id)
                .await?
                .or_not_found("EMOJI_NOT_FOUND", "emoji not found")?;
            ensure_own_and_open(&found, submitter.user_id, NOT_EDITABLE)?;
            let mut edit = Edit {
                name: request.name.as_deref().map(|name| self.submitted_name(name, &submitter)).transpose()?,
                aliases: request.aliases.as_option().map(|list| validate::aliases(&list.aliases)).transpose()?,
                license: request.license.as_deref().map(validate::license).transpose()?,
                sensitive: request.sensitive,
                ..Default::default()
            };
            self.core.cache.rate_limit(&format!("emoji-edit:{}", submitter.user_id), 60, HOUR).await?;
            if let Some(asset_id) = &request.asset_id {
                edit.picture = Some(self.picture(asset_id, submitter.user_id).await?);
            }

            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            emojis::serialize_submissions(&mut tx, submitter.user_id).await?;
            // The category is locked before the emoji, as everywhere: renaming
            // a category walks from the category to its emoji, and the two
            // must not wait on each other.
            if let Some(category) = &request.category {
                edit.category = Some(categories::resolve(&mut tx, category).await?);
            }
            // Checked again under the lock: a moderator may have decided meanwhile.
            let current = emojis::lock(&mut tx, emoji_id).await?;
            ensure_own_and_open(&current, submitter.user_id, NOT_EDITABLE)?;
            let (mut emoji, _) = emojis::edit(&mut tx, &mut fx, &current, edit).await?;
            // Changing a rejected emoji is how its author submits it again.
            if emoji.state == state::REJECTED {
                let trusted = self.skips_review(&mut tx, &submitter).await?;
                if !trusted {
                    self.ensure_room_in_queue(&mut tx, submitter.user_id).await?;
                }
                emoji = emojis::resubmit(&mut tx, &mut fx, &emoji, submitter.user_id, trusted).await?;
            }
            self.commit(tx, fx).await?;
            ok(UpdateSubmissionResponse {
                emoji: self.detail(&emoji, submitter.viewer()).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn withdraw_submission(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WithdrawSubmissionRequest>,
    ) -> ServiceResult<WithdrawSubmissionResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let emoji_id = parse_id(request.emoji_id)?;
            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            let emoji = emojis::lock(&mut tx, emoji_id).await?;
            ensure_own_and_open(&emoji, user_id, NOT_WITHDRAWABLE)?;
            emojis::delete(&mut tx, &mut fx, &emoji, RemovedBecause::Deleted, Some(user_id)).await?;
            self.commit(tx, fx).await?;
            ok(WithdrawSubmissionResponse::default())
        })
        .await
    }

    async fn list_my_submissions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMySubmissionsRequest>,
    ) -> ServiceResult<ListMySubmissionsResponse> {
        handle(async {
            let submitter = Submitter::of(&ctx)?;
            let only_state = Some(request.state.to_i32() as i16).filter(|state| *state != 0);
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            // IDs are time-ordered, so descending IDs are newest first.
            let mut rows: Vec<EmojiRow> = sqlx::query_as(concat!(
                "SELECT ",
                emoji_columns!(),
                " FROM emojis e
                  WHERE e.author_id = $1
                    AND ($2::smallint IS NULL OR e.state = $2)
                    AND ($3::uuid IS NULL OR e.id < $3)
                  ORDER BY e.id DESC
                  LIMIT $4"
            ))
            .bind(submitter.user_id)
            .bind(only_state)
            .bind(before)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let page = page.finish(&mut rows, |row| row.id);
            ok(ListMySubmissionsResponse {
                emojis: self.details(&rows, submitter.viewer()).await?,
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }
}
