//! Database rows, the numbers stored in them, and the names derived from
//! IDs (LiveKit rooms, realtime topics, paths in the app).
use tokuly_core::prelude::*;
use tokuly_proto::spaces as pb;

/// `spaces.audience`: `tokuly.spaces.v1.SpaceAudience`.
pub mod audience {
    pub const PUBLIC: i16 = 1;
    pub const FRIENDS: i16 = 2;
    pub const INVITED: i16 = 3;
}

/// `spaces.state`: `tokuly.spaces.v1.SpaceState`.
pub mod state {
    pub const SCHEDULED: i16 = 1;
    pub const LIVE: i16 = 2;
    pub const ENDED: i16 = 3;
    pub const CANCELLED: i16 = 4;
}

/// `spaces.end_reason`: `tokuly.spaces.v1.SpaceEndReason`.
pub mod end {
    pub const HOST_ENDED: i16 = 1;
    pub const HOST_LEFT: i16 = 2;
    pub const EMPTY: i16 = 3;
    pub const MODERATION: i16 = 4;
    pub const ACCOUNT_CLOSED: i16 = 5;
    pub const NEVER_STARTED: i16 = 6;
}

/// `space_participants.role`: `tokuly.spaces.v1.SpaceRole`. Ordered: a
/// higher number may do everything a lower one may.
pub mod role {
    pub const LISTENER: i16 = 1;
    pub const SPEAKER: i16 = 2;
    pub const COHOST: i16 = 3;
    pub const HOST: i16 = 4;
}

/// `space_participants.leave_reason`: `tokuly.spaces.v1.SpaceLeaveReason`.
pub mod leave {
    pub const LEFT: i16 = 1;
    pub const REMOVED: i16 = 2;
    pub const BANNED: i16 = 3;
    pub const DISCONNECTED: i16 = 4;
    pub const ENDED: i16 = 5;
}

/// `hangout_members.state`: `tokuly.spaces.v1.HangoutMemberState`, plus
/// one value the API never shows.
pub mod member {
    pub const INVITED: i16 = 1;
    pub const MEMBER: i16 = 2;
    /// Was invited or a member and is not any more. The row stays for a
    /// while so that the person can still report the hangout.
    pub const FORMER: i16 = 3;
}

/// Columns of [`SpaceRow`], for a table aliased `s`.
pub const SPACE: &str = "s.id, s.host_id, s.title, s.tags, s.language, s.audience, s.state, s.end_reason, \
     s.scheduled_at, s.started_at, s.ended_at, s.listener_count, s.speaker_count, s.hand_count, s.peak_listeners, \
     s.total_listeners, s.reminder_count, s.pin_text, s.pin_url, s.pinned_by, s.pinned_at, s.host_absent_since, \
     s.empty_since, s.deleted_at, s.created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SpaceRow {
    pub id: Uuid,
    pub host_id: Uuid,
    pub title: String,
    pub tags: Vec<String>,
    pub language: String,
    pub audience: i16,
    pub state: i16,
    pub end_reason: i16,
    pub scheduled_at: Option<Time>,
    pub started_at: Option<Time>,
    pub ended_at: Option<Time>,
    pub listener_count: i32,
    pub speaker_count: i32,
    pub hand_count: i32,
    pub peak_listeners: i32,
    pub total_listeners: i32,
    pub reminder_count: i32,
    pub pin_text: String,
    pub pin_url: String,
    pub pinned_by: Option<Uuid>,
    pub pinned_at: Option<Time>,
    pub host_absent_since: Option<Time>,
    pub empty_since: Option<Time>,
    /// Set once the host removed the room from their history: it reads as
    /// gone, and is only kept for reports and moderation.
    pub deleted_at: Option<Time>,
    pub created_at: Time,
}

impl SpaceRow {
    pub fn is_live(&self) -> bool {
        self.state == state::LIVE
    }

    pub fn is_scheduled(&self) -> bool {
        self.state == state::SCHEDULED
    }

    /// Ended or cancelled: nothing about it changes any more.
    pub fn is_over(&self) -> bool {
        self.state == state::ENDED || self.state == state::CANCELLED
    }

    pub fn is_public(&self) -> bool {
        self.audience == audience::PUBLIC
    }

    /// Whole minutes the room was live (rounded up), 0 if it never was.
    pub fn minutes(&self) -> i64 {
        match (self.started_at, self.ended_at) {
            (Some(started), Some(ended)) => ((ended - started).num_seconds().max(0) + 59) / 60,
            _ => 0,
        }
    }

    pub fn pin(&self) -> Option<pb::SpacePin> {
        if self.pin_text.is_empty() && self.pin_url.is_empty() {
            return None;
        }
        Some(pb::SpacePin {
            text: self.pin_text.clone(),
            url: self.pin_url.clone(),
            pinned_at: opt_ts(self.pinned_at),
            pinned_by_id: self.pinned_by.map(|id| id.to_string()).unwrap_or_default(),
            ..Default::default()
        })
    }

