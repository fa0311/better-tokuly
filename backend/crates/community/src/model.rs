//! Database rows and the numbers stored in their enum-like columns.
//!
//! Every query that reads a whole row goes through the column macros here,
//! with fixed table aliases: `c` communities, `m` members, `ch` channels,
//! `t` threads, `r` replies (with `t` joined), `e` events.
use serde_json::Value;
use tokuly_core::prelude::*;
use tokuly_media::assets::image_from_json;
use tokuly_proto::{
    common::Image,
    community::{
        self as pb, AttendanceStatus, CommunityVisibility, HiddenReason, NotificationLevel, PostState, PostingPolicy,
    },
};

use crate::permissions::{MemberGrants, Role};

/// `CommunityVisibility` as stored.
pub mod visibility {
    pub const PUBLIC: i32 = 1;
    pub const RESTRICTED: i32 = 2;
    pub const HIDDEN: i32 = 3;
}

/// `PostState` as stored.
pub mod state {
    pub const VISIBLE: i32 = 1;
    pub const HIDDEN: i32 = 2;
}

/// `HiddenReason` as stored.
pub mod hidden {
    pub const NONE: i32 = 0;
    pub const REPORTS: i32 = 1;
    pub const MUTED_WORD: i32 = 2;
}

/// `NotificationLevel` as stored.
pub mod level {
    pub const ALL: i32 = 1;
    pub const MENTIONS: i32 = 2;
    pub const NONE: i32 = 3;
}

/// `PostingPolicy` as stored.
pub mod posting {
    pub const EVERYONE: i32 = 1;
    pub const MODERATORS: i32 = 2;
}

/// `community_revisions.kind`: what an earlier version belongs to.
pub mod revised {
    pub const THREAD: i32 = 1;
    pub const REPLY: i32 = 2;
    pub const EVENT: i32 = 3;
}

/// Edits of its text one thread, reply or event takes. Every edit keeps
/// what it replaced for the staff; the cap keeps that history bounded
/// without ever dropping the original.
pub const MAX_EDITS: i32 = 30;

/// `AttendanceStatus` as stored.
pub mod attendance {
    pub const GOING: i32 = 1;
    pub const WAITLISTED: i32 = 2;
    pub const INTERESTED: i32 = 3;
    pub const NOT_GOING: i32 = 4;
}

const _: () = {
    // SQL in this crate compares these columns with literals.
    assert!(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC as i32 == visibility::PUBLIC);
    assert!(CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED as i32 == visibility::RESTRICTED);
    assert!(CommunityVisibility::COMMUNITY_VISIBILITY_HIDDEN as i32 == visibility::HIDDEN);
    assert!(PostState::POST_STATE_VISIBLE as i32 == state::VISIBLE);
    assert!(PostState::POST_STATE_HIDDEN as i32 == state::HIDDEN);
    assert!(HiddenReason::HIDDEN_REASON_REPORTS as i32 == hidden::REPORTS);
    assert!(HiddenReason::HIDDEN_REASON_MUTED_WORD as i32 == hidden::MUTED_WORD);
    assert!(NotificationLevel::NOTIFICATION_LEVEL_ALL as i32 == level::ALL);
    assert!(NotificationLevel::NOTIFICATION_LEVEL_MENTIONS as i32 == level::MENTIONS);
    assert!(NotificationLevel::NOTIFICATION_LEVEL_NONE as i32 == level::NONE);
    assert!(PostingPolicy::POSTING_POLICY_EVERYONE as i32 == posting::EVERYONE);
    assert!(PostingPolicy::POSTING_POLICY_MODERATORS as i32 == posting::MODERATORS);
    assert!(AttendanceStatus::ATTENDANCE_STATUS_GOING as i32 == attendance::GOING);
    assert!(AttendanceStatus::ATTENDANCE_STATUS_WAITLISTED as i32 == attendance::WAITLISTED);
    assert!(AttendanceStatus::ATTENDANCE_STATUS_INTERESTED as i32 == attendance::INTERESTED);
    assert!(AttendanceStatus::ATTENDANCE_STATUS_NOT_GOING as i32 == attendance::NOT_GOING);
};

