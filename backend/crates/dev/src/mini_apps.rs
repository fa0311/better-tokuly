//! Mini apps (`dev_mini_apps`): an application's web page that people open
//! inside Tokuly. `MiniAppService` is the owner's manifest and the person's
//! grant; `MiniAppReviewService` is staff approving a manifest.
//!
//! What runs is the approved copy (`live_*`), taken from the draft at the
//! moment of approval, so an edit is reviewed before anyone sees it. The
//! bridge itself runs in the clients (`docs/miniapps.md`): the server keeps
//! the manifest, its review and what each person granted.
use std::{collections::HashSet, time::Duration};

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::{Image, LocalizedText},
    dev::*,
};

use crate::{
    Dev,
    apps::{AppRow, is_loopback_literal},
    clean_name, clean_text,
    images::ImageChange,
};

const MAX_PAGE_SIZE: i64 = 100;
const MAX_URL_LEN: usize = 2000;
const MODERATOR: &str = "moderator";

/// The mini-app scopes: what the bridge may hand a mini app, in the words
/// people agree to. `(scope, ja, en)`.
pub const SCOPES: &[(&str, &str, &str)] = &[
    ("profile", "あなたの名前・ユーザー名・アイコンを見る", "See your name, username and profile picture"),
    (
        "friends",
        "あなたがその都度選んだ友達の名前・ユーザー名・アイコンを見る（友達リストは見えません）",
        "See the name, username and picture of the friends you pick each time (never your friend list)",
    ),
];

fn describe(scopes: &[String]) -> Vec<MiniAppScope> {
    SCOPES
        .iter()
        .filter(|(scope, ..)| scopes.iter().any(|asked| asked == scope))
        .map(|(scope, ja, en)| MiniAppScope {
            scope: (*scope).to_owned(),
            description: LocalizedText { ja: (*ja).to_owned(), en: (*en).to_owned(), ..Default::default() }.into(),
            ..Default::default()
        })
        .collect()
}

/// Known scopes, each once, in catalogue order (`MINI_APP_SCOPE_UNKNOWN`).
fn check_scopes(scopes: &[String]) -> AppResult<Vec<String>> {
    if let Some(unknown) = scopes.iter().find(|asked| !SCOPES.iter().any(|(scope, ..)| scope == asked)) {
        bail!(invalid, "MINI_APP_SCOPE_UNKNOWN", "{unknown:?} is not a mini-app scope");
    }
    Ok(SCOPES
        .iter()
        .filter(|(scope, ..)| scopes.iter().any(|asked| asked == scope))
        .map(|(scope, ..)| (*scope).to_owned())
        .collect())
}

/// Checks a launch URL and returns it with its origin: `https` on a domain
/// that is not Tokuly's own (nor under it), or `http` on a loopback address
/// where development allows it. No credentials, no fragment.
pub(crate) fn launch_url(input: &str, allow_loopback: bool, tokuly_host: &str) -> AppResult<(String, String)> {
    let invalid = |why: &str| AppError::invalid("LAUNCH_URL_INVALID", format!("the launch URL {why}"));
    let input = input.trim();
    if input.is_empty() || input.len() > MAX_URL_LEN {
        return Err(invalid("must be 1-2000 characters"));
    }
    let url = url::Url::parse(input).map_err(|_| invalid("is not an absolute URL"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("must not contain credentials"));
    }
    if url.fragment().is_some() {
        return Err(invalid("must not have a fragment"));
    }
    match url.scheme() {
        "https" => {
            let domain = url.domain().ok_or_else(|| invalid("must name a domain"))?;
            let tokuly = tokuly_host.to_ascii_lowercase();
            if domain == tokuly || domain.ends_with(&format!(".{tokuly}")) {
                return Err(invalid("must not be on Tokuly's own site"));
            }
        }
        "http" if allow_loopback && is_loopback_literal(&url) => {}
        _ => return Err(invalid("must use https")),
    }
    Ok((input.to_owned(), url.origin().ascii_serialization()))
}

