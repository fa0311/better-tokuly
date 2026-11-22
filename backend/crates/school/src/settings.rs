//! Per-account settings and `SchoolService`.
//!
//! A user without a row has the defaults: nothing is shared beyond friends,
//! and every notification that is not a direct reminder is off.
use std::collections::HashMap;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::school::*;

use crate::{School, timer::Cycle};

const VISIBLE_TO_FRIENDS: i16 = StudyVisibility::STUDY_VISIBILITY_FRIENDS as i16;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct SettingsRow {
    pub class_soon_minutes: i32,
    pub study_visibility: i16,
    pub share_study_activity: bool,
    pub join_ranking: bool,
    pub cheer_notifications: bool,
    pub daily_goal_minutes: i32,
    pub pomodoro_work: i32,
    pub pomodoro_break: i32,
    pub pomodoro_long_break: i32,
    pub pomodoro_rounds: i32,
    pub assignment_reminders: bool,
}

impl Default for SettingsRow {
    fn default() -> Self {
        Self {
            class_soon_minutes: 0,
            study_visibility: VISIBLE_TO_FRIENDS,
            share_study_activity: true,
            join_ranking: false,
            cheer_notifications: false,
            daily_goal_minutes: 0,
            pomodoro_work: 25,
            pomodoro_break: 5,
            pomodoro_long_break: 15,
            pomodoro_rounds: 4,
            assignment_reminders: true,
        }
    }
}

const COLUMNS: &str = "class_soon_minutes, study_visibility, share_study_activity, join_ranking, cheer_notifications, \
                       daily_goal_minutes, pomodoro_work, pomodoro_break, pomodoro_long_break, pomodoro_rounds, assignment_reminders";

impl SettingsRow {
    /// Friends may see this user studying.
    pub fn shows_studying(&self) -> bool {
        self.study_visibility == VISIBLE_TO_FRIENDS
    }

    /// The pomodoro lengths a timer started without any gets.
    pub fn pomodoro(&self) -> Cycle {
        Cycle {
            work_ms: self.pomodoro_work as i64 * 60_000,
            break_ms: self.pomodoro_break as i64 * 60_000,
            long_break_ms: self.pomodoro_long_break as i64 * 60_000,
            rounds: self.pomodoro_rounds as i64,
        }
    }

    fn to_proto(&self) -> SchoolSettings {
        SchoolSettings {
            class_soon_minutes: self.class_soon_minutes,
            study_visibility: (self.study_visibility as i32).into(),
            share_study_activity: self.share_study_activity,
            join_ranking: self.join_ranking,
            cheer_notifications: self.cheer_notifications,
            daily_goal_minutes: self.daily_goal_minutes,
            pomodoro: self.pomodoro().to_config().into(),
            assignment_reminders: self.assignment_reminders,
            ..Default::default()
        }
    }
}

/// A user's settings (the defaults when they never changed any).
pub(crate) async fn of(core: &Core, user_id: Uuid) -> AppResult<SettingsRow> {
    of_on(&mut *core.db.acquire().await?, user_id).await
}

/// [`of`] on a connection the caller holds (inside a transaction).
pub(crate) async fn of_on(conn: &mut sqlx::PgConnection, user_id: Uuid) -> AppResult<SettingsRow> {
    let row: Option<SettingsRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM school_settings WHERE user_id = $1")))
            .bind(user_id)
            .fetch_optional(conn)
            .await?;
    Ok(row.unwrap_or_default())
}

/// Settings of several users; users without a row are absent (use the default).
pub(crate) async fn of_many(core: &Core, user_ids: &[Uuid]) -> AppResult<HashMap<Uuid, SettingsRow>> {
    #[derive(sqlx::FromRow)]
    struct Keyed {
        user_id: Uuid,
        #[sqlx(flatten)]
        settings: SettingsRow,
    }
    let rows: Vec<Keyed> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT user_id, {COLUMNS} FROM school_settings WHERE user_id = ANY($1)"
    )))
    .bind(user_ids)
    .fetch_all(&core.db)
    .await?;
    Ok(rows.into_iter().map(|row| (row.user_id, row.settings)).collect())
}

fn invalid(message: &str) -> AppError {
    AppError::invalid("SETTINGS_INVALID", message.to_owned())
}

