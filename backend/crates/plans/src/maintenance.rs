//! Background work: reactions to other domains' events, everything that is
//! sent when its time comes (reminders, attendance, poll deadlines,
//! milestones), and the personal data export.
use std::collections::HashMap;

use async_trait::async_trait;
use tokuly_core::{
    events::{self as bus, ConsumeOptions, Event},
    export::DataExporter,
    outbound::{self, ActivityAudience, ActivityEntry, NotificationRequest},
    prelude::*,
};
use tokuly_identity::maintenance::UserRef;
use tokuly_proto::{
    common::SubjectType,
    plans::{PlanChange, PlanRole, Rsvp},
};
use tokuly_social::{
    events::{BlockChanged, FRIEND_REMOVED, FriendRemoved, USER_BLOCKED},
    graph,
};

use crate::{
    Plans,
    anniversaries::anniversary_ended,
    copy,
    events::{PLAN_ATTENDED, PlanAttended},
    plan::{PlanRow, plan_changed, plan_columns, plan_path},
    polls::poll_deleted,
};

const USER_DELETED: &str = "identity.user.deleted";
const CONTENT_REMOVED: &str = "admin.content.removed";

/// Payload of `admin.content.removed`, as far as this domain reads it.
#[derive(Debug, serde::Deserialize)]
struct ContentRemoved {
    #[serde(default)]
    subject_type: i32,
    subject_id: Uuid,
}

/// What one pass of the worker did (see [`Plans::run_due_work`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkDone {
    /// Free statuses whose window ended.
    pub statuses_expired: u64,
    /// Plans whose "tomorrow" / "soon" reminder went out.
    pub day_before_reminders: u64,
    pub soon_reminders: u64,
    /// Plans whose attendance was announced.
    pub plans_attended: u64,
    pub polls_closed: u64,
    /// Polls whose automatic reminder went out.
    pub polls_reminded: u64,
    /// Countdowns that reached their day.
    pub countdowns_reached: u64,
    /// Counters checked for a milestone.
    pub counters_checked: u64,
}

impl Plans {
    /// Consumes the events this domain reacts to until shutdown.
    pub(crate) async fn run_event_consumer(&self) {
        let plans = self.clone();
        bus::consume(
            self.core.bus.clone(),
            "plans",
            &[USER_DELETED, FRIEND_REMOVED, USER_BLOCKED, CONTENT_REMOVED],
            ConsumeOptions::default(),
            self.core.shutdown.clone(),
            move |event| {
                let plans = plans.clone();
                async move { plans.handle_event(&event).await }
            },
        )
        .await;
    }

