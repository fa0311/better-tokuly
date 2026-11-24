//! Notifications: the inbox, push delivery and preferences.
//!
//! Other domains never call this crate. They describe a notification with
//! `tokuly_core::outbound::notify` inside their own transaction, and the
//! consumer here ([`consumer`]) picks the request up from the event stream:
//!
//! ```text
//! notify.requested ─▶ consumer ─┬─▶ inbox entry (inbox.rs)        ─▶ realtime NotifyEvent
//!                               └─▶ notify.push job (delivery.rs) ─▶ Web Push / FCM / APNs (push/)
//! ```
//!
//! Entries about content that is deleted or taken down are withdrawn
//! ([`withdraw`]).
//!
//! What a user receives is governed by their preferences ([`preferences`]):
//! per-category toggles, a global push pause, quiet hours and per-person
//! mutes. Push tokens are registered per session ([`devices`]).
use std::{collections::HashMap, ops::Deref, sync::Arc};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;

pub mod categories;
pub mod config;
pub mod consumer;
pub mod delivery;
pub mod devices;
pub mod events;
pub mod inbox;
mod inbox_svc;
pub mod maintenance;
pub mod preferences;
mod preferences_svc;
pub mod push;
mod push_svc;
pub mod sanitize;
pub mod withdraw;

pub use config::NotifyConfig;
pub use push_svc::TEST_KIND;

use crate::push::{
    PushChannel,
    apns::Apns,
    fcm::Fcm,
    vapid::{self, Vapid},
    webpush::{EndpointPolicy, WebPush},
};

pub struct NotifyInner {
    pub core: Core,
    pub cfg: NotifyConfig,
    /// The Web Push application-server key.
    pub(crate) vapid: Arc<Vapid>,
    /// Which Web Push endpoints may be registered and sent to.
    pub(crate) endpoint_policy: EndpointPolicy,
    /// Configured push channels by `PushChannelKind` number. Web Push is
    /// always present; FCM and APNs only when they have credentials.
    pub(crate) channels: HashMap<i32, Arc<dyn PushChannel>>,
}

/// The notify domain. Cheap to clone.
#[derive(Clone)]
pub struct Notify(Arc<NotifyInner>);

impl Deref for Notify {
    type Target = NotifyInner;

    fn deref(&self) -> &NotifyInner {
        &self.0
    }
}

impl Notify {
    /// `_identity` is unused: actor summaries come from
    /// `tokuly_identity::summaries`, which only needs the database. The
    /// parameter keeps the constructor shape every domain shares.
    pub fn new(core: Core, _identity: Identity, cfg: NotifyConfig) -> anyhow::Result<Self> {
        cfg.validate(core.cfg.is_production())?;
        let http = push::http_client()?;

        let vapid_secret = if cfg.vapid_private_key.trim().is_empty() {
            vapid::derive_secret(&core.keys)
        } else {
            vapid::parse_secret(&cfg.vapid_private_key)?
        };
        let vapid_subject = vapid::subject_from(
            &cfg.vapid_subject,
            core.cfg.public_url(),
            &core.cfg.mail_from,
        );
        let vapid = Arc::new(Vapid::new(&vapid_secret, vapid_subject)?);
        let endpoint_policy = EndpointPolicy::from_config(&cfg);

        let mut channels: Vec<Arc<dyn PushChannel>> = vec![Arc::new(WebPush::new(
            http.clone(),
            vapid.clone(),
            endpoint_policy.clone(),
        ))];
        if cfg.fcm_enabled() {
            channels.push(Arc::new(Fcm::from_service_account(
                &cfg.fcm_service_account_json,
                http.clone(),
            )?));
        }
        if cfg.apns_enabled() {
            channels.push(Arc::new(Apns::new(
                &cfg.apns_key_p8,
                &cfg.apns_key_id,
                &cfg.apns_team_id,
                &cfg.apns_topic,
                http,
            )?));
        }
        let channels = channels
            .into_iter()
            .map(|channel| (channel.kind() as i32, channel))
            .collect();

        core.exporters
            .register(Arc::new(maintenance::NotifyExporter { core: core.clone() }));
        Ok(Self(Arc::new(NotifyInner {
            core,
            cfg,
            vapid,
            endpoint_policy,
            channels,
        })))
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::notify::{InboxServiceExt, PreferencesServiceExt, PushServiceExt};
        let me = Arc::new(self.clone());
        let router = InboxServiceExt::register(me.clone(), router);
        let router = PreferencesServiceExt::register(me.clone(), router);
        PushServiceExt::register(me, router)
    }
}