impl School {
    async fn update_settings(&self, user_id: Uuid, request: UpdateSchoolSettingsRequest) -> AppResult<SettingsRow> {
        let mut tx = self.core.db.begin().await?;
        crate::lock_user(&mut tx, "settings", user_id).await?;
        let mut settings = of_on(&mut tx, user_id).await?;

        if let Some(minutes) = request.class_soon_minutes {
            ensure!((0..=120).contains(&minutes), invalid, "SETTINGS_INVALID", "class_soon_minutes must be 0-120");
            settings.class_soon_minutes = minutes;
        }
        if let Some(visibility) = request.study_visibility {
            settings.study_visibility = match visibility.as_known() {
                Some(v @ (StudyVisibility::STUDY_VISIBILITY_FRIENDS | StudyVisibility::STUDY_VISIBILITY_NOBODY)) => {
                    v as i16
                }
                _ => return Err(invalid("study_visibility must be friends or nobody")),
            };
        }
        if let Some(share) = request.share_study_activity {
            settings.share_study_activity = share;
        }
        if let Some(join) = request.join_ranking {
            settings.join_ranking = join;
        }
        if let Some(cheers) = request.cheer_notifications {
            settings.cheer_notifications = cheers;
        }
        if let Some(goal) = request.daily_goal_minutes {
            ensure!((0..=1440).contains(&goal), invalid, "SETTINGS_INVALID", "daily_goal_minutes must be 0-1440");
            settings.daily_goal_minutes = goal;
        }
        if let Some(pomodoro) = request.pomodoro.as_option() {
            let cycle = Cycle::from_config(pomodoro, settings.pomodoro()).map_err(|e| invalid(&e.message))?;
            let config = cycle.to_config();
            settings.pomodoro_work = config.work_minutes;
            settings.pomodoro_break = config.break_minutes;
            settings.pomodoro_long_break = config.long_break_minutes;
            settings.pomodoro_rounds = config.rounds;
        }
        if let Some(reminders) = request.assignment_reminders {
            settings.assignment_reminders = reminders;
        }

        sqlx::query(
            "INSERT INTO school_settings (user_id, class_soon_minutes, study_visibility, share_study_activity, join_ranking,
                 cheer_notifications, daily_goal_minutes, pomodoro_work, pomodoro_break, pomodoro_long_break, pomodoro_rounds, assignment_reminders)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (user_id) DO UPDATE SET
                 class_soon_minutes = EXCLUDED.class_soon_minutes, study_visibility = EXCLUDED.study_visibility,
                 share_study_activity = EXCLUDED.share_study_activity, join_ranking = EXCLUDED.join_ranking,
                 cheer_notifications = EXCLUDED.cheer_notifications, daily_goal_minutes = EXCLUDED.daily_goal_minutes,
                 pomodoro_work = EXCLUDED.pomodoro_work, pomodoro_break = EXCLUDED.pomodoro_break,
                 pomodoro_long_break = EXCLUDED.pomodoro_long_break, pomodoro_rounds = EXCLUDED.pomodoro_rounds,
                 assignment_reminders = EXCLUDED.assignment_reminders",
        )
        .bind(user_id)
        .bind(settings.class_soon_minutes)
        .bind(settings.study_visibility)
        .bind(settings.share_study_activity)
        .bind(settings.join_ranking)
        .bind(settings.cheer_notifications)
        .bind(settings.daily_goal_minutes)
        .bind(settings.pomodoro_work)
        .bind(settings.pomodoro_break)
        .bind(settings.pomodoro_long_break)
        .bind(settings.pomodoro_rounds)
        .bind(settings.assignment_reminders)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(settings)
    }
}

impl SchoolService for School {
    async fn get_school_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetSchoolSettingsRequest>,
    ) -> ServiceResult<GetSchoolSettingsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_reads(user_id).await?;
            let settings = of(&self.core, user_id).await?;
            ok(GetSchoolSettingsResponse { settings: settings.to_proto().into(), ..Default::default() })
        })
        .await
    }

    async fn update_school_settings(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateSchoolSettingsRequest>,
    ) -> ServiceResult<UpdateSchoolSettingsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            // Switching study visibility is announced to every friend.
            self.core
                .cache
                .rate_limit(&format!("school:settings:{user_id}"), 30, std::time::Duration::from_secs(60))
                .await?;
            let request = request.to_owned_message();
            let before = of(&self.core, user_id).await?;
            let settings = self.update_settings(user_id, request).await?;
            let at = self.now();
            // Each of these settings has something that follows from it.
            if settings.class_soon_minutes != before.class_soon_minutes {
                self.reschedule_class_soon(user_id, at).await?;
            }
            if settings.assignment_reminders != before.assignment_reminders {
                self.reschedule_assignment_reminders(user_id, at).await?;
            }
            if settings.shows_studying() != before.shows_studying() {
                self.announce_studying(user_id, at, !settings.shows_studying()).await?;
            }
            ok(UpdateSchoolSettingsResponse { settings: settings.to_proto().into(), ..Default::default() })
        })
        .await
    }
}
