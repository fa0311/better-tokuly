//! "ひま": telling friends you are free for a while.
//!
//! A user has at most one status. It names a window (now, today, tonight,
//! this weekend or custom), an audience (all friends, best friends or chosen
//! friends) and optionally a note and a rough area. Friends in the audience
//! see it in their "who is free" list and on the owner's profile, and answer
//! "あそぼ!" in one tap. Setting a status notifies nobody, except the friends
//! who asked to be told about this person ("alerts"), once per window.
//!
//! The audience is checked whenever a status is read, so an ended friendship
//! takes effect immediately.
use std::{collections::HashSet, time::Duration as StdDuration};

use buffa::Enumeration as _;
use chrono::{Datelike as _, Duration};
use chrono_tz::Tz;
use tokuly_core::{
    events as bus,
    outbound::{self, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::{LatLng, PageInfo, SubjectType, UserSummary},
    plans::{plan_time, plans_event::Event as Push, *},
};
use tokuly_social::graph;

use crate::{
    Plans, copy, dates,
    events::{FREE_SET, FreeSet},
    input, people,
};

const MAX_NOTE: usize = 60;
const MAX_AREA_NAME: usize = 40;
const MAX_CHOSEN: usize = 50;
const MAX_ANSWER: usize = 100;
/// Most friends one user can ask to be told about.
const MAX_ALERTS: i64 = 200;
/// Longest custom window, and how far ahead one may start.
const MAX_CUSTOM_DAYS: i64 = 14;
const MAX_AHEAD_DAYS: i64 = 30;
/// When a status for best friends is set, at most this many of the friends
/// who asked to be told are checked for being one (most recent requests first).
const MAX_BEST_FRIEND_CHECKS: usize = 100;
/// "Tonight" begins at this local hour.
const TONIGHT_HOUR: u32 = 18;
/// Where free statuses live in the app.
const FREE_PATH: &str = "/plans/free";

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct FreeRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub window_kind: i16,
    pub starts_at: Time,
    pub ends_at: Time,
    pub note: String,
    pub area_lat: Option<f64>,
    pub area_lng: Option<f64>,
    pub area_name: String,
    pub audience: i16,
    pub chosen_ids: Vec<Uuid>,
    pub created_at: Time,
}

macro_rules! free_columns {
    () => {
        "s.id, s.user_id, s.window_kind, s.starts_at, s.ends_at, s.note, s.area_lat, s.area_lng, s.area_name, s.audience, s.chosen_ids, s.created_at"
    };
}

impl FreeRow {
    pub(crate) fn window(&self) -> FreeWindow {
        FreeWindow::from_i32(self.window_kind as i32).unwrap_or(FreeWindow::FREE_WINDOW_UNSPECIFIED)
    }

    fn audience(&self) -> FreeAudience {
        FreeAudience::from_i32(self.audience as i32).unwrap_or(FreeAudience::FREE_AUDIENCE_UNSPECIFIED)
    }