fn image(icon: &Option<serde_json::Value>) -> Option<Image> {
    icon.as_ref().and_then(|json| serde_json::from_value(json.clone()).ok())
}

fn icon_holder(app_id: Uuid) -> String {
    format!("dev:mini-app-icon:{app_id}")
}

/// Keeps the approved copy's icon alive while the draft's changes.
fn live_icon_holder(app_id: Uuid) -> String {
    format!("dev:mini-app-live-icon:{app_id}")
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct MiniAppRow {
    app_id: Uuid,
    name: String,
    icon: Option<serde_json::Value>,
    launch_url: String,
    scopes: Vec<String>,
    revision: i32,
    review_state: String,
    review_note: String,
    submitted_at: Option<Time>,
    reviewed_at: Option<Time>,
    live_name: Option<String>,
    live_icon: Option<serde_json::Value>,
    live_launch_url: Option<String>,
    live_scopes: Option<Vec<String>>,
    approved_at: Option<Time>,
}

const SELECT: &str =
    "SELECT app_id, name, icon, launch_url, scopes, revision, review_state, review_note, submitted_at, reviewed_at,
        live_name, live_icon, live_launch_url, live_scopes, approved_at FROM dev_mini_apps";

impl MiniAppRow {
    fn state(&self) -> MiniAppReviewState {
        match self.review_state.as_str() {
            "in_review" => MiniAppReviewState::MINI_APP_REVIEW_STATE_IN_REVIEW,
            "approved" => MiniAppReviewState::MINI_APP_REVIEW_STATE_APPROVED,
            "rejected" => MiniAppReviewState::MINI_APP_REVIEW_STATE_REJECTED,
            _ => MiniAppReviewState::MINI_APP_REVIEW_STATE_DRAFT,
        }
    }

    fn live(&self) -> Option<MiniAppManifest> {
        Some(MiniAppManifest {
            name: self.live_name.clone()?,
            icon: image(&self.live_icon).into(),
            launch_url: self.live_launch_url.clone()?,
            scopes: self.live_scopes.clone()?,
            ..Default::default()
        })
    }

    fn to_review(&self, app: PublicApp) -> MiniAppReview {
        MiniAppReview {
            app_id: self.app_id.to_string(),
            app: app.into(),
            draft: MiniAppManifest {
                name: self.name.clone(),
                icon: image(&self.icon).into(),
                launch_url: self.launch_url.clone(),
                scopes: self.scopes.clone(),
                ..Default::default()
            }
            .into(),
            state: self.state().into(),
            revision: self.revision,
            review_note: self.review_note.clone(),
            submitted_at: opt_ts(self.submitted_at),
            reviewed_at: opt_ts(self.reviewed_at),
            live: self.live().into(),
            approved_at: opt_ts(self.approved_at),
            ..Default::default()
        }
    }
}

impl Dev {
    async fn mini_app_row(&self, app_id: Uuid) -> AppResult<Option<MiniAppRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("{SELECT} WHERE app_id = $1")))
            .bind(app_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    async fn review_for_owner(&self, app: &AppRow) -> AppResult<MiniAppReview> {
        let row =
            self.mini_app_row(app.id).await?.or_not_found("MINI_APP_NOT_FOUND", "this application has no mini app")?;
        let owner = tokuly_identity::summary(&self.core, app.owner_id).await?;
        Ok(row.to_review(app.to_public(owner)))
    }

    /// An application of `owner` that moderation has not taken down.
    async fn active_owned_app(&self, app_id: &str, owner: Uuid) -> AppResult<AppRow> {
        let app = self.owned_app(parse_id(app_id)?, owner).await?;
        ensure!(app.is_active(), precondition, "APP_DISABLED", "this application was disabled by moderation");
        Ok(app)
    }

    /// An approved mini app people may open, with its copy as they open it.
    async fn openable_mini_app(&self, app_id: &str) -> AppResult<(MiniApp, Vec<String>)> {
        let not_found = || AppError::not_found("MINI_APP_NOT_FOUND", "mini app not found");
        let app_id = Uuid::parse_str(app_id).map_err(|_| not_found())?;
        let app = self.find_app(app_id).await?.filter(|app| app.is_usable(&self.identity)).ok_or_else(not_found)?;
        let live = self.mini_app_row(app_id).await?.and_then(|row| row.live()).ok_or_else(not_found)?;
        let origin =
            url::Url::parse(&live.launch_url).map(|url| url.origin().ascii_serialization()).unwrap_or_default();
        let developer = tokuly_identity::summary(&self.core, app.owner_id).await?;
        let mini_app = MiniApp {
            app_id: app_id.to_string(),
            name: live.name,
            icon: live.icon,
            launch_url: live.launch_url,
            origin,
            scopes: describe(&live.scopes),
            developer: developer.into(),
            ..Default::default()
        };
        Ok((mini_app, live.scopes))
    }

    /// What `user_id` granted, narrowed to what the approved copy asks for.
    async fn granted_scopes(&self, app_id: Uuid, user_id: Uuid, asked: &[String]) -> AppResult<Vec<String>> {
        let granted: Option<Vec<String>> =
            sqlx::query_scalar("SELECT scopes FROM dev_mini_app_grants WHERE app_id = $1 AND user_id = $2")
                .bind(app_id)
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        let granted: HashSet<String> = granted.unwrap_or_default().into_iter().collect();
        Ok(asked.iter().filter(|scope| granted.contains(*scope)).cloned().collect())
    }

    /// Releases what a mini app holds, on the caller's transaction (its
    /// application is being deleted, or the mini app is).
    pub(crate) async fn release_mini_app_icons(&self, tx: &mut sqlx::PgConnection, app_id: Uuid) -> AppResult<()> {
        if let Ok(images) = self.images() {
            images.release(tx, &icon_holder(app_id)).await?;
            images.release(tx, &live_icon_holder(app_id)).await?;
        }
        Ok(())
    }
}

