//! Stored rooms and hangouts → API messages, per viewer.
//!
//! A room or hangout is rendered either for one person (their own relation
//! to it filled in as `viewer`) or for nobody in particular (realtime
//! pushes, which many people receive).
use std::collections::HashMap;

use tokuly_core::prelude::*;
use tokuly_identity::users;
use tokuly_proto::{common::UserSummary, spaces as pb};

use crate::{
    Spaces,
    model::{HANGOUT, HangoutRow, PARTICIPANT, ParticipantRow, SpaceRow, end, member, role},
    rules,
};

/// Stage members shown with a room in lists (the detail view has all).
const STAGE_PREVIEW: i64 = 4;
/// Friends shown with a room in "live among friends".
const FRIENDS_PREVIEW: usize = 5;

/// What a list of rooms carries besides the rooms themselves.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Detail<'a> {
    /// Every stage member rather than a preview.
    pub full_stage: bool,
    /// Friends of the viewer inside each room, by room.
    pub friends_inside: Option<&'a HashMap<Uuid, Vec<Uuid>>>,
}

#[derive(sqlx::FromRow)]
struct ViewerFacts {
    space_id: Uuid,
    role: Option<i16>,
    inside: bool,
    hand_raised: bool,
    stage_invited: bool,
    reminder_set: bool,
    invited: bool,
}

/// A hangout member with whether they are in the voice room.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct MemberView {
    pub hangout_id: Uuid,
    pub user_id: Uuid,
    pub state: i16,
    pub invited_by: Option<Uuid>,
    pub muted: bool,
    pub joined_at: Option<Time>,
    pub created_at: Time,
    pub inside_since: Option<Time>,
}

fn participant_proto(
    row: &ParticipantRow,
    users: &HashMap<Uuid, UserSummary>,
    show_invitation: bool,
) -> pb::SpaceParticipant {
    // Of someone who is gone, the record says what they were at most.
    let role = if row.is_inside() { row.role } else { row.top_role };
    pb::SpaceParticipant {
        user: users::pick(users, row.user_id).into(),
        role: i32::from(role).into(),
        hand_raised_at: opt_ts(row.hand_raised_at),
        stage_invited: show_invitation && row.stage_invited_at.is_some(),
        joined_at: ts(row.joined_at),
        left_at: opt_ts(row.left_at),
        leave_reason: i32::from(row.leave_reason).into(),
        ..Default::default()
    }
}

impl Spaces {
    /// One participant. `show_invitation` reveals a pending stage
    /// invitation: only for moderators and the person themselves.
    pub(crate) async fn render_participant(
        &self,
        row: &ParticipantRow,
        show_invitation: bool,
    ) -> AppResult<pb::SpaceParticipant> {
        let users = tokuly_identity::summaries(&self.core, &[row.user_id]).await?;
        Ok(participant_proto(row, &users, show_invitation))
    }

    pub(crate) async fn render_participants(
        &self,
        rows: &[ParticipantRow],
        viewer: Uuid,
        viewer_moderates: bool,
    ) -> AppResult<Vec<pb::SpaceParticipant>> {
        let ids: Vec<Uuid> = rows.iter().map(|row| row.user_id).collect();
        let users = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(rows.iter().map(|row| participant_proto(row, &users, viewer_moderates || row.user_id == viewer)).collect())
    }

    /// One room in full, as `viewer` sees it (`None`: as nobody in
    /// particular, for pushes and anonymous readers).
    pub(crate) async fn render_space(&self, space: &SpaceRow, viewer: Option<Uuid>) -> AppResult<pb::Space> {
        let detail = Detail { full_stage: true, friends_inside: None };
        let mut rendered = self.render_spaces(std::slice::from_ref(space), viewer, detail).await?;
        Ok(rendered.pop().expect("one room in, one room out"))
    }