    /// The status as anyone allowed to see it sees it (owner-only fields unset).
    fn to_proto(&self, user: UserSummary, now: Time) -> FreeStatus {
        FreeStatus {
            id: self.id.to_string(),
            user: user.into(),
            window: self.window().into(),
            starts_at: ts(self.starts_at),
            ends_at: ts(self.ends_at),
            note: self.note.clone(),
            area: self.area_lat.zip(self.area_lng).map(|(lat, lng)| LatLng { lat, lng, ..Default::default() }).into(),
            area_name: self.area_name.clone(),
            free_now: self.starts_at <= now,
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

/// Resolves a window preset in the owner's time zone to `(starts_at, ends_at)`.
pub(crate) fn resolve_window(
    window: FreeWindow,
    starts_at: Option<Time>,
    ends_at: Option<Time>,
    now: Time,
    zone: Tz,
    now_hours: i64,
) -> AppResult<(Time, Time)> {
    let today = dates::day_in(zone, now);
    let midnight = dates::day_end(today, zone);
    match window {
        FreeWindow::FREE_WINDOW_UNSPECIFIED => bail!(invalid, "WINDOW_REQUIRED", "say for how long you are free"),
        FreeWindow::FREE_WINDOW_NOW => Ok((now, now + Duration::hours(now_hours))),
        FreeWindow::FREE_WINDOW_TODAY => Ok((now, midnight)),
        FreeWindow::FREE_WINDOW_TONIGHT => Ok((now.max(dates::local_instant(today, TONIGHT_HOUR, zone)), midnight)),
        FreeWindow::FREE_WINDOW_WEEKEND => {
            // Monday = 0 ... Saturday = 5, Sunday = 6 (whose Saturday was yesterday).
            let weekday = today.weekday().num_days_from_monday() as i64;
            let saturday = today + Duration::days(5 - weekday);
            let sunday = saturday + Duration::days(1);
            let start = if weekday >= 5 { now } else { dates::day_start(saturday, zone) };
            Ok((start, dates::day_end(sunday, zone)))
        }
        FreeWindow::FREE_WINDOW_CUSTOM => {
            let Some(end) = ends_at else { bail!(invalid, "WINDOW_INVALID", "a custom window needs an end") };
            let start = starts_at.unwrap_or(now).max(now);
            ensure!(end > start, invalid, "WINDOW_INVALID", "the window must end after it starts, and in the future");
            ensure!(
                end - start <= Duration::days(MAX_CUSTOM_DAYS),
                invalid,
                "WINDOW_INVALID",
                "a window lasts at most {MAX_CUSTOM_DAYS} days"
            );
            ensure!(
                start - now <= Duration::days(MAX_AHEAD_DAYS),
                invalid,
                "WINDOW_INVALID",
                "a window starts at most {MAX_AHEAD_DAYS} days ahead"
            );
            Ok((start, end))
        }
    }
}

/// Rounds a position to two decimals of a degree: a grid of about 1 km, so
/// an area never gives a position away.
pub(crate) fn round_area((lat, lng): (f64, f64)) -> (f64, f64) {
    ((lat * 100.0).round() / 100.0, (lng * 100.0).round() / 100.0)
}

/// An answer row joined with nothing: who, what, when.
#[derive(sqlx::FromRow)]
struct AnswerRow {
    user_id: Uuid,
    message: String,
    created_at: Time,
}

impl Plans {
    /// Says "ひま" for `user_id`, replacing their current status.
    pub(crate) async fn set_free_status(&self, user_id: Uuid, req: &SetFreeStatusRequest) -> AppResult<FreeStatus> {
        let core = &self.core;
        core.cache.rate_limit(&format!("plans:free:set:{user_id}"), 20, StdDuration::from_secs(3600)).await?;

        let now = now();
        let zone = dates::zone_of(core, user_id).await?;
        let window = req.window.as_known().unwrap_or(FreeWindow::FREE_WINDOW_UNSPECIFIED);
        let (starts_at, ends_at) = resolve_window(
            window,
            req.starts_at.as_option().map(from_ts),
            req.ends_at.as_option().map(from_ts),
            now,
            zone,
            self.cfg.free_now_hours,
        )?;
        let note = input::line(&req.note, MAX_NOTE, "NOTE_TOO_LONG")?;
        let area = input::coordinates(req.area.as_option(), "AREA_INVALID")?.map(round_area);
        let area_name = input::line(&req.area_name, MAX_AREA_NAME, "AREA_NAME_TOO_LONG")?;
        let audience = match req.audience.as_known() {
            Some(audience) if audience != FreeAudience::FREE_AUDIENCE_UNSPECIFIED => audience,
            _ => bail!(invalid, "AUDIENCE_REQUIRED", "say which friends may see the status"),
        };
        let chosen = if audience == FreeAudience::FREE_AUDIENCE_CHOSEN {
            let chosen = input::people(&req.chosen_user_ids, user_id)?;
            ensure!(!chosen.is_empty(), invalid, "CHOSEN_REQUIRED", "choose at least one friend");
            ensure!(chosen.len() <= MAX_CHOSEN, invalid, "TOO_MANY_CHOSEN", "choose at most {MAX_CHOSEN} friends");
            people::require_friends(core, user_id, &chosen).await?;
            chosen
        } else {
            Vec::new()
        };

        // Decided before the transaction: finding out who may see the status
        // can take a while (best friends are asked about one by one).
        let draft = FreeRow {
            id: new_id(),
            user_id,
            window_kind: window as i32 as i16,
            starts_at,
            ends_at,
            note,
            area_lat: area.map(|(lat, _)| lat),
            area_lng: area.map(|(_, lng)| lng),
            area_name,
            audience: audience as i32 as i16,
            chosen_ids: chosen,
            created_at: now,
        };
        let owner = tokuly_identity::summary(core, user_id).await?;
        let told = self.friends_to_tell(&draft).await?;
        let previous = self.current_free_status(user_id).await?;

        let mut tx = core.db.begin().await?;
        // One status per user: two racing calls replace each other in turn.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("plans:free:{}", user_id.simple()))
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM free_statuses WHERE user_id = $1").bind(user_id).execute(&mut *tx).await?;
        let row: FreeRow = sqlx::query_as(concat!(
            "INSERT INTO free_statuses AS s (id, user_id, window_kind, starts_at, ends_at, note, area_lat, area_lng, area_name, audience, chosen_ids)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING ",
            free_columns!()
        ))
        .bind(draft.id)
        .bind(user_id)
        .bind(draft.window_kind)
        .bind(starts_at)
        .bind(ends_at)
        .bind(&draft.note)
        .bind(draft.area_lat)
        .bind(draft.area_lng)
        .bind(&draft.area_name)
        .bind(draft.audience)
        .bind(&draft.chosen_ids)
        .fetch_one(&mut *tx)
        .await?;
        let set = FreeSet {
            user_id,
            status_id: row.id,
            window: window as i32,
            starts_at,
            ends_at,
            audience: audience as i32,
        };
        bus::emit(&mut *tx, FREE_SET, Some(user_id), &set).await?;

        // The friends who asked to be told, as far as they may see this status.
        if !told.is_empty() {
            let line = copy::free_line(copy::free_window(window, starts_at, now, zone), &row.note);
            let text = copy::friend_free(&people::name_of(&owner), line);
            for friend in &told {
                let request = NotificationRequest::new(
                    *friend,
                    "plans.friend_free",
                    text.title.clone(),
                    text.body.clone(),
                    FREE_PATH,
                )
                .actor(user_id)
                .subject(SubjectType::SUBJECT_TYPE_USER, user_id)
                .collapse(format!("plans.friend_free:{user_id}"))
                .data(serde_json::json!({ "status_id": row.id }));
                outbound::notify(&mut *tx, &request).await?;
            }
            sqlx::query("UPDATE free_alerts SET notified_starts_at = $3, notified_ends_at = $4 WHERE friend_id = $1 AND user_id = ANY($2)")
                .bind(user_id)
                .bind(&told)
                .bind(starts_at)
                .bind(ends_at)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;

        // Live "who is free" lists: the friends who see the new status get
        // it, the ones who only saw the old one see it end.
        let reached = match self.free_audience(&row).await? {
            Some(friends) => friends,
            None => told,
        };
        let before = match &previous {
            Some(previous) => self.free_audience(previous).await?.unwrap_or_default(),
            None => Vec::new(),
        };
        let seen_by: HashSet<Uuid> = reached.iter().copied().collect();
        let status = row.to_proto(owner, now);
        self.push(
            reached,
            Push::FreeStatusChanged(Box::new(FreeStatusChanged {
                user_id: user_id.to_string(),
                status: status.into(),
                ..Default::default()
            })),
        )
        .await;
        self.push(before.into_iter().filter(|friend| !seen_by.contains(friend)), free_ended(user_id)).await;

        self.free_status_for_owner(&row).await
    }

    /// Ends `user_id`'s status early.
    pub(crate) async fn clear_free_status(&self, user_id: Uuid) -> AppResult<()> {
        let Some(row) = self.current_free_status(user_id).await? else { return Ok(()) };
        sqlx::query("DELETE FROM free_statuses WHERE id = $1").bind(row.id).execute(&self.core.db).await?;
        if let Some(friends) = self.free_audience(&row).await? {
            self.push(friends, free_ended(user_id)).await;
        }
        Ok(())
    }

    /// The status of an active account that has not ended yet.
    pub(crate) async fn current_free_status(&self, user_id: Uuid) -> AppResult<Option<FreeRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            free_columns!(),
            " FROM free_statuses s JOIN users u ON u.id = s.user_id
              WHERE s.user_id = $1 AND s.ends_at > now() AND u.deleted_at IS NULL AND u.banned_at IS NULL"
        ))
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// `owner`'s current status if `viewer` may see it.
    pub(crate) async fn visible_free_status(&self, viewer: Uuid, owner: Uuid) -> AppResult<Option<FreeRow>> {
        let Some(row) = self.current_free_status(owner).await? else { return Ok(None) };
        Ok(if viewer == owner || self.in_free_audience(&row, viewer).await? { Some(row) } else { None })
    }

    /// Whether `viewer` (not the owner) is in the audience of a status right now.
    async fn in_free_audience(&self, row: &FreeRow, viewer: Uuid) -> AppResult<bool> {
        match row.audience() {
            FreeAudience::FREE_AUDIENCE_FRIENDS => graph::are_friends(&self.core, row.user_id, viewer).await,
            FreeAudience::FREE_AUDIENCE_CHOSEN => {
                Ok(row.chosen_ids.contains(&viewer) && graph::are_friends(&self.core, row.user_id, viewer).await?)
            }
            FreeAudience::FREE_AUDIENCE_BEST_FRIENDS => self.is_best_friend(row.user_id, viewer).await,
            FreeAudience::FREE_AUDIENCE_UNSPECIFIED => Ok(false),
        }
    }

    /// Everyone in the audience of a status, for pushing it. `None` for best
    /// friends: the social domain can say whether one person is a best
    /// friend, but has no way to list them, so that audience finds the
    /// status when it opens the list instead of being pushed it.
    async fn free_audience(&self, row: &FreeRow) -> AppResult<Option<Vec<Uuid>>> {
        let friends = graph::friend_ids(&self.core, row.user_id).await?;
        Ok(match row.audience() {
            FreeAudience::FREE_AUDIENCE_FRIENDS => Some(friends),
            FreeAudience::FREE_AUDIENCE_CHOSEN => {
                Some(friends.into_iter().filter(|friend| row.chosen_ids.contains(friend)).collect())
            }
            FreeAudience::FREE_AUDIENCE_BEST_FRIENDS => None,
            FreeAudience::FREE_AUDIENCE_UNSPECIFIED => Some(Vec::new()),
        })
    }

    /// The friends to notify about a new status: they asked to be told about
    /// the owner, may see the status, and have not been told about a window
    /// overlapping this one.
    async fn friends_to_tell(&self, row: &FreeRow) -> AppResult<Vec<Uuid>> {
        let core = &self.core;
        let waiting: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM free_alerts
             WHERE friend_id = $1 AND NOT COALESCE(notified_starts_at < $3 AND notified_ends_at > $2, false)
             ORDER BY created_at DESC, user_id",
        )
        .bind(row.user_id)
        .bind(row.starts_at)
        .bind(row.ends_at)
        .fetch_all(&core.db)
        .await?;
        if waiting.is_empty() {
            return Ok(waiting);
        }
        let friends: HashSet<Uuid> = graph::friend_ids(core, row.user_id).await?.into_iter().collect();
        let waiting = waiting.into_iter().filter(|friend| friends.contains(friend));
        match row.audience() {
            FreeAudience::FREE_AUDIENCE_FRIENDS => Ok(waiting.collect()),
            FreeAudience::FREE_AUDIENCE_CHOSEN => {
                Ok(waiting.filter(|friend| row.chosen_ids.contains(friend)).collect())
            }
            FreeAudience::FREE_AUDIENCE_BEST_FRIENDS => {
                // Best friends are asked about one by one (see `Plans::friendship`),
                // so only so many of the people waiting are considered.
                let mut told = Vec::new();
                for friend in waiting.take(MAX_BEST_FRIEND_CHECKS) {
                    if self.is_best_friend(row.user_id, friend).await? {
                        told.push(friend);
                    }
                }
                Ok(told)
            }
            FreeAudience::FREE_AUDIENCE_UNSPECIFIED => Ok(Vec::new()),
        }
    }

    /// A status with the fields only its owner sees.
    pub(crate) async fn free_status_for_owner(&self, row: &FreeRow) -> AppResult<FreeStatus> {
        let core = &self.core;
        let mut status = row.to_proto(tokuly_identity::summary(core, row.user_id).await?, now());
        status.audience = row.audience().into();
        status.chosen = tokuly_identity::summaries_ordered(core, &row.chosen_ids).await?;
        let answers: i64 = sqlx::query_scalar("SELECT count(*) FROM free_answers WHERE status_id = $1")
            .bind(row.id)
            .fetch_one(&core.db)
            .await?;
        status.answer_count = answers as i32;
        Ok(status)
    }

    /// Statuses as `viewer`, who is in their audience, sees them.
    async fn free_statuses_for_viewer(&self, rows: &[FreeRow], viewer: Uuid) -> AppResult<Vec<FreeStatus>> {
        let core = &self.core;
        let owners: Vec<Uuid> = rows.iter().map(|row| row.user_id).collect();
        let summaries = tokuly_identity::summaries(core, &owners).await?;
        let status_ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let answered: HashSet<Uuid> =
            sqlx::query_scalar("SELECT status_id FROM free_answers WHERE user_id = $1 AND status_id = ANY($2)")
                .bind(viewer)
                .bind(&status_ids)
                .fetch_all(&core.db)
                .await?
                .into_iter()
                .collect();
        let now = now();
        Ok(rows
            .iter()
            .map(|row| {
                let mut status = row.to_proto(tokuly_identity::users::pick(&summaries, row.user_id), now);
                status.answered_by_me = answered.contains(&row.id);
                status
            })
            .collect())
    }

    /// The caller's own status, or a friend's when the caller may see it.
    pub(crate) async fn get_free_status(&self, viewer: Uuid, owner: Uuid) -> AppResult<Option<FreeStatus>> {
        let Some(row) = self.visible_free_status(viewer, owner).await? else { return Ok(None) };
        if viewer == owner {
            return Ok(Some(self.free_status_for_owner(&row).await?));
        }
        Ok(self.free_statuses_for_viewer(&[row], viewer).await?.pop())
    }

    /// Friends who are free and let `viewer` know: running statuses first
    /// (they started earlier), then upcoming ones by start time.
    pub(crate) async fn list_free_friends(&self, viewer: Uuid, page: &Page) -> AppResult<(Vec<FreeStatus>, PageInfo)> {
        let core = &self.core;
        let hidden: HashSet<Uuid> = graph::blocked_ids_either(core, viewer).await?.into_iter().collect();
        let friends: Vec<Uuid> =
            graph::friend_ids(core, viewer).await?.into_iter().filter(|friend| !hidden.contains(friend)).collect();
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<FreeRow> = sqlx::query_as(concat!(
            "SELECT ",
            free_columns!(),
            " FROM free_statuses s JOIN users u ON u.id = s.user_id
              WHERE s.user_id = ANY($1) AND s.ends_at > now() AND u.deleted_at IS NULL AND u.banned_at IS NULL
                AND (s.audience <> 3 OR s.chosen_ids @> ARRAY[$2]::uuid[])
                AND ($3::timestamptz IS NULL OR (s.starts_at, s.id) > ($3, $4))
              ORDER BY s.starts_at, s.id
              LIMIT $5"
        ))
        .bind(&friends)
        .bind(viewer)
        .bind(after.map(|(starts_at, _)| starts_at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.starts_at, row.id));

        // Whether the viewer is one of the owner's best friends is only known one pair at a time.
        let mut visible = Vec::with_capacity(rows.len());
        for row in rows {
            if row.audience() != FreeAudience::FREE_AUDIENCE_BEST_FRIENDS
                || self.is_best_friend(row.user_id, viewer).await?
            {
                visible.push(row);
            }
        }
        Ok((self.free_statuses_for_viewer(&visible, viewer).await?, info))
    }

    /// "あそぼ!": `user_id` answers `owner`'s status.
    pub(crate) async fn answer_free_status(
        &self,
        user_id: Uuid,
        owner: Uuid,
        message: &str,
    ) -> AppResult<AnswerFreeStatusResponse> {
        let core = &self.core;
        ensure!(owner != user_id, invalid, "CANNOT_ANSWER_SELF", "you cannot answer your own status");
        core.cache.rate_limit(&format!("plans:free:answer:{user_id}"), 30, StdDuration::from_secs(60)).await?;
        let message = input::line(message, MAX_ANSWER, "MESSAGE_TOO_LONG")?;
        let row = self
            .visible_free_status(user_id, owner)
            .await?
            .or_not_found("FREE_STATUS_NOT_FOUND", "this friend has no status you can answer")?;
        let sender = tokuly_identity::summary(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        // `xmax = 0` holds for a row this statement inserted, not for one it updated.
        let (created_at, first): (Time, bool) = sqlx::query_as(
            "INSERT INTO free_answers (status_id, user_id, message) VALUES ($1, $2, $3)
             ON CONFLICT (status_id, user_id) DO UPDATE SET message = EXCLUDED.message
             RETURNING created_at, (xmax = 0)",
        )
        .bind(row.id)
        .bind(user_id)
        .bind(&message)
        .fetch_one(&mut *tx)
        .await?;
        if first {
            let text = copy::nudge(&people::name_of(&sender), &message);
            let request = NotificationRequest::new(owner, "plans.nudge", text.title, text.body, FREE_PATH)
                .actor(user_id)
                .subject(SubjectType::SUBJECT_TYPE_USER, user_id)
                .collapse(format!("plans.nudge:{user_id}"))
                .data(serde_json::json!({ "status_id": row.id }));
            outbound::notify(&mut *tx, &request).await?;
        }
        tx.commit().await?;

        let answer = FreeAnswer { user: sender.into(), message, created_at: ts(created_at), ..Default::default() };
        self.push([owner], Push::FreeAnswer(Box::new(answer.clone()))).await;

        let status = self.free_statuses_for_viewer(std::slice::from_ref(&row), user_id).await?.pop();
        let zone = dates::zone_of(core, user_id).await?;
        Ok(AnswerFreeStatusResponse {
            answer: answer.into(),
            status: status.into(),
            plan_draft: plan_draft(&row, zone).into(),
            ..Default::default()
        })
    }

    /// Who answered `owner`'s current status, newest first.
    pub(crate) async fn list_free_answers(&self, owner: Uuid, page: &Page) -> AppResult<(Vec<FreeAnswer>, PageInfo)> {
        let core = &self.core;
        let Some(row) = self.current_free_status(owner).await? else { return Ok((Vec::new(), PageInfo::default())) };
        let hidden = graph::blocked_ids_either(core, owner).await?;
        let before: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<AnswerRow> = sqlx::query_as(
            "SELECT user_id, message, created_at FROM free_answers
             WHERE status_id = $1 AND NOT (user_id = ANY($2))
               AND ($3::timestamptz IS NULL OR (created_at, user_id) < ($3, $4))
             ORDER BY created_at DESC, user_id DESC
             LIMIT $5",
        )
        .bind(row.id)
        .bind(&hidden)
        .bind(before.map(|(created_at, _)| created_at))
        .bind(before.map(|(_, user_id)| user_id))
        .bind(page.fetch())
        .fetch_all(&core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.created_at, row.user_id));
        let users: Vec<Uuid> = rows.iter().map(|row| row.user_id).collect();
        let summaries = tokuly_identity::summaries(core, &users).await?;
        let answers = rows
            .into_iter()
            .map(|row| FreeAnswer {
                user: tokuly_identity::users::pick(&summaries, row.user_id).into(),
                message: row.message,
                created_at: ts(row.created_at),
                ..Default::default()
            })
            .collect();
        Ok((answers, info))
    }

    /// Turns "tell me when `friend` is free" on or off for `user_id`.
    pub(crate) async fn set_free_alert(&self, user_id: Uuid, friend: Uuid, enabled: bool) -> AppResult<()> {
        let core = &self.core;
        if !enabled {
            sqlx::query("DELETE FROM free_alerts WHERE user_id = $1 AND friend_id = $2")
                .bind(user_id)
                .bind(friend)
                .execute(&core.db)
                .await?;
            return Ok(());
        }
        people::require_friends(core, user_id, &[friend]).await?;
        let (existing, total): (bool, i64) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM free_alerts WHERE user_id = $1 AND friend_id = $2), (SELECT count(*) FROM free_alerts WHERE user_id = $1)",
        )
        .bind(user_id)
        .bind(friend)
        .fetch_one(&core.db)
        .await?;
        ensure!(
            existing || total < MAX_ALERTS,
            precondition,
            "TOO_MANY_ALERTS",
            "you can ask to be told about at most {MAX_ALERTS} friends"
        );
        sqlx::query("INSERT INTO free_alerts (user_id, friend_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(user_id)
            .bind(friend)
            .execute(&core.db)
            .await?;
        Ok(())
    }