    pub fn counts(&self) -> pb::SpaceCounts {
        pb::SpaceCounts {
            space_id: self.id.to_string(),
            listener_count: self.listener_count,
            speaker_count: self.speaker_count,
            hand_count: self.hand_count,
            peak_listeners: self.peak_listeners,
            ..Default::default()
        }
    }
}

/// Columns of [`ParticipantRow`], for a table aliased `p`.
pub const PARTICIPANT: &str = "p.space_id, p.user_id, p.role, p.top_role, p.joined_at, p.left_at, p.leave_reason, \
     p.hand_raised_at, p.hand_public, p.stage_since, p.left_on_stage, p.stage_invited_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ParticipantRow {
    pub space_id: Uuid,
    pub user_id: Uuid,
    pub role: i16,
    /// The highest role they ever held in the room.
    pub top_role: i16,
    pub joined_at: Time,
    pub left_at: Option<Time>,
    /// `leave::*` of the last departure; 0 while inside.
    pub leave_reason: i16,
    pub hand_raised_at: Option<Time>,
    /// The raised hand went up while its owner was visible: everyone in
    /// the room may see it. Otherwise only the moderators and the person.
    pub hand_public: bool,
    /// When they last came on stage; `None` off stage.
    pub stage_since: Option<Time>,
    /// The last departure was from the stage.
    pub left_on_stage: bool,
    pub stage_invited_at: Option<Time>,
}

impl ParticipantRow {
    pub fn is_inside(&self) -> bool {
        self.left_at.is_none()
    }
}

/// Columns of [`HangoutRow`], for a table aliased `h`.
pub const HANGOUT: &str = "h.id, h.owner_id, h.name, h.emoji, h.members_can_invite, h.member_count, \
     h.inside_count, h.last_active_at, h.last_announced_at, h.created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HangoutRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub emoji: String,
    pub members_can_invite: bool,
    pub member_count: i32,
    pub inside_count: i32,
    pub last_active_at: Option<Time>,
    pub last_announced_at: Option<Time>,
    pub created_at: Time,
}

/// Someone's place in a hangout: member or invited.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HangoutMemberRow {
    pub state: i16,
    pub invited_by: Option<Uuid>,
}

impl HangoutMemberRow {
    pub fn is_member(&self) -> bool {
        self.state == member::MEMBER
    }
}

/// Someone in a voice room as the reconciliation with LiveKit sees them.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InsideRow {
    pub user_id: Uuid,
    pub joined_at: Time,
    pub seen_at: Option<Time>,
    pub missing_since: Option<Time>,
    /// The role in a room; [`role::SPEAKER`] for everyone in a hangout.
    pub role: i16,
}

// ───────────────────────────── names ─────────────────────────────

const SPACE_ROOM_PREFIX: &str = "tokuly-space-";
const HANGOUT_ROOM_PREFIX: &str = "tokuly-hangout-";

/// A LiveKit room of this domain, named after what it carries audio for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Venue {
    Space(Uuid),
    Hangout(Uuid),
}

impl Venue {
    /// The LiveKit room name.
    pub fn room(self) -> String {
        match self {
            Self::Space(id) => format!("{SPACE_ROOM_PREFIX}{}", id.simple()),
            Self::Hangout(id) => format!("{HANGOUT_ROOM_PREFIX}{}", id.simple()),
        }
    }

    /// The venue a LiveKit room name belongs to; `None` for rooms of other
    /// domains (calls) and of other applications on the same server.
    pub fn of_room(room: &str) -> Option<Self> {
        if let Some(id) = room.strip_prefix(SPACE_ROOM_PREFIX) {
            return Uuid::parse_str(id).ok().map(Self::Space);
        }
        let id = room.strip_prefix(HANGOUT_ROOM_PREFIX)?;
        Uuid::parse_str(id).ok().map(Self::Hangout)
    }
}

/// The realtime topic of a room.
pub fn space_topic(space_id: Uuid) -> String {
    format!("space:{space_id}")
}

/// Path of a room in the app.
pub fn space_path(space_id: Uuid) -> String {
    format!("/spaces/{space_id}")
}

