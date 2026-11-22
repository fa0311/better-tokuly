//! Place alerts ("tell me when Aoi gets home") and SOS. Also `SafetyService`.
//!
//! Alerts are evaluated by the pipeline on the subject's own samples
//! ([`Geo::evaluate_alerts`]), never by clients, and never against a position
//! the alert's owner is not allowed to see: while the subject hides their
//! precise location from the owner (ghost mode), the alert neither fires nor
//! remembers which side of the fence the subject is on.
use std::time::Duration;

use chrono::{DateTime, Utc};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::geo::{create_place_alert_request::Where, geo_event::Event, *};
use tokuly_social::graph;

use crate::{
    Geo,
    ctx::{AlertCtx, OwnerCtx},
    geocode::Gazetteer,
    math,
    places::{self, MAX_RADIUS_M, MIN_RADIUS_M},
    state::Pin,
    visibility,
};

const PENDING: i16 = AlertStatus::ALERT_STATUS_PENDING_APPROVAL as i16;
const ACTIVE: i16 = AlertStatus::ALERT_STATUS_ACTIVE as i16;
const DECLINED: i16 = AlertStatus::ALERT_STATUS_DECLINED as i16;
const FIRED: i16 = AlertStatus::ALERT_STATUS_FIRED as i16;
const EXPIRED: i16 = AlertStatus::ALERT_STATUS_EXPIRED as i16;
const ARRIVE: i32 = AlertTrigger::ALERT_TRIGGER_ARRIVE as i32;
const LEAVE: i32 = AlertTrigger::ALERT_TRIGGER_LEAVE as i32;

/// Radius of an alert around a bare point when none is given.
const DEFAULT_ALERT_RADIUS_M: f64 = 150.0;
/// Alerts one person may have waiting or active.
const MAX_ALERTS: i64 = 30;
/// A recurring alert fires at most this often.
const REFIRE_AFTER_MS: i64 = 5 * 60 * 1000;
/// Once inside, the subject counts as inside until this much beyond the radius.
const EXIT_FACTOR: f64 = 1.25;
const EXIT_MARGIN_M: f64 = 20.0;
const MAX_SOS_MESSAGE_CHARS: usize = 200;
const MAX_SOS_RECIPIENTS: usize = 100;

#[derive(Debug, Clone, sqlx::FromRow)]
struct AlertRow {
    id: Uuid,
    owner_id: Uuid,
    subject_id: Uuid,
    place_name: String,
    lat: Option<f64>,
    lng: Option<f64>,
    radius_m: Option<f64>,
    subject_place_label: i16,
    trigger: i16,
    recurring: bool,
    status: i16,
    created_at: DateTime<Utc>,
    last_fired_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}

const ALERT_COLUMNS: &str = "id, owner_id, subject_id, place_name, lat, lng, radius_m, subject_place_label, trigger, recurring, status, created_at, last_fired_at, expires_at";

#[derive(Debug, Clone, sqlx::FromRow)]
struct SosRow {
    id: Uuid,
    user_id: Uuid,
    message: String,
    notify_user_ids: Vec<Uuid>,
    started_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
}

const SOS_COLUMNS: &str = "id, user_id, message, notify_user_ids, started_at, resolved_at";

fn alert_not_found() -> AppError {
    AppError::not_found("ALERT_NOT_FOUND", "alert not found")
}

/// "自宅" / "home" for labelled places, the alert's own name otherwise.
fn place_words(label: i32, place_name: &str) -> (String, String) {
    match label {
        1 => ("自宅".into(), "home".into()),
        2 => ("職場".into(), "work".into()),
        3 => ("学校".into(), "school".into()),
        _ if place_name.is_empty() => ("指定の場所".into(), "the place".into()),
        _ => (format!("「{place_name}」"), place_name.to_owned()),
    }
}

/// Whether the subject is inside the fence, given where they were before
/// (leaving takes going a little beyond the radius, so GPS wobble at the
/// edge does not fire an alert over and over).
fn is_inside(distance_m: f64, radius_m: f64, was_inside: Option<bool>) -> bool {
    match was_inside {
        Some(true) => distance_m <= radius_m * EXIT_FACTOR + EXIT_MARGIN_M,
        _ => distance_m <= radius_m,
    }
}

/// What one evaluation of an alert decides.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// Nothing to store.
    Unchanged,
    /// Forget which side the subject is on (the owner may not see them).
    Forget,
    /// Remember the new side; `fire` if the alert goes off.
    Moved { inside: bool, fire: bool },
}