    /// Reacts to one event from another domain. Every reaction is idempotent
    /// (events are delivered at least once). Public so tests can deliver
    /// events directly.
    pub async fn handle_event(&self, event: &Event) -> AppResult<()> {
        match event.topic.as_str() {
            USER_DELETED => self.forget_user(event.data::<UserRef>()?.user_id).await,
            FRIEND_REMOVED => {
                let removed: FriendRemoved = event.data()?;
                self.separate(removed.user_id, removed.friend_id).await
            }
            USER_BLOCKED => {
                let blocked: BlockChanged = event.data()?;
                self.separate(blocked.user_id, blocked.blocked_id).await
            }
            CONTENT_REMOVED => {
                let removed: ContentRemoved = event.data()?;
                if removed.subject_type == SubjectType::SUBJECT_TYPE_PLAN as i32 {
                    self.take_down_plan(removed.subject_id).await
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }

    /// Sends everything that has become due: expired statuses are dropped,
    /// plan reminders go out, attendance of finished plans is announced,
    /// polls past their deadline close, people who have not answered a poll
    /// are reminded, countdowns that reached zero and anniversary milestones
    /// are announced. Run by the worker every few seconds; several processes
    /// may run it at once (rows are claimed with `SKIP LOCKED`). Public so
    /// tests and operators can trigger a pass.
    pub async fn run_due_work(&self) -> AppResult<WorkDone> {
        Ok(WorkDone {
            statuses_expired: self.expire_free_statuses().await?,
            day_before_reminders: self.send_plan_reminders(Reminder::DayBefore).await?,
            soon_reminders: self.send_plan_reminders(Reminder::Soon).await?,
            plans_attended: self.announce_attendance().await?,
            polls_closed: self.close_due_polls().await?,
            polls_reminded: self.remind_due_polls().await?,
            countdowns_reached: self.notify_due_countdowns().await?,
            counters_checked: self.announce_due_milestones().await?,
        })
    }

    /// Reminds the people going (or maybe going) to the plans whose reminder
    /// of this kind is due. Returns how many plans were handled.
    async fn send_plan_reminders(&self, reminder: Reminder) -> AppResult<u64> {
        let core = &self.core;
        // A reminder that came due again for a start time it was already
        // sent for (the lead time was changed back and forth) is dropped.
        let resent = match reminder {
            Reminder::DayBefore => {
                "UPDATE plans SET day_before_due_at = NULL WHERE day_before_due_at <= now() AND day_before_sent_for = sort_at"
            }
            Reminder::Soon => {
                "UPDATE plans SET before_due_at = NULL WHERE before_due_at <= now() AND before_sent_for = sort_at"
            }
        };
        sqlx::query(resent).execute(&core.db).await?;

        // A reminder speaks for the host. Who is in a block with each host is
        // asked first: the transaction below uses its own connection only,
        // and claims only the plans of the hosts asked about.
        let hosts: Vec<Uuid> = sqlx::query_scalar(match reminder {
            Reminder::DayBefore => "SELECT DISTINCT host_id FROM plans WHERE day_before_due_at <= now()",
            Reminder::Soon => "SELECT DISTINCT host_id FROM plans WHERE before_due_at <= now()",
        })
        .fetch_all(&core.db)
        .await?;
        let mut hidden: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for host in &hosts {
            hidden.insert(*host, graph::blocked_ids_either(core, *host).await?);
        }

        let mut tx = core.db.begin().await?;
        let due: Vec<PlanRow> = match reminder {
            Reminder::DayBefore => {
                sqlx::query_as(concat!(
                    "UPDATE plans p SET day_before_due_at = NULL, day_before_sent_for = sort_at
                     WHERE p.id IN (SELECT id FROM plans WHERE day_before_due_at <= now() AND host_id = ANY($1) ORDER BY day_before_due_at LIMIT 100 FOR UPDATE SKIP LOCKED)
                     RETURNING ",
                    plan_columns!()
                ))
                .bind(&hosts)
                .fetch_all(&mut *tx)
                .await?
            }
            Reminder::Soon => {
                sqlx::query_as(concat!(
                    "UPDATE plans p SET before_due_at = NULL, before_sent_for = sort_at
                     WHERE p.id IN (SELECT id FROM plans WHERE before_due_at <= now() AND host_id = ANY($1) ORDER BY before_due_at LIMIT 100 FOR UPDATE SKIP LOCKED)
                     RETURNING ",
                    plan_columns!()
                ))
                .bind(&hosts)
                .fetch_all(&mut *tx)
                .await?
            }
        };
        let now = now();
        for plan in &due {
            // A reminder that comes after the plan started (the worker was
            // down) would only confuse.
            let Some(starts_at) = plan.sort_at.filter(|starts_at| *starts_at > now && plan.is_active()) else {
                continue;
            };
            let text = match reminder {
                Reminder::DayBefore => copy::reminder_day_before(&plan.name(), &plan.schedule().text()),
                Reminder::Soon => {
                    // Normally the configured lead time; the real one when the worker ran late.
                    let left = ((starts_at - now).num_seconds() + 59) / 60;
                    let minutes = if (left - plan.remind_minutes_before as i64).abs() <= 2 {
                        plan.remind_minutes_before
                    } else {
                        left.max(1) as i32
                    };
                    copy::reminder_soon(&plan.name(), minutes, &plan.place_name)
                }
            };
            // Not to someone in a block with the host: blocking the host
            // stops what the host's plan sends.
            let going: Vec<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM plan_participants WHERE plan_id = $1 AND rsvp IN ($2, $3) AND NOT (user_id = ANY($4))",
            )
            .bind(plan.id)
            .bind(Rsvp::RSVP_GOING as i32 as i16)
            .bind(Rsvp::RSVP_MAYBE as i32 as i16)
            .bind(hidden.get(&plan.host_id).map(Vec::as_slice).unwrap_or_default())
            .fetch_all(&mut *tx)
            .await?;
            for user_id in going {
                let mut request = NotificationRequest::new(
                    user_id,
                    "plans.reminder",
                    text.title.clone(),
                    text.body.clone(),
                    plan_path(plan.id),
                )
                .subject(SubjectType::SUBJECT_TYPE_PLAN, plan.id)
                .collapse(format!("plans.reminder:{}", plan.id));
                // The host is the one behind a reminder, for whatever the
                // notify domain decides by actor (mutes, blocks).
                if user_id != plan.host_id {
                    request = request.actor(plan.host_id);
                }
                outbound::notify(&mut *tx, &request).await?;
            }
        }
        tx.commit().await?;
        Ok(due.len() as u64)
    }

    /// For plans whose time has passed: emits `plans.plan.attended` and
    /// records an activity entry for everyone who was going. Once per plan:
    /// a plan that is moved to another day afterwards is not counted again.
    /// Returns how many plans were handled.
    async fn announce_attendance(&self) -> AppResult<u64> {
        let mut tx = self.core.db.begin().await?;
        let over: Vec<PlanRow> = sqlx::query_as(concat!(
            "UPDATE plans p SET attended_due_at = NULL, attended_at = now()
             WHERE p.id IN (SELECT id FROM plans WHERE attended_due_at <= now() AND attended_at IS NULL ORDER BY attended_due_at LIMIT 100 FOR UPDATE SKIP LOCKED)
             RETURNING ",
            plan_columns!()
        ))
        .fetch_all(&mut *tx)
        .await?;
        for plan in over.iter().filter(|plan| plan.is_active()) {
            let attendees: Vec<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM plan_participants WHERE plan_id = $1 AND rsvp = $2 ORDER BY user_id",
            )
            .bind(plan.id)
            .bind(Rsvp::RSVP_GOING as i32 as i16)
            .fetch_all(&mut *tx)
            .await?;
            for user_id in &attendees {
                let attended = PlanAttended {
                    plan_id: plan.id,
                    user_id: *user_id,
                    host_id: plan.host_id,
                    attendee_ids: attendees.clone(),
                };
                bus::emit(&mut *tx, PLAN_ATTENDED, Some(*user_id), &attended).await?;
                // One entry per person, in their own log, naming the others:
                // together the entries are visible to the participants and nobody else.
                let mut entry = ActivityEntry::new(
                    *user_id,
                    "plans.attended",
                    ActivityAudience::Private,
                    copy::plan_attended_summary(&plan.name()),
                )
                .subject(SubjectType::SUBJECT_TYPE_PLAN, plan.id)
                .with(attendees.iter().copied().filter(|other| other != user_id).take(20))
                .path(plan_path(plan.id));
                if let Some((lat, lng)) = plan.place_lat.zip(plan.place_lng) {
                    entry = entry.at(lat, lng);
                }
                outbound::activity(&mut *tx, &entry).await?;
            }
        }
        tx.commit().await?;
        Ok(over.len() as u64)
    }

    /// Removes everything this domain holds about a deleted account. Plans
    /// the account hosted pass to someone else in them (or disappear when
    /// nobody is left); where the account is part of a cost split its place
    /// stays, as a declined guest without a comment, so that the others'
    /// balances still add up — the account itself is an anonymous tombstone
    /// by then.
    async fn forget_user(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;

        // ひま, countdowns, settings.
        const SIMPLE: &[&str] = &[
            "DELETE FROM free_statuses WHERE user_id = $1",
            "DELETE FROM free_answers WHERE user_id = $1",
            "DELETE FROM free_alerts WHERE user_id = $1 OR friend_id = $1",
            "UPDATE free_statuses SET chosen_ids = array_remove(chosen_ids, $1) WHERE chosen_ids @> ARRAY[$1]::uuid[]",
            "DELETE FROM plan_countdowns WHERE user_id = $1",
            "UPDATE plan_countdowns SET shared_ids = array_remove(shared_ids, $1) WHERE shared_ids @> ARRAY[$1]::uuid[]",
            "DELETE FROM plan_settings WHERE user_id = $1",
            "DELETE FROM plan_past_participants WHERE user_id = $1",
        ];
        for statement in SIMPLE {
            sqlx::query(*statement).bind(user_id).execute(&mut *tx).await?;
        }

        // Counters end for the other person too.
        let ended: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
            "DELETE FROM plan_anniversaries WHERE user_lo = $1 OR user_hi = $1 RETURNING id, user_lo, user_hi",
        )
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;

        // Their polls disappear; in other people's polls their answers do.
        let poll_members: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT m.poll_id, m.user_id FROM date_poll_participants m JOIN date_polls q ON q.id = m.poll_id WHERE q.creator_id = $1 AND m.user_id <> $1",
        )
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM date_polls WHERE creator_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM date_poll_answers WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM date_poll_participants WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;

