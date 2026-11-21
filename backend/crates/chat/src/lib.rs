//! Chat (DM・グループ・通話): direct and group conversations, messages,
//! reactions, read state, POV requests, voice/video calls and the optional
//! AI writing assistant.
//!
//! # For other domains
//!
//! - [`Chat::open_direct`] — get or create the direct conversation of two
//!   users for a flow the server itself starts (no privacy checks, no
//!   message request).
//! - [`Chat::post_text`] — post a text message as a member.
//! - [`events`] — the payloads of the `chat.*` domain events.
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`access`] | membership, visibility and "may this be sent" rules |
//! | [`content`] | text rules free of I/O: limits, mentions, links, emoji |
//! | [`model`] | database rows and the numbers stored in them |
//! | [`send`] | the message pipeline every new message goes through |
//! | [`render`] | stored messages → API messages, per viewer |
//! | [`conversations`], [`members`], [`state`] | conversations, group membership, each member's private state |
//! | [`messages`], [`reactions`], [`links`] | editing, deleting, paging, search, forwards; reactions and pins; link previews |
//! | [`signals`] | typing and co-presence from the realtime gateway |
//! | [`pov`] | POV requests |
//! | [`calls`], [`livekit`] | calls and the LiveKit access tokens / API |
//! | [`assist`] | the AI assistant (Anthropic Messages API) |
//! | [`push`], [`notices`] | realtime pushes; notification requests and their wording |
//! | [`cleanup`], [`subjects`], [`workers`] | other domains' events; subject resolver and data export; background work |
//! | `*_svc` | the five Connect services: thin handlers over the modules above |
use std::{ops::Deref, sync::Arc};

use tokuly_core::Core;
use tokuly_identity::Identity;

pub mod config;
pub mod content;
pub mod events;
pub mod livekit;

mod access;
mod assist;
mod calls;
mod cleanup;
mod conversation_svc;
mod conversations;
mod extra_svc;
mod links;
mod members;
mod message_svc;
mod messages;
mod model;
mod notices;
mod pov;
mod push;
mod reactions;
mod render;
mod send;
mod signals;
mod state;
mod subjects;
mod workers;

pub use calls::{CONNECT_GRACE_SECONDS, MAX_RINGING_GROUP_MEMBERS};
pub use config::ChatConfig;
pub use send::{JOB_LINK_PREVIEW, LinkPreviewJob};
pub use signals::PRESENCE_TTL_SECONDS;

pub struct ChatInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: ChatConfig,
    /// `None` when calls are not configured.
    pub(crate) livekit: Option<livekit::LiveKit>,
    /// `None` when the AI assistant is not configured.
    pub(crate) assist: Option<assist::Assistant>,
}

/// The chat domain. Cheap to clone.
#[derive(Clone)]
pub struct Chat(Arc<ChatInner>);

impl Deref for Chat {
    type Target = ChatInner;

    fn deref(&self) -> &ChatInner {
        &self.0
    }
}

impl Chat {
    /// Builds the domain and plugs it into the generic ones: chat messages
    /// become reportable subjects and personal data exports gain a `chat`
    /// section.
    pub fn new(core: Core, identity: Identity, cfg: ChatConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let livekit = livekit::LiveKit::from_config(&cfg)?;
        let assist = assist::Assistant::from_config(&cfg)?;
        subjects::register(&core);
        Ok(Self(Arc::new(ChatInner { core, identity, cfg, livekit, assist })))
    }

    /// Registers the five chat services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::chat::{
            AssistServiceExt, CallServiceExt, ConversationServiceExt, MessageServiceExt, PovServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = ConversationServiceExt::register(me.clone(), router);
        let router = MessageServiceExt::register(me.clone(), router);
        let router = PovServiceExt::register(me.clone(), router);
        let router = CallServiceExt::register(me.clone(), router);
        AssistServiceExt::register(me, router)
    }

    /// The LiveKit deployment calls run on, when configured. For tooling
    /// and tests that talk to LiveKit with the server's credentials.
    pub fn livekit(&self) -> Option<&livekit::LiveKit> {
        self.livekit.as_ref()
    }
}