impl MiniAppService for Dev {
    async fn list_mini_app_scopes(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListMiniAppScopesRequest>,
    ) -> ServiceResult<ListMiniAppScopesResponse> {
        handle(async {
            let all: Vec<String> = SCOPES.iter().map(|(scope, ..)| (*scope).to_owned()).collect();
            ok(ListMiniAppScopesResponse { scopes: describe(&all), ..Default::default() })
        })
        .await
    }

    async fn get_mini_app_review(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetMiniAppReviewRequest>,
    ) -> ServiceResult<GetMiniAppReviewResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let app = self.owned_app(parse_id(request.app_id)?, owner).await?;
            ok(GetMiniAppReviewResponse { review: self.review_for_owner(&app).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn set_mini_app_manifest(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetMiniAppManifestRequest>,
    ) -> ServiceResult<SetMiniAppManifestResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let req = request.to_owned_message();
            let app = self.active_owned_app(&req.app_id, owner).await?;
            let name = clean_name(&req.name, 30, "MINI_APP_NAME_INVALID", "the mini app's name")?;
            let (launch_url, _) =
                launch_url(&req.launch_url, self.cfg.mini_app_allow_loopback, &self.core.cfg.public_host())?;
            let scopes = check_scopes(&req.scopes)?;
            self.core.cache.rate_limit(&format!("dev-mini-app-update:{owner}"), 60, Duration::from_secs(3600)).await?;
            // Looked up before the transaction: it reads through the pool.
            let icon = ImageChange::resolve(self, owner, req.icon_asset_id.as_deref()).await?;
            let mut tx = self.core.db.begin().await?;
            // Any change is a new revision to review; a review under way is abandoned.
            sqlx::query(
                "INSERT INTO dev_mini_apps (app_id, name, launch_url, scopes) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (app_id) DO UPDATE
                    SET name = EXCLUDED.name, launch_url = EXCLUDED.launch_url, scopes = EXCLUDED.scopes,
                        revision = dev_mini_apps.revision + 1, review_state = 'draft', review_note = '',
                        submitted_at = NULL, reviewed_at = NULL, reviewed_by = NULL",
            )
            .bind(app.id)
            .bind(&name)
            .bind(&launch_url)
            .bind(&scopes)
            .execute(&mut *tx)
            .await?;
            if let Some(icon) = icon.apply(self, &mut tx, &icon_holder(app.id)).await? {
                sqlx::query("UPDATE dev_mini_apps SET icon = $2 WHERE app_id = $1")
                    .bind(app.id)
                    .bind(icon)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            ok(SetMiniAppManifestResponse { review: self.review_for_owner(&app).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn submit_mini_app(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SubmitMiniAppRequest>,
    ) -> ServiceResult<SubmitMiniAppResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let app = self.active_owned_app(request.app_id, owner).await?;
            let row = self
                .mini_app_row(app.id)
                .await?
                .or_not_found("MINI_APP_NOT_FOUND", "this application has no mini app")?;
            match row.review_state.as_str() {
                "in_review" => {}
                "draft" => {
                    self.core
                        .cache
                        .rate_limit(&format!("dev-mini-app-submit:{}", app.id), 10, Duration::from_secs(86_400))
                        .await?;
                    // Only the revision just read: an edit in between is a draft of its own.
                    sqlx::query(
                        "UPDATE dev_mini_apps SET review_state = 'in_review', submitted_at = now()
                         WHERE app_id = $1 AND revision = $2 AND review_state = 'draft'",
                    )
                    .bind(app.id)
                    .bind(row.revision)
                    .execute(&self.core.db)
                    .await?;
                }
                _ => bail!(precondition, "MINI_APP_UNCHANGED", "change the manifest before submitting it again"),
            }
            ok(SubmitMiniAppResponse { review: self.review_for_owner(&app).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_mini_app(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteMiniAppRequest>,
    ) -> ServiceResult<DeleteMiniAppResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            // What moderation took down stays on record, its mini app included.
            let app = self.active_owned_app(request.app_id, owner).await?;
            let mut tx = self.core.db.begin().await?;
            let deleted =
                sqlx::query("DELETE FROM dev_mini_apps WHERE app_id = $1").bind(app.id).execute(&mut *tx).await?;
            ensure!(deleted.rows_affected() == 1, not_found, "MINI_APP_NOT_FOUND", "this application has no mini app");
            self.release_mini_app_icons(&mut tx, app.id).await?;
            tx.commit().await?;
            ok(DeleteMiniAppResponse::default())
        })
        .await
    }

    async fn get_mini_app(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetMiniAppRequest>,
    ) -> ServiceResult<GetMiniAppResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            self.limit_consent(user_id, "view", 120).await?;
            let (mini_app, asked) = self.openable_mini_app(request.app_id).await?;
            let granted_scopes = self.granted_scopes(parse_id(&mini_app.app_id)?, user_id, &asked).await?;
            ok(GetMiniAppResponse { mini_app: mini_app.into(), granted_scopes, ..Default::default() })
        })
        .await
    }

    async fn grant_mini_app(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GrantMiniAppRequest>,
    ) -> ServiceResult<GrantMiniAppResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            self.limit_consent(user_id, "approve", 30).await?;
            let req = request.to_owned_message();
            let (mini_app, asked) = self.openable_mini_app(&req.app_id).await?;
            if let Some(extra) = req.scopes.iter().find(|scope| !asked.contains(scope)) {
                bail!(invalid, "MINI_APP_SCOPE_UNKNOWN", "this mini app does not ask for {extra:?}");
            }
            let app_id = parse_id(&mini_app.app_id)?;
            sqlx::query(
                "INSERT INTO dev_mini_app_grants (app_id, user_id, scopes) VALUES ($1, $2, $3)
                 ON CONFLICT (app_id, user_id) DO UPDATE
                    SET scopes = (SELECT array_agg(DISTINCT s ORDER BY s) FROM unnest(dev_mini_app_grants.scopes || EXCLUDED.scopes) AS s),
                        updated_at = now()",
            )
            .bind(app_id)
            .bind(user_id)
            .bind(&req.scopes)
            .execute(&self.core.db)
            .await?;
            let granted_scopes = self.granted_scopes(app_id, user_id, &asked).await?;
            ok(GrantMiniAppResponse { granted_scopes, ..Default::default() })
        })
        .await
    }

    async fn revoke_mini_app_grant(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeMiniAppGrantRequest>,
    ) -> ServiceResult<RevokeMiniAppGrantResponse> {
        handle(async {
            let user_id = ctx.require_session()?.user_id;
            if let Ok(app_id) = Uuid::parse_str(request.app_id) {
                sqlx::query("DELETE FROM dev_mini_app_grants WHERE app_id = $1 AND user_id = $2")
                    .bind(app_id)
                    .bind(user_id)
                    .execute(&self.core.db)
                    .await?;
            }
            ok(RevokeMiniAppGrantResponse::default())
        })
        .await
    }
}

