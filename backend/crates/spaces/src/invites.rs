//! Invitations to rooms and "remind me when it starts".
//!
//! An invitation is the way into a friends or invited-only room for
//! someone who would not get in otherwise, and a nudge for everyone else.
//! People can only invite their own friends, so an invitation never
//! reaches a stranger; and someone who keeps being invited by the same
//! person is only told the first few times (see [`crate::limits`]).
//!
//! An invitation that is taken back opens no door any more, but its row
//! stays (`revoked_at`): whoever received it can still report the room.
use tokuly_core::{outbound, prelude::*};
use tokuly_identity::users;
use tokuly_proto::{
    common::{PageInfo, UserSummary},
    spaces as pb,
};
use tokuly_social::graph;

use crate::{
    Spaces,
    limits::Limit,
    model::{SPACE, SpaceRow},
    notices,
    participants::not_moderator,
    rooms::{Tx, lock_space, space_ended},
};

/// People one call can invite.
pub(crate) const MAX_INVITES_PER_CALL: usize = 50;
/// Invitations a room can hold.
const MAX_INVITES_PER_ROOM: i64 = 500;

/// Who a call invited.
#[derive(Debug, Clone, Default)]
pub(crate) struct Invited {
    /// Everyone who is newly invited.
    pub all: Vec<Uuid>,
    /// Those among them who were told (a notification was requested; the
    /// realtime push follows the commit). The rest were invited quietly:
    /// the inviter has rung them often enough for today.
    pub announced: Vec<Uuid>,
}

fn not_friends() -> AppError {
    AppError::precondition("NOT_FRIENDS", "you can only invite your friends")
}

impl Spaces {
    /// Parses and checks a list of people `inviter` wants to invite to
    /// something `owner` runs: up to 50 at a time, each a friend of the
    /// inviter. The inviter and the owner themselves are dropped from the
    /// list, and so — without a word — is anyone on either side of a block
    /// with the owner: saying so would tell the inviter about a block
    /// between two other people.
    pub(crate) async fn invitable(&self, inviter: Uuid, owner: Uuid, raw_ids: &[String]) -> AppResult<Vec<Uuid>> {
        let mut ids = parse_ids(raw_ids)?;
        ids.sort_unstable();
        ids.dedup();
        ids.retain(|id| *id != inviter && *id != owner);
        if ids.is_empty() {
            return Ok(ids);
        }
        ensure!(
            ids.len() <= MAX_INVITES_PER_CALL,
            invalid,
            "TOO_MANY_INVITES",
            "invite at most {MAX_INVITES_PER_CALL} people at a time"
        );
        let friends = graph::friend_ids(&self.core, inviter).await?;
        if !ids.iter().all(|id| friends.contains(id)) {
            return Err(not_friends());
        }
        if owner != inviter {
            let hidden = graph::blocked_ids_either(&self.core, owner).await?;
            ids.retain(|id| !hidden.contains(id));
        }
        Ok(ids)
    }

    /// Stores invitations to a room, in the caller's transaction, and
    /// requests a notification for those the inviter may still ring (see
    /// [`Spaces::may_announce_invitation`]; that budget lives in the cache,
    /// so asking it does not need a second database connection). People who
    /// are banned from the room or invited already are skipped.
    pub(crate) async fn invite_in_tx(
        &self,
        tx: &mut Tx<'_>,
        space: &SpaceRow,
        inviter_id: Uuid,
        inviter: &UserSummary,
        invitees: &[Uuid],
    ) -> AppResult<Invited> {
        if invitees.is_empty() {
            return Ok(Invited::default());
        }
        // An invitation that was taken back comes to life again.
        let invited: Vec<Uuid> = sqlx::query_scalar(
            "INSERT INTO space_invites AS i (space_id, user_id, invited_by)
             SELECT $1, invitee, $3 FROM unnest($2::uuid[]) AS invitee
             WHERE NOT EXISTS (SELECT 1 FROM space_bans b WHERE b.space_id = $1 AND b.user_id = invitee)
             ON CONFLICT (space_id, user_id) DO UPDATE
                 SET invited_by = EXCLUDED.invited_by, created_at = now(), revoked_at = NULL
                 WHERE i.revoked_at IS NOT NULL
             RETURNING user_id",
        )
        .bind(space.id)
        .bind(invitees)
        .bind(inviter_id)
        .fetch_all(&mut **tx)
        .await?;
        let open: i64 =
            sqlx::query_scalar("SELECT count(*) FROM space_invites WHERE space_id = $1 AND revoked_at IS NULL")
                .bind(space.id)
                .fetch_one(&mut **tx)
                .await?;
        ensure!(
            open <= MAX_INVITES_PER_ROOM,
            precondition,
            "TOO_MANY_INVITES",
            "a room holds at most {MAX_INVITES_PER_ROOM} invitations"
        );
        let mut announced = Vec::with_capacity(invited.len());
        for invitee in &invited {
            if self.may_announce_invitation(inviter_id, *invitee).await? {
                outbound::notify(&mut **tx, &notices::room_invitation(*invitee, inviter, inviter_id, space)).await?;
                announced.push(*invitee);
            }
        }
        Ok(Invited { all: invited, announced })
    }