/// Path of a hangout in the app.
pub fn hangout_path(hangout_id: Uuid) -> String {
    format!("/hangouts/{hangout_id}")
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn stored_numbers_are_the_proto_numbers() {
        assert_eq!(audience::PUBLIC as i32, pb::SpaceAudience::SPACE_AUDIENCE_PUBLIC as i32);
        assert_eq!(audience::FRIENDS as i32, pb::SpaceAudience::SPACE_AUDIENCE_FRIENDS as i32);
        assert_eq!(audience::INVITED as i32, pb::SpaceAudience::SPACE_AUDIENCE_INVITED as i32);
        assert_eq!(state::SCHEDULED as i32, pb::SpaceState::SPACE_STATE_SCHEDULED as i32);
        assert_eq!(state::LIVE as i32, pb::SpaceState::SPACE_STATE_LIVE as i32);
        assert_eq!(state::ENDED as i32, pb::SpaceState::SPACE_STATE_ENDED as i32);
        assert_eq!(state::CANCELLED as i32, pb::SpaceState::SPACE_STATE_CANCELLED as i32);
        assert_eq!(end::HOST_ENDED as i32, pb::SpaceEndReason::SPACE_END_REASON_HOST_ENDED as i32);
        assert_eq!(end::HOST_LEFT as i32, pb::SpaceEndReason::SPACE_END_REASON_HOST_LEFT as i32);
        assert_eq!(end::EMPTY as i32, pb::SpaceEndReason::SPACE_END_REASON_EMPTY as i32);
        assert_eq!(end::MODERATION as i32, pb::SpaceEndReason::SPACE_END_REASON_MODERATION as i32);
        assert_eq!(end::ACCOUNT_CLOSED as i32, pb::SpaceEndReason::SPACE_END_REASON_ACCOUNT_CLOSED as i32);
        assert_eq!(end::NEVER_STARTED as i32, pb::SpaceEndReason::SPACE_END_REASON_NEVER_STARTED as i32);
        assert_eq!(role::LISTENER as i32, pb::SpaceRole::SPACE_ROLE_LISTENER as i32);
        assert_eq!(role::SPEAKER as i32, pb::SpaceRole::SPACE_ROLE_SPEAKER as i32);
        assert_eq!(role::COHOST as i32, pb::SpaceRole::SPACE_ROLE_COHOST as i32);
        assert_eq!(role::HOST as i32, pb::SpaceRole::SPACE_ROLE_HOST as i32);
        assert_eq!(leave::LEFT as i32, pb::SpaceLeaveReason::SPACE_LEAVE_REASON_LEFT as i32);
        assert_eq!(leave::REMOVED as i32, pb::SpaceLeaveReason::SPACE_LEAVE_REASON_REMOVED as i32);
        assert_eq!(leave::BANNED as i32, pb::SpaceLeaveReason::SPACE_LEAVE_REASON_BANNED as i32);
        assert_eq!(leave::DISCONNECTED as i32, pb::SpaceLeaveReason::SPACE_LEAVE_REASON_DISCONNECTED as i32);
        assert_eq!(leave::ENDED as i32, pb::SpaceLeaveReason::SPACE_LEAVE_REASON_ENDED as i32);
        assert_eq!(member::INVITED as i32, pb::HangoutMemberState::HANGOUT_MEMBER_STATE_INVITED as i32);
        assert_eq!(member::MEMBER as i32, pb::HangoutMemberState::HANGOUT_MEMBER_STATE_MEMBER as i32);
    }

    #[test]
    fn venues_and_room_names_round_trip() {
        let id = new_id();
        for venue in [Venue::Space(id), Venue::Hangout(id)] {
            assert_eq!(Venue::of_room(&venue.room()), Some(venue));
        }
        assert_eq!(Venue::Space(id).room(), format!("tokuly-space-{}", id.simple()));
        assert_eq!(Venue::of_room(&format!("tokuly-call-{}", id.simple())), None, "a call of the chat domain");
        assert_eq!(Venue::of_room("tokuly-space-nonsense"), None);
        assert_eq!(Venue::of_room(""), None);
        assert_eq!(space_topic(id), format!("space:{id}"));
    }

    #[test]
    fn minutes_round_up() {
        let started = now();
        let mut space = SpaceRow {
            id: new_id(),
            host_id: new_id(),
            title: "t".into(),
            tags: vec![],
            language: String::new(),
            audience: audience::PUBLIC,
            state: state::ENDED,
            end_reason: end::HOST_ENDED,
            scheduled_at: None,
            started_at: Some(started),
            ended_at: Some(started + Duration::seconds(61)),
            listener_count: 0,
            speaker_count: 0,
            hand_count: 0,
            peak_listeners: 0,
            total_listeners: 0,
            reminder_count: 0,
            pin_text: String::new(),
            pin_url: String::new(),
            pinned_by: None,
            pinned_at: None,
            host_absent_since: None,
            empty_since: None,
            deleted_at: None,
            created_at: started,
        };
        assert_eq!(space.minutes(), 2);
        assert!(space.pin().is_none() && space.is_over() && !space.is_live());
        space.ended_at = Some(started);
        assert_eq!(space.minutes(), 0);
        space.started_at = None;
        assert_eq!(space.minutes(), 0, "a cancelled room never ran");
        space.pin_url = "https://example.com".into();
        assert_eq!(space.pin().unwrap().url, "https://example.com");
    }
}
