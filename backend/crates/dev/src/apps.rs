//! OAuth applications (`dev_apps`) and `AppService`, the developer's side.
use std::time::Duration;

use chrono::Utc;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{api_tokens::Scopes, events, keys::secure_eq, outbound, prelude::*};
use tokuly_proto::{
    common::{Image, SubjectType},
    dev::*,
};

use crate::{
    Dev, clean_name, clean_text,
    events::AppEvent,
    images::{ImageChange, app_icon_holder},
    lock_limit, scopes,
    secrets::{self, SecretKind},
};

const MAX_PAGE_SIZE: i64 = 100;
const MAX_REDIRECT_URIS: usize = 10;
const MAX_URL_LEN: usize = 2000;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct AppRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub description: String,
    pub icon: Option<serde_json::Value>,
    pub homepage_url: String,
    pub privacy_policy_url: String,
    pub redirect_uris: Vec<String>,
    pub scopes: Vec<String>,
    pub confidential: bool,
    pub device_flow: bool,
    pub secret_hash: Option<String>,
    pub secret_hint: String,
    pub previous_secret_hash: Option<String>,
    pub previous_secret_expires_at: Option<Time>,
    pub status: String,
    pub disabled_reason: String,
    pub created_at: Time,
    pub updated_at: Time,
    /// Users with an authorization.
    pub user_count: i64,
    /// Whether the owner's account is in good standing.
    pub owner_active: bool,
}

macro_rules! app_columns {
    () => {
        "p.id, p.owner_id, p.name, p.description, p.icon, p.homepage_url, p.privacy_policy_url, p.redirect_uris, p.scopes, p.confidential, p.device_flow, \
         p.secret_hash, p.secret_hint, p.previous_secret_hash, p.previous_secret_expires_at, p.status, p.disabled_reason, p.created_at, p.updated_at, \
         (SELECT count(*) FROM dev_authorizations a WHERE a.app_id = p.id) AS user_count, \
         COALESCE((SELECT u.banned_at IS NULL AND u.deleted_at IS NULL FROM users u WHERE u.id = p.owner_id), false) AS owner_active"
    };
}
pub(crate) use app_columns;

fn image(icon: &Option<serde_json::Value>) -> Option<Image> {
    icon.as_ref().and_then(|json| serde_json::from_value(json.clone()).ok())
}

impl AppRow {
    /// Not taken down by moderation.
    pub(crate) fn is_active(&self) -> bool {
        self.status == "active"
    }

    /// Whether the application may be used at all: not taken down, and its
    /// owner neither suspended nor gone. Everything users and clients see
    /// treats an application that is not usable as one that does not exist.
    pub(crate) fn is_usable(&self) -> bool {
        self.is_active() && self.owner_active
    }

    pub(crate) fn scopes(&self) -> Scopes {
        Scopes::new(self.scopes.iter().cloned())
    }

    /// Whether `secret` is the application's client secret (the current
    /// one, or the previous one while it is still honoured).
    pub(crate) fn secret_matches(&self, dev: &Dev, secret: &str) -> bool {
        if !secrets::is(secret, SecretKind::ClientSecret) {
            return false;
        }
        let presented = dev.core.keys.token_hash(secret);
        let matches = |stored: &Option<String>| {
            stored.as_ref().is_some_and(|stored| secure_eq(stored.as_bytes(), presented.as_bytes()))
        };
        matches(&self.secret_hash)
            || (self.previous_secret_expires_at.is_some_and(|at| at > Utc::now())
                && matches(&self.previous_secret_hash))
    }