    /// Tells invited people over realtime, once the invitations are
    /// committed.
    pub(crate) async fn announce_invitations(&self, space: &SpaceRow, inviter: &UserSummary, invitees: &[Uuid]) {
        if invitees.is_empty() {
            return;
        }
        match self.render_space(space, None).await {
            Ok(proto) => {
                let invitation = pb::SpaceInvitation {
                    space: proto.into(),
                    invited_by: inviter.clone().into(),
                    ..Default::default()
                };
                self.push_users(invitees.iter().copied(), invitation).await;
            }
            Err(e) => tracing::warn!(space_id = %space.id, error = %e, "invitation push failed"),
        }
    }

    /// Invites friends of the caller to a room. Returns how many new
    /// invitations went out.
    pub(crate) async fn invite_to_space(&self, user_id: Uuid, space_id: Uuid, raw_ids: &[String]) -> AppResult<i32> {
        let core = &self.core;
        let space = self.visible_space(Some(user_id), space_id).await?;
        if space.is_over() {
            return Err(space_ended());
        }
        // A public room is everybody's to share; the others are opened by
        // their moderators only.
        let allowed = self.is_moderator(&space, user_id).await?
            || (space.is_public()
                && sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS (SELECT 1 FROM space_participants WHERE space_id = $1 AND user_id = $2 AND left_at IS NULL)",
                )
                .bind(space_id)
                .bind(user_id)
                .fetch_one(&core.db)
                .await?);
        if !allowed {
            return Err(not_moderator());
        }
        let invitees = self.invitable(user_id, space.host_id, raw_ids).await?;
        if invitees.is_empty() {
            return Ok(0);
        }
        self.limit(Limit::InviteToRoom, user_id).await?;
        let inviter = tokuly_identity::summary(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        let locked = lock_space(&mut tx, space_id).await?;
        if locked.is_over() {
            return Err(space_ended());
        }
        let invited = self.invite_in_tx(&mut tx, &locked, user_id, &inviter, &invitees).await?;
        tx.commit().await?;
        self.announce_invitations(&locked, &inviter, &invited.announced).await;
        Ok(invited.all.len() as i32)
    }

    /// Takes an invitation back.
    pub(crate) async fn revoke_space_invite(&self, user_id: Uuid, space_id: Uuid, target: Uuid) -> AppResult<()> {
        let space = self.visible_space(Some(user_id), space_id).await?;
        self.require_moderator(&space, user_id).await?;
        self.limit(Limit::Moderate, user_id).await?;
        sqlx::query(
            "UPDATE space_invites SET revoked_at = now() WHERE space_id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(space_id)
        .bind(target)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// The people invited to a room, newest first.
    pub(crate) async fn list_space_invites(
        &self,
        user_id: Uuid,
        space_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<pb::SpaceInvite>, PageInfo)> {
        let space = self.visible_space(Some(user_id), space_id).await?;
        self.require_moderator(&space, user_id).await?;
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<(Uuid, Uuid, Time)> = sqlx::query_as(
            "SELECT user_id, invited_by, created_at FROM space_invites
             WHERE space_id = $1 AND revoked_at IS NULL AND ($2::timestamptz IS NULL OR (created_at, user_id) < ($2, $3))
             ORDER BY created_at DESC, user_id DESC
             LIMIT $4",
        )
        .bind(space_id)
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |(user_id, _, at)| (*at, *user_id));
        let ids: Vec<Uuid> = rows.iter().map(|(user_id, _, _)| *user_id).collect();
        let people = tokuly_identity::summaries(&self.core, &ids).await?;
        let invites = rows
            .iter()
            .map(|(user_id, invited_by, at)| pb::SpaceInvite {
                user: users::pick(&people, *user_id).into(),
                invited_by_id: invited_by.to_string(),
                invited_at: ts(*at),
                ..Default::default()
            })
            .collect();
        Ok((invites, info))
    }

    /// Asks to be told when an upcoming room starts, or takes that back.
    pub(crate) async fn set_reminder(&self, user_id: Uuid, space_id: Uuid, remind: bool) -> AppResult<SpaceRow> {
        let space = self.visible_space(Some(user_id), space_id).await?;
        let not_upcoming = || AppError::precondition("SPACE_NOT_SCHEDULED", "only an upcoming room can remind you");
        if !space.is_scheduled() {
            return Err(not_upcoming());
        }
        self.limit(Limit::Reminder, user_id).await?;
        // Under the room's lock, so that the count on the room is the
        // number of reminders whoever else sets one at the same moment.
        let mut tx = self.core.db.begin().await?;
        if !lock_space(&mut tx, space_id).await?.is_scheduled() {
            return Err(not_upcoming());
        }
        if remind {
            sqlx::query("INSERT INTO space_reminders (space_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(space_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("DELETE FROM space_reminders WHERE space_id = $1 AND user_id = $2")
                .bind(space_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        let space: SpaceRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE spaces s SET reminder_count = (SELECT count(*) FROM space_reminders r WHERE r.space_id = s.id)
             WHERE s.id = $1
             RETURNING {SPACE}"
        )))
        .bind(space_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(space)
    }
}