/// Parses a JSON array column of proto-JSON messages; rows written by an
/// older build that no longer parse read as empty.
pub fn list_from_json<T: serde::de::DeserializeOwned>(value: &Value) -> Vec<T> {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

pub fn to_json<T: serde::Serialize + ?Sized>(value: &T) -> AppResult<Value> {
    serde_json::to_value(value).map_err(|e| AppError::internal(anyhow::anyhow!("community: encode: {e}")))
}

// ───────────────────────────── communities ─────────────────────────────

macro_rules! community_columns {
    () => {
        "c.id, c.handle::text AS handle, c.name, c.description, c.icon, c.icon_asset_id, c.banner, c.banner_asset_id, \
         c.rules, c.tags, c.category, c.language, c.visibility, c.school_name, c.join_questions, c.members_can_invite, \
         c.members_can_create_events, c.muted_words, c.auto_hide_reports, c.owner_id, c.member_count, c.thread_count, \
         c.last_activity_at, c.deleted_at, c.created_at"
    };
}
pub(crate) use community_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CommunityRow {
    pub id: Uuid,
    pub handle: String,
    pub name: String,
    pub description: String,
    pub icon: Option<Value>,
    pub icon_asset_id: Option<Uuid>,
    pub banner: Option<Value>,
    pub banner_asset_id: Option<Uuid>,
    pub rules: Value,
    pub tags: Vec<String>,
    pub category: i32,
    pub language: String,
    pub visibility: i32,
    pub school_name: String,
    pub join_questions: Vec<String>,
    pub members_can_invite: bool,
    pub members_can_create_events: bool,
    pub muted_words: Vec<String>,
    pub auto_hide_reports: i32,
    pub owner_id: Uuid,
    pub member_count: i32,
    pub thread_count: i32,
    pub last_activity_at: Time,
    pub deleted_at: Option<Time>,
    pub created_at: Time,
}

impl CommunityRow {
    pub fn is_public(&self) -> bool {
        self.visibility == visibility::PUBLIC
    }

    pub fn is_hidden(&self) -> bool {
        self.visibility == visibility::HIDDEN
    }

    /// What the community's settings open to plain members.
    pub fn grants(&self) -> MemberGrants {
        MemberGrants { invite: self.members_can_invite, create_events: self.members_can_create_events }
    }

    /// Path of the community in the web app.
    pub fn path(&self) -> String {
        format!("/c/{}", self.handle)
    }

    pub fn icon(&self) -> Option<Image> {
        image_from_json(self.icon.as_ref())
    }

    pub fn icon_url(&self) -> Option<String> {
        self.icon().map(|image| image.url).filter(|url| !url.is_empty())
    }
}

/// The realtime topic of a community: `community:<uuid>`.
pub fn topic(community_id: Uuid) -> String {
    format!("community:{community_id}")
}

macro_rules! member_columns {
    () => {
        "m.community_id, m.user_id, m.role, m.notification_level, m.show_on_profile, m.timeout_until, m.timeout_reason, \
         m.read_all_at, m.joined_at"
    };
}
pub(crate) use member_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemberRow {
    pub community_id: Uuid,
    pub user_id: Uuid,
    pub role: i32,
    pub notification_level: i32,
    pub show_on_profile: bool,
    pub timeout_until: Option<Time>,
    pub timeout_reason: String,
    pub read_all_at: Option<Time>,
    pub joined_at: Time,
}

impl MemberRow {
    /// The member's role; an unknown number reads as a plain member.
    pub fn role(&self) -> Role {
        Role::from_code(self.role).unwrap_or(Role::Member)
    }

    /// When the member's timeout ends, while one is running.
    pub fn timeout(&self, now: Time) -> Option<Time> {
        self.timeout_until.filter(|until| *until > now)
    }

    /// Nothing older than this counts as unread for the member.
    pub fn read_floor(&self) -> Time {
        self.read_all_at.map_or(self.joined_at, |read| read.max(self.joined_at))
    }

    /// The member as others see them. Timeouts are shown to the staff and
    /// to the member only.
    pub fn to_proto(&self, user: tokuly_proto::common::UserSummary, with_timeout: bool) -> pb::Member {
        let timeout = self.timeout(now()).filter(|_| with_timeout);
        pb::Member {
            user: user.into(),
            role: self.role().to_proto().into(),
            joined_at: ts(self.joined_at),
            timeout_until: opt_ts(timeout),
            timeout_reason: if timeout.is_some() { self.timeout_reason.clone() } else { String::new() },
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BanRow {
    pub community_id: Uuid,
    pub user_id: Uuid,
    pub reason: String,
    pub banned_by: Option<Uuid>,
    pub expires_at: Option<Time>,
    pub created_at: Time,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InviteRow {
    pub id: Uuid,
    pub community_id: Uuid,
    pub code: String,
    pub created_by: Uuid,
    pub note: String,
    pub max_uses: i32,
    pub use_count: i32,
    pub expires_at: Option<Time>,
    pub revoked_at: Option<Time>,
    pub created_at: Time,
}

pub const INVITE_COLUMNS: &str =
    "id, community_id, code, created_by, note, max_uses, use_count, expires_at, revoked_at, created_at";

impl InviteRow {
    /// Whether the invite can still be used.
    pub fn is_usable(&self, now: Time) -> bool {
        self.revoked_at.is_none()
            && self.expires_at.is_none_or(|expires| expires > now)
            && (self.max_uses == 0 || self.use_count < self.max_uses)
    }
}

// ───────────────────────────── boards ─────────────────────────────

macro_rules! channel_columns {
    () => {
        "ch.id, ch.community_id, ch.name, ch.description, ch.emoji, ch.position, ch.posting, ch.anonymous, \
         ch.slow_mode_seconds, ch.thread_count, ch.last_activity_at"
    };
}
pub(crate) use channel_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChannelRow {
    pub id: Uuid,
    pub community_id: Uuid,
    pub name: String,
    pub description: String,
    pub emoji: String,
    pub position: i32,
    pub posting: i32,
    pub anonymous: bool,
    pub slow_mode_seconds: i32,
    pub thread_count: i32,
    pub last_activity_at: Time,
}

impl ChannelRow {
    /// Whether a member of `role` may post here at all.
    pub fn open_to(&self, role: Role) -> bool {
        self.posting != posting::MODERATORS || role.is_staff()
    }
}

macro_rules! thread_columns {
    () => {
        "t.id, t.short_id, t.community_id, t.channel_id, t.author_id, t.anonymous, t.title, t.body, t.mentions, t.images, \
         t.asset_ids, t.stamp_id, t.tags, t.pinned_at, t.locked_at, t.announcement, t.announced_at, t.state, \
         t.hidden_reason, t.reviewed_at, t.reply_count, t.like_count, t.comment_count, \
         t.hot_score, t.last_activity_at, t.edited_at, t.edit_count, t.deleted_at, t.created_at"
    };
}
pub(crate) use thread_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ThreadRow {
    pub id: Uuid,
    pub short_id: String,
    pub community_id: Uuid,
    pub channel_id: Uuid,
    pub author_id: Uuid,
    pub anonymous: bool,
    pub title: String,
    pub body: String,
    pub mentions: Value,
    pub images: Value,
    pub asset_ids: Vec<Uuid>,
    pub stamp_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub pinned_at: Option<Time>,
    pub locked_at: Option<Time>,
    pub announcement: bool,
    pub announced_at: Option<Time>,
    pub state: i32,
    pub hidden_reason: i32,
    pub reviewed_at: Option<Time>,
    pub reply_count: i32,
    pub like_count: i64,
    pub comment_count: i32,
    pub hot_score: f64,
    pub last_activity_at: Time,
    pub edited_at: Option<Time>,
    pub edit_count: i32,
    pub deleted_at: Option<Time>,
    pub created_at: Time,
}

impl ThreadRow {
    pub fn is_visible(&self) -> bool {
        self.state == state::VISIBLE && self.deleted_at.is_none()
    }

    pub fn is_locked(&self) -> bool {
        self.locked_at.is_some()
    }

    /// Path of the thread in the web app.
    pub fn path(&self, community: &CommunityRow) -> String {
        format!("{}/t/{}", community.path(), self.short_id)
    }

    pub fn mentions(&self) -> Vec<pb::Mention> {
        list_from_json(&self.mentions)
    }

    pub fn images(&self) -> Vec<Image> {
        list_from_json(&self.images)
    }

    pub fn image_url(&self) -> Option<String> {
        self.images().into_iter().next().map(|image| image.url).filter(|url| !url.is_empty())
    }
}

/// A reply with what rendering needs from its thread and from the reply it
/// answers. Select with [`reply_select`].
macro_rules! reply_select {
    () => {
        "SELECT r.id, r.thread_id, r.community_id, r.author_id, r.anonymous, r.seq, r.body, r.mentions, r.images, \
                r.asset_ids, r.stamp_id, r.reply_to_id, r.state, r.hidden_reason, r.reviewed_at, \
                r.like_count, r.comment_count, r.edited_at, r.edit_count, r.deleted_at, r.created_at, \
                t.author_id AS thread_author_id, t.short_id AS thread_short_id, t.title AS thread_title, \
                t.channel_id AS channel_id, t.locked_at AS thread_locked_at, t.state AS thread_state, \
                t.deleted_at AS thread_deleted_at, \
                q.author_id AS quoted_author_id, q.body AS quoted_body, \
                (q.state = 1 AND q.deleted_at IS NULL) AS quoted_available \
         FROM community_replies r \
         JOIN community_threads t ON t.id = r.thread_id \
         LEFT JOIN community_replies q ON q.id = r.reply_to_id "
    };
}
pub(crate) use reply_select;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReplyRow {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub community_id: Uuid,
    pub author_id: Uuid,
    pub anonymous: bool,
    pub seq: i32,
    pub body: String,
    pub mentions: Value,
    pub images: Value,
    pub asset_ids: Vec<Uuid>,
    pub stamp_id: Option<Uuid>,
    pub reply_to_id: Option<Uuid>,
    pub state: i32,
    pub hidden_reason: i32,
    pub reviewed_at: Option<Time>,
    pub like_count: i64,
    pub comment_count: i32,
    pub edited_at: Option<Time>,
    pub edit_count: i32,
    pub deleted_at: Option<Time>,
    pub created_at: Time,
    pub thread_author_id: Uuid,
    pub thread_short_id: String,
    pub thread_title: String,
    pub channel_id: Uuid,
    pub thread_locked_at: Option<Time>,
    pub thread_state: i32,
    pub thread_deleted_at: Option<Time>,
    pub quoted_author_id: Option<Uuid>,
    pub quoted_body: Option<String>,
    pub quoted_available: Option<bool>,
}

impl ReplyRow {
    pub fn is_visible(&self) -> bool {
        self.state == state::VISIBLE && self.deleted_at.is_none()
    }

    /// Path of the reply in the web app.
    pub fn path(&self, community: &CommunityRow) -> String {
        format!("{}/t/{}?reply={}", community.path(), self.thread_short_id, self.id)
    }

    pub fn mentions(&self) -> Vec<pb::Mention> {
        list_from_json(&self.mentions)
    }

    pub fn images(&self) -> Vec<Image> {
        list_from_json(&self.images)
    }
}

// ───────────────────────────── events ─────────────────────────────

macro_rules! event_columns {
    () => {
        "e.id, e.community_id, e.organizer_id, e.title, e.description, e.starts_at, e.ends_at, e.online, e.online_url, \
         e.place_name, e.place_address, e.place_lat, e.place_lng, e.cover, e.cover_asset_id, e.capacity, e.going_count, \
         e.waitlist_count, e.interested_count, e.attended_count, e.like_count, e.comment_count, e.cancelled_at, \
         e.cancel_reason, e.edit_count, e.deleted_at, e.created_at"
    };
}
pub(crate) use event_columns;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventRow {
    pub id: Uuid,
    pub community_id: Uuid,
    pub organizer_id: Uuid,
    pub title: String,
    pub description: String,
    pub starts_at: Time,
    pub ends_at: Time,
    pub online: bool,
    pub online_url: String,
    pub place_name: String,
    pub place_address: String,
    pub place_lat: Option<f64>,
    pub place_lng: Option<f64>,
    pub cover: Option<Value>,
    pub cover_asset_id: Option<Uuid>,
    pub capacity: i32,
    pub going_count: i32,
    pub waitlist_count: i32,
    pub interested_count: i32,
    pub attended_count: i32,
    pub like_count: i64,
    pub comment_count: i32,
    pub cancelled_at: Option<Time>,
    pub cancel_reason: String,
    pub edit_count: i32,
    pub deleted_at: Option<Time>,
    pub created_at: Time,
}

impl EventRow {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled_at.is_some()
    }

    /// Whether a place is free for one more person who is going.
    pub fn has_room(&self) -> bool {
        self.capacity == 0 || self.going_count < self.capacity
    }

    /// Path of the event in the web app.
    pub fn path(&self, community: &CommunityRow) -> String {
        format!("{}/events/{}", community.path(), self.id)
    }

    pub fn cover_url(&self) -> Option<String> {
        image_from_json(self.cover.as_ref()).map(|image| image.url).filter(|url| !url.is_empty())
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AttendeeRow {
    pub user_id: Uuid,
    pub status: i32,
    pub responded_at: Time,
    pub queued_at: Option<Time>,
    pub checked_in_at: Option<Time>,
}

pub const ATTENDEE_COLUMNS: &str = "user_id, status, responded_at, queued_at, checked_in_at";

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn member(joined_ago: Duration) -> MemberRow {
        MemberRow {
            community_id: Uuid::nil(),
            user_id: Uuid::nil(),
            role: 1,
            notification_level: level::ALL,
            show_on_profile: false,
            timeout_until: None,
            timeout_reason: String::new(),
            read_all_at: None,
            joined_at: now() - joined_ago,
        }
    }

    #[test]
    fn timeouts_end_by_themselves() {
        let at = now();
        let mut row = member(Duration::days(3));
        assert!(row.timeout(at).is_none());
        row.timeout_until = Some(at + Duration::hours(1));
        assert!(row.timeout(at).is_some());
        row.timeout_until = Some(at - Duration::seconds(1));
        assert!(row.timeout(at).is_none());
    }

    #[test]
    fn nothing_before_joining_or_marking_all_read_is_unread() {
        let mut row = member(Duration::days(3));
        assert_eq!(row.read_floor(), row.joined_at);
        let read = now() - Duration::hours(1);
        row.read_all_at = Some(read);
        assert_eq!(row.read_floor(), read);
    }

    #[test]
    fn unknown_roles_read_as_member() {
        let mut row = member(Duration::days(1));
        row.role = 42;
        assert_eq!(row.role(), Role::Member);
        row.role = 3;
        assert_eq!(row.role(), Role::Admin);
    }

    #[test]
    fn invites_stop_working_when_expired_revoked_or_used_up() {
        let at = now();
        let invite = InviteRow {
            id: Uuid::nil(),
            community_id: Uuid::nil(),
            code: "abc".into(),
            created_by: Uuid::nil(),
            note: String::new(),
            max_uses: 2,
            use_count: 1,
            expires_at: Some(at + Duration::hours(1)),
            revoked_at: None,
            created_at: at,
        };
        assert!(invite.is_usable(at));
        assert!(!InviteRow { use_count: 2, ..invite.clone() }.is_usable(at));
        assert!(!InviteRow { expires_at: Some(at - Duration::seconds(1)), ..invite.clone() }.is_usable(at));
        assert!(!InviteRow { revoked_at: Some(at), ..invite.clone() }.is_usable(at));
        assert!(InviteRow { max_uses: 0, use_count: 999, expires_at: None, ..invite }.is_usable(at));
    }
}
