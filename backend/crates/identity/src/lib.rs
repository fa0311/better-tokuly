//! Accounts, authentication, sessions, profiles and settings.
//!
//! Other domains depend on this crate for [`users::summaries`] (turning user
//! IDs into `UserSummary`) and [`settings::privacy_of`]. Domains that know
//! more about a user than identity does (friendship, distance, counters)
//! plug into profiles through [`ProfileExtension`].
use std::{collections::HashMap, ops::Deref, sync::Arc};

use async_trait::async_trait;
use parking_lot::RwLock;
use tokuly_core::prelude::*;
use tokuly_proto::identity::{Me, Profile, Relationship};
use webauthn_rs::{Webauthn, WebauthnBuilder};

mod account_svc;
mod auth_svc;
pub mod config;
mod connections;
mod mailer;
pub mod maintenance;
mod moderation;
mod passkeys;
pub mod password;
mod profile_svc;
pub mod sessions;
pub mod settings;
mod totp;
pub mod users;

pub use config::IdentityConfig;
pub use users::{UserRow, summaries, summaries_ordered, summary};

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
    async fn enrich_profile(
        &self,
        _viewer: Option<Uuid>,
        _owner: Uuid,
        _profile: &mut Profile,
    ) -> AppResult<()> {
        Ok(())
    }

    /// Relationship and mutual-friend count between `viewer` and each user.
    /// Users missing from the map are treated as unrelated.
    async fn relationships(
        &self,
        _viewer: Uuid,
        _user_ids: &[Uuid],
    ) -> AppResult<HashMap<Uuid, (Relationship, i32)>> {
        Ok(HashMap::new())
    }

    /// Whether `viewer` may see `owner` at all (false hides the profile, e.g.
    /// when either side blocked the other).
    async fn can_view(&self, _viewer: Uuid, _owner: Uuid) -> AppResult<bool> {
        Ok(true)
    }
}

pub struct IdentityInner {
    pub core: Core,
    pub cfg: IdentityConfig,
    pub(crate) webauthn: Webauthn,
    extensions: RwLock<Vec<Arc<dyn ProfileExtension>>>,
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
        let origin = url::Url::parse(core.cfg.public_url())?;
        let rp_id = core.cfg.public_host();
        let webauthn = WebauthnBuilder::new(&rp_id, &origin)
            .map_err(|e| anyhow::anyhow!("webauthn configuration: {e}"))?
            .rp_name("Tokuly")
            // Development setups reach the app on several localhost ports.
            .allow_any_port(!core.cfg.is_production())
            .build()
            .map_err(|e| anyhow::anyhow!("webauthn configuration: {e}"))?;
        Ok(Self(Arc::new(IdentityInner {
            core,
            cfg,
            webauthn,
            extensions: RwLock::new(Vec::new()),
        })))
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
        use tokuly_proto::identity::{
            AccountServiceExt, AuthServiceExt, ProfileServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = AuthServiceExt::register(me.clone(), router);
        let router = AccountServiceExt::register(me.clone(), router);
        ProfileServiceExt::register(me, router)
    }

    /// The account as its owner sees it.
    pub async fn me(&self, user: &UserRow) -> AppResult<Me> {
        let passkeys: i64 = sqlx::query_scalar("SELECT count(*) FROM passkeys WHERE user_id = $1")
            .bind(user.id)
            .fetch_one(&self.core.db)
            .await?;
        let features = self.core.flags.for_user(user.id).await;
        Ok(user.to_me(passkeys as i32, features))
    }
}