fn judge(
    alert: &AlertCtx,
    target: Option<(f64, f64, f64)>,
    pin: &Pin,
    owner_sees_precisely: bool,
    now_ms: i64,
) -> Verdict {
    if !owner_sees_precisely {
        return if alert.inside.is_some() { Verdict::Forget } else { Verdict::Unchanged };
    }
    let Some((lat, lng, radius_m)) = target else { return Verdict::Unchanged };
    let inside = is_inside(math::distance(pin.position(), (lat, lng)), radius_m, alert.inside);
    match alert.inside {
        Some(was) if was == inside => Verdict::Unchanged,
        // First look: remember the side, but nothing happened yet.
        None => Verdict::Moved { inside, fire: false },
        Some(_) => {
            let wanted = (inside && alert.trigger == ARRIVE) || (!inside && alert.trigger == LEAVE);
            let rested = alert.last_fired_ms.is_none_or(|fired| now_ms - fired >= REFIRE_AFTER_MS);
            Verdict::Moved { inside, fire: wanted && rested }
        }
    }
}

impl Geo {
    async fn alert_row(&self, alert_id: Uuid) -> AppResult<Option<AlertRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {ALERT_COLUMNS} FROM geo_place_alerts WHERE id = $1")))
            .bind(alert_id)
            .fetch_optional(&self.core.db)
            .await?)
    }

    async fn alert_messages(&self, rows: &[AlertRow]) -> AppResult<Vec<PlaceAlert>> {
        let ids: Vec<Uuid> = rows.iter().flat_map(|row| [row.owner_id, row.subject_id]).collect();
        let users = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok(rows
            .iter()
            .map(|row| PlaceAlert {
                id: row.id.to_string(),
                owner: tokuly_identity::users::pick(&users, row.owner_id).into(),
                subject: tokuly_identity::users::pick(&users, row.subject_id).into(),
                place_name: row.place_name.clone(),
                center: row.lat.zip(row.lng).map(|(lat, lng)| math::lat_lng(lat, lng)).into(),
                radius_meters: row.radius_m.unwrap_or(0.0),
                trigger: (row.trigger as i32).into(),
                recurring: row.recurring,
                status: (row.status as i32).into(),
                created_at: ts(row.created_at),
                last_fired_at: opt_ts(row.last_fired_at),
                subject_place_label: (row.subject_place_label as i32).into(),
                expires_at: opt_ts(row.expires_at),
                ..Default::default()
            })
            .collect())
    }

    async fn alert_message(&self, row: &AlertRow) -> AppResult<PlaceAlert> {
        Ok(self.alert_messages(std::slice::from_ref(row)).await?.pop().unwrap_or_default())
    }

    /// Tells both sides of an alert about its current state.
    async fn push_alert(&self, row: &AlertRow) -> AppResult<()> {
        let message = self.alert_message(row).await?;
        self.push_many([row.owner_id, row.subject_id], Event::PlaceAlertUpdated(Box::new(message))).await;
        Ok(())
    }

    async fn display_name(&self, user_id: Uuid) -> AppResult<String> {
        Ok(tokuly_identity::summary(&self.core, user_id).await?.display_name)
    }

    /// Checks the alerts watching `subject` against their new position.
    /// Called by the pipeline for live samples only.
    pub(crate) async fn evaluate_alerts(&self, subject: Uuid, ctx: &OwnerCtx, pin: &Pin) -> AppResult<()> {
        if ctx.alerts.is_empty() {
            return Ok(());
        }
        let now = Utc::now();
        let mut changed = false;
        for alert in &ctx.alerts {
            let target = alert
                .target
                .or_else(|| ctx.labelled_place(alert.label).map(|place| (place.lat, place.lng, place.radius_m)));
            let precise = visibility::sees_precisely(ctx, alert.owner_id, now);
            match judge(alert, target, pin, precise, now.timestamp_millis()) {
                Verdict::Unchanged => {}
                Verdict::Forget => {
                    sqlx::query("UPDATE geo_place_alerts SET inside = NULL WHERE id = $1")
                        .bind(alert.id)
                        .execute(&self.core.db)
                        .await?;
                    changed = true;
                }
                Verdict::Moved { inside, fire } => {
                    changed = true;
                    if fire {
                        self.fire_alert(alert.id, inside, now).await?;
                    } else {
                        sqlx::query("UPDATE geo_place_alerts SET inside = $2 WHERE id = $1 AND status = $3")
                            .bind(alert.id)
                            .bind(inside)
                            .bind(ACTIVE)
                            .execute(&self.core.db)
                            .await?;
                    }
                }
            }
        }
        if changed {
            self.invalidate_ctx(&[subject]).await;
        }
        Ok(())
    }

    /// An alert goes off: the owner is notified, a one-time alert is done.
    async fn fire_alert(&self, alert_id: Uuid, inside: bool, now: DateTime<Utc>) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        let fired: Option<AlertRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE geo_place_alerts SET inside = $2, last_fired_at = $3,
                    status = CASE WHEN recurring THEN status ELSE {FIRED} END,
                    expires_at = CASE WHEN recurring THEN expires_at ELSE NULL END
             WHERE id = $1 AND status = {ACTIVE}
             RETURNING {ALERT_COLUMNS}"
        )))
        .bind(alert_id)
        .bind(inside)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = fired else { return Ok(()) };
        let name = self.display_name(row.subject_id).await?;
        let (place_ja, place_en) = place_words(row.subject_place_label as i32, &row.place_name);
        let title = if inside {
            Localized::new(format!("{name}さんが{place_ja}に到着しました!"), format!("{name} arrived at {place_en}"))
        } else {
            Localized::new(format!("{name}さんが{place_ja}を出発しました"), format!("{name} left {place_en}"))
        };
        let notification =
            NotificationRequest::new(row.owner_id, "geo.place_alert", title, Localized::default(), "/map")
                .actor(row.subject_id)
                .collapse(format!("geo.place_alert:{}", row.id))
                .data(serde_json::json!({ "alert_id": row.id, "arrived": inside }));
        outbound::notify(&mut *tx, &notification).await?;
        tx.commit().await?;

        let message = self.alert_message(&row).await?;
        self.push(
            row.owner_id,
            Event::PlaceAlertFired(Box::new(PlaceAlertFired {
                alert: message.clone().into(),
                fired_at: ts(now),
                ..Default::default()
            })),
        )
        .await;
        if row.status == FIRED {
            // A one-time alert is over for the subject as well.
            self.push(row.subject_id, Event::PlaceAlertUpdated(Box::new(message))).await;
        }
        Ok(())
    }

    /// Lapses alerts that waited too long (for approval, or to fire once).
    pub async fn expire_place_alerts(&self) -> AppResult<u64> {
        let rows: Vec<AlertRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE geo_place_alerts SET status = {EXPIRED} WHERE status IN ({PENDING}, {ACTIVE}) AND expires_at <= now() RETURNING {ALERT_COLUMNS}"
        )))
        .fetch_all(&self.core.db)
        .await?;
        for row in &rows {
            self.invalidate_ctx(&[row.subject_id]).await;
            self.push_alert(row).await?;
        }
        Ok(rows.len() as u64)
    }

    /// Removes the alerts two users had on each other (they are no longer friends).
    pub(crate) async fn drop_alerts_between(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        let removed = sqlx::query("DELETE FROM geo_place_alerts WHERE (owner_id = $1 AND subject_id = $2) OR (owner_id = $2 AND subject_id = $1)")
            .bind(a)
            .bind(b)
            .execute(&self.core.db)
            .await?
            .rows_affected();
        if removed > 0 {
            self.invalidate_ctx(&[a, b]).await;
        }
        Ok(())
    }

    // ── SOS ─────────────────────────────────────────────────────────────────

    /// An SOS as `viewer` sees it: the sender gets the list of people alerted,
    /// everyone gets the sender's pin as they are allowed to see it (precise
    /// while the SOS is active).
    async fn sos_message(&self, row: &SosRow, viewer: Uuid) -> AppResult<Sos> {
        let location = if viewer == row.user_id {
            self.own_location(row.user_id).await?.0
        } else {
            self.location_as_seen(row.user_id, viewer).await?
        };
        Ok(Sos {
            id: row.id.to_string(),
            user: tokuly_identity::summary(&self.core, row.user_id).await?.into(),
            message: row.message.clone(),
            started_at: ts(row.started_at),
            resolved_at: opt_ts(row.resolved_at),
            location: location.into(),
            notified_user_ids: if viewer == row.user_id {
                row.notify_user_ids.iter().map(Uuid::to_string).collect()
            } else {
                Vec::new()
            },
            ..Default::default()
        })
    }

    async fn active_sos(&self, user_id: Uuid) -> AppResult<Option<SosRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {SOS_COLUMNS} FROM geo_sos WHERE user_id = $1 AND resolved_at IS NULL"
        )))
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// Shows everyone involved the state of an SOS and the pin that goes with it.
    async fn broadcast_sos(&self, row: &SosRow) -> AppResult<()> {
        self.invalidate_ctx(&[row.user_id]).await;
        for viewer in row.notify_user_ids.iter().chain([&row.user_id]) {
            let message = self.sos_message(row, *viewer).await?;
            self.push(*viewer, Event::SosUpdated(Box::new(message))).await;
        }
        // Precise while active, back under ghost mode once resolved.
        self.refresh_viewers(row.user_id, &row.notify_user_ids).await
    }

    /// Ends an SOS (by its sender, or by the sweeper after the maximum time).
    async fn end_sos(&self, row: SosRow) -> AppResult<()> {
        let name = self.display_name(row.user_id).await?;
        let mut tx = self.core.db.begin().await?;
        for recipient in &row.notify_user_ids {
            let notification = NotificationRequest::new(
                *recipient,
                "geo.sos_resolved",
                Localized::new(format!("{name}さんのSOSは解除されました"), format!("{name}'s SOS was resolved")),
                Localized::default(),
                "/map",
            )
            .actor(row.user_id)
            .collapse(format!("geo.sos:{}", row.id));
            outbound::notify(&mut *tx, &notification).await?;
        }
        tx.commit().await?;
        self.broadcast_sos(&row).await
    }

    /// Resolves SOS alerts nobody resolved within the configured time, so
    /// that ghost mode cannot stay overridden forever.
    pub async fn expire_sos(&self) -> AppResult<u64> {
        let rows: Vec<SosRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE geo_sos SET resolved_at = now() WHERE resolved_at IS NULL AND started_at <= now() - make_interval(hours => $1) RETURNING {SOS_COLUMNS}"
        )))
        .bind(self.cfg.sos_max_hours as i32)
        .fetch_all(&self.core.db)
        .await?;
        let count = rows.len() as u64;
        for row in rows {
            self.end_sos(row).await?;
        }
        Ok(count)
    }

    /// Takes a user off the SOS alerts of someone they are no longer friends with.
    pub(crate) async fn drop_sos_recipient(&self, sender: Uuid, recipient: Uuid) -> AppResult<()> {
        let changed = sqlx::query("UPDATE geo_sos SET notify_user_ids = array_remove(notify_user_ids, $2) WHERE user_id = $1 AND resolved_at IS NULL AND $2 = ANY(notify_user_ids)")
            .bind(sender)
            .bind(recipient)
            .execute(&self.core.db)
            .await?
            .rows_affected();
        if changed > 0 {
            self.invalidate_ctx(&[sender]).await;
        }
        Ok(())
    }
}