        // Plans, one by one.
        let plans: Vec<PlanRow> = sqlx::query_as(concat!(
            "SELECT ",
            plan_columns!(),
            " FROM plans p JOIN plan_participants me ON me.plan_id = p.id AND me.user_id = $1 ORDER BY p.id FOR UPDATE OF p"
        ))
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut changed_plans = Vec::new();
        for plan in &plans {
            if plan.host_id == user_id {
                // A co-host takes over, else whoever is going, else anyone.
                let successor: Option<Uuid> = sqlx::query_scalar(
                    "SELECT user_id FROM plan_participants WHERE plan_id = $1 AND user_id <> $2
                     ORDER BY role, (rsvp = $3) DESC, created_at, user_id LIMIT 1",
                )
                .bind(plan.id)
                .bind(user_id)
                .bind(Rsvp::RSVP_GOING as i32 as i16)
                .fetch_optional(&mut *tx)
                .await?;
                let Some(successor) = successor else {
                    sqlx::query("DELETE FROM plans WHERE id = $1").bind(plan.id).execute(&mut *tx).await?;
                    continue;
                };
                sqlx::query("UPDATE plan_participants SET role = $3, rsvp = $4, waitlisted_at = NULL WHERE plan_id = $1 AND user_id = $2")
                    .bind(plan.id)
                    .bind(successor)
                    .bind(PlanRole::PLAN_ROLE_HOST as i32 as i16)
                    .bind(Rsvp::RSVP_GOING as i32 as i16)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("UPDATE plans SET host_id = $2 WHERE id = $1")
                    .bind(plan.id)
                    .bind(successor)
                    .execute(&mut *tx)
                    .await?;
            }
            let in_split: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM plan_expenses WHERE plan_id = $1 AND paid_by = $2)
                     OR EXISTS(SELECT 1 FROM plan_expense_shares s JOIN plan_expenses e ON e.id = s.expense_id WHERE e.plan_id = $1 AND s.user_id = $2)
                     OR EXISTS(SELECT 1 FROM plan_settlements WHERE plan_id = $1 AND (from_user_id = $2 OR to_user_id = $2))",
            )
            .bind(plan.id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if in_split {
                sqlx::query("UPDATE plan_participants SET role = $3, rsvp = $4, comment = '', waitlisted_at = NULL WHERE plan_id = $1 AND user_id = $2")
                    .bind(plan.id)
                    .bind(user_id)
                    .bind(PlanRole::PLAN_ROLE_GUEST as i32 as i16)
                    .bind(Rsvp::RSVP_DECLINED as i32 as i16)
                    .execute(&mut *tx)
                    .await?;
            } else {
                sqlx::query("DELETE FROM plan_participants WHERE plan_id = $1 AND user_id = $2")
                    .bind(plan.id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query(
                "UPDATE plan_checklist_items SET claimed_by = NULL WHERE plan_id = $1 AND claimed_by = $2 AND NOT done",
            )
            .bind(plan.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
            // Their place, if they had one, goes to the waitlist.
            if plan.is_active() {
                self.promote_waitlisted(&mut tx, plan).await?;
            }
            changed_plans.push(plan.id);
        }
        let plan_members: Vec<(Uuid, Uuid)> =
            sqlx::query_as("SELECT plan_id, user_id FROM plan_participants WHERE plan_id = ANY($1) AND user_id <> $2")
                .bind(&changed_plans)
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        tx.commit().await?;

        for (anniversary_id, lo, hi) in ended {
            self.push([if lo == user_id { hi } else { lo }], anniversary_ended(anniversary_id)).await;
        }
        for (poll_id, members) in grouped(poll_members) {
            self.push(members, poll_deleted(poll_id)).await;
        }
        for (plan_id, members) in grouped(plan_members) {
            self.push(members, plan_changed(plan_id, PlanChange::PLAN_CHANGE_PARTICIPANTS)).await;
        }
        Ok(())
    }

    /// Two people stopped being friends, or one blocked the other: they
    /// leave each other's audiences, unanswered invitations between them are
    /// withdrawn, each leaves the other's polls, and their counters end.
    /// Plans they share (and answered) keep both, listed as before. The
    /// end of a friendship and a block do exactly the same here, so nothing
    /// in this domain tells someone which of the two happened.
    ///
    /// Events can arrive late. If the two are friends by now (they made up
    /// before the event got here) nothing is separated: what they share today
    /// was set up after the state the event describes.
    async fn separate(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        let core = &self.core;
        if graph::are_friends(core, a, b).await? {
            return Ok(());
        }
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        let mut tx = core.db.begin().await?;

        // Audiences: chosen friends of a status or a countdown, "tell me when
        // they are free", and the "あそぼ!" one sent the other.
        const BOTH_WAYS: &[&str] = &[
            "UPDATE free_statuses SET chosen_ids = array_remove(chosen_ids, $2) WHERE user_id = $1 AND chosen_ids @> ARRAY[$2]::uuid[]",
            "DELETE FROM free_alerts WHERE user_id = $1 AND friend_id = $2",
            "DELETE FROM free_answers a USING free_statuses s WHERE a.status_id = s.id AND s.user_id = $1 AND a.user_id = $2",
            "UPDATE plan_countdowns SET shared_ids = array_remove(shared_ids, $2) WHERE user_id = $1 AND shared_ids @> ARRAY[$2]::uuid[]",
        ];
        for statement in BOTH_WAYS {
            for (owner, other) in [(a, b), (b, a)] {
                sqlx::query(*statement).bind(owner).bind(other).execute(&mut *tx).await?;
            }
        }

        // Invitations to a plan that one extended to the other (or to a plan
        // the other hosts) and that were never answered.
        // Whoever loses an invitation this way can still report the plan they
        // were shown (see `subject`): blocking someone must not be what makes
        // their invitation unreportable.
        let uninvited: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "WITH gone AS (
                 DELETE FROM plan_participants m USING plans p
                 WHERE m.plan_id = p.id AND m.rsvp = $3
                   AND ((m.user_id = $1 AND (m.invited_by = $2 OR p.host_id = $2)) OR (m.user_id = $2 AND (m.invited_by = $1 OR p.host_id = $1)))
                 RETURNING m.plan_id, m.user_id, m.invited_by
             )
             INSERT INTO plan_past_participants (plan_id, user_id, invited_by, title)
             SELECT gone.plan_id, gone.user_id, gone.invited_by, concat_ws(' ', NULLIF(p.emoji, ''), p.title)
             FROM gone JOIN plans p ON p.id = gone.plan_id
             ON CONFLICT (plan_id, user_id) DO UPDATE SET invited_by = EXCLUDED.invited_by, title = EXCLUDED.title, left_at = now()
             RETURNING plan_id, user_id",
        )
        .bind(a)
        .bind(b)
        .bind(Rsvp::RSVP_INVITED as i32 as i16)
        .fetch_all(&mut *tx)
        .await?;

        // Polls of the other: each leaves them, answers included. A poll is
        // among the creator's friends; someone who no longer is one does not
        // keep reading it.
        sqlx::query(
            "DELETE FROM date_poll_answers g USING date_polls q
             WHERE g.poll_id = q.id AND ((q.creator_id = $1 AND g.user_id = $2) OR (q.creator_id = $2 AND g.user_id = $1))",
        )
        .bind(a)
        .bind(b)
        .execute(&mut *tx)
        .await?;
        let left_polls: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "DELETE FROM date_poll_participants m USING date_polls q
             WHERE m.poll_id = q.id AND ((q.creator_id = $1 AND m.user_id = $2) OR (q.creator_id = $2 AND m.user_id = $1))
             RETURNING m.poll_id, m.user_id",
        )
        .bind(a)
        .bind(b)
        .fetch_all(&mut *tx)
        .await?;

        // Counters of the two, invitations included.
        let ended: Vec<Uuid> =
            sqlx::query_scalar("DELETE FROM plan_anniversaries WHERE user_lo = $1 AND user_hi = $2 RETURNING id")
                .bind(lo)
                .bind(hi)
                .fetch_all(&mut *tx)
                .await?;
        tx.commit().await?;

        for (plan_id, user_id) in uninvited {
            self.push([user_id], plan_changed(plan_id, PlanChange::PLAN_CHANGE_REMOVED)).await;
        }
        for (poll_id, user_id) in left_polls {
            self.push([user_id], poll_deleted(poll_id)).await;
        }
        for anniversary_id in ended {
            self.push([a, b], anniversary_ended(anniversary_id)).await;
        }
        Ok(())
    }

    /// A moderator removed a plan: it disappears for everyone in it.
    async fn take_down_plan(&self, plan_id: Uuid) -> AppResult<()> {
        let taken_down = sqlx::query("UPDATE plans SET removed_at = now(), day_before_due_at = NULL, before_due_at = NULL, attended_due_at = NULL WHERE id = $1 AND removed_at IS NULL")
            .bind(plan_id)
            .execute(&self.core.db)
            .await?
            .rows_affected()
            > 0;
        if taken_down {
            let everyone: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM plan_participants WHERE plan_id = $1")
                .bind(plan_id)
                .fetch_all(&self.core.db)
                .await?;
            self.push(everyone, plan_changed(plan_id, PlanChange::PLAN_CHANGE_REMOVED)).await;
        }
        Ok(())
    }
}

