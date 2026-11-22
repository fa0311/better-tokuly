//! What the domain tells the generic ones: notification requests and
//! their wording, the activity entry of a public room, and search
//! documents.
use tokuly_core::{
    outbound::{ActivityAudience, ActivityEntry, Localized, NotificationRequest, SearchDoc},
    prelude::*,
};
use tokuly_proto::common::{SubjectType, UserSummary};

use crate::model::{HangoutRow, SpaceRow, hangout_path, space_path};

/// Friends of the host: their friends room went live.
pub const KIND_STARTED: &str = "spaces.started";
/// A room someone asked to be reminded of went live; and, to the host,
/// the planned time of their room has come.
pub const KIND_REMINDER: &str = "spaces.reminder";
/// Invited to a room or to a hangout.
pub const KIND_INVITED: &str = "spaces.invited";
/// A friend is the first one inside a shared hangout.
pub const KIND_HANGOUT_JOINED: &str = "spaces.hangout_joined";

/// Kind of the activity entry a public room leaves when it starts.
pub const ACTIVITY_STARTED: &str = "spaces.started";
/// `SearchDoc::kind` of public rooms.
pub const SEARCH_KIND: &str = "space";

/// The name a person goes by in notifications.
pub(crate) fn name_of(user: &UserSummary) -> String {
    if user.deleted {
        "退会したユーザー".to_owned()
    } else if user.display_name.is_empty() {
        format!("@{}", user.handle)
    } else {
        user.display_name.clone()
    }
}

/// A hangout as notifications name it: emoji and name.
fn hangout_label(hangout: &HangoutRow) -> String {
    if hangout.emoji.is_empty() { hangout.name.clone() } else { format!("{} {}", hangout.emoji, hangout.name) }
}

fn about_space(request: NotificationRequest, space: &SpaceRow) -> NotificationRequest {
    request
        .subject(SubjectType::SUBJECT_TYPE_SPACE, space.id)
        .data(serde_json::json!({ "space_id": space.id, "title": space.title }))
}

/// To a friend of the host: a friends room went live.
pub(crate) fn started(recipient: Uuid, host: &UserSummary, space: &SpaceRow) -> NotificationRequest {
    let name = name_of(host);
    let request = NotificationRequest::new(
        recipient,
        KIND_STARTED,
        Localized::new(format!("{name}がルームを始めたよ!"), format!("{name} started a room")),
        Localized::new(format!("「{}」をのぞいてみよう!", space.title), format!("“{}” — drop in!", space.title)),
        space_path(space.id),
    )
    .actor(space.host_id)
    .collapse(format!("spaces:started:{}", space.id));
    about_space(request, space)
}

/// To someone who asked to be reminded: the room is live now.
pub(crate) fn reminder(recipient: Uuid, host: &UserSummary, space: &SpaceRow) -> NotificationRequest {
    let name = name_of(host);
    let request = NotificationRequest::new(
        recipient,
        KIND_REMINDER,
        Localized::new("ルームが始まったよ!", "The room is starting!"),
        Localized::new(
            format!("{name}の「{}」が今スタート!", space.title),
            format!("{name}'s “{}” just went live", space.title),
        ),
        space_path(space.id),
    )
    .actor(space.host_id)
    .collapse(format!("spaces:started:{}", space.id));
    about_space(request, space)
}

/// To the host: the planned time of their room has come.
pub(crate) fn time_to_start(space: &SpaceRow) -> NotificationRequest {
    let request = NotificationRequest::new(
        space.host_id,
        KIND_REMINDER,
        Localized::new("ルームの時間だよ!", "It's time for your room!"),
        Localized::new(format!("「{}」を始めよう!", space.title), format!("Start “{}” now", space.title)),
        space_path(space.id),
    )
    .collapse(format!("spaces:nudge:{}", space.id));
    about_space(request, space)
}

/// To someone invited to a room.
pub(crate) fn room_invitation(
    recipient: Uuid,
    inviter: &UserSummary,
    inviter_id: Uuid,
    space: &SpaceRow,
) -> NotificationRequest {
    let name = name_of(inviter);
    let request = NotificationRequest::new(
        recipient,
        KIND_INVITED,
        Localized::new(format!("{name}がルームに招待したよ!"), format!("{name} invited you to a room")),
        Localized::new(format!("「{}」", space.title), format!("“{}”", space.title)),
        space_path(space.id),
    )
    .actor(inviter_id)
    .collapse(format!("spaces:invited:{}", space.id));
    about_space(request, space)
}

