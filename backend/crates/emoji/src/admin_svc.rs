//! `EmojiAdminService`: what moderators do — review, direct changes,
//! categories, trusted submitters, import and export.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{
    events,
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::emoji::{__buffa::oneof::bulk_update_emoji_request::Change, *};

use crate::{
    Emoji, catalog,
    catalog_svc::MODERATOR,
    categories,
    emojis::{self, Admission, Edit, Effects, NewEmoji, Tx},
    events::{AddedVia, RemovedBecause, SUBMITTER_TRUST_CHANGED, SubmitterTrust},
    export, import,
    model::{EmojiRow, Viewer, emoji_columns},
    validate::{self, MAX_ALIASES, MAX_BULK_IDS},
};

const HOUR: Duration = Duration::from_secs(3600);
/// Decisions (approve, reject, disable, enable) on one emoji per hour.
const DECISIONS_PER_EMOJI_AN_HOUR: u32 = 30;
/// Changes of one user's trusted flag per hour.
const TRUST_CHANGES_PER_USER_AN_HOUR: u32 = 10;

/// The moderator calling an RPC of this service (`FORBIDDEN` for anyone else).
fn moderator(ctx: &RequestContext) -> AppResult<Uuid> {
    Ok(ctx.require_role(MODERATOR)?.user_id)
}

fn name_taken() -> AppError {
    AppError::already_exists("NAME_TAKEN", "another emoji has this name")
}

