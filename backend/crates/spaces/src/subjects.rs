//! What the spaces domain plugs into the rest of the backend: the guard of
//! the realtime topic `space:<id>`, the subject resolver that makes rooms
//! and hangouts reportable, the "in a room" card on profiles, and the
//! personal data export.
use std::sync::Arc;

use async_trait::async_trait;
use tokuly_core::{
    export::DataExporter,
    prelude::*,
    realtime::TopicGuard,
    subject::{SubjectInfo, SubjectResolver},
};
use tokuly_identity::{ProfileExtension, settings};
use tokuly_proto::{
    common::{LocalizedText, SubjectType},
    identity::{Profile, ProfileCard},
};
use tokuly_social::graph;

use crate::{
    Spaces,
    model::{Venue, hangout_path, member, role, space_path, state},
    rules,
};

/// Kind of the profile card shown while someone is in a room or hangout
/// the viewer could enter.
pub const CARD_IN_ROOM: &str = "spaces.in_room";
/// Where the card sits among the cards of a profile: near the top, it is
/// about right now.
const CARD_ORDER: i32 = 20;

/// Registers the guard, the resolver, the exporter and the profile card.
/// Called once from `Spaces::new`.
pub(crate) fn register(spaces: &Spaces) {
    let core = &spaces.core;
    core.topic_guards.register("space", Arc::new(SpaceGuard(spaces.clone())));
    core.subjects.register(SubjectType::SUBJECT_TYPE_SPACE, Arc::new(SpaceSubjects(spaces.clone())));
    core.exporters.register(Arc::new(SpacesExporter { core: core.clone() }));
    spaces.identity.add_profile_extension(Arc::new(InRoomCard(spaces.clone())));
}

/// Who may follow `space:<id>`: whoever may see the room. Public rooms are
/// open to everyone, signed in or not; the others to the host, the people
/// inside, the invited and — for friends rooms — the host's friends.
struct SpaceGuard(Spaces);

#[async_trait]
impl TopicGuard for SpaceGuard {
    async fn may_subscribe(&self, subscriber: Option<Uuid>, topic: &str) -> AppResult<bool> {
        let mut segments = topic.split(':');
        let (Some("space"), Some(space_id), None) = (segments.next(), segments.next(), segments.next()) else {
            return Ok(false);
        };
        let Ok(space_id) = Uuid::parse_str(space_id) else { return Ok(false) };
        let Some(space) = self.0.find_space(space_id).await? else { return Ok(false) };
        Ok(self.0.access(subscriber, &space).await?.may_see())
    }
}

/// Rooms and hangouts as subjects, for reports and for linking
/// notifications. Comments and reactions are not for audio rooms.
///
/// A room resolves for whoever may see it — and for everyone who was ever
/// inside it or invited to it, whatever happened since: someone who was
/// removed, banned or blocked, whose invitation was taken back, or whose
/// host deleted the room afterwards can still report what they were part
/// of. A hangout resolves for its members and invitees, former ones
/// included, also after its owner closed it.
struct SpaceSubjects(Spaces);

impl SpaceSubjects {
    /// Whether `viewer` was ever inside a room or invited to it.
    async fn was_part_of_room(&self, space_id: Uuid, viewer: Uuid) -> AppResult<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM space_participants WHERE space_id = $1 AND user_id = $2)
                 OR EXISTS (SELECT 1 FROM space_invites WHERE space_id = $1 AND user_id = $2)",
        )
        .bind(space_id)
        .bind(viewer)
        .fetch_one(&self.0.core.db)
        .await?)
    }
}

