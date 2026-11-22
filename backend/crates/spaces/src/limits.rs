//! Rate limits: everything one account can repeat to bother other people
//! or to keep the server busy has one.
//!
//! Three kinds live here:
//!
//! - **Per person** ([`Spaces::limit`]): the caller is refused with
//!   `RATE_LIMITED` once they are over the budget of a [`Limit`].
//! - **Per pair** ([`Spaces::may_announce_invitation`],
//!   [`Spaces::limit_stage_invitation`]): how often one person may reach
//!   the same other person. Inviting someone, taking the invitation back
//!   and inviting again must not ring their phone every time.
//! - **Per room** ([`Spaces::room_budget`]): fan-out to everyone in a room
//!   (emoji bursts, raised hands) is thinned out when a large room produces
//!   more than anyone could watch.
//!
//! A deployment can switch all of them off (`TOKULY_RATE_LIMITS=false`).
use std::time::Duration;

use tokuly_core::prelude::*;

use crate::Spaces;

/// A per-person budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Limit {
    /// Creating rooms, scheduled or not.
    CreateRoom,
    /// Taking a room live: friends may be told, the activity feed is.
    GoLive,
    /// Changing a room; everyone looking at it is pushed the change.
    UpdateRoom,
    /// Entering rooms and hangouts: the people inside are told.
    Enter,
    /// Raising and lowering a hand: the room is told.
    Hand,
    /// What moderators do to people and the stage; each reaches the room,
    /// the person and LiveKit.
    Moderate,
    /// Calls that invite people to a room.
    InviteToRoom,
    /// Pinning; the room is pushed every change.
    Pin,
    /// Setting and clearing reminders.
    Reminder,
    /// Lists and rankings, which take several queries each.
    Discover,
    /// Creating hangouts.
    CreateHangout,
    /// Calls that invite people to a hangout.
    InviteToHangout,
    /// Changing a hangout and its members; the members are pushed each change.
    ManageHangout,
}

impl Limit {
    /// Name in the key, how many, in how long.
    fn budget(self) -> (&'static str, u32, Duration) {
        const MINUTE: Duration = Duration::from_secs(60);
        const HOUR: Duration = Duration::from_secs(3600);
        const DAY: Duration = Duration::from_secs(86400);
        match self {
            Self::CreateRoom => ("create", 20, HOUR),
            Self::GoLive => ("live", 6, HOUR),
            Self::UpdateRoom => ("update", 30, MINUTE),
            Self::Enter => ("join", 30, MINUTE),
            Self::Hand => ("hand", 10, MINUTE),
            Self::Moderate => ("moderate", 120, MINUTE),
            Self::InviteToRoom => ("invite", 30, HOUR),
            Self::Pin => ("pin", 20, MINUTE),
            Self::Reminder => ("reminder", 60, MINUTE),
            Self::Discover => ("discover", 120, MINUTE),
            Self::CreateHangout => ("hangout-create", 10, DAY),
            Self::InviteToHangout => ("hangout-invite", 60, HOUR),
            Self::ManageHangout => ("hangout-manage", 60, MINUTE),
        }
    }
}

/// Times a day one person's invitations are announced to the same other
/// person (rooms and hangouts together). Further invitations still count;
/// they just arrive quietly.
const INVITATION_ANNOUNCEMENTS_PER_DAY: u32 = 3;
/// Stage invitations one moderator may send the same listener per minute.
const STAGE_INVITATIONS_PER_MINUTE: u32 = 4;
/// Emoji bursts a room passes on in ten seconds, whoever sends them.
const ROOM_REACTIONS_PER_10S: u32 = 300;
/// Raised and lowered hands a room announces one by one in ten seconds
/// (the count of raised hands always follows).
const ROOM_HANDS_PER_10S: u32 = 100;

/// A fan-out to everyone in a room that is thinned out when there is too
/// much of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoomFanout {
    Reactions,
    Hands,
}

impl Spaces {
    /// Counts one use of `limit` by `who`; `RATE_LIMITED` when they are
    /// over it.
    pub(crate) async fn limit(&self, limit: Limit, who: Uuid) -> AppResult<()> {
        let (name, max, window) = limit.budget();
        self.core.cache.rate_limit(&format!("spaces-{name}:{who}"), max, window).await
    }

    /// [`Limit::Discover`] for callers who may be anonymous: per account,
    /// or per client address.
    pub(crate) async fn limit_discovery(&self, viewer: Option<Uuid>, client_ip: &str) -> AppResult<()> {
        let (name, max, window) = Limit::Discover.budget();
        let who = match viewer {
            Some(viewer) => viewer.to_string(),
            None => format!("ip:{client_ip}"),
        };
        self.core.cache.rate_limit(&format!("spaces-{name}:{who}"), max, window).await
    }

    /// Counts against a budget that thins things out instead of refusing:
    /// `true` while there is budget left.
    async fn within(&self, key: &str, max: u32, window: Duration) -> AppResult<bool> {
        match self.core.cache.rate_limit(key, max, window).await {
            Ok(()) => Ok(true),
            Err(e) if e.reason == "RATE_LIMITED" => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Whether an invitation from `inviter` to `invitee` should still be
    /// announced (notification and realtime) today.
    pub(crate) async fn may_announce_invitation(&self, inviter: Uuid, invitee: Uuid) -> AppResult<bool> {
        let key = format!("spaces-invite-pair:{inviter}:{invitee}");
        self.within(&key, INVITATION_ANNOUNCEMENTS_PER_DAY, Duration::from_secs(86400)).await
    }

    /// `RATE_LIMITED` when `moderator` keeps inviting the same listener on
    /// stage (each invitation interrupts them).
    pub(crate) async fn limit_stage_invitation(&self, moderator: Uuid, listener: Uuid) -> AppResult<()> {
        let key = format!("spaces-stage-pair:{moderator}:{listener}");
        self.core.cache.rate_limit(&key, STAGE_INVITATIONS_PER_MINUTE, Duration::from_secs(60)).await
    }

    /// Whether a room still passes on one more of `fanout` right now.
    pub(crate) async fn room_budget(&self, space_id: Uuid, fanout: RoomFanout) -> bool {
        let (name, max) = match fanout {
            RoomFanout::Reactions => ("react", ROOM_REACTIONS_PER_10S),
            RoomFanout::Hands => ("hands", ROOM_HANDS_PER_10S),
        };
        // A cache that cannot be reached must not silence a room.
        self.within(&format!("spaces-room-{name}:{space_id}"), max, Duration::from_secs(10)).await.unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_budget_has_a_key_of_its_own() {
        let limits = [
            Limit::CreateRoom,
            Limit::GoLive,
            Limit::UpdateRoom,
            Limit::Enter,
            Limit::Hand,
            Limit::Moderate,
            Limit::InviteToRoom,
            Limit::Pin,
            Limit::Reminder,
            Limit::Discover,
            Limit::CreateHangout,
            Limit::InviteToHangout,
            Limit::ManageHangout,
        ];
        let mut names: Vec<&str> = limits.iter().map(|limit| limit.budget().0).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), limits.len());
        assert!(limits.iter().all(|limit| limit.budget().1 > 0 && !limit.budget().2.is_zero()));
    }
}
