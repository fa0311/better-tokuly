//! Accounts, authentication, sessions, profiles and settings.
//!
//! Other domains depend on this crate for [`users::summaries`] (turning user
//! IDs into `UserSummary`) and [`settings::privacy_of`]. Domains that know
//! more about a user than identity does (friendship, distance, counters)
//! plug into profiles through [`ProfileExtension`].
//!
//! Modules:
//! - `auth_svc`, `account_svc`, `profile_svc` — the three services' handlers;
//! - [`users`] — account rows, summaries, handle rules and validation;
//! - [`sessions`] — sessions and refresh-token rotation; [`device_accounts`]
//!   — several accounts on one device and switching between them (with the
//!   threat model); [`handoff`] — opening the web signed in from an app;
//! - [`extras`] — the プロフ帳 (profile extras, their audiences, prompts,
//!   search terms) and [`link_check`] — verifying profile links by a link back;
//! - [`age`] — who counts as a minor, for every domain; [`birthdays`] — who
//!   sees a birthday;
//! - [`bots`] — which accounts are bots and whose, and [`bots::humans_only`];
//! - [`settings`], `passkeys`, `totp`, `connections` (Spotify), [`moderation`],
//!   [`maintenance`] (workers, purge, export, event payloads).
use std::{collections::HashMap, ops::Deref, sync::Arc};

use async_trait::async_trait;
use parking_lot::RwLock;
use tokuly_core::prelude::*;
use tokuly_proto::identity::{Me, Profile, Relationship};
use webauthn_rs::{Webauthn, WebauthnBuilder};

mod account_svc;
pub mod age;
mod auth_svc;
pub mod birthdays;
pub mod bots;
pub mod config;
mod connections;
pub mod device_accounts;
pub mod handoff;
pub mod extras;
pub mod link_check;
mod mailer;
pub mod maintenance;
pub mod moderation;
mod passkeys;
pub mod password;
mod profile_svc;
pub mod sessions;
pub mod settings;
mod totp;
pub mod users;

pub use config::IdentityConfig;
pub use users::{UserRow, summaries, summaries_ordered, summary};

/// The columns this domain seals with the server secret (`tokuly rekey`).
pub const SEALED: &[tokuly_core::rekey::Sealed] = {
    use tokuly_core::rekey::Sealed;
    &[
        Sealed::text("users", "totp_secret", "totp"),
        Sealed::bytes("sessions", "refresh_sealed", "refresh").hashed("refresh_hash"),
        Sealed::bytes("connections", "access_token", "connection"),
        Sealed::bytes("connections", "refresh_token", "connection"),
    ]
};

/// Lets other domains contribute to profiles and user search results.
#[async_trait]
pub trait ProfileExtension: Send + Sync + 'static {
    /// Fills in the parts of `profile` this domain owns (relationship,
    /// counters, distance...). `viewer` is `None` for anonymous requests.
    ///
    /// A domain without a field of its own on the profile pushes a
    /// `ProfileCard` onto `profile.cards` (kind `"<domain>.<thing>"`, text
    /// in both languages); respect the owner's privacy when deciding
    /// whether `viewer` gets to see it.
    async fn enrich_profile(&self, _viewer: Option<Uuid>, _owner: Uuid, _profile: &mut Profile) -> AppResult<()> {
        Ok(())
    }

    /// Relationship and mutual-friend count between `viewer` and each user.
    /// Users missing from the map are treated as unrelated.
    async fn relationships(&self, _viewer: Uuid, _user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, (Relationship, i32)>> {
        Ok(HashMap::new())
    }

    /// Whether `viewer` may see `owner` at all (false hides the profile, e.g.
    /// when either side blocked the other).
    async fn can_view(&self, _viewer: Uuid, _owner: Uuid) -> AppResult<bool> {
        Ok(true)
    }

    /// What `viewer` (`None`: signed out) is to `owner` for the audiences of
    /// the プロフ帳 — friend, follower or neither, whether `owner` is locked,
    /// in their close friends, in which of `circles` (the owner's circles
    /// something is shared with) — or `None` when this domain does not know
    /// (the social domain's audience API answers it, wired by the server).
    async fn audience_tie(
        &self,
        _viewer: Option<Uuid>,
        _owner: Uuid,
        _circles: &[Uuid],
    ) -> AppResult<Option<extras::AudienceTie>> {
        Ok(None)
    }

    /// Which of `owners` have locked accounts (their public parts reach only
    /// their followers and friends).
    async fn locked_among(&self, _owners: &[Uuid]) -> AppResult<std::collections::HashSet<Uuid>> {
        Ok(std::collections::HashSet::new())
    }

    /// Those among `user_ids` that `viewer` may not see, the same rule as
    /// [`Self::can_view`] for a whole list at once (profile viewers, search
    /// results). The default asks `can_view` per user; a domain that can
    /// answer with one query should override it.
    async fn hidden_among(&self, viewer: Uuid, user_ids: &[Uuid]) -> AppResult<std::collections::HashSet<Uuid>> {
        let mut hidden = std::collections::HashSet::new();
        for &user_id in user_ids {
            if user_id != viewer && !self.can_view(viewer, user_id).await? {
                hidden.insert(user_id);
            }
        }
        Ok(hidden)
    }
}