#[async_trait]
impl SubjectResolver for SpaceSubjects {
    async fn resolve(&self, id: &str, viewer: Option<Uuid>) -> AppResult<Option<SubjectInfo>> {
        // A malformed ID names nothing; that is "not found", not a client error.
        let Ok(id) = Uuid::parse_str(id) else { return Ok(None) };
        let spaces = &self.0;
        if let Some(space) = spaces.find_space_record(id).await? {
            let visible = space.deleted_at.is_none() && spaces.access(viewer, &space).await?.may_see();
            let involved = match viewer {
                Some(viewer) if !visible => self.was_part_of_room(id, viewer).await?,
                _ => false,
            };
            if !visible && !involved {
                return Ok(None);
            }
            return Ok(Some(SubjectInfo {
                id,
                owner_id: space.host_id,
                title: space.title,
                path: space_path(id),
                image_url: None,
                interactions_enabled: false,
            }));
        }
        let Some(viewer) = viewer else { return Ok(None) };
        // Any trace of the viewer in the hangout will do, and the hangout
        // may be closed.
        let hangout: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT h.owner_id, h.name FROM hangouts h
             JOIN hangout_members m ON m.hangout_id = h.id AND m.user_id = $2
             WHERE h.id = $1",
        )
        .bind(id)
        .bind(viewer)
        .fetch_optional(&spaces.core.db)
        .await?;
        Ok(hangout.map(|(owner_id, name)| SubjectInfo {
            id,
            owner_id,
            title: name,
            path: hangout_path(id),
            image_url: None,
            interactions_enabled: false,
        }))
    }
}

/// "In a room right now" on profiles, shown to friends who could enter
/// the same room or hangout.
struct InRoomCard(Spaces);

impl Spaces {
    /// The room or hangout `owner` is inside, if `viewer` is a friend who
    /// could enter it. A friend who hides their online status only shows
    /// while on stage in a room; in a hangout the members see each other
    /// anyway.
    pub(crate) async fn where_is(&self, viewer: Uuid, owner: Uuid) -> AppResult<Option<(Venue, String, String, i16)>> {
        let core = &self.core;
        if viewer == owner || !graph::are_friends(core, owner, viewer).await? {
            return Ok(None);
        }
        let in_room: Option<(Uuid, i16)> = sqlx::query_as(
            "SELECT p.space_id, p.role FROM space_participants p JOIN spaces s ON s.id = p.space_id
             WHERE p.user_id = $1 AND p.left_at IS NULL AND s.state = $2
             ORDER BY p.joined_at DESC LIMIT 1",
        )
        .bind(owner)
        .bind(state::LIVE)
        .fetch_optional(&core.db)
        .await?;
        if let Some((space_id, role)) = in_room {
            let shown = rules::on_stage(role) || settings::privacy_of(core, owner).await?.share_online_status;
            let Some(space) = self.find_space(space_id).await? else { return Ok(None) };
            let banned: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM space_bans WHERE space_id = $1 AND user_id = $2)")
                    .bind(space_id)
                    .bind(viewer)
                    .fetch_one(&core.db)
                    .await?;
            if shown && !banned && self.access(Some(viewer), &space).await?.may_see() {
                return Ok(Some((Venue::Space(space_id), space.title, String::new(), role)));
            }
            return Ok(None);
        }
        let in_hangout: Option<(Uuid, String, String)> = sqlx::query_as(
            "SELECT h.id, h.name, h.emoji FROM hangout_presence pr
             JOIN hangouts h ON h.id = pr.hangout_id
             JOIN hangout_members mine ON mine.hangout_id = h.id AND mine.user_id = $2 AND mine.state = $3
             WHERE pr.user_id = $1
             ORDER BY pr.joined_at DESC LIMIT 1",
        )
        .bind(owner)
        .bind(viewer)
        .bind(member::MEMBER)
        .fetch_optional(&core.db)
        .await?;
        Ok(in_hangout.map(|(hangout_id, name, emoji)| (Venue::Hangout(hangout_id), name, emoji, 0)))
    }
}