/// Which of a plan's two reminders.
#[derive(Clone, Copy)]
enum Reminder {
    DayBefore,
    Soon,
}

/// Groups `(key, member)` pairs by key.
fn grouped(pairs: Vec<(Uuid, Uuid)>) -> HashMap<Uuid, Vec<Uuid>> {
    let mut groups: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (key, member) in pairs {
        groups.entry(key).or_default().push(member);
    }
    groups
}

/// The `plans` section of a personal data export.
pub(crate) struct PlansExporter {
    core: Core,
}

impl PlansExporter {
    pub(crate) fn new(core: Core) -> Self {
        Self { core }
    }

    /// Runs a query that aggregates its rows into one JSON value.
    async fn json(&self, sql: &'static str, user_id: Uuid) -> AppResult<serde_json::Value> {
        Ok(sqlx::query_scalar(sql).bind(user_id).fetch_one(&self.core.db).await?)
    }
}

#[async_trait]
impl DataExporter for PlansExporter {
    fn name(&self) -> &'static str {
        "plans"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        Ok(serde_json::json!({
            "free_status": self.json(
                "SELECT COALESCE((SELECT jsonb_build_object(
                     'window', window_kind, 'starts_at', starts_at, 'ends_at', ends_at, 'note', note, 'area_lat', area_lat, 'area_lng', area_lng,
                     'area_name', area_name, 'audience', audience, 'chosen_user_ids', chosen_ids, 'set_at', created_at)
                 FROM free_statuses WHERE user_id = $1), 'null'::jsonb)", user_id).await?,
            "free_answers_sent": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('to_user_id', s.user_id, 'message', a.message, 'at', a.created_at) ORDER BY a.created_at), '[]'::jsonb)
                 FROM free_answers a JOIN free_statuses s ON s.id = a.status_id WHERE a.user_id = $1", user_id).await?,
            "free_alerts": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('friend_id', friend_id, 'since', created_at) ORDER BY created_at), '[]'::jsonb)
                 FROM free_alerts WHERE user_id = $1", user_id).await?,
            "plans": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'id', p.id, 'title', p.title, 'emoji', p.emoji, 'description', p.description, 'host_id', p.host_id,
                     'start_at', p.start_at, 'start_date', p.start_date, 'end_at', p.end_at, 'end_date', p.end_date, 'time_zone', p.time_zone,
                     'place_name', p.place_name, 'place_address', p.place_address, 'place_lat', p.place_lat, 'place_lng', p.place_lng,
                     'status', p.status, 'cancel_reason', p.cancel_reason, 'created_at', p.created_at,
                     'my_role', m.role, 'my_rsvp', m.rsvp, 'my_comment', m.comment, 'invited_by', m.invited_by, 'responded_at', m.responded_at) ORDER BY p.id), '[]'::jsonb)
                 FROM plans p JOIN plan_participants m ON m.plan_id = p.id AND m.user_id = $1 WHERE p.removed_at IS NULL", user_id).await?,
            "checklist_items": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'plan_id', plan_id, 'kind', kind, 'text', text, 'written_by_me', created_by = $1, 'claimed_by_me', claimed_by = $1,
                     'ticked_by_me', done_by = $1, 'created_at', created_at) ORDER BY id), '[]'::jsonb)
                 FROM plan_checklist_items WHERE created_by = $1 OR claimed_by = $1 OR done_by = $1", user_id).await?,
            "expenses": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'plan_id', e.plan_id, 'title', e.title, 'amount', e.amount, 'currency', e.currency, 'paid_by_me', e.paid_by = $1,
                     'recorded_by_me', e.created_by = $1, 'my_share', s.amount, 'created_at', e.created_at) ORDER BY e.id), '[]'::jsonb)
                 FROM plan_expenses e LEFT JOIN plan_expense_shares s ON s.expense_id = e.id AND s.user_id = $1
                 WHERE e.paid_by = $1 OR e.created_by = $1 OR s.user_id IS NOT NULL", user_id).await?,
            "settlements": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'plan_id', plan_id, 'from_user_id', from_user_id, 'to_user_id', to_user_id, 'amount', amount, 'currency', currency,
                     'recorded_by_me', recorded_by = $1, 'at', created_at, 'taken_back_at', removed_at) ORDER BY id), '[]'::jsonb)
                 FROM plan_settlements WHERE from_user_id = $1 OR to_user_id = $1", user_id).await?,
            "polls": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'id', q.id, 'title', q.title, 'description', q.description, 'created_by_me', q.creator_id = $1, 'status', q.status,
                     'deadline', q.deadline, 'my_comment', m.comment, 'answered_at', m.answered_at,
                     'my_answers', (SELECT COALESCE(jsonb_agg(jsonb_build_object(
                          'start_at', s.start_at, 'date', s.slot_date, 'answer', g.answer) ORDER BY s.sort_at), '[]'::jsonb)
                          FROM date_poll_answers g JOIN date_poll_slots s ON s.id = g.slot_id WHERE g.poll_id = q.id AND g.user_id = $1)) ORDER BY q.id), '[]'::jsonb)
                 FROM date_polls q JOIN date_poll_participants m ON m.poll_id = q.id AND m.user_id = $1", user_id).await?,
            "countdowns": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'title', title, 'emoji', emoji, 'date', target_date, 'repeats_yearly', repeats_yearly, 'sharing', sharing,
                     'shared_with', shared_ids, 'created_at', created_at) ORDER BY id), '[]'::jsonb)
                 FROM plan_countdowns WHERE user_id = $1", user_id).await?,
            "anniversaries": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'kind', kind, 'with_user_id', CASE WHEN user_lo = $1 THEN user_hi ELSE user_lo END, 'start_date', start_date,
                     'title', title, 'emoji', emoji, 'status', status, 'started_by_me', created_by = $1,
                     'shown_on_my_profile', CASE WHEN user_lo = $1 THEN show_lo ELSE show_hi END, 'created_at', created_at) ORDER BY id), '[]'::jsonb)
                 FROM plan_anniversaries WHERE user_lo = $1 OR user_hi = $1", user_id).await?,
            "former_plans": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object(
                     'plan_id', plan_id, 'title', title, 'invited_by', invited_by, 'left_at', left_at) ORDER BY left_at), '[]'::jsonb)
                 FROM plan_past_participants WHERE user_id = $1", user_id).await?,
            // The switch this domain offers; the setting itself is identity's.
            "settings": serde_json::json!({
                "share_birthday": tokuly_identity::birthdays::audience_of(&tokuly_identity::settings::privacy_of(&self.core, user_id).await?)
                    != tokuly_proto::identity::Audience::AUDIENCE_NOBODY,
            }),
        }))
    }
}