/// To someone invited to a hangout.
pub(crate) fn hangout_invitation(
    recipient: Uuid,
    inviter: &UserSummary,
    inviter_id: Uuid,
    hangout: &HangoutRow,
) -> NotificationRequest {
    let name = name_of(inviter);
    let label = hangout_label(hangout);
    NotificationRequest::new(
        recipient,
        KIND_INVITED,
        Localized::new(format!("{name}がたまり場に招待したよ!"), format!("{name} invited you to a hangout")),
        Localized::new(format!("「{label}」に参加しよう!"), format!("Join “{label}”")),
        hangout_path(hangout.id),
    )
    .actor(inviter_id)
    .collapse(format!("spaces:hangout-invited:{}", hangout.id))
    .data(serde_json::json!({ "hangout_id": hangout.id, "name": hangout.name }))
}

/// To a member who is a friend of the first person inside a hangout.
pub(crate) fn hangout_joined(
    recipient: Uuid,
    who: &UserSummary,
    who_id: Uuid,
    hangout: &HangoutRow,
) -> NotificationRequest {
    let name = name_of(who);
    let label = hangout_label(hangout);
    NotificationRequest::new(
        recipient,
        KIND_HANGOUT_JOINED,
        Localized::new(format!("{name}がたまり場にいるよ!"), format!("{name} is in the hangout")),
        Localized::new(format!("「{label}」で話そう!"), format!("Come talk in “{label}”")),
        hangout_path(hangout.id),
    )
    .actor(who_id)
    .collapse(format!("spaces:hangout:{}", hangout.id))
    .data(serde_json::json!({ "hangout_id": hangout.id, "name": hangout.name }))
}

/// The entry a public room leaves in its host's activity when it starts.
pub(crate) fn started_activity(space: &SpaceRow) -> ActivityEntry {
    ActivityEntry::new(
        space.host_id,
        ACTIVITY_STARTED,
        ActivityAudience::Public,
        Localized::new(format!("ルーム「{}」を始めたよ!", space.title), format!("Started a room: “{}”", space.title)),
    )
    .subject(SubjectType::SUBJECT_TYPE_SPACE, space.id)
    .path(space_path(space.id))
    .data(serde_json::json!({ "space_id": space.id, "tags": space.tags, "language": space.language }))
}