    /// The friends `user_id` asked to be told about, most recently added first.
    pub(crate) async fn list_free_alerts(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<UserSummary>, PageInfo)> {
        let before: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<(Uuid, Time)> = sqlx::query_as(
            "SELECT friend_id, created_at FROM free_alerts
             WHERE user_id = $1 AND ($2::timestamptz IS NULL OR (created_at, friend_id) < ($2, $3))
             ORDER BY created_at DESC, friend_id DESC
             LIMIT $4",
        )
        .bind(user_id)
        .bind(before.map(|(created_at, _)| created_at))
        .bind(before.map(|(_, friend)| friend))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |(friend, created_at)| (*created_at, *friend));
        let friends: Vec<Uuid> = rows.into_iter().map(|(friend, _)| friend).collect();
        Ok((tokuly_identity::summaries_ordered(&self.core, &friends).await?, info))
    }

    /// Deletes the statuses whose window is over. Returns how many.
    pub(crate) async fn expire_free_statuses(&self) -> AppResult<u64> {
        Ok(sqlx::query("DELETE FROM free_statuses WHERE ends_at <= now()")
            .execute(&self.core.db)
            .await?
            .rows_affected())
    }
}

/// The event telling a friend that `user_id`'s status is gone.
fn free_ended(user_id: Uuid) -> Push {
    Push::FreeStatusChanged(Box::new(FreeStatusChanged { user_id: user_id.to_string(), ..Default::default() }))
}