#[async_trait]
impl ProfileExtension for InRoomCard {
    async fn enrich_profile(&self, viewer: Option<Uuid>, owner: Uuid, profile: &mut Profile) -> AppResult<()> {
        let Some(viewer) = viewer else { return Ok(()) };
        let Some((venue, title, emoji, role)) = self.0.where_is(viewer, owner).await? else { return Ok(()) };
        let card = match venue {
            Venue::Space(space_id) => ProfileCard {
                kind: CARD_IN_ROOM.to_owned(),
                title: LocalizedText {
                    ja: "ルームにいるよ!".into(),
                    en: "In a room right now".into(),
                    ..Default::default()
                }
                .into(),
                text: LocalizedText {
                    ja: format!("「{title}」"), en: format!("“{title}”"), ..Default::default()
                }
                .into(),
                emoji: "🎙️".to_owned(),
                path: space_path(space_id),
                data: [
                    ("space_id".to_owned(), space_id.to_string()),
                    ("role".to_owned(), rules::role_name(role.max(role::LISTENER)).to_owned()),
                ]
                .into_iter()
                .collect(),
                order: CARD_ORDER,
                ..Default::default()
            },
            Venue::Hangout(hangout_id) => ProfileCard {
                kind: CARD_IN_ROOM.to_owned(),
                title: LocalizedText {
                    ja: "たまり場にいるよ!".into(),
                    en: "In a hangout right now".into(),
                    ..Default::default()
                }
                .into(),
                text: LocalizedText {
                    ja: format!("「{title}」"), en: format!("“{title}”"), ..Default::default()
                }
                .into(),
                emoji: if emoji.is_empty() { rules::DEFAULT_HANGOUT_EMOJI.to_owned() } else { emoji },
                path: hangout_path(hangout_id),
                data: [("hangout_id".to_owned(), hangout_id.to_string())].into_iter().collect(),
                order: CARD_ORDER,
                ..Default::default()
            },
        };
        profile.cards.push(card);
        Ok(())
    }
}

/// The `spaces` section of a personal data export: the rooms the user
/// hosted and was in, their invitations and reminders, and their hangouts.
struct SpacesExporter {
    core: Core,
}

#[async_trait]
impl DataExporter for SpacesExporter {
    fn name(&self) -> &'static str {
        "spaces"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        let db = &self.core.db;
        let section = |sql: &'static str| async move {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_all(db).await?;
            AppResult::Ok(serde_json::Value::Array(rows))
        };
        Ok(serde_json::json!({
            "hosted_rooms": section(
                "SELECT jsonb_build_object('id', id, 'title', title, 'tags', tags, 'language', language,
                        'audience', audience, 'state', state, 'end_reason', end_reason, 'scheduled_at', scheduled_at,
                        'started_at', started_at, 'ended_at', ended_at, 'peak_listeners', peak_listeners,
                        'total_listeners', total_listeners, 'pin_text', pin_text, 'pin_url', pin_url, 'created_at', created_at)
                 FROM spaces WHERE host_id = $1 ORDER BY id"
            ).await?,
            "rooms_joined": section(
                "SELECT jsonb_build_object('space_id', p.space_id, 'title', s.title, 'role', p.role,
                        'first_joined_at', p.first_joined_at, 'last_joined_at', p.joined_at, 'left_at', p.left_at,
                        'seconds', p.seconds)
                 FROM space_participants p JOIN spaces s ON s.id = p.space_id
                 WHERE p.user_id = $1 ORDER BY p.first_joined_at, p.space_id"
            ).await?,
            "room_invitations": section(
                "SELECT jsonb_build_object('space_id', space_id, 'invited_by', invited_by, 'invited_at', created_at)
                 FROM space_invites WHERE user_id = $1 ORDER BY created_at, space_id"
            ).await?,
            "room_invitations_sent": section(
                "SELECT jsonb_build_object('space_id', space_id, 'user_id', user_id, 'invited_at', created_at)
                 FROM space_invites WHERE invited_by = $1 ORDER BY created_at, space_id, user_id"
            ).await?,
            "reminders": section(
                "SELECT jsonb_build_object('space_id', space_id, 'set_at', created_at)
                 FROM space_reminders WHERE user_id = $1 ORDER BY created_at, space_id"
            ).await?,
            "hangouts": section(
                "SELECT jsonb_build_object('id', h.id, 'name', h.name, 'emoji', h.emoji, 'owner', h.owner_id = $1,
                        'state', m.state, 'muted', m.muted, 'invited_by', m.invited_by, 'joined_at', m.joined_at,
                        'created_at', h.created_at)
                 FROM hangout_members m JOIN hangouts h ON h.id = m.hangout_id
                 WHERE m.user_id = $1 ORDER BY h.id"
            ).await?,
        }))
    }
}