/// The search document of a room. Only public rooms that are upcoming or
/// live can be found; for anything else the document is removed.
pub(crate) fn search_doc(space: &SpaceRow) -> SearchDoc {
    let when = space.started_at.or(space.scheduled_at).unwrap_or(space.created_at);
    SearchDoc {
        kind: SEARCH_KIND.to_owned(),
        id: space.id,
        title: space.title.clone(),
        text: String::new(),
        tags: space.tags.clone(),
        owner_id: Some(space.host_id),
        public: space.is_public() && !space.is_over(),
        image_url: None,
        path: space_path(space.id),
        created_at: when.timestamp(),
        score: f64::from(space.listener_count.max(space.reminder_count)),
        extra: serde_json::json!({
            "state": space.state,
            "language": space.language,
            "scheduled_at": space.scheduled_at.map(|at| at.timestamp()),
            "started_at": space.started_at.map(|at| at.timestamp()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{audience, state};

    fn space() -> SpaceRow {
        SpaceRow {
            id: new_id(),
            host_id: new_id(),
            title: "放課後トーク".into(),
            tags: vec!["雑談".into()],
            language: "ja".into(),
            audience: audience::PUBLIC,
            state: state::LIVE,
            end_reason: 0,
            scheduled_at: None,
            started_at: Some(now()),
            ended_at: None,
            listener_count: 12,
            speaker_count: 2,
            hand_count: 0,
            peak_listeners: 15,
            total_listeners: 20,
            reminder_count: 3,
            pin_text: String::new(),
            pin_url: String::new(),
            pinned_by: None,
            pinned_at: None,
            host_absent_since: None,
            empty_since: None,
            deleted_at: None,
            created_at: now(),
        }
    }

    fn user(name: &str) -> UserSummary {
        UserSummary { id: new_id().to_string(), handle: "taro".into(), display_name: name.into(), ..Default::default() }
    }

    #[test]
    fn people_are_named_by_display_name_then_handle() {
        assert_eq!(name_of(&user("たろう")), "たろう");
        assert_eq!(name_of(&user("")), "@taro");
        assert_eq!(name_of(&UserSummary { deleted: true, ..user("x") }), "退会したユーザー");
    }

    #[test]
    fn notifications_point_at_the_room_and_name_the_actor() {
        let space = space();
        let recipient = new_id();
        let started = started(recipient, &user("たろう"), &space);
        assert_eq!(started.kind, "spaces.started");
        assert_eq!(started.title.ja, "たろうがルームを始めたよ!");
        assert_eq!(started.body.en, "“放課後トーク” — drop in!");
        assert_eq!(started.actor_id, Some(space.host_id));
        assert_eq!(started.path, format!("/spaces/{}", space.id));
        assert_eq!(started.subject, Some((SubjectType::SUBJECT_TYPE_SPACE as i32, space.id)));
        assert!(started.inbox && started.push && !started.urgent);

        // Someone who is both a friend and asked for a reminder hears of it once.
        let reminder = reminder(recipient, &user("たろう"), &space);
        assert_eq!(reminder.kind, "spaces.reminder");
        assert_eq!(reminder.collapse_key, started.collapse_key);

        let nudge = time_to_start(&space);
        assert_eq!(
            (nudge.user_id, nudge.actor_id),
            (space.host_id, None),
            "from the system, so it is not dropped as the host's own doing"
        );
        let inviter = new_id();
        let invitation = room_invitation(recipient, &user("はなこ"), inviter, &space);
        assert_eq!((invitation.kind.as_str(), invitation.actor_id), ("spaces.invited", Some(inviter)));
        assert_eq!(invitation.data["space_id"], space.id.to_string());
    }

    #[test]
    fn hangout_notifications_show_emoji_and_name() {
        let hangout = HangoutRow {
            id: new_id(),
            owner_id: new_id(),
            name: "いつメン".into(),
            emoji: "🛋️".into(),
            members_can_invite: true,
            member_count: 3,
            inside_count: 1,
            last_active_at: None,
            last_announced_at: None,
            created_at: now(),
        };
        let who = new_id();
        let joined = hangout_joined(new_id(), &user("はなこ"), who, &hangout);
        assert_eq!(joined.kind, "spaces.hangout_joined");
        assert_eq!(joined.title.ja, "はなこがたまり場にいるよ!");
        assert_eq!(joined.body.ja, "「🛋️ いつメン」で話そう!");
        assert_eq!(joined.collapse_key.as_deref(), Some(format!("spaces:hangout:{}", hangout.id).as_str()));
        assert_eq!(joined.path, format!("/hangouts/{}", hangout.id));
        let invited = hangout_invitation(new_id(), &user("はなこ"), who, &hangout);
        assert_eq!(invited.kind, "spaces.invited");
        assert_eq!(invited.data["hangout_id"], hangout.id.to_string());
    }

    #[test]
    fn only_public_rooms_that_can_still_be_entered_are_searchable() {
        let live = space();
        let doc = search_doc(&live);
        assert!(doc.public);
        assert_eq!((doc.kind.as_str(), doc.title.as_str(), doc.score), ("space", "放課後トーク", 12.0));
        assert_eq!(doc.owner_id, Some(live.host_id));
        assert!(search_doc(&SpaceRow { state: state::SCHEDULED, ..live.clone() }).public);
        assert!(!search_doc(&SpaceRow { state: state::ENDED, ..live.clone() }).public);
        assert!(!search_doc(&SpaceRow { state: state::CANCELLED, ..live.clone() }).public);
        assert!(!search_doc(&SpaceRow { audience: audience::FRIENDS, ..live.clone() }).public);
        let activity = started_activity(&live);
        assert_eq!((activity.kind.as_str(), activity.audience), ("spaces.started", ActivityAudience::Public));
    }
}