    fn to_proto(&self) -> App {
        App {
            id: self.id.to_string(),
            name: self.name.clone(),
            description: self.description.clone(),
            icon: image(&self.icon).into(),
            homepage_url: self.homepage_url.clone(),
            privacy_policy_url: self.privacy_policy_url.clone(),
            redirect_uris: self.redirect_uris.clone(),
            scopes: self.scopes.clone(),
            client_type: if self.confidential {
                AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL
            } else {
                AppClientType::APP_CLIENT_TYPE_PUBLIC
            }
            .into(),
            client_secret_hint: self.secret_hint.clone(),
            status: if self.is_active() { AppStatus::APP_STATUS_ACTIVE } else { AppStatus::APP_STATUS_DISABLED }.into(),
            disabled_reason: self.disabled_reason.clone(),
            user_count: self.user_count.min(i32::MAX as i64) as i32,
            created_at: ts(self.created_at),
            updated_at: ts(self.updated_at),
            previous_secret_expires_at: opt_ts(self.previous_secret_expires_at.filter(|at| *at > Utc::now())),
            device_flow: self.device_flow,
            ..Default::default()
        }
    }

    pub(crate) fn to_public(&self, owner: tokuly_proto::common::UserSummary) -> PublicApp {
        PublicApp {
            id: self.id.to_string(),
            name: self.name.clone(),
            description: self.description.clone(),
            icon: image(&self.icon).into(),
            homepage_url: self.homepage_url.clone(),
            privacy_policy_url: self.privacy_policy_url.clone(),
            owner: owner.into(),
            created_at: ts(self.created_at),
            user_count: self.user_count.min(i32::MAX as i64) as i32,
            ..Default::default()
        }
    }
}

/// An `https` URL for an application's pages, or empty.
fn page_url(input: &str) -> AppResult<String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(String::new());
    }
    let ok = input.len() <= MAX_URL_LEN
        && url::Url::parse(input).is_ok_and(|url| {
            url.scheme() == "https" && url.host_str().is_some() && url.username().is_empty() && url.password().is_none()
        });
    ensure!(ok, invalid, "URL_INVALID", "{input:?} is not an https URL");
    Ok(input.to_owned())
}

/// Whether a host is a loopback address literal (not `localhost`, which a
/// machine may resolve elsewhere: RFC 8252 section 8.3).
fn is_loopback_literal(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    }
}

