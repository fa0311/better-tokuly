//! Notes (ノート: short posts, 「今何してる?」) and stories (記憶: a titled
//! set of photos and videos friends can see for a day).
//!
//! # For other domains
//!
//! - Notes and stories are subjects (`SUBJECT_TYPE_NOTE`, `SUBJECT_TYPE_STORY`):
//!   the generic domains resolve them through `core.subjects`, with the same
//!   visibility rules every RPC here applies.
//! - [`events`] holds the payloads of the events this domain emits.
//! - [`Notes::post_note`] posts a note without going through the API (the
//!   demo simulator).
//!
//! # Map of the crate
//!
//! | module | what lives there |
//! |---|---|
//! | [`visibility`] | who may see a note or a story — the one place that decides it |
//! | `authors` | locked accounts: this domain's copy, and listing their public notes |
//! | [`text`] | what is read out of note text: length, hashtags, mentions, links |
//! | [`rules`] | the posting rules and who agreed to them |
//! | [`model`] | database rows, paths, topics, holder names |
//! | [`posting`] | posting, deleting, changing visibility, pinning, link previews |
//! | [`reactions`] | likes, renotes, view counting |
//! | [`timelines`] | every list of notes, threads |
//! | [`hashtags`] | the hashtag directory, trending, completion |
//! | `hydrate` | note rows → the `Note` messages one viewer gets |
//! | [`stories`], [`highlights`] | stories, their viewers and reactions, the archive, highlights |
//! | `note_svc`, `story_svc` | the two Connect services: thin handlers over the modules above |
//! | `copy` | notification and activity texts |
//! | `subjects` | subject resolvers, data export, note count on profiles |
//! | `cleanup`, `workers` | reactions to other domains' events; background workers |
use std::{ops::Deref, sync::Arc};

use tokuly_core::prelude::*;
use tokuly_identity::Identity;
use tokuly_proto::notes::{NotesEvent, notes_event::Event as Push};

mod authors;
pub mod channels;
mod cleanup;
pub mod config;
mod content;
mod copy;
mod draft_svc;
pub mod drafts;
pub mod editing;
pub mod events;
mod fanout;
pub mod hashtags;
pub mod highlights;
mod hydrate;
mod insights;
pub mod markup;
pub mod model;
pub mod mutes;
mod note_svc;
mod polls;
pub mod posting;
pub mod reactions;
pub mod rules;
pub mod stories;
mod story_svc;
mod subjects;
pub mod text;
pub mod timelines;
pub mod visibility;
mod workers;

pub use config::NotesConfig;

use posting::NewNote;

pub struct NotesInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: NotesConfig,
}

/// The notes domain. Cheap to clone.
#[derive(Clone)]
pub struct Notes(Arc<NotesInner>);

impl Deref for Notes {
    type Target = NotesInner;

    fn deref(&self) -> &NotesInner {
        &self.0
    }
}

impl Notes {
    /// Builds the domain and plugs it into the rest of the backend: subject
    /// resolvers for notes and stories, the `notes` section of the personal
    /// data export, and the note count on profiles.
    pub fn new(core: Core, identity: Identity, cfg: NotesConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let notes = Self(Arc::new(NotesInner { core, identity, cfg }));
        subjects::register(&notes);
        Ok(notes)
    }

    /// Registers `NoteService` and `StoryService` on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::notes::{NoteDraftServiceExt, NoteMuteServiceExt, NoteServiceExt, StoryServiceExt};
        let me = Arc::new(self.clone());
        let router = NoteServiceExt::register(me.clone(), router);
        let router = NoteDraftServiceExt::register(me.clone(), router);
        let router = NoteMuteServiceExt::register(me.clone(), router);
        StoryServiceExt::register(me, router)
    }

    /// Posts a plain public-by-default note for `author` and returns its ID.
    ///
    /// For the demo simulator and seeding: the author is taken to have
    /// agreed to the rules (the agreement is recorded) and the posting rate
    /// limits do not apply. Everything else — mentions, hashtags, the
    /// activity entry, notifications — happens as for a note posted through
    /// the API.
    pub async fn post_note(&self, author: Uuid, text: &str) -> AppResult<Uuid> {
        rules::agree(&self.core, author).await?;
        let note = self.store_note(author, new_id(), NewNote { text: text.to_owned(), ..Default::default() }).await?;
        Ok(note.id)
    }

    /// Pushes a notes event to every connected device of one user.
    pub(crate) async fn push_user(&self, user_id: Uuid, event: Push) {
        self.core.rt.notify_user(user_id, &notes_event(event)).await;
    }

    /// Pushes the same notes event to several users.
    pub(crate) async fn push_users(&self, user_ids: impl IntoIterator<Item = Uuid>, event: Push) {
        self.core.rt.notify_users(user_ids, &notes_event(event)).await;
    }

    /// Pushes a notes event to the clients subscribed to a topic.
    pub(crate) async fn push_topic(&self, topic: &str, event: Push) {
        self.core.rt.notify_topic(topic, &notes_event(event)).await;
    }
}

fn notes_event(event: Push) -> NotesEvent {
    NotesEvent { event: Some(event), ..Default::default() }
}
