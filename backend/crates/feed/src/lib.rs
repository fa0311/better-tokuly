//! Feed: the activity log ("ログ"), map stories, the home feed, personalized
//! news and recaps.
//!
//! Other domains never call this crate. They describe what a user did with
//! `tokuly_core::outbound::activity` inside their own transaction, and the
//! consumer here stores the entry once:
//!
//! ```text
//! activity.recorded ─▶ consumer ─▶ activities ─┬─▶ my log / profile list   (activity_svc.rs)
//! identity.user.created ("joined")             ├─▶ friends' feed, cards    (cards.rs)
//!                                              ├─▶ map stories             (stories.rs)
//!                                              ├─▶ home feed               (home_svc.rs)
//!                                              └─▶ recaps                  (recap.rs)
//! ```
//!
//! Every one of those is a read of the same table that decides what the
//! reader may see when they ask (audience, the author's hidden kinds, blocks
//! — see [`activity`]). Nothing is fanned out to followers, so a friendship
//! that ends or a block needs no clean-up.
//!
//! News is independent of activities: RSS/Atom sources are fetched on a
//! schedule and ranked per reader from what they open (see [`news`]).
//!
//! Layout:
//! - [`activity`]: entry storage, ingestion, conversion to API messages;
//!   [`kinds`]: kind names, labels and card types.
//! - `activity_svc`, `stories`, `home_svc`, `recap`, `news::svc`,
//!   `news::sources`: the six RPC services.
//! - [`maintenance`]: the event consumer, workers, retention, account
//!   deletion and data export.
//! - [`withdrawals`]: entries about content that was deleted, narrowed or
//!   expired since they were stored.
use std::{ops::Deref, sync::Arc, time::Duration};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;
use tokuly_proto::common::SubjectType;

pub mod activity;
mod activity_svc;
mod cards;
pub mod config;
pub mod events;
mod home_svc;
pub mod kinds;
pub mod maintenance;
pub mod news;
pub mod recap;
mod stories;
pub mod withdrawals;

pub use activity::IngestOutcome;
pub use config::FeedConfig;
pub use maintenance::RetentionReport;
pub use news::refresh::RefreshOutcome;

use tokuly_core::egress::Policy;

use crate::news::tagging::TagDictionary;

pub struct FeedInner {
    pub core: Core,
    pub cfg: FeedConfig,
    /// What the news fetcher may reach.
    pub(crate) news_policy: Policy,
    /// HTTP client of the news fetcher; resolves names through the SSRF guard.
    pub(crate) news_http: reqwest::Client,
    /// Keywords that tag news articles.
    pub(crate) dictionary: TagDictionary,
}

/// The feed domain. Cheap to clone.
#[derive(Clone)]
pub struct Feed(Arc<FeedInner>);

impl Deref for Feed {
    type Target = FeedInner;

    fn deref(&self) -> &FeedInner {
        &self.0
    }
}

impl Feed {
    /// Builds the domain: checks the configuration, sets up the guarded
    /// news fetcher, and registers the `feed` section of the personal data
    /// export and the subject resolver for news articles (so they can be
    /// commented on, liked and reported).
    ///
    /// `_identity` is unused: user summaries come from
    /// `tokuly_identity::summaries`, which only needs the database. The
    /// parameter keeps the constructor shape every domain shares.
    pub fn new(core: Core, _identity: Identity, cfg: FeedConfig) -> anyhow::Result<Self> {
        cfg.validate(core.cfg.is_production())?;
        let news_policy = Policy { allow_loopback: cfg.news_allow_loopback };
        let news_http = news::fetch::guarded_client(news_policy, Duration::from_secs(cfg.news_fetch_timeout_seconds))?;
        let dictionary = TagDictionary::from_config(&cfg.news_tag_keywords)?;

        core.exporters.register(Arc::new(maintenance::FeedExporter { core: core.clone() }));
        core.subjects.register(
            SubjectType::SUBJECT_TYPE_NEWS_ARTICLE,
            Arc::new(news::articles::NewsSubjects { core: core.clone() }),
        );
        Ok(Self(Arc::new(FeedInner { core, cfg, news_policy, news_http, dictionary })))
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::feed::{
            ActivityServiceExt, HomeFeedServiceExt, MapStoryServiceExt, NewsAdminServiceExt, NewsServiceExt,
            RecapServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = ActivityServiceExt::register(me.clone(), router);
        let router = MapStoryServiceExt::register(me.clone(), router);
        let router = HomeFeedServiceExt::register(me.clone(), router);
        let router = NewsServiceExt::register(me.clone(), router);
        let router = NewsAdminServiceExt::register(me.clone(), router);
        RecapServiceExt::register(me, router)
    }
}