/// A plan for the two, prefilled from the status that was answered: what is
/// left of its window, its area (when it has a name) and its owner as the
/// invitee.
fn plan_draft(row: &FreeRow, zone: Tz) -> CreatePlanRequest {
    let start = row.starts_at.max(now());
    let place = (!row.area_name.is_empty()).then(|| PlanPlace {
        name: row.area_name.clone(),
        location: row.area_lat.zip(row.area_lng).map(|(lat, lng)| LatLng { lat, lng, ..Default::default() }).into(),
        ..Default::default()
    });
    CreatePlanRequest {
        title: if row.note.is_empty() { copy::HANG_OUT_TITLE.to_owned() } else { row.note.clone() },
        time: PlanTime {
            start: Some(plan_time::Start::StartAt(Box::new(start.into()))),
            end: Some(plan_time::End::EndAt(Box::new(row.ends_at.into()))),
            time_zone: zone.name().to_owned(),
            ..Default::default()
        }
        .into(),
        place: place.into(),
        invitee_ids: vec![row.user_id.to_string()],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    const TOKYO: Tz = chrono_tz::Asia::Tokyo;

    /// A Tokyo wall-clock time as an instant.
    fn tokyo(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Time {
        TOKYO.with_ymd_and_hms(year, month, day, hour, minute, 0).unwrap().with_timezone(&chrono::Utc)
    }

    fn resolve(window: FreeWindow, now: Time) -> (Time, Time) {
        resolve_window(window, None, None, now, TOKYO, 3).unwrap()
    }

    #[test]
    fn presets_follow_the_owners_clock() {
        // Sunday 2026-11-22, 15:30 in Tokyo.
        let now = tokyo(2026, 11, 22, 15, 30);
        assert_eq!(resolve(FreeWindow::FREE_WINDOW_NOW, now), (now, tokyo(2026, 11, 22, 18, 30)));
        assert_eq!(resolve(FreeWindow::FREE_WINDOW_TODAY, now), (now, tokyo(2026, 11, 23, 0, 0)));
        assert_eq!(
            resolve(FreeWindow::FREE_WINDOW_TONIGHT, now),
            (tokyo(2026, 11, 22, 18, 0), tokyo(2026, 11, 23, 0, 0))
        );
        // "Tonight" asked at 21:00 starts right away.
        let evening = tokyo(2026, 11, 22, 21, 0);
        assert_eq!(resolve(FreeWindow::FREE_WINDOW_TONIGHT, evening), (evening, tokyo(2026, 11, 23, 0, 0)));
    }

    #[test]
    fn the_weekend_is_the_coming_saturday_and_sunday() {
        // Wednesday: the whole weekend lies ahead.
        let wednesday = tokyo(2026, 11, 25, 12, 0);
        assert_eq!(
            resolve(FreeWindow::FREE_WINDOW_WEEKEND, wednesday),
            (tokyo(2026, 11, 28, 0, 0), tokyo(2026, 11, 30, 0, 0))
        );
        // Saturday and Sunday: from now until Sunday night.
        let saturday = tokyo(2026, 11, 28, 10, 0);
        assert_eq!(resolve(FreeWindow::FREE_WINDOW_WEEKEND, saturday), (saturday, tokyo(2026, 11, 30, 0, 0)));
        let sunday = tokyo(2026, 11, 29, 23, 0);
        assert_eq!(resolve(FreeWindow::FREE_WINDOW_WEEKEND, sunday), (sunday, tokyo(2026, 11, 30, 0, 0)));
    }

    #[test]
    fn custom_windows_are_bounded() {
        let now = tokyo(2026, 11, 22, 15, 30);
        let custom = |start: Option<Time>, end: Option<Time>| {
            resolve_window(FreeWindow::FREE_WINDOW_CUSTOM, start, end, now, TOKYO, 3)
        };
        let tomorrow = tokyo(2026, 11, 23, 13, 0);
        assert_eq!(
            custom(Some(tomorrow), Some(tomorrow + Duration::hours(4))).unwrap(),
            (tomorrow, tomorrow + Duration::hours(4))
        );
        // No start, or one in the past, means from now.
        assert_eq!(custom(None, Some(tomorrow)).unwrap(), (now, tomorrow));
        assert_eq!(custom(Some(now - Duration::hours(5)), Some(tomorrow)).unwrap(), (now, tomorrow));
        for (start, end) in [
            (None, None),
            (None, Some(now - Duration::minutes(1))),
            (Some(tomorrow), Some(tomorrow)),
            (None, Some(now + Duration::days(15))),
            (Some(now + Duration::days(31)), Some(now + Duration::days(32))),
        ] {
            assert_eq!(custom(start, end).unwrap_err().reason, "WINDOW_INVALID");
        }
        assert_eq!(
            resolve_window(FreeWindow::FREE_WINDOW_UNSPECIFIED, None, None, now, TOKYO, 3).unwrap_err().reason,
            "WINDOW_REQUIRED"
        );
    }

    #[test]
    fn areas_are_rounded_to_about_a_kilometre() {
        assert_eq!(round_area((35.658_034, 139.701_636)), (35.66, 139.7));
        assert_eq!(round_area((-33.856_784, 151.215_297)), (-33.86, 151.22));
    }
}