/// The WebAuthn relying party for the public URL, or `None` when its host is
/// an IP address: a relying party ID must be a domain name, so a server
/// reached by LAN IP (`http://192.168.1.10`) runs without passkeys rather
/// than not at all. Every other misconfiguration still fails start-up.
fn webauthn_for(cfg: &tokuly_core::config::Config) -> anyhow::Result<Option<Webauthn>> {
    let origin = url::Url::parse(cfg.public_url())?;
    if matches!(origin.host(), Some(url::Host::Ipv4(_) | url::Host::Ipv6(_))) {
        return Ok(None);
    }
    let rp_id = cfg.public_host();
    let webauthn = WebauthnBuilder::new(&rp_id, &origin)
        .map_err(|e| anyhow::anyhow!("webauthn configuration: {e}"))?
        .rp_name("Tokuly")
        // Development setups reach the app on several localhost ports.
        .allow_any_port(!cfg.is_production())
        .build()
        .map_err(|e| anyhow::anyhow!("webauthn configuration: {e}"))?;
    Ok(Some(webauthn))
}

pub struct IdentityInner {
    pub core: Core,
    pub cfg: IdentityConfig,
    /// The WebAuthn relying party; `None` when the public URL cannot be one
    /// (see [`webauthn_for`]), and the passkey RPCs answer PASSKEYS_UNAVAILABLE.
    pub(crate) webauthn: Option<Webauthn>,
    extensions: RwLock<Vec<Arc<dyn ProfileExtension>>>,
    /// Fetches in flight per host for the profile link check.
    pub(crate) link_hosts: parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>,
}

/// The identity domain. Cheap to clone.
#[derive(Clone)]
pub struct Identity(Arc<IdentityInner>);

impl Deref for Identity {
    type Target = IdentityInner;

    fn deref(&self) -> &IdentityInner {
        &self.0
    }
}

impl Identity {
    pub fn new(core: Core, cfg: IdentityConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !(core.cfg.is_production() && cfg.profile_links_allow_loopback),
            "TOKULY_PROFILE_LINKS_ALLOW_LOOPBACK must not be set in production"
        );
        let webauthn = webauthn_for(&core.cfg)?;
        if webauthn.is_none() {
            tracing::warn!(
                public_url = core.cfg.public_url(),
                "passkeys are disabled: the public URL is an IP address, and WebAuthn needs a domain name \
                 (localhost works); set TOKULY_PUBLIC_URL to a domain to enable them"
            );
        }
        Ok(Self(Arc::new(IdentityInner {
            core,
            cfg,
            webauthn,
            extensions: RwLock::new(Vec::new()),
            link_hosts: parking_lot::Mutex::new(HashMap::new()),
        })))
    }

    /// Whether passkeys can be registered and used on this server.
    pub fn passkeys_available(&self) -> bool {
        self.webauthn.is_some()
    }

    /// Registers a domain that enriches profiles. Call once at startup.
    pub fn add_profile_extension(&self, extension: Arc<dyn ProfileExtension>) {
        self.extensions.write().push(extension);
    }

    pub(crate) fn profile_extensions(&self) -> Vec<Arc<dyn ProfileExtension>> {
        self.extensions.read().clone()
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::identity::{AccountServiceExt, AuthServiceExt, ProfileServiceExt};
        let me = Arc::new(self.clone());
        let router = AuthServiceExt::register(me.clone(), router);
        let router = AccountServiceExt::register(me.clone(), router);
        ProfileServiceExt::register(me, router)
    }

    /// The account as its owner sees it: everything the settings screens
    /// need, the プロフ帳 with its audiences included.
    pub async fn me(&self, user: &UserRow) -> AppResult<Me> {
        let passkeys: i64 = sqlx::query_scalar("SELECT count(*) FROM passkeys WHERE user_id = $1")
            .bind(user.id)
            .fetch_one(&self.core.db)
            .await?;
        let features = self.core.flags.for_user(user.id).await;
        let mut me = user.to_me(passkeys as i32, features);
        me.minor = age::is_minor(&self.core, user.id).await?;
        me.too_young = age::too_young_on(user.birthdate, chrono::Utc::now().date_naive(), self.cfg.min_age);
        me.extras = self.own_extras(user.id, user.birthdate).await?.into();
        if let Some(owner) = user.bot_owner_id {
            me.bot_owner = users::summary(&self.core, owner).await?.into();
        }
        Ok(me)
    }
}