impl SafetyService for Geo {
    async fn create_place_alert(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreatePlaceAlertRequest>,
    ) -> ServiceResult<CreatePlaceAlertResponse> {
        handle(async {
            let owner = ctx.user_id()?;
            let req = request.to_owned_message();
            let subject = parse_id(&req.subject_user_id)?;
            ensure!(subject != owner && graph::are_friends(&self.core, owner, subject).await?, precondition, "NOT_FRIENDS", "place alerts are for friends");
            let trigger = match req.trigger.as_known() {
                Some(AlertTrigger::ALERT_TRIGGER_ARRIVE) => ARRIVE,
                Some(AlertTrigger::ALERT_TRIGGER_LEAVE) => LEAVE,
                _ => bail!(invalid, "INVALID_TRIGGER", "an alert fires on arrival or on leaving"),
            };
            let radius = |fallback: f64| -> AppResult<f64> {
                if req.radius_meters == 0.0 {
                    return Ok(fallback);
                }
                ensure!(req.radius_meters.is_finite() && (MIN_RADIUS_M..=MAX_RADIUS_M).contains(&req.radius_meters), invalid, "INVALID_RADIUS", "the radius must be {MIN_RADIUS_M}-{MAX_RADIUS_M} meters");
                Ok(req.radius_meters)
            };
            let given_name = req.place_name.trim();
            ensure!(given_name.chars().count() <= 60 && !given_name.chars().any(char::is_control), invalid, "INVALID_NAME", "a place name has at most 60 characters");
            // (name, fixed target, label of the subject's own place)
            let (place_name, target, label): (String, Option<(f64, f64, f64)>, i16) = match &req.r#where {
                Some(Where::PlaceId(place_id)) => {
                    let place = self.own_place(owner, place_id).await?;
                    let name = if given_name.is_empty() { place.name.clone() } else { given_name.to_owned() };
                    (name, Some((place.lat, place.lng, radius(place.radius_m)?)), 0)
                }
                Some(Where::Center(center)) => {
                    let (lat, lng) = places::clean_point(Some(center))?;
                    let name = if given_name.is_empty() {
                        let ja = self.speaks_japanese(owner).await?;
                        Gazetteer::shared().nearest(lat, lng).map(|found| found.locality(ja).to_owned()).unwrap_or_default()
                    } else {
                        given_name.to_owned()
                    };
                    (name, Some((lat, lng, radius(DEFAULT_ALERT_RADIUS_M)?)), 0)
                }
                Some(Where::SubjectPlaceLabel(label)) => {
                    let label = match label.as_known() {
                        Some(PlaceLabel::PLACE_LABEL_HOME | PlaceLabel::PLACE_LABEL_WORK | PlaceLabel::PLACE_LABEL_SCHOOL) => label.to_i32(),
                        _ => bail!(invalid, "MISSING_TARGET", "the friend's place must be home, work or school"),
                    };
                    // One answer for "has no such place" and "does not share labels": neither is the asker's business.
                    let theirs = self.ctx_of(subject).await?;
                    ensure!(
                        theirs.privacy.share_place_labels && theirs.labelled_place(label).is_some(),
                        precondition,
                        "SUBJECT_PLACE_UNAVAILABLE",
                        "this friend has no such place to be alerted about"
                    );
                    (String::new(), None, label as i16)
                }
                None => bail!(invalid, "MISSING_TARGET", "say where the alert is"),
            };
            self.core.cache.rate_limit(&format!("geo-alert:{owner}"), 30, Duration::from_secs(3600)).await?;
            let open: i64 = sqlx::query_scalar("SELECT count(*) FROM geo_place_alerts WHERE owner_id = $1 AND status IN ($2, $3)")
                .bind(owner)
                .bind(PENDING)
                .bind(ACTIVE)
                .fetch_one(&self.core.db)
                .await?;
            ensure!(open < MAX_ALERTS, precondition, "TOO_MANY_ALERTS", "at most {MAX_ALERTS} alerts at a time");

            // Recurring alerts wait for the subject's approval; one-time
            // alerts work at once and lapse if they never fire.
            let (status, lifetime_hours) = if req.recurring { (PENDING, self.cfg.pending_alert_days * 24) } else { (ACTIVE, self.cfg.one_time_alert_hours) };
            let mut tx = self.core.db.begin().await?;
            let row: AlertRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "INSERT INTO geo_place_alerts
                     (owner_id, subject_id, place_name, lat, lng, radius_m, subject_place_label, trigger, recurring, status, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now() + make_interval(hours => $11))
                 RETURNING {ALERT_COLUMNS}"
            )))
            .bind(owner)
            .bind(subject)
            .bind(&place_name)
            .bind(target.map(|t| t.0))
            .bind(target.map(|t| t.1))
            .bind(target.map(|t| t.2))
            .bind(label)
            .bind(trigger as i16)
            .bind(req.recurring)
            .bind(status)
            .bind(lifetime_hours as i32)
            .fetch_one(&mut *tx)
            .await?;
            let name = self.display_name(owner).await?;
            let (place_ja, place_en) = place_words(label as i32, &place_name);
            let (verb_ja, verb_en) = if trigger == ARRIVE { ("に着いたら", "arrive at") } else { ("を出たら", "leave") };
            let notification = if req.recurring {
                NotificationRequest::new(
                    subject,
                    "geo.place_alert_request",
                    Localized::new(format!("{name}さんから通知のリクエストが届きました"), format!("{name} asked to be alerted about you")),
                    Localized::new(
                        format!("あなたが{place_ja}{verb_ja}毎回{name}さんに通知します。承認しますか?"),
                        format!("{name} would be told every time you {verb_en} {place_en}. Approve?"),
                    ),
                    "/safety/alerts",
                )
            } else {
                NotificationRequest::new(
                    subject,
                    "geo.place_alert_set",
                    Localized::new(format!("{name}さんが通知を設定しました"), format!("{name} set an alert about you")),
                    Localized::new(
                        format!("あなたが{place_ja}{verb_ja}{name}さんに一度だけ通知します"),
                        format!("{name} will be told once when you {verb_en} {place_en}"),
                    ),
                    "/safety/alerts",
                )
            };
            outbound::notify(&mut *tx, &notification.actor(owner).data(serde_json::json!({ "alert_id": row.id }))).await?;
            tx.commit().await?;
            self.invalidate_ctx(&[subject]).await;
            self.push_alert(&row).await?;
            ok(CreatePlaceAlertResponse { alert: self.alert_message(&row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn respond_place_alert(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RespondPlaceAlertRequest>,
    ) -> ServiceResult<RespondPlaceAlertResponse> {
        handle(async {
            let subject = ctx.user_id()?;
            let alert_id = parse_id(request.alert_id)?;
            let approve = request.approve;
            let existing =
                self.alert_row(alert_id).await?.filter(|row| row.subject_id == subject).ok_or_else(alert_not_found)?;
            ensure!(
                existing.status == PENDING,
                precondition,
                "ALERT_NOT_PENDING",
                "this alert is not waiting for an answer"
            );
            let mut tx = self.core.db.begin().await?;
            let row: Option<AlertRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "UPDATE geo_place_alerts SET status = $2, responded_at = now(), expires_at = NULL
                 WHERE id = $1 AND status = {PENDING} RETURNING {ALERT_COLUMNS}"
            )))
            .bind(alert_id)
            .bind(if approve { ACTIVE } else { DECLINED })
            .fetch_optional(&mut *tx)
            .await?;
            let Some(row) = row else {
                bail!(precondition, "ALERT_NOT_PENDING", "this alert is not waiting for an answer")
            };
            let name = self.display_name(subject).await?;
            let title = if approve {
                Localized::new(format!("{name}さんが通知を承認しました"), format!("{name} approved your alert"))
            } else {
                Localized::new(format!("{name}さんが通知を承認しませんでした"), format!("{name} declined your alert"))
            };
            let notification = NotificationRequest::new(
                row.owner_id,
                "geo.place_alert_response",
                title,
                Localized::default(),
                "/safety/alerts",
            )
            .actor(subject)
            .data(serde_json::json!({ "alert_id": row.id, "approved": approve }));
            outbound::notify(&mut *tx, &notification).await?;
            tx.commit().await?;
            self.invalidate_ctx(&[subject]).await;
            self.push_alert(&row).await?;
            ok(RespondPlaceAlertResponse { alert: self.alert_message(&row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_place_alerts(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListPlaceAlertsRequest>,
    ) -> ServiceResult<ListPlaceAlertsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            // The owner also sees how their alerts ended, for a week.
            let mine: Vec<AlertRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {ALERT_COLUMNS} FROM geo_place_alerts
                 WHERE owner_id = $1 AND (status IN ({PENDING}, {ACTIVE}) OR created_at > now() - interval '7 days')
                 ORDER BY created_at DESC LIMIT 200"
            )))
            .bind(user_id)
            .fetch_all(&self.core.db)
            .await?;
            let about_me: Vec<AlertRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {ALERT_COLUMNS} FROM geo_place_alerts WHERE subject_id = $1 AND status IN ({PENDING}, {ACTIVE}) ORDER BY created_at DESC LIMIT 200"
            )))
            .bind(user_id)
            .fetch_all(&self.core.db)
            .await?;
            ok(ListPlaceAlertsResponse { mine: self.alert_messages(&mine).await?, about_me: self.alert_messages(&about_me).await?, ..Default::default() })
        })
        .await
    }

    async fn delete_place_alert(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeletePlaceAlertRequest>,
    ) -> ServiceResult<DeletePlaceAlertResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let alert_id = parse_id(request.alert_id)?;
            let row: Option<AlertRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "DELETE FROM geo_place_alerts WHERE id = $1 AND (owner_id = $2 OR subject_id = $2) RETURNING {ALERT_COLUMNS}"
            )))
            .bind(alert_id)
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
            let mut row = row.ok_or_else(alert_not_found)?;
            self.invalidate_ctx(&[row.subject_id]).await;
            // There is no "deleted" event: the other side sees the alert lapse.
            row.status = EXPIRED;
            self.push_alert(&row).await?;
            ok(DeletePlaceAlertResponse::default())
        })
        .await
    }

    async fn trigger_sos(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, TriggerSosRequest>,
    ) -> ServiceResult<TriggerSosResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let message = req.message.trim();
            ensure!(
                message.chars().count() <= MAX_SOS_MESSAGE_CHARS,
                invalid,
                "MESSAGE_TOO_LONG",
                "an SOS message has at most {MAX_SOS_MESSAGE_CHARS} characters"
            );
            ensure!(
                req.notify_user_ids.len() <= MAX_SOS_RECIPIENTS,
                invalid,
                "TOO_MANY_USERS",
                "at most {MAX_SOS_RECIPIENTS} people per SOS"
            );
            if let Some(active) = self.active_sos(user_id).await? {
                return ok(TriggerSosResponse {
                    sos: self.sos_message(&active, user_id).await?.into(),
                    ..Default::default()
                });
            }
            let friends = graph::friend_ids(&self.core, user_id).await?;
            let mut recipients = parse_ids(&req.notify_user_ids)?;
            recipients.sort();
            recipients.dedup();
            ensure!(
                recipients.iter().all(|id| friends.contains(id)),
                precondition,
                "NOT_FRIENDS",
                "an SOS can only alert friends"
            );
            if recipients.is_empty() {
                recipients = friends;
            }
            ensure!(
                !recipients.is_empty(),
                precondition,
                "NO_RECIPIENTS",
                "there is nobody to alert: add a friend first"
            );
            self.core.cache.rate_limit(&format!("geo-sos:{user_id}"), 10, Duration::from_secs(3600)).await?;

            let name = self.display_name(user_id).await?;
            let mut tx = self.core.db.begin().await?;
            let inserted: Option<SosRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "INSERT INTO geo_sos (user_id, message, notify_user_ids) VALUES ($1, $2, $3)
                 ON CONFLICT (user_id) WHERE resolved_at IS NULL DO NOTHING RETURNING {SOS_COLUMNS}"
            )))
            .bind(user_id)
            .bind(message)
            .bind(&recipients)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(row) = inserted else {
                // Two taps raced: the first one is the SOS.
                drop(tx);
                let active =
                    self.active_sos(user_id).await?.ok_or_else(|| AppError::unavailable("SOS_BUSY", "try again"))?;
                return ok(TriggerSosResponse {
                    sos: self.sos_message(&active, user_id).await?.into(),
                    ..Default::default()
                });
            };
            let body = if message.is_empty() {
                Localized::new("タップして現在地を確認してください", "Tap to see where they are")
            } else {
                Localized::same(message)
            };
            for recipient in &row.notify_user_ids {
                let notification = NotificationRequest::new(
                    *recipient,
                    "geo.sos",
                    Localized::new(format!("🆘 {name}さんがSOSを発信しました"), format!("🆘 {name} sent an SOS")),
                    body.clone(),
                    format!("/map?sos={}", row.id),
                )
                .actor(user_id)
                .collapse(format!("geo.sos:{}", row.id))
                .data(serde_json::json!({ "sos_id": row.id }))
                .urgent();
                outbound::notify(&mut *tx, &notification).await?;
            }
            tx.commit().await?;
            self.broadcast_sos(&row).await?;
            ok(TriggerSosResponse { sos: self.sos_message(&row, user_id).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn resolve_sos(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ResolveSosRequest>,
    ) -> ServiceResult<ResolveSosResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let sos_id = parse_id(request.sos_id)?;
            let resolved: Option<SosRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "UPDATE geo_sos SET resolved_at = now() WHERE id = $1 AND user_id = $2 AND resolved_at IS NULL RETURNING {SOS_COLUMNS}"
            )))
            .bind(sos_id)
            .bind(user_id)
            .fetch_optional(&self.core.db)
            .await?;
            match resolved {
                Some(row) => self.end_sos(row).await?,
                None => {
                    // Resolving twice is fine.
                    let known: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM geo_sos WHERE id = $1 AND user_id = $2)")
                        .bind(sos_id)
                        .bind(user_id)
                        .fetch_one(&self.core.db)
                        .await?;
                    ensure!(known, not_found, "SOS_NOT_FOUND", "SOS not found");
                }
            }
            ok(ResolveSosResponse::default())
        })
        .await
    }

    async fn get_active_sos(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetActiveSosRequest>,
    ) -> ServiceResult<GetActiveSosResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let mine = match self.active_sos(user_id).await? {
                Some(row) => Some(self.sos_message(&row, user_id).await?),
                None => None,
            };
            let friends = graph::friend_ids(&self.core, user_id).await?;
            let rows: Vec<SosRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {SOS_COLUMNS} FROM geo_sos WHERE resolved_at IS NULL AND notify_user_ids @> ARRAY[$1]::uuid[] AND user_id = ANY($2) ORDER BY started_at DESC"
            )))
            .bind(user_id)
            .bind(&friends)
            .fetch_all(&self.core.db)
            .await?;
            let mut from_friends = Vec::with_capacity(rows.len());
            for row in &rows {
                from_friends.push(self.sos_message(row, user_id).await?);
            }
            ok(GetActiveSosResponse { mine: mine.into(), friends: from_friends, ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: (f64, f64) = (35.6595, 139.7005);

    fn alert(trigger: i32, inside: Option<bool>) -> AlertCtx {
        AlertCtx {
            id: Uuid::now_v7(),
            owner_id: Uuid::now_v7(),
            target: Some((HOME.0, HOME.1, 100.0)),
            label: 0,
            trigger,
            recurring: true,
            inside,
            last_fired_ms: None,
        }
    }

    fn pin_at(meters_from_home: f64) -> Pin {
        let (lat, lng) = math::destination(HOME.0, HOME.1, 90.0, meters_from_home);
        Pin { lat, lng, recorded_ms: 1, sharing: true, ..Default::default() }
    }

    fn verdict(alert: &AlertCtx, meters_from_home: f64) -> Verdict {
        judge(alert, alert.target, &pin_at(meters_from_home), true, 10_000_000)
    }

    #[test]
    fn the_first_look_remembers_the_side_without_firing() {
        assert_eq!(verdict(&alert(ARRIVE, None), 20.0), Verdict::Moved { inside: true, fire: false });
        assert_eq!(verdict(&alert(LEAVE, None), 500.0), Verdict::Moved { inside: false, fire: false });
    }

    #[test]
    fn arriving_fires_arrive_alerts_and_leaving_fires_leave_alerts() {
        assert_eq!(verdict(&alert(ARRIVE, Some(false)), 50.0), Verdict::Moved { inside: true, fire: true });
        assert_eq!(verdict(&alert(LEAVE, Some(false)), 50.0), Verdict::Moved { inside: true, fire: false });
        assert_eq!(verdict(&alert(LEAVE, Some(true)), 400.0), Verdict::Moved { inside: false, fire: true });
        assert_eq!(verdict(&alert(ARRIVE, Some(true)), 400.0), Verdict::Moved { inside: false, fire: false });
        assert_eq!(verdict(&alert(ARRIVE, Some(true)), 50.0), Verdict::Unchanged);
        assert_eq!(verdict(&alert(ARRIVE, Some(false)), 400.0), Verdict::Unchanged);
    }

    #[test]
    fn wobble_at_the_edge_does_not_count_as_leaving() {
        // Radius 100 m: in at 100, still in at 140, out beyond 145.
        assert_eq!(verdict(&alert(LEAVE, Some(true)), 140.0), Verdict::Unchanged);
        assert_eq!(verdict(&alert(LEAVE, Some(true)), 150.0), Verdict::Moved { inside: false, fire: true });
        assert_eq!(
            verdict(&alert(ARRIVE, Some(false)), 120.0),
            Verdict::Unchanged,
            "entering takes being within the radius"
        );
        assert!(is_inside(100.0, 100.0, None) && !is_inside(100.1, 100.0, None));
    }

    #[test]
    fn a_recurring_alert_rests_between_firings() {
        let mut recent = alert(ARRIVE, Some(false));
        recent.last_fired_ms = Some(10_000_000 - 60_000);
        assert_eq!(verdict(&recent, 50.0), Verdict::Moved { inside: true, fire: false });
        recent.last_fired_ms = Some(10_000_000 - REFIRE_AFTER_MS);
        assert_eq!(verdict(&recent, 50.0), Verdict::Moved { inside: true, fire: true });
    }

    #[test]
    fn ghost_mode_silences_alerts_and_forgets_the_side() {
        let watching = alert(ARRIVE, Some(false));
        assert_eq!(judge(&watching, watching.target, &pin_at(50.0), false, 0), Verdict::Forget);
        let forgotten = alert(ARRIVE, None);
        assert_eq!(judge(&forgotten, forgotten.target, &pin_at(50.0), false, 0), Verdict::Unchanged);
        // Visible again while already inside: a first look, so nothing fires for
        // an arrival that happened under ghost mode.
        assert_eq!(verdict(&forgotten, 50.0), Verdict::Moved { inside: true, fire: false });
    }

    #[test]
    fn an_alert_on_a_place_the_subject_does_not_have_does_nothing() {
        let labelled = AlertCtx { target: None, label: 1, ..alert(ARRIVE, Some(false)) };
        assert_eq!(judge(&labelled, None, &pin_at(0.0), true, 0), Verdict::Unchanged);
    }

    #[test]
    fn places_are_worded_by_label_or_name() {
        assert_eq!(place_words(1, "whatever"), ("自宅".to_owned(), "home".to_owned()));
        assert_eq!(place_words(0, "駅"), ("「駅」".to_owned(), "駅".to_owned()));
        assert_eq!(place_words(0, "").1, "the place");
    }
}
