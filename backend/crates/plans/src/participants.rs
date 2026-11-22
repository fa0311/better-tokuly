//! Who is in a plan: invitations, answers (RSVP), the waitlist, roles,
//! leaving and being removed.
//!
//! Everything that changes who is in a plan, or who is going, runs under the
//! plan's row lock (see [`load_access`]), so two people answering at the same
//! moment cannot both take the last place. While that lock is held nothing
//! is asked through the connection pool: what the social domain has to say
//! (friends, blocks) is asked before the transaction starts.
use std::{collections::HashSet, time::Duration};

use connectrpc::ErrorCode;

use sqlx::PgConnection;
use tokuly_core::{
    events as bus,
    outbound::{self, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::SubjectType,
    plans::{plans_event::Event as Push, *},
};
use tokuly_social::graph;

use crate::{
    Plans, copy,
    events::{PLAN_RSVP_CHANGED, PlanRsvpChanged as RsvpChangedEvent},
    input, people,
    plan::{
        Access, ParticipantRow, PlanRow, counts_of, load_access, participant_columns, participant_ids, plan_changed,
        plan_path,
    },
};

const MAX_COMMENT: usize = 140;
/// Calls to `InviteToPlan` one account may make in an hour (each friend is
/// also limited on their own, see [`people::limit_invitations`]).
const INVITE_CALLS_PER_HOUR: u32 = 60;
/// Answers (RSVP) one account may give: every one of them reaches the host.
const ANSWERS_PER_MINUTE: u32 = 12;
const ANSWERS_PER_HOUR: u32 = 60;

/// Hosts always may invite; a guest only when the host allows it, and not
/// after declining. The plan must not be cancelled.
fn ensure_may_invite(access: &Access) -> AppResult<()> {
    access.ensure_active()?;
    let may_invite =
        access.is_organizer() || (access.plan.guests_can_invite && access.me.rsvp() != Rsvp::RSVP_DECLINED);
    ensure!(may_invite, denied, "CANNOT_INVITE", "you cannot invite people to this plan");
    Ok(())
}

/// The plan changed hands between the first look and the lock; the caller
/// simply tries again.
fn changed() -> AppError {
    AppError::new(ErrorCode::Aborted, "PLAN_CHANGED", "the plan changed while it was being processed; try again")
}

fn code(rsvp: Rsvp) -> i16 {
    rsvp as i32 as i16
}

impl Plans {
    /// Adds `invitees` (friends of `inviter`, not yet in the plan) as guests
    /// who have not answered, and tells them.
    pub(crate) async fn invite(
        &self,
        conn: &mut PgConnection,
        plan: &PlanRow,
        inviter: Uuid,
        invitees: &[Uuid],
    ) -> AppResult<()> {
        if invitees.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO plan_participants (plan_id, user_id, role, rsvp, invited_by)
             SELECT $1, invitee, $3, $4, $5 FROM unnest($2::uuid[]) AS invitee",
        )
        .bind(plan.id)
        .bind(invitees)
        .bind(PlanRole::PLAN_ROLE_GUEST as i32 as i16)
        .bind(code(Rsvp::RSVP_INVITED))
        .bind(inviter)
        .execute(&mut *conn)
        .await?;
        let by = people::name_on(conn, inviter).await?;
        let text = copy::invited(&by, &plan.name(), &plan.schedule().text());
        for invitee in invitees {
            let request = NotificationRequest::new(
                *invitee,
                "plans.invited",
                text.title.clone(),
                text.body.clone(),
                plan_path(plan.id),
            )
            .actor(inviter)
            .subject(SubjectType::SUBJECT_TYPE_PLAN, plan.id)
            .collapse(format!("plans.invited:{}", plan.id));
            outbound::notify(&mut *conn, &request).await?;
        }
        Ok(())
    }

    /// Invites more friends of `user_id` to a plan. Returns the plan and how
    /// many people were newly invited.
    pub(crate) async fn invite_to_plan(
        &self,
        user_id: Uuid,
        plan_id: Uuid,
        user_ids: &[String],
    ) -> AppResult<(Plan, i32)> {
        let core = &self.core;
        core.cache
            .rate_limit(&format!("plans:invite:{user_id}"), INVITE_CALLS_PER_HOUR, Duration::from_secs(3600))
            .await?;
        let wanted = input::people(user_ids, user_id)?;
        people::require_friends(core, user_id, &wanted).await?;

        // A first look without a lock: who hosts, who is in the plan already.
        // What the social domain is asked on that basis (and the per-friend
        // invitation limit) comes before the plan is locked.
        let (host, in_plan) = {
            let mut conn = core.db.acquire().await?;
            let access = load_access(&mut conn, plan_id, user_id, false).await?;
            ensure_may_invite(&access)?;
            (access.plan.host_id, participant_ids(&mut conn, plan_id).await?)
        };
        // Nobody is brought into a plan whose host they are in a block with
        // (in either direction); they are left out without a word.
        let hidden_from_host: HashSet<Uuid> = graph::blocked_ids_either(core, host).await?.into_iter().collect();
        let candidates: Vec<Uuid> =
            wanted.into_iter().filter(|id| !in_plan.contains(id) && !hidden_from_host.contains(id)).collect();
        people::limit_invitations(core, user_id, &candidates).await?;

        let mut tx = core.db.begin().await?;
        let access = load_access(&mut tx, plan_id, user_id, true).await?;
        ensure_may_invite(&access)?;
        // The blocks were looked up for the host seen above.
        if access.plan.host_id != host {
            return Err(changed());
        }
        let already_in: HashSet<Uuid> = participant_ids(&mut tx, plan_id).await?.into_iter().collect();
        // Leaving is for good: whoever left by themselves is not brought
        // back (and not notified again), without a word to the inviter.
        let left: HashSet<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM plan_past_participants WHERE plan_id = $1 AND left_by_choice AND user_id = ANY($2)",
        )
        .bind(plan_id)
        .bind(&candidates)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        let invitees: Vec<Uuid> =
            candidates.into_iter().filter(|id| !already_in.contains(id) && !left.contains(id)).collect();
        ensure!(
            (already_in.len() + invitees.len()) as i64 <= self.cfg.max_participants,
            invalid,
            "TOO_MANY_PARTICIPANTS",
            "a plan holds at most {} people",
            self.cfg.max_participants
        );
        self.invite(&mut tx, &access.plan, user_id, &invitees).await?;
        tx.commit().await?;

        self.push(invitees.iter().copied(), plan_changed(plan_id, PlanChange::PLAN_CHANGE_INVITED)).await;
        if !invitees.is_empty() {
            self.push(already_in, plan_changed(plan_id, PlanChange::PLAN_CHANGE_PARTICIPANTS)).await;
        }
        Ok((self.plan_for(access.plan, user_id).await?, invitees.len() as i32))
    }

    /// RSVP: `user_id` answers going, maybe or declined. Returns the plan and
    /// what they ended up as (waitlisted when they wanted to go and the plan
    /// is full).
    pub(crate) async fn respond_to_plan(
        &self,
        user_id: Uuid,
        plan_id: Uuid,
        wanted: Rsvp,
        comment: &str,
    ) -> AppResult<(Plan, Rsvp)> {
        ensure!(
            matches!(wanted, Rsvp::RSVP_GOING | Rsvp::RSVP_MAYBE | Rsvp::RSVP_DECLINED),
            invalid,
            "RSVP_INVALID",
            "answer going, maybe or declined"
        );
        let comment = input::line(comment, MAX_COMMENT, "COMMENT_TOO_LONG")?;
        let core = &self.core;
        // Every change of mind reaches the host and everyone's screen.
        core.cache.rate_limit(&format!("plans:rsvp:{user_id}"), ANSWERS_PER_MINUTE, Duration::from_secs(60)).await?;
        core.cache
            .rate_limit(&format!("plans:rsvp:hour:{user_id}"), ANSWERS_PER_HOUR, Duration::from_secs(3600))
            .await?;
        // Asked before the plan is locked: does the host hear about the
        // answer? Not when the two are in a block with each other.
        let host = self.plan_access(plan_id, user_id).await?.plan.host_id;
        let host_hears = host != user_id && !graph::is_blocked_either(core, host, user_id).await?;

        let mut tx = core.db.begin().await?;
        let access = load_access(&mut tx, plan_id, user_id, true).await?;
        access.ensure_active()?;
        ensure!(!access.plan.is_over(now()), precondition, "PLAN_OVER", "the plan is over");
        ensure!(
            !access.is_host() || wanted == Rsvp::RSVP_GOING,
            precondition,
            "HOST_IS_GOING",
            "the host is going; hand the plan over or cancel it instead"
        );

        let previous = access.me.rsvp();
        let outcome = match wanted {
            // Someone who is going stays going; someone waiting keeps their place in the queue.
            Rsvp::RSVP_GOING if previous == Rsvp::RSVP_GOING => Rsvp::RSVP_GOING,
            Rsvp::RSVP_GOING if self.has_room(&mut tx, &access.plan).await? => Rsvp::RSVP_GOING,
            Rsvp::RSVP_GOING => Rsvp::RSVP_WAITLISTED,
            other => other,
        };
        sqlx::query(
            "UPDATE plan_participants SET rsvp = $3, comment = $4, responded_at = now(),
                    waitlisted_at = CASE WHEN $3 = $5 THEN COALESCE(waitlisted_at, now()) END
             WHERE plan_id = $1 AND user_id = $2",
        )
        .bind(plan_id)
        .bind(user_id)
        .bind(code(outcome))
        .bind(&comment)
        .bind(code(Rsvp::RSVP_WAITLISTED))
        .execute(&mut *tx)
        .await?;

        if outcome != previous {
            let changed = RsvpChangedEvent { plan_id, user_id, rsvp: outcome as i32, previous: previous as i32 };
            bus::emit(&mut *tx, PLAN_RSVP_CHANGED, Some(user_id), &changed).await?;
        }
        let news = outcome != previous || comment != access.me.comment;
        // (If the plan was handed over in the meantime, the new host simply sees the answer in the plan.)
        if news && host_hears && access.plan.host_id == host {
            let from = people::name_on(&mut tx, user_id).await?;
            let text = copy::rsvp(&from, outcome, &access.plan.name(), &comment);
            let request =
                NotificationRequest::new(access.plan.host_id, "plans.rsvp", text.title, text.body, plan_path(plan_id))
                    .actor(user_id)
                    .subject(SubjectType::SUBJECT_TYPE_PLAN, plan_id)
                    .collapse(format!("plans.rsvp:{plan_id}"));
            outbound::notify(&mut *tx, &request).await?;
        }
        // Giving up a place lets the next person in.
        let promoted = if previous == Rsvp::RSVP_GOING && outcome != Rsvp::RSVP_GOING {
            self.promote_waitlisted(&mut tx, &access.plan).await?
        } else {
            Vec::new()
        };
        tx.commit().await?;

        let mut changed = vec![user_id];
        changed.extend(promoted);
        self.push_rsvps(plan_id, &changed).await?;
        Ok((self.plan_for(access.plan, user_id).await?, outcome))
    }

    /// Removing people, leaving and changing roles redraw the plan on every
    /// participant's screen; one account may do so this often.
    async fn limit_management(&self, user_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("plans:manage:{user_id}"), 300, Duration::from_secs(3600)).await
    }

    /// Whether one more person can be going.
    async fn has_room(&self, conn: &mut PgConnection, plan: &PlanRow) -> AppResult<bool> {
        if plan.capacity == 0 {
            return Ok(true);
        }
        let going: i64 = sqlx::query_scalar("SELECT count(*) FROM plan_participants WHERE plan_id = $1 AND rsvp = $2")
            .bind(plan.id)
            .bind(code(Rsvp::RSVP_GOING))
            .fetch_one(conn)
            .await?;
        Ok(going < plan.capacity as i64)
    }

    /// Moves people from the waitlist to going while there is room, longest
    /// wait first, and tells them. Returns who moved up.
    pub(crate) async fn promote_waitlisted(&self, conn: &mut PgConnection, plan: &PlanRow) -> AppResult<Vec<Uuid>> {
        // Without a limit everyone waiting gets in.
        let promoted: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE plan_participants SET rsvp = $2, waitlisted_at = NULL, responded_at = now()
             WHERE plan_id = $1 AND user_id IN (
                 SELECT user_id FROM plan_participants
                 WHERE plan_id = $1 AND rsvp = $3
                 ORDER BY waitlisted_at, user_id
                 LIMIT CASE WHEN $4 = 0 THEN NULL
                            ELSE GREATEST($4 - (SELECT count(*) FROM plan_participants WHERE plan_id = $1 AND rsvp = $2), 0) END
             )
             RETURNING user_id",
        )
        .bind(plan.id)
        .bind(code(Rsvp::RSVP_GOING))
        .bind(code(Rsvp::RSVP_WAITLISTED))
        .bind(plan.capacity as i64)
        .fetch_all(&mut *conn)
        .await?;
        let text = copy::promoted(&plan.name());
        for user_id in &promoted {
            let changed = RsvpChangedEvent {
                plan_id: plan.id,
                user_id: *user_id,
                rsvp: Rsvp::RSVP_GOING as i32,
                previous: Rsvp::RSVP_WAITLISTED as i32,
            };
            bus::emit(&mut *conn, PLAN_RSVP_CHANGED, Some(*user_id), &changed).await?;
            let request = NotificationRequest::new(
                *user_id,
                "plans.promoted",
                text.title.clone(),
                text.body.clone(),
                plan_path(plan.id),
            )
            .subject(SubjectType::SUBJECT_TYPE_PLAN, plan.id)
            .collapse(format!("plans.promoted:{}", plan.id));
            outbound::notify(&mut *conn, &request).await?;
        }
        Ok(promoted)
    }

    /// Pushes the current answer of each of `changed` to everyone in the
    /// plan, with the new counts.
    pub(crate) async fn push_rsvps(&self, plan_id: Uuid, changed: &[Uuid]) -> AppResult<()> {
        if changed.is_empty() {
            return Ok(());
        }
        let everyone: Vec<ParticipantRow> =
            sqlx::query_as(concat!("SELECT ", participant_columns!(), " FROM plan_participants WHERE plan_id = $1"))
                .bind(plan_id)
                .fetch_all(&self.core.db)
                .await?;
        let counts = counts_of(&everyone);
        let recipients: Vec<Uuid> = everyone.iter().map(|participant| participant.user_id).collect();
        for participant in everyone.iter().filter(|participant| changed.contains(&participant.user_id)) {
            let event = PlanRsvpChanged {
                plan_id: plan_id.to_string(),
                user_id: participant.user_id.to_string(),
                rsvp: participant.rsvp().into(),
                counts: counts.clone().into(),
                ..Default::default()
            };
            self.push(recipients.iter().copied(), Push::PlanRsvpChanged(Box::new(event))).await;
        }
        Ok(())
    }

    /// Takes `target` out of a plan: their row, their open claims on the
    /// checklist, and — if they were going — their place, which goes to the
    /// waitlist. Refused while they are part of the cost split, because the
    /// others' balances would no longer add up.
    async fn drop_participant(
        &self,
        conn: &mut PgConnection,
        plan: &PlanRow,
        target: &ParticipantRow,
        by_choice: bool,
    ) -> AppResult<Vec<Uuid>> {
        let in_split: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM plan_expenses WHERE plan_id = $1 AND paid_by = $2)
                 OR EXISTS(SELECT 1 FROM plan_expense_shares s JOIN plan_expenses e ON e.id = s.expense_id WHERE e.plan_id = $1 AND s.user_id = $2)
                 OR EXISTS(SELECT 1 FROM plan_settlements WHERE plan_id = $1 AND (from_user_id = $2 OR to_user_id = $2))",
        )
        .bind(plan.id)
        .bind(target.user_id)
        .fetch_one(&mut *conn)
        .await?;
        ensure!(!in_split, precondition, "HAS_EXPENSES", "this person is part of the cost split");
        sqlx::query("DELETE FROM plan_participants WHERE plan_id = $1 AND user_id = $2")
            .bind(plan.id)
            .bind(target.user_id)
            .execute(&mut *conn)
            .await?;
        // They keep the right to report what they were shown (see `subject`).
        sqlx::query(
            "INSERT INTO plan_past_participants (plan_id, user_id, invited_by, title, left_by_choice) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (plan_id, user_id) DO UPDATE
             SET invited_by = EXCLUDED.invited_by, title = EXCLUDED.title, left_by_choice = EXCLUDED.left_by_choice, left_at = now()",
        )
        .bind(plan.id)
        .bind(target.user_id)
        .bind(target.invited_by)
        .bind(plan.name())
        .bind(by_choice)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE plan_checklist_items SET claimed_by = NULL WHERE plan_id = $1 AND claimed_by = $2 AND NOT done",
        )
        .bind(plan.id)
        .bind(target.user_id)
        .execute(&mut *conn)
        .await?;
        if target.rsvp() == Rsvp::RSVP_GOING && plan.is_active() {
            self.promote_waitlisted(conn, plan).await
        } else {
            Ok(Vec::new())
        }
    }

    /// The host or a co-host takes `target` out of the plan.
    pub(crate) async fn remove_plan_participant(&self, user_id: Uuid, plan_id: Uuid, target: Uuid) -> AppResult<Plan> {
        self.limit_management(user_id).await?;
        let mut tx = self.core.db.begin().await?;
        let access = load_access(&mut tx, plan_id, user_id, true).await?;
        access.ensure_organizer()?;
        let participant = find_participant(&mut tx, plan_id, target).await?;
        ensure!(
            participant.role() != PlanRole::PLAN_ROLE_HOST,
            precondition,
            "CANNOT_REMOVE_HOST",
            "the host cannot be removed"
        );
        // Co-hosts manage the guests, not each other.
        ensure!(
            access.is_host() || !participant.is_organizer(),
            denied,
            "NOT_THE_HOST",
            "only the host can remove a co-host"
        );
        let promoted = self.drop_participant(&mut tx, &access.plan, &participant, false).await?;
        let remaining = participant_ids(&mut tx, plan_id).await?;
        tx.commit().await?;

        self.push([target], plan_changed(plan_id, PlanChange::PLAN_CHANGE_REMOVED)).await;
        self.push(remaining, plan_changed(plan_id, PlanChange::PLAN_CHANGE_PARTICIPANTS)).await;
        self.push_rsvps(plan_id, &promoted).await?;
        self.plan_for(access.plan, user_id).await
    }

    /// `user_id` leaves a plan for good: they cannot be invited to it again.
    pub(crate) async fn leave_plan(&self, user_id: Uuid, plan_id: Uuid) -> AppResult<()> {
        self.limit_management(user_id).await?;
        let mut tx = self.core.db.begin().await?;
        let access = load_access(&mut tx, plan_id, user_id, true).await?;
        ensure!(!access.is_host(), precondition, "HOST_CANNOT_LEAVE", "hand the plan over or cancel it");
        let promoted = self.drop_participant(&mut tx, &access.plan, &access.me, true).await?;
        let remaining = participant_ids(&mut tx, plan_id).await?;
        tx.commit().await?;

        self.push([user_id], plan_changed(plan_id, PlanChange::PLAN_CHANGE_REMOVED)).await;
        self.push(remaining, plan_changed(plan_id, PlanChange::PLAN_CHANGE_PARTICIPANTS)).await;
        self.push_rsvps(plan_id, &promoted).await
    }

    /// The host makes `target` a co-host or a guest again, or hands the plan
    /// over to them.
    pub(crate) async fn set_plan_role(
        &self,
        user_id: Uuid,
        plan_id: Uuid,
        target: Uuid,
        role: PlanRole,
    ) -> AppResult<Plan> {
        ensure!(role != PlanRole::PLAN_ROLE_UNSPECIFIED, invalid, "ROLE_INVALID", "choose a role");
        self.limit_management(user_id).await?;
        let mut tx = self.core.db.begin().await?;
        let mut access = load_access(&mut tx, plan_id, user_id, true).await?;
        access.ensure_host()?;
        ensure!(
            target != user_id,
            invalid,
            "ROLE_INVALID",
            "the host's own role only changes by handing the plan over"
        );
        find_participant(&mut tx, plan_id, target).await?;

        if role == PlanRole::PLAN_ROLE_HOST {
            // The new host is going, like every host; the old one stays as a co-host.
            sqlx::query("UPDATE plan_participants SET role = $3, rsvp = $4, waitlisted_at = NULL, responded_at = COALESCE(responded_at, now()) WHERE plan_id = $1 AND user_id = $2")
                .bind(plan_id)
                .bind(target)
                .bind(PlanRole::PLAN_ROLE_HOST as i32 as i16)
                .bind(code(Rsvp::RSVP_GOING))
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE plan_participants SET role = $3 WHERE plan_id = $1 AND user_id = $2")
                .bind(plan_id)
                .bind(user_id)
                .bind(PlanRole::PLAN_ROLE_CO_HOST as i32 as i16)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE plans SET host_id = $2 WHERE id = $1")
                .bind(plan_id)
                .bind(target)
                .execute(&mut *tx)
                .await?;
            access.plan.host_id = target;
        } else {
            sqlx::query("UPDATE plan_participants SET role = $3 WHERE plan_id = $1 AND user_id = $2")
                .bind(plan_id)
                .bind(target)
                .bind(role as i32 as i16)
                .execute(&mut *tx)
                .await?;
        }
        let everyone = participant_ids(&mut tx, plan_id).await?;
        tx.commit().await?;

        self.push(everyone, plan_changed(plan_id, PlanChange::PLAN_CHANGE_PARTICIPANTS)).await;
        self.plan_for(access.plan, user_id).await
    }
}

/// A participant of a plan, or `PARTICIPANT_NOT_FOUND`.
async fn find_participant(conn: &mut PgConnection, plan_id: Uuid, user_id: Uuid) -> AppResult<ParticipantRow> {
    sqlx::query_as(concat!(
        "SELECT ",
        participant_columns!(),
        " FROM plan_participants WHERE plan_id = $1 AND user_id = $2"
    ))
    .bind(plan_id)
    .bind(user_id)
    .fetch_optional(conn)
    .await?
    .or_not_found("PARTICIPANT_NOT_FOUND", "this person is not in the plan")
}