/// The IDs of a bulk request: 1 to [`MAX_BULK_IDS`] of them (`EMOJI_IDS_INVALID`).
fn bulk_ids(emoji_ids: &[String]) -> AppResult<Vec<Uuid>> {
    ensure!(
        (1..=MAX_BULK_IDS).contains(&emoji_ids.len()),
        invalid,
        "EMOJI_IDS_INVALID",
        "name between 1 and {MAX_BULK_IDS} emoji"
    );
    let mut ids =
        parse_ids(emoji_ids).map_err(|_| AppError::invalid("EMOJI_IDS_INVALID", "an emoji ID is malformed"))?;
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// The edit a bulk change makes to one emoji.
fn bulk_edit(change: &BulkChange, emoji: &EmojiRow) -> Edit {
    match change {
        BulkChange::Category(category) => Edit { category: Some(category.clone()), ..Default::default() },
        BulkChange::AddAliases(added) => {
            let mut aliases = emoji.aliases.clone();
            for alias in added {
                if aliases.len() < MAX_ALIASES && !aliases.contains(alias) {
                    aliases.push(alias.clone());
                }
            }
            Edit { aliases: Some(aliases), ..Default::default() }
        }
        BulkChange::RemoveAliases(removed) => {
            let aliases = emoji.aliases.iter().filter(|alias| !removed.contains(alias)).cloned().collect();
            Edit { aliases: Some(aliases), ..Default::default() }
        }
        BulkChange::SetAliases(aliases) => Edit { aliases: Some(aliases.clone()), ..Default::default() },
        BulkChange::License(license) => Edit { license: Some(license.clone()), ..Default::default() },
        BulkChange::Sensitive(sensitive) => Edit { sensitive: Some(*sensitive), ..Default::default() },
        BulkChange::LocalOnly(local_only) => Edit { local_only: Some(*local_only), ..Default::default() },
    }
}

/// A validated bulk change.
enum BulkChange {
    Category(Option<String>),
    AddAliases(Vec<String>),
    RemoveAliases(Vec<String>),
    SetAliases(Vec<String>),
    License(String),
    Sensitive(bool),
    LocalOnly(bool),
}

#[derive(sqlx::FromRow)]
struct TrustedRow {
    user_id: Uuid,
    trusted_by: Option<Uuid>,
    trusted_at: Option<Time>,
    approved_count: i64,
}

impl Emoji {
    /// The user a moderator wants to credit: must be a live account
    /// (`USER_NOT_FOUND`). Empty means nobody.
    async fn credited_author(&self, author_id: &str) -> AppResult<Option<Uuid>> {
        let Some(author_id) = parse_opt_id(author_id)? else { return Ok(None) };
        let active = tokuly_identity::users::filter_active(&self.core, &[author_id]).await?;
        ensure!(active.contains(&author_id), not_found, "USER_NOT_FOUND", "user not found");
        Ok(Some(author_id))
    }

    /// Runs one moderator decision on one emoji and returns it as the
    /// moderator sees it afterwards.
    async fn decide<F>(&self, moderator_id: Uuid, reference: &str, decision: F) -> AppResult<EmojiDetail>
    where
        F: AsyncFnOnce(&mut Tx<'_>, &mut Effects, &EmojiRow) -> AppResult<EmojiRow>,
    {
        let found = emojis::require(&self.core, reference).await?;
        // Every decision tells the author. Nobody decides on one emoji
        // dozens of times an hour except to pester its author.
        self.core.cache.rate_limit(&format!("emoji-decide:{}", found.id), DECISIONS_PER_EMOJI_AN_HOUR, HOUR).await?;
        let mut tx = self.core.db.begin().await?;
        let mut fx = Effects::default();
        let current = emojis::lock(&mut tx, found.id).await?;
        let decided = decision(&mut tx, &mut fx, &current).await?;
        self.commit(tx, fx).await?;
        self.detail(&decided, Viewer::moderator(moderator_id)).await
    }

    /// All categories in display order, after a change to them.
    async fn categories_now(&self) -> AppResult<Vec<Category>> {
        Ok(catalog::snapshot(&self.core).await?.categories())
    }
}

impl EmojiAdminService for Emoji {
    async fn list_review_queue(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListReviewQueueRequest>,
    ) -> ServiceResult<ListReviewQueueResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let cursor: Option<(Time, Uuid)> = page.decode()?;
            let (after_at, after_id) = cursor.unzip();
            let mut rows: Vec<EmojiRow> = sqlx::query_as(concat!(
                "SELECT ",
                emoji_columns!(),
                " FROM emojis e
                  WHERE e.state = 1 AND ($1::timestamptz IS NULL OR (e.created_at, e.id) > ($1, $2))
                  ORDER BY e.created_at, e.id
                  LIMIT $3"
            ))
            .bind(after_at)
            .bind(after_id)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let page = page.finish(&mut rows, |row| (row.created_at, row.id));
            let pending_count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM emojis WHERE state = 1").fetch_one(&self.core.db).await?;
            ok(ListReviewQueueResponse {
                emojis: self.details(&rows, Viewer::moderator(moderator_id)).await?,
                page: page.into(),
                pending_count: pending_count as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn approve_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ApproveEmojiRequest>,
    ) -> ServiceResult<ApproveEmojiResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let emoji = self
                .decide(moderator_id, request.emoji_id, async |tx, fx, emoji| {
                    emojis::approve(tx, fx, emoji, moderator_id).await
                })
                .await?;
            ok(ApproveEmojiResponse { emoji: emoji.into(), ..Default::default() })
        })
        .await
    }

    async fn reject_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RejectEmojiRequest>,
    ) -> ServiceResult<RejectEmojiResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let reason = validate::reason(request.reason)?;
            let emoji = self
                .decide(moderator_id, request.emoji_id, async |tx, fx, emoji| {
                    emojis::reject(tx, fx, emoji, moderator_id, &reason).await
                })
                .await?;
            ok(RejectEmojiResponse { emoji: emoji.into(), ..Default::default() })
        })
        .await
    }

    async fn list_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListEmojiRequest>,
    ) -> ServiceResult<ListEmojiResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let only_state = Some(request.state.to_i32() as i16).filter(|state| *state != 0);
            // Names and aliases are stored lowercase; the query is compared as typed otherwise.
            ensure!(
                request.query.chars().count() <= validate::QUERY_MAX_CHARS,
                invalid,
                "QUERY_TOO_LONG",
                "the query is at most {} characters",
                validate::QUERY_MAX_CHARS
            );
            let query = Some(validate::strip_colons(request.query).to_lowercase()).filter(|query| !query.is_empty());
            let author_id = parse_opt_id(request.author_id)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let before: Option<Uuid> = page.decode()?;
            let mut rows: Vec<EmojiRow> = sqlx::query_as(concat!(
                "SELECT ",
                emoji_columns!(),
                " FROM emojis e
                  WHERE ($1::smallint IS NULL OR e.state = $1)
                    AND ($2::text IS NULL OR strpos(e.name, $2) > 0
                         OR EXISTS (SELECT 1 FROM unnest(e.aliases) AS alias WHERE strpos(alias, $2) > 0))
                    AND ($3::text IS NULL
                         OR CASE WHEN $3 = '' THEN e.category IS NULL ELSE lower(e.category) = lower($3) END)
                    AND ($4::uuid IS NULL OR e.author_id = $4)
                    AND ($5::uuid IS NULL OR e.id < $5)
                  ORDER BY e.id DESC
                  LIMIT $6"
            ))
            .bind(only_state)
            .bind(query)
            .bind(request.category.map(str::trim))
            .bind(author_id)
            .bind(before)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let page = page.finish(&mut rows, |row| row.id);
            ok(ListEmojiResponse {
                emojis: self.details(&rows, Viewer::moderator(moderator_id)).await?,
                page: page.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn add_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddEmojiRequest>,
    ) -> ServiceResult<AddEmojiResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let request = request.to_owned_message();
            let name = validate::name(&request.name)?;
            let aliases = validate::aliases(&request.aliases)?;
            let license = validate::license(&request.license)?;
            let author_id = self.credited_author(&request.author_id).await?;
            let picture = self.picture(&request.asset_id, moderator_id).await?;

            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            let category = categories::resolve(&mut tx, &request.category).await?;
            let new = NewEmoji {
                name,
                picture,
                category,
                aliases,
                license,
                sensitive: request.sensitive,
                local_only: request.local_only,
                author_id,
            };
            let emoji = emojis::create(&mut tx, &mut fx, &new, Admission::Catalog(AddedVia::Staff), Some(moderator_id))
                .await?
                .ok_or_else(name_taken)?;
            self.commit(tx, fx).await?;
            ok(AddEmojiResponse {
                emoji: self.detail(&emoji, Viewer::moderator(moderator_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateEmojiRequest>,
    ) -> ServiceResult<UpdateEmojiResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let request = request.to_owned_message();
            let found = emojis::require(&self.core, &request.emoji_id).await?;
            let mut edit = Edit {
                name: request.name.as_deref().map(validate::name).transpose()?,
                aliases: request.aliases.as_option().map(|list| validate::aliases(&list.aliases)).transpose()?,
                license: request.license.as_deref().map(validate::license).transpose()?,
                sensitive: request.sensitive,
                local_only: request.local_only,
                ..Default::default()
            };
            if let Some(author_id) = &request.author_id {
                edit.author_id = Some(self.credited_author(author_id).await?);
            }
            if let Some(asset_id) = &request.asset_id {
                edit.picture = Some(self.picture(asset_id, moderator_id).await?);
            }

            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            // Category first, then the emoji (see `UpdateSubmission`).
            if let Some(category) = &request.category {
                edit.category = Some(categories::resolve(&mut tx, category).await?);
            }
            let current = emojis::lock(&mut tx, found.id).await?;
            let (emoji, changed) = emojis::edit(&mut tx, &mut fx, &current, edit).await?;
            if !changed.is_empty() {
                emojis::record_edit(&mut tx, emoji.id, moderator_id, &changed).await?;
                fx.author_updates.push(emoji.id);
            }
            self.commit(tx, fx).await?;
            ok(UpdateEmojiResponse {
                emoji: self.detail(&emoji, Viewer::moderator(moderator_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn disable_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DisableEmojiRequest>,
    ) -> ServiceResult<DisableEmojiResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let reason = validate::reason(request.reason)?;
            let emoji = self
                .decide(moderator_id, request.emoji_id, async |tx, fx, emoji| {
                    emojis::disable(tx, fx, emoji, Some(moderator_id), &reason, None).await
                })
                .await?;
            ok(DisableEmojiResponse { emoji: emoji.into(), ..Default::default() })
        })
        .await
    }

    async fn enable_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, EnableEmojiRequest>,
    ) -> ServiceResult<EnableEmojiResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let emoji = self
                .decide(moderator_id, request.emoji_id, async |tx, fx, emoji| {
                    emojis::enable(tx, fx, emoji, moderator_id).await
                })
                .await?;
            ok(EnableEmojiResponse { emoji: emoji.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteEmojiRequest>,
    ) -> ServiceResult<DeleteEmojiResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let found = emojis::require(&self.core, request.emoji_id).await?;
            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            let emoji = emojis::lock(&mut tx, found.id).await?;
            emojis::delete(&mut tx, &mut fx, &emoji, RemovedBecause::Deleted, Some(moderator_id)).await?;
            self.commit(tx, fx).await?;
            ok(DeleteEmojiResponse::default())
        })
        .await
    }

    async fn bulk_update_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BulkUpdateEmojiRequest>,
    ) -> ServiceResult<BulkUpdateEmojiResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let request = request.to_owned_message();
            let emoji_ids = bulk_ids(&request.emoji_ids)?;
            let Some(change) = request.change else {
                bail!(invalid, "CHANGE_REQUIRED", "say what to change");
            };

            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            let change = match change {
                Change::SetCategory(category) => BulkChange::Category(categories::resolve(&mut tx, &category).await?),
                Change::AddAliases(list) => BulkChange::AddAliases(validate::aliases(&list.aliases)?),
                Change::RemoveAliases(list) => BulkChange::RemoveAliases(validate::aliases(&list.aliases)?),
                Change::SetAliases(list) => BulkChange::SetAliases(validate::aliases(&list.aliases)?),
                Change::SetLicense(license) => BulkChange::License(validate::license(&license)?),
                Change::SetSensitive(sensitive) => BulkChange::Sensitive(sensitive),
                Change::SetLocalOnly(local_only) => BulkChange::LocalOnly(local_only),
            };
            let mut updated_count = 0;
            for current in emojis::lock_many(&mut tx, &emoji_ids).await? {
                let (emoji, changed) = emojis::edit(&mut tx, &mut fx, &current, bulk_edit(&change, &current)).await?;
                if !changed.is_empty() {
                    emojis::record_edit(&mut tx, emoji.id, moderator_id, &changed).await?;
                    updated_count += 1;
                }
            }
            self.commit(tx, fx).await?;
            ok(BulkUpdateEmojiResponse { updated_count, ..Default::default() })
        })
        .await
    }

    async fn bulk_delete_emoji(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, BulkDeleteEmojiRequest>,
    ) -> ServiceResult<BulkDeleteEmojiResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let request = request.to_owned_message();
            let emoji_ids = bulk_ids(&request.emoji_ids)?;
            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            let doomed = emojis::lock_many(&mut tx, &emoji_ids).await?;
            for emoji in &doomed {
                emojis::delete(&mut tx, &mut fx, emoji, RemovedBecause::Deleted, Some(moderator_id)).await?;
            }
            self.commit(tx, fx).await?;
            ok(BulkDeleteEmojiResponse { deleted_count: doomed.len() as i32, ..Default::default() })
        })
        .await
    }

    async fn set_submitter_trusted(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetSubmitterTrustedRequest>,
    ) -> ServiceResult<SetSubmitterTrustedResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let user_id = parse_id(request.user_id)?;
            let trusted = request.trusted;
            let active = tokuly_identity::users::filter_active(&self.core, &[user_id]).await?;
            ensure!(active.contains(&user_id), not_found, "USER_NOT_FOUND", "user not found");
            // Being trusted is announced to the user; flipping the flag back
            // and forth must not become a way to flood them.
            self.core.cache.rate_limit(&format!("emoji-trust:{user_id}"), TRUST_CHANGES_PER_USER_AN_HOUR, HOUR).await?;

            let mut tx = self.core.db.begin().await?;
            let changed = if trusted {
                sqlx::query(
                    "INSERT INTO emoji_submitters (user_id, trusted, trusted_by, trusted_at) VALUES ($1, true, $2, now())
                     ON CONFLICT (user_id) DO UPDATE SET trusted = true, trusted_by = EXCLUDED.trusted_by, trusted_at = now()
                     WHERE NOT emoji_submitters.trusted",
                )
                .bind(user_id)
                .bind(moderator_id)
                .execute(&mut *tx)
                .await?
            } else {
                sqlx::query(
                    "UPDATE emoji_submitters SET trusted = false, trusted_by = NULL, trusted_at = NULL WHERE user_id = $1 AND trusted",
                )
                .bind(user_id)
                .execute(&mut *tx)
                .await?
            };
            if changed.rows_affected() == 1 {
                let change = SubmitterTrust { user_id, trusted, moderator_id };
                events::emit(&mut *tx, SUBMITTER_TRUST_CHANGED, Some(moderator_id), &change).await?;
                if trusted {
                    let notice = NotificationRequest::new(
                        user_id,
                        "emoji.trusted",
                        Localized::new("認定絵文字クリエイターになったよ!", "You are now a trusted emoji creator!"),
                        Localized::new(
                            "これからは審査なしで絵文字を追加できるよ!",
                            "Your emoji now join the catalogue without review.",
                        ),
                        "/emoji/submissions",
                    );
                    outbound::notify(&mut *tx, &notice).await?;
                }
            }
            tx.commit().await?;
            ok(SetSubmitterTrustedResponse::default())
        })
        .await
    }

    async fn list_trusted_submitters(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListTrustedSubmittersRequest>,
    ) -> ServiceResult<ListTrustedSubmittersResponse> {
        handle(async {
            moderator(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let cursor: Option<(Time, Uuid)> = page.decode()?;
            let (before_at, before_id) = cursor.unzip();
            let mut rows: Vec<TrustedRow> = sqlx::query_as(
                "SELECT s.user_id, s.trusted_by, s.trusted_at,
                        (SELECT count(*) FROM emojis e WHERE e.author_id = s.user_id AND e.state = 2) AS approved_count
                 FROM emoji_submitters s
                 WHERE s.trusted AND ($1::timestamptz IS NULL OR (s.trusted_at, s.user_id) < ($1, $2))
                 ORDER BY s.trusted_at DESC, s.user_id DESC
                 LIMIT $3",
            )
            .bind(before_at)
            .bind(before_id)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let page = page.finish(&mut rows, |row| (row.trusted_at.unwrap_or_default(), row.user_id));
            let user_ids: Vec<Uuid> =
                rows.iter().flat_map(|row| [Some(row.user_id), row.trusted_by]).flatten().collect();
            let users = tokuly_identity::summaries(&self.core, &user_ids).await?;
            let submitters = rows
                .iter()
                .map(|row| TrustedSubmitter {
                    user: Some(tokuly_identity::users::pick(&users, row.user_id)).into(),
                    trusted_at: opt_ts(row.trusted_at),
                    trusted_by: row.trusted_by.and_then(|id| users.get(&id).cloned()).into(),
                    approved_count: row.approved_count as i32,
                    ..Default::default()
                })
                .collect();
            ok(ListTrustedSubmittersResponse { submitters, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn create_category(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateCategoryRequest>,
    ) -> ServiceResult<CreateCategoryResponse> {
        handle(async {
            moderator(&ctx)?;
            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            categories::create(&mut tx, &mut fx, request.name).await?;
            self.commit(tx, fx).await?;
            ok(CreateCategoryResponse { categories: self.categories_now().await?, ..Default::default() })
        })
        .await
    }

    async fn rename_category(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RenameCategoryRequest>,
    ) -> ServiceResult<RenameCategoryResponse> {
        handle(async {
            moderator(&ctx)?;
            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            categories::rename(&mut tx, &mut fx, request.name, request.new_name).await?;
            self.commit(tx, fx).await?;
            ok(RenameCategoryResponse { categories: self.categories_now().await?, ..Default::default() })
        })
        .await
    }

    async fn reorder_categories(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReorderCategoriesRequest>,
    ) -> ServiceResult<ReorderCategoriesResponse> {
        handle(async {
            moderator(&ctx)?;
            let request = request.to_owned_message();
            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            categories::reorder(&mut tx, &mut fx, &request.names).await?;
            self.commit(tx, fx).await?;
            ok(ReorderCategoriesResponse { categories: self.categories_now().await?, ..Default::default() })
        })
        .await
    }

    async fn delete_category(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteCategoryRequest>,
    ) -> ServiceResult<DeleteCategoryResponse> {
        handle(async {
            moderator(&ctx)?;
            let mut tx = self.core.db.begin().await?;
            let mut fx = Effects::default();
            categories::delete(&mut tx, &mut fx, request.name, request.move_to).await?;
            self.commit(tx, fx).await?;
            ok(DeleteCategoryResponse { categories: self.categories_now().await?, ..Default::default() })
        })
        .await
    }

    async fn start_import(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartImportRequest>,
    ) -> ServiceResult<StartImportResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let asset_id = parse_id(request.asset_id)?;
            let policy = import::Policy::from_proto(request.on_conflict.to_i32());
            let category = match request.category.trim() {
                "" => None,
                name => Some(validate::category_name(name)?),
            };
            self.core.cache.rate_limit(&format!("emoji-import:{moderator_id}"), 20, HOUR).await?;
            let import = self.begin_import(moderator_id, asset_id, policy, category).await?;
            ok(StartImportResponse { import: import.into(), ..Default::default() })
        })
        .await
    }

    async fn get_import(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetImportRequest>,
    ) -> ServiceResult<GetImportResponse> {
        handle(async {
            moderator(&ctx)?;
            let import_id = parse_id(request.import_id)?;
            let page = match request.page.as_option() {
                Some(page) => Page::new(page.limit, page.cursor, 100, 500),
                None => Page::new(0, "", 100, 500),
            };
            let (import, entries, page) = self.import_view(import_id, &page).await?;
            ok(GetImportResponse { import: import.into(), entries, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn list_imports(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListImportsRequest>,
    ) -> ServiceResult<ListImportsResponse> {
        handle(async {
            moderator(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (imports, page) = self.imports_page(&page).await?;
            ok(ListImportsResponse { imports, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn start_export(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartExportRequest>,
    ) -> ServiceResult<StartExportResponse> {
        handle(async {
            let moderator_id = moderator(&ctx)?;
            let scope = match request.category.map(str::trim) {
                None => export::Scope::Everything,
                Some("") => export::Scope::Uncategorized,
                Some(name) => {
                    let snapshot = catalog::snapshot(&self.core).await?;
                    let stored = snapshot
                        .category_named(name)
                        .ok_or_else(|| AppError::not_found("CATEGORY_NOT_FOUND", "there is no such category"))?;
                    export::Scope::Category(stored.to_owned())
                }
            };
            self.core.cache.rate_limit(&format!("emoji-export:{moderator_id}"), 20, HOUR).await?;
            let export = self.begin_export(moderator_id, scope).await?;
            ok(StartExportResponse { export: export.into(), ..Default::default() })
        })
        .await
    }

    async fn get_export(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetExportRequest>,
    ) -> ServiceResult<GetExportResponse> {
        handle(async {
            moderator(&ctx)?;
            let export = self.export_view(parse_id(request.export_id)?).await?;
            ok(GetExportResponse { export: export.into(), ..Default::default() })
        })
        .await
    }

    async fn list_exports(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListExportsRequest>,
    ) -> ServiceResult<ListExportsResponse> {
        handle(async {
            moderator(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (exports, page) = self.exports_page(&page).await?;
            ok(ListExportsResponse { exports, page: page.into(), ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emoji_with(aliases: &[&str]) -> EmojiRow {
        EmojiRow {
            id: new_id(),
            name: "blobcat".into(),
            state: crate::model::state::APPROVED,
            asset_id: new_id(),
            image: serde_json::json!({}),
            animated: false,
            category: None,
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
            license: String::new(),
            sensitive: false,
            local_only: false,
            author_id: None,
            review_note: String::new(),
            reviewed_at: None,
            approved_at: None,
            catalog_version: 0,
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn bulk_requests_name_between_one_and_two_hundred_emoji() {
        assert_eq!(bulk_ids(&[]).unwrap_err().reason, "EMOJI_IDS_INVALID");
        assert_eq!(bulk_ids(&["nope".to_owned()]).unwrap_err().reason, "EMOJI_IDS_INVALID");
        let many: Vec<String> = (0..=MAX_BULK_IDS).map(|_| new_id().to_string()).collect();
        assert_eq!(bulk_ids(&many).unwrap_err().reason, "EMOJI_IDS_INVALID");
        let id = new_id().to_string();
        assert_eq!(bulk_ids(&[id.clone(), id]).unwrap().len(), 1);
    }

    #[test]
    fn added_aliases_fill_up_to_the_limit_without_duplicates() {
        let emoji = emoji_with(&["cat", "neko"]);
        let added = BulkChange::AddAliases(vec!["neko".into(), "ねこ".into()]);
        assert_eq!(bulk_edit(&added, &emoji).aliases.unwrap(), ["cat", "neko", "ねこ"]);

        let full: Vec<String> = (0..MAX_ALIASES).map(|i| format!("a{i}")).collect();
        let emoji = emoji_with(&full.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(bulk_edit(&added, &emoji).aliases.unwrap(), full);
    }

    #[test]
    fn removed_and_replaced_aliases() {
        let emoji = emoji_with(&["cat", "neko", "ねこ"]);
        let removed = BulkChange::RemoveAliases(vec!["neko".into(), "dog".into()]);
        assert_eq!(bulk_edit(&removed, &emoji).aliases.unwrap(), ["cat", "ねこ"]);
        let replaced = BulkChange::SetAliases(vec!["kitty".into()]);
        assert_eq!(bulk_edit(&replaced, &emoji).aliases.unwrap(), ["kitty"]);
        let moved = BulkChange::Category(None);
        assert_eq!(bulk_edit(&moved, &emoji).category, Some(None));
    }
}