/// Checks a redirect URI at registration: `https`; `http` on a loopback
/// address or `localhost` (native apps and development); or, for public
/// clients, a private-use scheme in reverse-domain form
/// (`com.example.app:/callback`, RFC 8252 section 7.1). Never a fragment.
pub(crate) fn check_redirect_uri(uri: &str, confidential: bool) -> AppResult<()> {
    let invalid = |why: &str| AppError::invalid("REDIRECT_URI_INVALID", format!("redirect URI {uri:?} {why}"));
    if uri.len() > MAX_URL_LEN || uri.trim() != uri {
        return Err(invalid("is too long or has surrounding spaces"));
    }
    let url = url::Url::parse(uri).map_err(|_| invalid("is not an absolute URI"))?;
    if url.fragment().is_some() {
        return Err(invalid("must not have a fragment"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("must not contain credentials"));
    }
    match url.scheme() {
        "https" if url.host_str().is_some() => Ok(()),
        "http" if is_loopback_literal(&url) || url.host_str() == Some("localhost") => Ok(()),
        "http" => Err(invalid("must use https (http is only for loopback addresses)")),
        scheme if !confidential && scheme.contains('.') && !url.has_host() => Ok(()),
        _ => Err(invalid("must be https, a loopback http address, or (public clients) a reverse-domain app scheme")),
    }
}

/// Whether the `redirect_uri` of an authorization request is one of the
/// registered ones: an exact string match, except that the port of a
/// loopback address literal is free (RFC 8252 section 7.3).
pub(crate) fn redirect_uri_matches(registered: &[String], presented: &str) -> bool {
    if registered.iter().any(|uri| uri == presented) {
        return true;
    }
    let Ok(mut presented_url) = url::Url::parse(presented) else { return false };
    if presented_url.scheme() != "http" || !is_loopback_literal(&presented_url) || presented_url.set_port(None).is_err()
    {
        return false;
    }
    registered.iter().any(|uri| {
        url::Url::parse(uri).is_ok_and(|mut url| {
            url.scheme() == "http" && is_loopback_literal(&url) && url.set_port(None).is_ok() && url == presented_url
        })
    })
}

/// The validated, editable part of an application.
struct AppFields {
    name: String,
    description: String,
    homepage_url: String,
    privacy_policy_url: String,
    redirect_uris: Vec<String>,
    scopes: Scopes,
}

impl AppFields {
    fn check(
        name: &str,
        description: &str,
        homepage_url: &str,
        privacy_policy_url: &str,
        redirect_uris: &[String],
        scopes: &[String],
        confidential: bool,
    ) -> AppResult<Self> {
        let name = clean_name(name, 60, "APP_NAME_INVALID", "the application name")?;
        let description = clean_text(description, 500, "DESCRIPTION_TOO_LONG", "the description")?;
        ensure!(!redirect_uris.is_empty(), invalid, "REDIRECT_URI_REQUIRED", "register at least one redirect URI");
        ensure!(
            redirect_uris.len() <= MAX_REDIRECT_URIS,
            invalid,
            "TOO_MANY_REDIRECT_URIS",
            "at most {MAX_REDIRECT_URIS} redirect URIs"
        );
        for uri in redirect_uris {
            check_redirect_uri(uri, confidential)?;
        }
        let mut redirect_uris = redirect_uris.to_vec();
        redirect_uris.dedup();
        Ok(Self {
            name,
            description,
            homepage_url: page_url(homepage_url)?,
            privacy_policy_url: page_url(privacy_policy_url)?,
            redirect_uris,
            scopes: scopes::validate(scopes)?,
        })
    }
}

impl Dev {
    pub(crate) async fn find_app(&self, app_id: Uuid) -> AppResult<Option<AppRow>> {
        Ok(sqlx::query_as(concat!("SELECT ", app_columns!(), " FROM dev_apps p WHERE p.id = $1"))
            .bind(app_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    /// An application `owner` registered.
    pub(crate) async fn owned_app(&self, app_id: Uuid, owner: Uuid) -> AppResult<AppRow> {
        self.find_app(app_id)
            .await?
            .filter(|app| app.owner_id == owner)
            .or_not_found("APP_NOT_FOUND", "application not found")
    }

    /// The public card of an application that is not disabled.
    pub(crate) async fn find_public_app(&self, app_id: Uuid) -> AppResult<Option<PublicApp>> {
        let Some(app) = self.find_app(app_id).await?.filter(AppRow::is_usable) else { return Ok(None) };
        let owner = tokuly_identity::summary(&self.core, app.owner_id).await?;
        Ok(Some(app.to_public(owner)))
    }

    /// Public cards of several applications, by ID.
    pub(crate) async fn public_apps(&self, app_ids: &[Uuid]) -> AppResult<std::collections::HashMap<Uuid, PublicApp>> {
        let rows: Vec<AppRow> =
            sqlx::query_as(concat!("SELECT ", app_columns!(), " FROM dev_apps p WHERE p.id = ANY($1)"))
                .bind(app_ids)
                .fetch_all(&self.core.db)
                .await?;
        let owners: Vec<Uuid> = rows.iter().map(|app| app.owner_id).collect();
        let owners = tokuly_identity::summaries(&self.core, &owners).await?;
        Ok(rows
            .into_iter()
            .map(|app| (app.id, app.to_public(tokuly_identity::users::pick(&owners, app.owner_id))))
            .collect())
    }

    /// Removes an application with everything issued under it.
    pub(crate) async fn delete_app(&self, app: &AppRow, actor: Option<Uuid>) -> AppResult<()> {
        self.forget_app_tokens(app.id).await?;
        let mut tx = self.core.db.begin().await?;
        if let Ok(images) = self.images() {
            images.release(&mut tx, &app_icon_holder(app.id)).await?;
        }
        // Authorizations and tokens go with the row.
        sqlx::query("DELETE FROM dev_apps WHERE id = $1").bind(app.id).execute(&mut *tx).await?;
        events::emit(&mut *tx, "dev.app.deleted", actor, &AppEvent { app_id: app.id, owner_id: app.owner_id }).await?;
        outbound::subject_deleted(&mut *tx, SubjectType::SUBJECT_TYPE_APP, app.id).await?;
        tx.commit().await?;
        Ok(())
    }
}

impl AppService for Dev {
    async fn create_app(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateAppRequest>,
    ) -> ServiceResult<CreateAppResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let req = request.to_owned_message();
            let confidential = match req.client_type.as_known() {
                Some(AppClientType::APP_CLIENT_TYPE_CONFIDENTIAL) => true,
                Some(AppClientType::APP_CLIENT_TYPE_PUBLIC) => false,
                _ => bail!(invalid, "CLIENT_TYPE_REQUIRED", "say whether the client is confidential or public"),
            };
            let fields = AppFields::check(&req.name, &req.description, &req.homepage_url, &req.privacy_policy_url, &req.redirect_uris, &req.scopes, confidential)?;
            self.core.cache.rate_limit(&format!("dev-app-create:{owner}"), 10, Duration::from_secs(3600)).await?;
            // Looked up before the transaction: it reads through the pool.
            let icon = ImageChange::resolve(self, owner, Some(req.icon_asset_id.as_str()).filter(|id| !id.is_empty())).await?;
            let secret = confidential.then(|| secrets::generate(SecretKind::ClientSecret));
            let mut tx = self.core.db.begin().await?;
            lock_limit(&mut tx, "dev-apps", owner).await?;
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM dev_apps WHERE owner_id = $1").bind(owner).fetch_one(&mut *tx).await?;
            ensure!(count < self.cfg.max_apps, precondition, "TOO_MANY_APPS", "at most {} applications", self.cfg.max_apps);
            let app_id: Uuid = sqlx::query_scalar(
                "INSERT INTO dev_apps (owner_id, name, description, homepage_url, privacy_policy_url, redirect_uris, scopes, confidential, secret_hash, secret_hint, device_flow)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING id",
            )
            .bind(owner)
            .bind(&fields.name)
            .bind(&fields.description)
            .bind(&fields.homepage_url)
            .bind(&fields.privacy_policy_url)
            .bind(&fields.redirect_uris)
            .bind(fields.scopes.as_slice())
            .bind(confidential)
            .bind(secret.as_deref().map(|secret| self.core.keys.token_hash(secret)))
            .bind(secret.as_deref().map(secrets::hint).unwrap_or_default())
            .bind(req.device_flow)
            .fetch_one(&mut *tx)
            .await?;
            if let Some(icon) = icon.apply(self, &mut tx, &app_icon_holder(app_id)).await? {
                sqlx::query("UPDATE dev_apps SET icon = $2 WHERE id = $1").bind(app_id).bind(icon).execute(&mut *tx).await?;
            }
            events::emit(&mut *tx, "dev.app.created", Some(owner), &AppEvent { app_id, owner_id: owner }).await?;
            tx.commit().await?;
            let app = self.owned_app(app_id, owner).await?;
            ok(CreateAppResponse { app: app.to_proto().into(), client_secret: secret.unwrap_or_default(), ..Default::default() })
        })
        .await
    }

    async fn update_app(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateAppRequest>,
    ) -> ServiceResult<UpdateAppResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let req = request.to_owned_message();
            let app = self.owned_app(parse_id(&req.app_id)?, owner).await?;
            let fields = AppFields::check(&req.name, &req.description, &req.homepage_url, &req.privacy_policy_url, &req.redirect_uris, &req.scopes, app.confidential)?;
            self.core.cache.rate_limit(&format!("dev-app-update:{owner}"), 60, Duration::from_secs(3600)).await?;
            let icon = ImageChange::resolve(self, owner, req.icon_asset_id.as_deref()).await?;
            let mut tx = self.core.db.begin().await?;
            sqlx::query("UPDATE dev_apps SET name = $2, description = $3, homepage_url = $4, privacy_policy_url = $5, redirect_uris = $6, scopes = $7, device_flow = $8 WHERE id = $1")
                .bind(app.id)
                .bind(&fields.name)
                .bind(&fields.description)
                .bind(&fields.homepage_url)
                .bind(&fields.privacy_policy_url)
                .bind(&fields.redirect_uris)
                .bind(fields.scopes.as_slice())
                .bind(req.device_flow)
                .execute(&mut *tx)
                .await?;
            if let Some(icon) = icon.apply(self, &mut tx, &app_icon_holder(app.id)).await? {
                sqlx::query("UPDATE dev_apps SET icon = $2 WHERE id = $1").bind(app.id).bind(icon).execute(&mut *tx).await?;
            }
            events::emit(&mut *tx, "dev.app.updated", Some(owner), &AppEvent { app_id: app.id, owner_id: owner }).await?;
            tx.commit().await?;
            ok(UpdateAppResponse { app: self.owned_app(app.id, owner).await?.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn get_app(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetAppRequest>,
    ) -> ServiceResult<GetAppResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let app = self.owned_app(parse_id(request.app_id)?, owner).await?;
            ok(GetAppResponse { app: app.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn list_apps(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListAppsRequest>,
    ) -> ServiceResult<ListAppsResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE_SIZE);
            let after: Option<Uuid> = page.decode()?;
            let mut rows: Vec<AppRow> = sqlx::query_as(concat!(
                "SELECT ",
                app_columns!(),
                " FROM dev_apps p WHERE p.owner_id = $1 AND ($2::uuid IS NULL OR p.id < $2) ORDER BY p.id DESC LIMIT $3"
            ))
            .bind(owner)
            .bind(after)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| row.id);
            ok(ListAppsResponse {
                apps: rows.iter().map(AppRow::to_proto).collect(),
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_app(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteAppRequest>,
    ) -> ServiceResult<DeleteAppResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let app = self.owned_app(parse_id(request.app_id)?, owner).await?;
            // What moderation took down stays on record: its owner cannot make it vanish.
            ensure!(
                app.is_active(),
                precondition,
                "APP_DISABLED",
                "an application disabled by moderation cannot be deleted"
            );
            self.delete_app(&app, Some(owner)).await?;
            ok(DeleteAppResponse::default())
        })
        .await
    }

    async fn rotate_client_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RotateClientSecretRequest>,
    ) -> ServiceResult<RotateClientSecretResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let app = self.owned_app(parse_id(request.app_id)?, owner).await?;
            ensure!(app.confidential, precondition, "CLIENT_IS_PUBLIC", "a public client has no secret");
            self.core.cache.rate_limit(&format!("dev-secret-rotate:{}", app.id), 10, Duration::from_secs(3600)).await?;
            let secret = secrets::generate(SecretKind::ClientSecret);
            // The secret being replaced stays good for the overlap, unless it leaked.
            let overlap = (!request.revoke_previous).then(|| Utc::now() + self.cfg.secret_overlap());
            sqlx::query(
                "UPDATE dev_apps SET previous_secret_hash = CASE WHEN $4::timestamptz IS NULL THEN NULL ELSE secret_hash END,
                        previous_secret_expires_at = $4, secret_hash = $2, secret_hint = $3 WHERE id = $1",
            )
            .bind(app.id)
            .bind(self.core.keys.token_hash(&secret))
            .bind(secrets::hint(&secret))
            .bind(overlap)
            .execute(&self.core.db)
            .await?;
            let attrs = serde_json::json!({ "app_id": app.id, "name": app.name });
            self.identity.log_security_event(owner, "APP_SECRET_ROTATED", &ctx.client(), "", attrs).await;
            ok(RotateClientSecretResponse { app: self.owned_app(app.id, owner).await?.to_proto().into(), client_secret: secret, ..Default::default() })
        })
        .await
    }

    async fn list_app_users(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListAppUsersRequest>,
    ) -> ServiceResult<ListAppUsersResponse> {
        handle(async {
            let owner = ctx.require_session()?.user_id;
            let app = self.owned_app(parse_id(request.app_id)?, owner).await?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE_SIZE);
            // Most recently active first; never-used authorizations last.
            let after: Option<(Option<Time>, Uuid)> = page.decode()?;
            let (after_used, after_id) = after.unzip();
            let mut rows: Vec<(Uuid, Uuid, Vec<String>, Time, Option<Time>)> = sqlx::query_as(
                "SELECT id, user_id, scopes, created_at, last_used_at FROM dev_authorizations
                 WHERE app_id = $1
                   AND ($3::uuid IS NULL
                        OR (last_used_at IS NOT NULL AND $2::timestamptz IS NOT NULL AND (last_used_at, id) < ($2, $3))
                        OR (last_used_at IS NULL AND ($2::timestamptz IS NOT NULL OR id < $3)))
                 ORDER BY last_used_at DESC NULLS LAST, id DESC LIMIT $4",
            )
            .bind(app.id)
            .bind(after_used.flatten())
            .bind(after_id)
            .bind(page.fetch())
            .fetch_all(&self.core.db)
            .await?;
            let info = page.finish(&mut rows, |row| (row.4, row.0));
            let user_ids: Vec<Uuid> = rows.iter().map(|row| row.1).collect();
            let users = tokuly_identity::summaries(&self.core, &user_ids).await?;
            let users = rows
                .into_iter()
                .map(|(_, user_id, scopes, authorized_at, last_used_at)| AppUser {
                    user: tokuly_identity::users::pick(&users, user_id).into(),
                    scopes,
                    authorized_at: ts(authorized_at),
                    last_used_at: opt_ts(last_used_at),
                    ..Default::default()
                })
                .collect();
            ok(ListAppUsersResponse { users, page: info.into(), ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_uris_are_vetted_at_registration() {
        for (uri, confidential) in [
            ("https://app.example/callback", true),
            ("https://app.example/callback?x=1", true),
            ("http://127.0.0.1/callback", true),
            ("http://127.0.0.1:8912/callback", false),
            ("http://[::1]/cb", false),
            ("http://localhost:3000/callback", true),
            ("com.example.app:/oauth", false),
        ] {
            assert!(check_redirect_uri(uri, confidential).is_ok(), "{uri}");
        }
        for (uri, confidential) in [
            ("http://app.example/callback", true),
            ("https://app.example/callback#frag", true),
            ("https://user:pw@app.example/cb", true),
            ("/relative", true),
            ("javascript:alert(1)", false),
            ("data:text/html,x", false),
            ("myapp://callback", false),
            // A private-use scheme cannot be bound to a confidential client.
            ("com.example.app:/oauth", true),
            (" https://app.example/cb", true),
            ("http://localhost.evil.example/cb", true),
        ] {
            assert_eq!(check_redirect_uri(uri, confidential).unwrap_err().reason, "REDIRECT_URI_INVALID", "{uri}");
        }
    }

    #[test]
    fn redirect_uris_match_exactly_except_for_loopback_ports() {
        let registered = vec![
            "https://app.example/callback".to_owned(),
            "http://127.0.0.1/cb".to_owned(),
            "http://localhost:3000/cb".to_owned(),
        ];
        assert!(redirect_uri_matches(&registered, "https://app.example/callback"));
        for wrong in [
            "https://app.example/callback/",
            "https://app.example/callback?x=1",
            "https://app.example/Callback",
            "https://app.example:8443/callback",
            "https://app.example.evil.test/callback",
            "https://evil.test/?https://app.example/callback",
            "http://app.example/callback",
            "",
        ] {
            assert!(!redirect_uri_matches(&registered, wrong), "{wrong}");
        }
        // A native app listens on whatever port it got.
        assert!(redirect_uri_matches(&registered, "http://127.0.0.1:51234/cb"));
        assert!(!redirect_uri_matches(&registered, "http://127.0.0.1:51234/other"));
        assert!(!redirect_uri_matches(&registered, "http://127.0.0.2:51234/cb"));
        // `localhost` is a name, not a loopback literal: exact match only.
        assert!(redirect_uri_matches(&registered, "http://localhost:3000/cb"));
        assert!(!redirect_uri_matches(&registered, "http://localhost:4000/cb"));
    }

    #[test]
    fn page_urls_are_https_or_empty() {
        assert_eq!(page_url("  ").unwrap(), "");
        assert_eq!(page_url("https://app.example/privacy").unwrap(), "https://app.example/privacy");
        for bad in ["http://app.example", "ftp://app.example", "app.example", "https://u:p@app.example"] {
            assert_eq!(page_url(bad).unwrap_err().reason, "URL_INVALID", "{bad}");
        }
    }
}
