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
//! - [`Chat::post_notice`] — post a notice from the server into a
//!   conversation (no sender; see [`server_notices`]), and
//!   [`direct_conversations_of`] a user.
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
//! | [`assist`] | the AI assistant: providers (Anthropic, OpenAI, OpenAI-compatible), operator and user settings, consent |
//! | [`push`], [`notices`] | realtime pushes; notification requests and their wording |
//! | [`server_notices`] | notices other domains post into conversations |
//! | [`cleanup`], [`subjects`], [`workers`] | other domains' events; subject resolver and data export; background work |
//! | `*_svc`, `assist::svc` | the Connect services: thin handlers over the modules above |
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
mod server_notices;
mod signals;
mod state;
mod subjects;
mod workers;

pub use calls::{CONNECT_GRACE_SECONDS, MAX_RINGING_GROUP_MEMBERS};
pub use config::ChatConfig;
pub use conversations::direct_members;
pub use send::{JOB_LINK_PREVIEW, LinkPreviewJob};
pub use server_notices::{
    JOB_NOTICES, NOTICE_BATCH, NOTICES_PER_CONVERSATION_PER_DAY, NoticeBatch, direct_conversations_of,
};
pub use signals::PRESENCE_TTL_SECONDS;
pub use subjects::hold_message_media;

pub struct ChatInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: ChatConfig,
    /// `None` when calls are not configured.
    pub(crate) livekit: Option<livekit::LiveKit>,
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
        anyhow::ensure!(
            !(core.cfg.is_production() && cfg.ai_allow_loopback),
            "TOKULY_AI_ALLOW_LOOPBACK must not be set in production"
        );
        subjects::register(&core);
        Ok(Self(Arc::new(ChatInner { core, identity, cfg, livekit })))
    }

    /// Registers the chat services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::chat::{
            AssistAdminServiceExt, AssistServiceExt, CallServiceExt, ConversationServiceExt, MessageServiceExt,
            PovServiceExt,
        };
        let me = Arc::new(self.clone());
        let router = ConversationServiceExt::register(me.clone(), router);
        let router = MessageServiceExt::register(me.clone(), router);
        let router = PovServiceExt::register(me.clone(), router);
        let router = CallServiceExt::register(me.clone(), router);
        let router = AssistServiceExt::register(me.clone(), router);
        AssistAdminServiceExt::register(me, router)
    }

    /// The LiveKit deployment calls run on, when configured. For tooling
    /// and tests that talk to LiveKit with the server's credentials.
    pub fn livekit(&self) -> Option<&livekit::LiveKit> {
        self.livekit.as_ref()
    }
}