    /// Several rooms, in the order given.
    pub(crate) async fn render_spaces(
        &self,
        spaces: &[SpaceRow],
        viewer: Option<Uuid>,
        detail: Detail<'_>,
    ) -> AppResult<Vec<pb::Space>> {
        if spaces.is_empty() {
            return Ok(Vec::new());
        }
        let db = &self.core.db;
        let ids: Vec<Uuid> = spaces.iter().map(|space| space.id).collect();

        let stage: Vec<ParticipantRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {PARTICIPANT} FROM (
                 SELECT q.*, row_number() OVER (PARTITION BY q.space_id ORDER BY q.role DESC, q.joined_at, q.user_id) AS position
                 FROM space_participants q
                 WHERE q.space_id = ANY($1) AND q.left_at IS NULL AND q.role >= $2
             ) p
             WHERE p.position <= $3
             ORDER BY p.space_id, p.position"
        )))
        .bind(&ids)
        .bind(role::SPEAKER)
        .bind(if detail.full_stage { i64::MAX } else { STAGE_PREVIEW })
        .fetch_all(db)
        .await?;

        let facts: HashMap<Uuid, ViewerFacts> = match viewer {
            None => HashMap::new(),
            Some(viewer) => sqlx::query_as::<_, ViewerFacts>(
                "SELECT s.id AS space_id, p.role,
                        (p.user_id IS NOT NULL AND p.left_at IS NULL) AS inside,
                        (p.left_at IS NULL AND p.hand_raised_at IS NOT NULL) AS hand_raised,
                        (p.left_at IS NULL AND p.stage_invited_at IS NOT NULL) AS stage_invited,
                        EXISTS (SELECT 1 FROM space_reminders r WHERE r.space_id = s.id AND r.user_id = $2) AS reminder_set,
                        EXISTS (SELECT 1 FROM space_invites i
                                WHERE i.space_id = s.id AND i.user_id = $2 AND i.revoked_at IS NULL) AS invited
                 FROM unnest($1::uuid[]) AS s(id)
                 LEFT JOIN space_participants p ON p.space_id = s.id AND p.user_id = $2",
            )
            .bind(&ids)
            .bind(viewer)
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|facts| (facts.space_id, facts))
            .collect(),
        };

        let no_friends = HashMap::new();
        let friends_inside = detail.friends_inside.unwrap_or(&no_friends);

        let mut user_ids: Vec<Uuid> = spaces.iter().map(|space| space.host_id).collect();
        user_ids.extend(stage.iter().map(|row| row.user_id));
        user_ids.extend(friends_inside.values().flat_map(|friends| friends.iter().take(FRIENDS_PREVIEW)));
        user_ids.sort_unstable();
        user_ids.dedup();
        let users = tokuly_identity::summaries(&self.core, &user_ids).await?;

        Ok(spaces
            .iter()
            .map(|space| {
                let viewer_proto = viewer.map(|viewer| {
                    let facts = facts.get(&space.id);
                    let inside = facts.is_some_and(|facts| facts.inside);
                    let held = facts.and_then(|facts| facts.role).unwrap_or(role::LISTENER);
                    let role = if viewer == space.host_id {
                        role::HOST
                    } else if inside || rules::is_moderator(held) {
                        held.min(role::COHOST)
                    } else {
                        role::LISTENER
                    };
                    pb::SpaceViewer {
                        inside,
                        role: i32::from(role).into(),
                        hand_raised: facts.is_some_and(|facts| facts.hand_raised),
                        stage_invited: facts.is_some_and(|facts| facts.stage_invited),
                        reminder_set: facts.is_some_and(|facts| facts.reminder_set),
                        invited: facts.is_some_and(|facts| facts.invited),
                        ..Default::default()
                    }
                });
                pb::Space {
                    id: space.id.to_string(),
                    title: space.title.clone(),
                    tags: space.tags.clone(),
                    language: space.language.clone(),
                    audience: i32::from(space.audience).into(),
                    state: i32::from(space.state).into(),
                    host: users::pick(&users, space.host_id).into(),
                    scheduled_at: opt_ts(space.scheduled_at),
                    started_at: opt_ts(space.started_at),
                    ended_at: opt_ts(space.ended_at),
                    // That an account was closed is the account's business.
                    end_reason: match space.end_reason {
                        end::ACCOUNT_CLOSED if viewer != Some(space.host_id) => i32::from(end::HOST_LEFT).into(),
                        reason => i32::from(reason).into(),
                    },
                    listener_count: space.listener_count,
                    speaker_count: space.speaker_count,
                    hand_count: space.hand_count,
                    peak_listeners: space.peak_listeners,
                    total_listeners: space.total_listeners,
                    max_speakers: self.cfg.max_speakers,
                    pin: space.pin().into(),
                    stage: stage
                        .iter()
                        .filter(|row| row.space_id == space.id)
                        .map(|row| participant_proto(row, &users, false))
                        .collect(),
                    reminder_count: space.reminder_count,
                    created_at: ts(space.created_at),
                    viewer: viewer_proto.into(),
                    friends_inside: friends_inside
                        .get(&space.id)
                        .map(|friends| {
                            friends.iter().take(FRIENDS_PREVIEW).map(|friend| users::pick(&users, *friend)).collect()
                        })
                        .unwrap_or_default(),
                    ..Default::default()
                }
            })
            .collect())
    }

    // ───────────────────────────── hangouts ─────────────────────────────

    /// Everyone who is part of the given hangouts: members first (those in
    /// the voice room leading), then open invitations.
    pub(crate) async fn hangout_members(&self, hangout_ids: &[Uuid]) -> AppResult<Vec<MemberView>> {
        Ok(sqlx::query_as(
            "SELECT m.hangout_id, m.user_id, m.state, m.invited_by, m.muted, m.joined_at, m.created_at,
                    pr.joined_at AS inside_since
             FROM hangout_members m
             LEFT JOIN hangout_presence pr ON pr.hangout_id = m.hangout_id AND pr.user_id = m.user_id
             WHERE m.hangout_id = ANY($1) AND m.state <> $3
             ORDER BY m.hangout_id, (m.state = $2) DESC, (pr.user_id IS NOT NULL) DESC, pr.joined_at,
                      COALESCE(m.joined_at, m.created_at), m.user_id",
        )
        .bind(hangout_ids)
        .bind(member::MEMBER)
        .bind(member::FORMER)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// One hangout as `viewer` sees it (`None`: as nobody in particular).
    pub(crate) async fn render_hangout(&self, hangout: &HangoutRow, viewer: Option<Uuid>) -> AppResult<pb::Hangout> {
        let mut rendered = self.render_hangouts(std::slice::from_ref(hangout), viewer).await?;
        Ok(rendered.pop().expect("one hangout in, one hangout out"))
    }

    /// Several hangouts, in the order given.
    pub(crate) async fn render_hangouts(
        &self,
        hangouts: &[HangoutRow],
        viewer: Option<Uuid>,
    ) -> AppResult<Vec<pb::Hangout>> {
        if hangouts.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = hangouts.iter().map(|hangout| hangout.id).collect();
        let members = self.hangout_members(&ids).await?;
        let mut user_ids: Vec<Uuid> = hangouts.iter().map(|hangout| hangout.owner_id).collect();
        user_ids.extend(members.iter().map(|member| member.user_id));
        user_ids.sort_unstable();
        user_ids.dedup();
        let users = tokuly_identity::summaries(&self.core, &user_ids).await?;

        Ok(hangouts
            .iter()
            .map(|hangout| {
                let of_hangout = || members.iter().filter(|member| member.hangout_id == hangout.id);
                let me = viewer.and_then(|viewer| of_hangout().find(|member| member.user_id == viewer));
                // Someone who is only invited sees who they would be joining, not who else was asked.
                let sees_invitations = viewer.is_none() || me.is_some_and(|me| me.state == member::MEMBER);
                pb::Hangout {
                    id: hangout.id.to_string(),
                    name: hangout.name.clone(),
                    emoji: hangout.emoji.clone(),
                    owner: users::pick(&users, hangout.owner_id).into(),
                    members_can_invite: hangout.members_can_invite,
                    member_count: hangout.member_count,
                    inside_count: hangout.inside_count,
                    members: of_hangout()
                        .filter(|member| member.state == member::MEMBER || sees_invitations)
                        .map(|member| pb::HangoutMember {
                            user: users::pick(&users, member.user_id).into(),
                            state: i32::from(member.state).into(),
                            owner: member.user_id == hangout.owner_id,
                            inside: member.inside_since.is_some(),
                            inside_since: opt_ts(member.inside_since),
                            // Who brought whom in is the members' business.
                            invited_by_id: member
                                .invited_by
                                .filter(|_| sees_invitations)
                                .map(|id| id.to_string())
                                .unwrap_or_default(),
                            joined_at: ts(member.joined_at.unwrap_or(member.created_at)),
                            ..Default::default()
                        })
                        .collect(),
                    last_active_at: opt_ts(hangout.last_active_at),
                    created_at: ts(hangout.created_at),
                    viewer: me
                        .map(|me| {
                            let is_member = me.state == member::MEMBER;
                            let owner = me.user_id == hangout.owner_id;
                            pb::HangoutViewer {
                                state: i32::from(me.state).into(),
                                owner,
                                inside: me.inside_since.is_some(),
                                muted: me.muted,
                                can_invite: is_member && (owner || hangout.members_can_invite),
                                ..Default::default()
                            }
                        })
                        .into(),
                    ..Default::default()
                }
            })
            .collect())
    }

    /// A hangout by ID, rendered for nobody in particular; `None` when it
    /// is gone.
    pub(crate) async fn hangout_snapshot(&self, hangout_id: Uuid) -> AppResult<Option<(HangoutRow, pb::Hangout)>> {
        let hangout: Option<HangoutRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {HANGOUT} FROM hangouts h WHERE h.id = $1 AND h.deleted_at IS NULL"
        )))
        .bind(hangout_id)
        .fetch_optional(&self.core.db)
        .await?;
        match hangout {
            Some(hangout) => {
                let proto = self.render_hangout(&hangout, None).await?;
                Ok(Some((hangout, proto)))
            }
            None => Ok(None),
        }
    }
}
