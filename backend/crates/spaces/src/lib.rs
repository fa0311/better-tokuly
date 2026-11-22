//! Spaces (たまり場 / 音声ルーム): drop-in audio rooms and always-on hangout
//! rooms on LiveKit.
//!
//! Two products share the crate. A **room** is a talk with a stage and an
//! audience (host, co-hosts, speakers, listeners), scheduled or started on
//! the spot; it runs once and ends. A **hangout** is a voice room owned by a
//! group of friends that is simply open whenever someone is inside.
//!
//! The database is the truth about who is inside and in which role; LiveKit
//! (`core.livekit`) only carries the sound. Every change to a room or a
//! hangout happens under a lock on its row, and LiveKit is brought in line
//! afterwards — at once for role changes and removals, and again by the
//! reconciliation, which also removes people whose connection is gone.
//!
//! # For other domains
//!
//! - [`Spaces::friends_in_rooms`] — friends of a user who are inside a room
//!   or hangout that user could enter (friend lists, the map).
//! - [`events`] — the payloads of the `spaces.*` domain events.
//! - [`Spaces::http_routes`] — the LiveKit webhook endpoint, to be merged
//!   into the API's HTTP router.
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`rules`] | everything free of I/O: validation, roles, access, when a room ends, when a connection counts as gone |
//! | [`model`] | database rows, the numbers stored in them, LiveKit room names |
//! | [`audio`] | LiveKit: rooms, tokens per role, permission updates |
//! | [`rooms`] | a room's life: create, schedule, start, end, and the building blocks every change uses |
//! | [`participants`] | inside a room: enter, leave, hands, the stage, co-hosts, mute, remove, ban, reactions, pin |
//! | [`invites`] | invitations to rooms and "remind me" |
//! | [`limits`] | rate limits: per person, per pair of people, per room |
//! | [`discovery`] | live among friends, public rooms by listeners, upcoming, tags, friends in rooms |
//! | [`hangouts`] | hangouts: members, invitations, the voice room |
//! | [`presence`] | keeping "who is inside" true: reconciliation with LiveKit, hand-over and automatic end |
//! | [`webhook`] | LiveKit's webhook, as an HTTP route |
//! | [`render`], [`push`], [`notices`] | API messages per viewer; realtime pushes; notifications, activity and search documents |
//! | [`cleanup`], [`subjects`], [`workers`] | other domains' events; topic guard, subject resolver, profile card and data export; background work |
//! | `*_svc` | the two Connect services: thin handlers over the modules above |
use std::{ops::Deref, sync::Arc};

use tokuly_core::{Core, livekit::LiveKit};
use tokuly_identity::Identity;

pub mod config;
pub mod events;

mod audio;
mod cleanup;
mod discovery;
mod hangout_svc;
mod hangouts;
mod invites;
mod limits;
mod model;
mod notices;
mod participants;
mod presence;
mod push;
mod render;
mod rooms;
mod rules;
mod space_svc;
mod subjects;
mod webhook;
mod workers;

pub use config::SpacesConfig;
pub use discovery::FriendInRoom;
pub use model::Venue;
pub use notices::{KIND_HANGOUT_JOINED, KIND_INVITED, KIND_REMINDER, KIND_STARTED};
pub use presence::Reconciled;
pub use subjects::CARD_IN_ROOM;
pub use webhook::WEBHOOK_PATH;

pub struct SpacesInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: SpacesConfig,
    /// The LiveKit deployment audio runs on; not configured when the server
    /// has none.
    pub livekit: LiveKit,
}

/// The spaces domain. Cheap to clone.
#[derive(Clone)]
pub struct Spaces(Arc<SpacesInner>);

impl Deref for Spaces {
    type Target = SpacesInner;

    fn deref(&self) -> &SpacesInner {
        &self.0
    }
}

impl Spaces {
    /// Builds the domain on the server's LiveKit (`core.livekit`) and plugs
    /// it into the generic domains: the realtime topic `space:<id>` gets
    /// its guard, rooms and hangouts become reportable subjects, profiles
    /// gain the "in a room" card and personal data exports a `spaces`
    /// section.
    pub fn new(core: Core, identity: Identity, cfg: SpacesConfig) -> anyhow::Result<Self> {
        let livekit = core.livekit.clone();
        Self::with_livekit(core, identity, cfg, livekit)
    }

    /// [`Self::new`] on a LiveKit deployment of the caller's choosing. For
    /// tools and tests, e.g. to run the domain without audio
    /// (`LiveKit::disabled()`).
    pub fn with_livekit(core: Core, identity: Identity, cfg: SpacesConfig, livekit: LiveKit) -> anyhow::Result<Self> {
        cfg.validate()?;
        let spaces = Self(Arc::new(SpacesInner { core, identity, cfg, livekit }));
        subjects::register(&spaces);
        Ok(spaces)
    }

    /// Registers the two services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::spaces::{HangoutServiceExt, SpaceServiceExt};
        let me = Arc::new(self.clone());
        let router = SpaceServiceExt::register(me.clone(), router);
        HangoutServiceExt::register(me, router)
    }
}