impl MiniAppReviewService for Dev {
    async fn list_mini_app_reviews(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMiniAppReviewsRequest>,
    ) -> ServiceResult<ListMiniAppReviewsResponse> {
        handle(async {
            ctx.require_role(MODERATOR)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE_SIZE);
            let after: Option<(Time, Uuid)> = page.decode()?;
            let (after_at, after_id) = after.unzip();
            let mut rows: Vec<MiniAppRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "{SELECT} WHERE review_state = 'in_review'
                   AND ($1::timestamptz IS NULL OR (submitted_at, app_id) > ($1, $2))
                 ORDER BY submitted_at, app_id LIMIT $3"
            )))
            .bind(after_at)
            .bind(after_id)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| (row.submitted_at.unwrap_or_default(), row.app_id));
            let app_ids: Vec<Uuid> = rows.iter().map(|row| row.app_id).collect();
            let mut apps = self.public_apps(&app_ids).await?;
            let reviews = rows.iter().filter_map(|row| Some(row.to_review(apps.remove(&row.app_id)?))).collect();
            ok(ListMiniAppReviewsResponse { reviews, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn review_mini_app(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReviewMiniAppRequest>,
    ) -> ServiceResult<ReviewMiniAppResponse> {
        handle(async {
            let moderator = ctx.require_role(MODERATOR)?.user_id;
            let app_id = parse_id(request.app_id)?;
            let note = clean_text(request.note, 500, "REVIEW_NOTE_TOO_LONG", "the note")?;
            ensure!(request.approve || !note.is_empty(), invalid, "REVIEW_NOTE_REQUIRED", "say why it is rejected");
            let app = self.find_app(app_id).await?.or_not_found("MINI_APP_NOT_FOUND", "mini app not found")?;
            ensure!(
                app.owner_id != moderator,
                denied,
                "OWN_APP",
                "a mini app is reviewed by somebody else than its owner"
            );
            self.core.cache.rate_limit(&format!("dev-mini-app-review:{app_id}"), 30, Duration::from_secs(3600)).await?;
            let mut tx = self.core.db.begin().await?;
            // Only the revision the moderator read, and only while it waits.
            let decided: Option<(String, Option<serde_json::Value>)> = sqlx::query_as(
                "UPDATE dev_mini_apps SET review_state = CASE WHEN $3 THEN 'approved' ELSE 'rejected' END,
                        review_note = $4, reviewed_by = $5, reviewed_at = now(),
                        live_name = CASE WHEN $3 THEN name ELSE live_name END,
                        live_icon = CASE WHEN $3 THEN icon ELSE live_icon END,
                        live_launch_url = CASE WHEN $3 THEN launch_url ELSE live_launch_url END,
                        live_scopes = CASE WHEN $3 THEN scopes ELSE live_scopes END,
                        approved_at = CASE WHEN $3 THEN now() ELSE approved_at END
                 WHERE app_id = $1 AND revision = $2 AND review_state = 'in_review'
                 RETURNING name, icon",
            )
            .bind(app_id)
            .bind(request.revision)
            .bind(request.approve)
            .bind(&note)
            .bind(moderator)
            .fetch_optional(&mut *tx)
            .await?;
            let (name, icon) = decided.ok_or_else(|| {
                AppError::precondition("REVIEW_OUTDATED", "the manifest changed or was decided since")
            })?;
            if request.approve
                && let Ok(images) = self.images()
            {
                // The approved copy keeps its icon whatever the draft does next.
                match image(&icon).and_then(|icon| Uuid::parse_str(&icon.asset_id).ok()) {
                    Some(asset_id) => images.hold(&mut tx, asset_id, &live_icon_holder(app_id)).await?,
                    None => images.release(&mut tx, &live_icon_holder(app_id)).await?,
                }
            }
            let (title, body) = if request.approve {
                (
                    Localized::new("ミニアプリが公開されたよ！", "Your mini app is live!"),
                    Localized::new(
                        format!("「{name}」の審査が通りました。チャットから開けます。"),
                        format!("\"{name}\" passed review and can be opened from chats."),
                    ),
                )
            } else {
                (
                    Localized::new("ミニアプリの審査が通りませんでした", "Your mini app was not approved"),
                    Localized::new(format!("「{name}」: {note}"), format!("\"{name}\": {note}")),
                )
            };
            let notice = NotificationRequest::new(
                app.owner_id,
                "dev.mini_app_reviewed",
                title,
                body,
                format!("/developers/apps/{app_id}"),
            )
            .data(serde_json::json!({ "app_id": app_id, "approved": request.approve }));
            outbound::notify(&mut *tx, &notice).await?;
            tx.commit().await?;
            ok(ReviewMiniAppResponse { review: self.review_for_owner(&app).await?.into(), ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_urls_are_https_off_tokuly() {
        let (url, origin) = launch_url(" https://warikan.example/app?x=1 ", false, "tokuly.app").unwrap();
        assert_eq!((url.as_str(), origin.as_str()), ("https://warikan.example/app?x=1", "https://warikan.example"));
        assert_eq!(launch_url("https://a.example:8443/", false, "tokuly.app").unwrap().1, "https://a.example:8443");
        for bad in [
            "",
            "http://warikan.example/",
            "https://user:pw@warikan.example/",
            "https://warikan.example/#start",
            "https://tokuly.app/miniapps",
            "https://evil.tokuly.app/",
            "https://TOKULY.app/",
            "https://203.0.113.5/",
            "javascript:alert(1)",
            "data:text/html,hi",
            "/relative",
            "http://127.0.0.1:5180/",
        ] {
            assert_eq!(launch_url(bad, false, "tokuly.app").unwrap_err().reason, "LAUNCH_URL_INVALID", "{bad}");
        }
        // Development: a loopback address, never a name or another host.
        assert_eq!(launch_url("http://127.0.0.1:5180/", true, "localhost").unwrap().1, "http://127.0.0.1:5180");
        for bad in ["http://localhost:5180/", "http://192.168.1.2/"] {
            assert_eq!(launch_url(bad, true, "localhost").unwrap_err().reason, "LAUNCH_URL_INVALID", "{bad}");
        }
    }

    #[test]
    fn scopes_come_from_the_catalogue_once_each() {
        let asked = ["friends", "profile", "friends"].map(str::to_owned);
        assert_eq!(check_scopes(&asked).unwrap(), ["profile", "friends"]);
        assert_eq!(check_scopes(&["geo:read".to_owned()]).unwrap_err().reason, "MINI_APP_SCOPE_UNKNOWN");
        assert!(check_scopes(&[]).unwrap().is_empty());
        assert_eq!(describe(&["friends".to_owned()]).len(), 1);
    }
}
