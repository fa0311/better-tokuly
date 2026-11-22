//! Events of a community: the calendar, RSVP with a waitlist that moves up
//! by itself, reminders, and check-in with a rotating code.
use std::{collections::HashMap, time::Duration};

use sqlx::{Postgres, Transaction};
use tokuly_core::{events as core_events, keys::Keys, outbound, prelude::*};
use tokuly_media::assets::{self, image_from_json};
use tokuly_proto::{
    common::{LatLng, PageInfo},
    community::{self as pb, ModerationAction, Permission, RsvpAnswer},
    media::AssetPurpose,
};

use crate::{
    Community,
    access::{Access, Caller, not_a_member, permission_denied},
    events::{self, EventAttended, EventChanged},
    model::{
        ATTENDEE_COLUMNS, AttendeeRow, CommunityRow, EventRow, MAX_EDITS, attendance, community_columns, event_columns,
        level, revised,
    },
    modlog::{self, LogEntry},
    notices::{self, EventUpdate, Reminder},
    threads::record_revision,
    validate,
};

/// Upload purposes accepted for an event's cover.
const COVER_PURPOSES: [AssetPurpose; 4] = [
    AssetPurpose::ASSET_PURPOSE_BANNER,
    AssetPurpose::ASSET_PURPOSE_CHANNEL_BANNER,
    AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE,
    AssetPurpose::ASSET_PURPOSE_THUMBNAIL,
];
/// Check-in opens this long before the start and closes this long after the end.
const CHECK_IN_OPENS_BEFORE: chrono::Duration = chrono::Duration::hours(1);
const CHECK_IN_CLOSES_AFTER: chrono::Duration = chrono::Duration::hours(6);
/// Label of the HMAC key of check-in codes.
const CODE_KEY_LABEL: &str = "community/check-in/v1";
/// Longest calendar range one request may ask for.
const MAX_RANGE: chrono::Duration = chrono::Duration::days(92);
/// Events reminded per pass of the worker.
const REMINDERS_PER_PASS: usize = 200;

pub fn event_not_found() -> AppError {
    AppError::not_found("EVENT_NOT_FOUND", "event not found")
}

pub fn cover_holder(event_id: Uuid) -> String {
    format!("community:event:{event_id}")
}

/// What an event says in words, as one text: the description, then the
/// place and the link on lines of their own. This is what the edit history
/// keeps, so that no part of an event can be reworded without a trace.
fn wording(event: &EventRow) -> String {
    let mut text = event.description.clone();
    for extra in [&event.place_name, &event.place_address, &event.online_url] {
        if !extra.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(extra);
        }
    }
    text
}

/// The check-in code of an event during one period: six digits from a
/// keyed hash of the event and the period's number. Knowing one code tells
/// nothing about the next, or about another event's.
pub fn check_in_code(keys: &Keys, event_id: Uuid, step: i64) -> String {
    let mut input = [0u8; 24];
    input[..16].copy_from_slice(event_id.as_bytes());
    input[16..].copy_from_slice(&step.to_be_bytes());
    let mac = keys.mac(CODE_KEY_LABEL, &input);
    let number = u32::from_be_bytes([mac[0], mac[1], mac[2], mac[3]]) % 1_000_000;
    format!("{number:06}")
}

/// Whether `code` is the code of the current period or of the one before
/// (a code is accepted a little longer than it is shown).
pub fn code_matches(keys: &Keys, event_id: Uuid, step: i64, code: &str) -> bool {
    let code = code.trim();
    code.len() == 6 && [step, step - 1].into_iter().any(|step| check_in_code(keys, event_id, step) == code)
}

/// Whether check-in is open at `at`.
pub fn check_in_open(event: &EventRow, at: Time) -> bool {
    at >= event.starts_at - CHECK_IN_OPENS_BEFORE && at <= event.ends_at + CHECK_IN_CLOSES_AFTER
}

/// Where a member ends up when they answer an event: `(status, queued)`.
/// "Going" keeps a place already held (or a place in the queue) and
/// otherwise takes a free place or joins the waitlist.
pub fn resolve_answer(answer: RsvpAnswer, current: Option<i32>, has_room: bool) -> Option<i32> {
    match answer {
        RsvpAnswer::RSVP_ANSWER_GOING => Some(match current {
            Some(attendance::GOING) => attendance::GOING,
            Some(attendance::WAITLISTED) => attendance::WAITLISTED,
            _ if has_room => attendance::GOING,
            _ => attendance::WAITLISTED,
        }),
        RsvpAnswer::RSVP_ANSWER_INTERESTED => Some(attendance::INTERESTED),
        RsvpAnswer::RSVP_ANSWER_NOT_GOING => Some(attendance::NOT_GOING),
        _ => None,
    }
}

/// What a change of status does to the three counters: `(going, waitlist, interested)`.
fn counter_deltas(from: Option<i32>, to: Option<i32>) -> (i32, i32, i32) {
    let weight = |status: Option<i32>| match status {
        Some(attendance::GOING) => (1, 0, 0),
        Some(attendance::WAITLISTED) => (0, 1, 0),
        Some(attendance::INTERESTED) => (0, 0, 1),
        _ => (0, 0, 0),
    };
    let (from, to) = (weight(from), weight(to));
    (to.0 - from.0, to.1 - from.1, to.2 - from.2)
}

async fn adjust_counters(tx: &mut Transaction<'_, Postgres>, event_id: Uuid, deltas: (i32, i32, i32)) -> AppResult<()> {
    sqlx::query(
        "UPDATE community_events SET going_count = greatest(going_count + $2, 0),
                waitlist_count = greatest(waitlist_count + $3, 0),
                interested_count = greatest(interested_count + $4, 0)
         WHERE id = $1",
    )
    .bind(event_id)
    .bind(deltas.0)
    .bind(deltas.1)
    .bind(deltas.2)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The event locked for the rest of the transaction: answers to one event
/// queue up, so its counters and its waitlist stay exact.
async fn lock_event(tx: &mut Transaction<'_, Postgres>, event_id: Uuid) -> AppResult<EventRow> {
    let event: Option<EventRow> = sqlx::query_as(concat!(
        "SELECT ",
        event_columns!(),
        " FROM community_events e WHERE e.id = $1 AND e.deleted_at IS NULL FOR UPDATE"
    ))
    .bind(event_id)
    .fetch_optional(&mut **tx)
    .await?;
    event.ok_or_else(event_not_found)
}

/// What an update changes; `None` leaves a field alone.
#[derive(Debug, Default)]
pub(crate) struct EventEdit {
    pub title: Option<String>,
    pub description: Option<String>,
    pub starts_at: Option<Time>,
    pub ends_at: Option<Time>,
    pub place: Option<validate::Place>,
    pub online: Option<bool>,
    pub online_url: Option<String>,
    /// `Some(None)` removes the cover.
    pub cover_asset_id: Option<Option<Uuid>>,
    pub capacity: Option<i32>,
}

impl Community {
    pub(crate) async fn event_by_id(&self, event_id: Uuid) -> AppResult<Option<EventRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            event_columns!(),
            " FROM community_events e WHERE e.id = $1 AND e.deleted_at IS NULL"
        ))
        .bind(event_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// The event named by `event_id` as `caller` may read it, with the
    /// caller's standing in its community.
    pub(crate) async fn open_event(&self, caller: &Caller, event_id: &str) -> AppResult<(EventRow, Access)> {
        let id = Uuid::parse_str(event_id.trim()).map_err(|_| event_not_found())?;
        let event = self.event_by_id(id).await?.ok_or_else(event_not_found)?;
        let access = match self.access_by_id(caller, event.community_id).await {
            Ok(access) => access,
            Err(error) if error.is_not_found() => return Err(event_not_found()),
            Err(error) => return Err(error),
        };
        access.require_read()?;
        Ok((event, access))
    }

    /// Whether the caller may edit, cancel and run check-in: the organiser
    /// (while a member) and the staff.
    fn manages_event(access: &Access, event: &EventRow) -> bool {
        access.moderates() || (access.is_member() && access.user_id == Some(event.organizer_id))
    }

    fn require_manager(access: &Access, event: &EventRow) -> AppResult<Uuid> {
        if !Self::manages_event(access, event) {
            return Err(permission_denied());
        }
        // A timeout silences: an organiser who is timed out does not edit,
        // move or cancel (each of which tells every attendee) until it is over.
        access.require_active()?;
        access.user_id.ok_or_else(permission_denied)
    }

    /// Events of the community of `access` as its caller sees them.
    pub(crate) async fn render_events(&self, access: &Access, rows: &[EventRow]) -> AppResult<Vec<pb::CalendarEvent>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let core = &self.core;
        let organizers: Vec<Uuid> = rows.iter().map(|row| row.organizer_id).collect();
        let users = tokuly_identity::summaries(core, &organizers).await?;
        let mut mine: HashMap<Uuid, (AttendeeRow, i64)> = HashMap::new();
        if let Some(user_id) = access.user_id {
            let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
            // The position on a waitlist counts who queued before the viewer.
            let answers: Vec<(Uuid, i32, Time, Option<Time>, Option<Time>, i64)> = sqlx::query_as(
                "SELECT a.event_id, a.status, a.responded_at, a.queued_at, a.checked_in_at,
                        CASE WHEN a.status = 2 THEN
                            (SELECT count(*) FROM community_event_attendees w
                             WHERE w.event_id = a.event_id AND w.status = 2
                               AND (w.queued_at, w.user_id) <= (a.queued_at, a.user_id))
                        ELSE 0 END
                 FROM community_event_attendees a WHERE a.event_id = ANY($1) AND a.user_id = $2",
            )
            .bind(&ids)
            .bind(user_id)
            .fetch_all(&core.db)
            .await?;
            for (event_id, status, responded_at, queued_at, checked_in_at, position) in answers {
                let row = AttendeeRow { user_id, status, responded_at, queued_at, checked_in_at };
                mine.insert(event_id, (row, position));
            }
        }
        let community = &access.community;
        Ok(rows
            .iter()
            .map(|row| {
                let answer = mine.get(&row.id);
                let can_manage = Self::manages_event(access, row);
                let going = answer.is_some_and(|(answer, _)| answer.status == attendance::GOING);
                let viewer = access.user_id.map(|_| pb::CalendarEventViewer {
                    status: answer.map_or(0, |(answer, _)| answer.status).into(),
                    waitlist_position: answer.map_or(0, |(_, position)| *position as i32),
                    checked_in_at: opt_ts(answer.and_then(|(answer, _)| answer.checked_in_at)),
                    can_manage,
                    ..Default::default()
                });
                let place = (!row.online).then(|| pb::EventPlace {
                    name: row.place_name.clone(),
                    address: row.place_address.clone(),
                    location: match (row.place_lat, row.place_lng) {
                        (Some(lat), Some(lng)) => Some(LatLng { lat, lng, ..Default::default() }),
                        _ => None,
                    }
                    .into(),
                    ..Default::default()
                });
                pb::CalendarEvent {
                    id: row.id.to_string(),
                    community_id: row.community_id.to_string(),
                    organizer: tokuly_identity::users::pick(&users, row.organizer_id).into(),
                    title: row.title.clone(),
                    description: row.description.clone(),
                    starts_at: ts(row.starts_at),
                    ends_at: ts(row.ends_at),
                    place: place.into(),
                    online: row.online,
                    // The link is for the people who are coming.
                    online_url: if going || can_manage { row.online_url.clone() } else { String::new() },
                    cover: image_from_json(row.cover.as_ref()).into(),
                    capacity: row.capacity,
                    going_count: row.going_count,
                    waitlist_count: row.waitlist_count,
                    interested_count: row.interested_count,
                    attended_count: row.attended_count,
                    cancelled: row.is_cancelled(),
                    cancel_reason: row.cancel_reason.clone(),
                    like_count: row.like_count,
                    comment_count: row.comment_count,
                    created_at: ts(row.created_at),
                    path: row.path(community),
                    viewer: viewer.into(),
                    ..Default::default()
                }
            })
            .collect())
    }

    pub(crate) async fn render_event(&self, access: &Access, row: &EventRow) -> AppResult<pb::CalendarEvent> {
        let mut events = self.render_events(access, std::slice::from_ref(row)).await?;
        events.pop().ok_or_else(event_not_found)
    }

    /// Tells the community's topic that an event changed (or went away).
    /// The event travels as an outsider sees it: no viewer, no link.
    pub(crate) async fn push_event_changed(&self, community: &CommunityRow, event_id: Uuid) {
        let outsider = Access {
            community: community.clone(),
            user_id: None,
            site_moderator: false,
            member: None,
            ban: None,
            requested: false,
        };
        // Only a public community's events travel on the topic (see
        // `pushable`); elsewhere followers fetch the event by its ID.
        let event = match self.event_by_id(event_id).await {
            Ok(Some(row)) if community.is_public() => self.render_event(&outsider, &row).await.ok(),
            _ => None,
        };
        let push = pb::CalendarEventChanged {
            community_id: community.id.to_string(),
            event_id: event_id.to_string(),
            event: event.into(),
            ..Default::default()
        };
        self.push_topic(community.id, push).await;
    }

    /// Puts an event on the calendar; the caller organises it and is going.
    pub(crate) async fn create_event(&self, access: &Access, req: &pb::CreateEventRequest) -> AppResult<EventRow> {
        access.require(Permission::PERMISSION_CREATE_EVENT)?;
        let organizer = access.require_active()?.user_id;
        let core = &self.core;
        let community = &access.community;
        let title = validate::title(&req.title)?;
        let description = validate::description(&req.description, validate::MAX_EVENT_DESCRIPTION_CHARS)?;
        let starts_at = req
            .starts_at
            .as_option()
            .map(from_ts)
            .ok_or_else(|| AppError::invalid("TIME_INVALID", "an event needs a start time"))?;
        let (starts_at, ends_at) = validate::event_times(starts_at, req.ends_at.as_option().map(from_ts), now(), true)?;
        let place = match (req.online, req.place.as_option()) {
            (true, _) => None,
            (false, Some(place)) => Some(validate::place(place)?),
            (false, None) => bail!(invalid, "PLACE_INVALID", "an event takes place somewhere, or online"),
        };
        let online_url = if req.online { validate::online_url(&req.online_url)? } else { String::new() };
        let capacity = validate::capacity(req.capacity)?;
        core.cache.rate_limit(&format!("community:event:{organizer}"), 20, Duration::from_secs(3600)).await?;
        let cover_id = parse_opt_id(&req.cover_asset_id)?;
        let cover = match cover_id {
            Some(id) => Some(assets::require_image_json(core, id, organizer, &COVER_PURPOSES).await?),
            None => None,
        };

        let mut tx = core.db.begin().await?;
        let event: EventRow = sqlx::query_as(concat!(
            "INSERT INTO community_events AS e
                 (community_id, organizer_id, title, description, starts_at, ends_at, online, online_url,
                  place_name, place_address, place_lat, place_lng, cover, cover_asset_id, capacity, going_count)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, 1)
             RETURNING ",
            event_columns!()
        ))
        .bind(community.id)
        .bind(organizer)
        .bind(&title)
        .bind(&description)
        .bind(starts_at)
        .bind(ends_at)
        .bind(req.online)
        .bind(&online_url)
        .bind(place.as_ref().map(|place| place.name.clone()).unwrap_or_default())
        .bind(place.as_ref().map(|place| place.address.clone()).unwrap_or_default())
        .bind(place.as_ref().and_then(|place| place.location).map(|(lat, _)| lat))
        .bind(place.as_ref().and_then(|place| place.location).map(|(_, lng)| lng))
        .bind(&cover)
        .bind(cover_id)
        .bind(capacity)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO community_event_attendees (event_id, user_id, status) VALUES ($1, $2, $3)")
            .bind(event.id)
            .bind(organizer)
            .bind(attendance::GOING)
            .execute(&mut *tx)
            .await?;
        if let Some(id) = cover_id {
            assets::attach(&mut *tx, id, &cover_holder(event.id)).await?;
        }
        core_events::emit(
            &mut *tx,
            events::EVENT_CREATED,
            Some(organizer),
            &EventChanged {
                community_id: community.id,
                event_id: event.id,
                organizer_id: organizer,
                public: community.is_public(),
            },
        )
        .await?;
        tx.commit().await?;
        self.push_event_changed(community, event.id).await;
        Ok(event)
    }

    /// Edits an event: the organiser or the staff.
    pub(crate) async fn update_event(&self, access: &Access, event: &EventRow, edit: EventEdit) -> AppResult<EventRow> {
        let actor = Self::require_manager(access, event)?;
        ensure!(!event.is_cancelled(), precondition, "EVENT_CANCELLED", "this event was cancelled");
        let core = &self.core;
        let community = &access.community;
        let moved = edit.starts_at.is_some_and(|starts_at| starts_at != event.starts_at);
        let (starts_at, ends_at) = validate::event_times(
            edit.starts_at.unwrap_or(event.starts_at),
            // An end that was not sent moves with the start, keeping the length.
            Some(edit.ends_at.unwrap_or(
                edit.starts_at.map_or(event.ends_at, |starts_at| starts_at + (event.ends_at - event.starts_at)),
            )),
            now(),
            moved,
        )?;
        // A place makes the event an in-person one; `online` makes it an online one.
        let online = match (&edit.place, edit.online) {
            (Some(_), _) => false,
            (None, Some(online)) => online,
            (None, None) => event.online,
        };
        ensure!(
            online || edit.place.is_some() || !event.place_name.is_empty(),
            invalid,
            "PLACE_INVALID",
            "an event takes place somewhere, or online"
        );
        let cover = match edit.cover_asset_id {
            Some(Some(id)) if Some(id) != event.cover_asset_id => {
                Some((Some(id), Some(assets::require_image_json(core, id, actor, &COVER_PURPOSES).await?)))
            }
            Some(None) => Some((None, None)),
            _ => None,
        };
        let (place_name, place_address, place_location) = match (&edit.place, online) {
            (Some(place), _) => (place.name.clone(), place.address.clone(), place.location),
            (None, true) => (String::new(), String::new(), None),
            (None, false) => {
                (event.place_name.clone(), event.place_address.clone(), event.place_lat.zip(event.place_lng))
            }
        };
        let online_url =
            if online { edit.online_url.clone().unwrap_or_else(|| event.online_url.clone()) } else { String::new() };
        self.limit("event-edit", actor, 30, Duration::from_secs(3600)).await?;

        let mut tx = core.db.begin().await?;
        let before = lock_event(&mut tx, event.id).await?;
        let reworded = edit.title.as_ref().is_some_and(|title| *title != before.title)
            || edit.description.as_ref().is_some_and(|description| *description != before.description)
            || place_name != before.place_name
            || place_address != before.place_address
            || online_url != before.online_url;
        ensure!(
            !reworded || before.edit_count < MAX_EDITS,
            precondition,
            "TOO_MANY_EDITS",
            "an event can be edited {MAX_EDITS} times"
        );
        if reworded {
            record_revision(&mut tx, revised::EVENT, event.id, &before.title, &wording(&before)).await?;
        }
        let updated: EventRow = sqlx::query_as(concat!(
            "UPDATE community_events AS e SET
                 title = coalesce($2, title), description = coalesce($3, description),
                 starts_at = $4, ends_at = $5, online = $6, online_url = $7,
                 place_name = $8, place_address = $9, place_lat = $10, place_lng = $11,
                 capacity = coalesce($12, capacity),
                 cover = CASE WHEN $13 THEN $14 ELSE cover END,
                 cover_asset_id = CASE WHEN $13 THEN $15 ELSE cover_asset_id END,
                 reminded_day_at = CASE WHEN $16 THEN NULL ELSE reminded_day_at END,
                 reminded_hour_at = CASE WHEN $16 THEN NULL ELSE reminded_hour_at END,
                 edit_count = edit_count + $17::int
             WHERE e.id = $1 RETURNING ",
            event_columns!()
        ))
        .bind(event.id)
        .bind(&edit.title)
        .bind(&edit.description)
        .bind(starts_at)
        .bind(ends_at)
        .bind(online)
        .bind(&online_url)
        .bind(&place_name)
        .bind(&place_address)
        .bind(place_location.map(|(lat, _)| lat))
        .bind(place_location.map(|(_, lng)| lng))
        .bind(edit.capacity)
        .bind(cover.is_some())
        .bind(cover.as_ref().and_then(|(_, image)| image.clone()))
        .bind(cover.as_ref().and_then(|(id, _)| *id))
        .bind(moved)
        .bind(reworded)
        .fetch_one(&mut *tx)
        .await?;
        if let Some((new_id, _)) = &cover {
            if let Some(old) = event.cover_asset_id {
                assets::detach(&mut *tx, old, &cover_holder(event.id)).await?;
            }
            if let Some(new) = new_id {
                assets::attach(&mut *tx, *new, &cover_holder(event.id)).await?;
            }
        }
        // More room lets people off the waitlist.
        self.promote(&mut tx, community, &updated).await?;
        if moved {
            for recipient in self.answered(&mut tx, event.id, actor).await? {
                let notice = notices::event_update(recipient, community, &updated, &EventUpdate::Rescheduled);
                outbound::notify(&mut *tx, &notice.actor(actor)).await?;
            }
        }
        tx.commit().await?;
        self.push_event_changed(community, event.id).await;
        self.event_by_id(event.id).await?.ok_or_else(event_not_found)
    }

    /// Everyone who answered an event with anything but "not going", `skip` aside.
    async fn answered(&self, tx: &mut Transaction<'_, Postgres>, event_id: Uuid, skip: Uuid) -> AppResult<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT user_id FROM community_event_attendees WHERE event_id = $1 AND status IN (1, 2, 3) AND user_id <> $2",
        )
        .bind(event_id)
        .bind(skip)
        .fetch_all(&mut **tx)
        .await?)
    }

    /// Calls an event off: the organiser or the staff. It stays visible.
    pub(crate) async fn cancel_event(&self, access: &Access, event: &EventRow, reason: &str) -> AppResult<EventRow> {
        let actor = Self::require_manager(access, event)?;
        let reason = validate::reason(reason, false)?;
        let community = &access.community;
        self.limit("event-edit", actor, 30, Duration::from_secs(3600)).await?;
        let mut tx = self.core.db.begin().await?;
        let cancelled: Option<EventRow> = sqlx::query_as(concat!(
            "UPDATE community_events AS e SET cancelled_at = now(), cancel_reason = $2
             WHERE e.id = $1 AND e.deleted_at IS NULL AND e.cancelled_at IS NULL RETURNING ",
            event_columns!()
        ))
        .bind(event.id)
        .bind(&reason)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(cancelled) = &cancelled {
            let update = EventUpdate::Cancelled { reason: reason.clone() };
            for recipient in self.answered(&mut tx, event.id, actor).await? {
                let notice = notices::event_update(recipient, community, cancelled, &update);
                outbound::notify(&mut *tx, &notice.actor(actor)).await?;
            }
            if actor != event.organizer_id {
                let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_EVENT_REMOVED)
                    .by(actor)
                    .event(event.id)
                    .target(event.organizer_id)
                    .reason(&reason)
                    .detail("title", &event.title)
                    .detail("how", "cancelled");
                modlog::record(&mut *tx, &entry).await?;
            }
        }
        tx.commit().await?;
        self.push_event_changed(community, event.id).await;
        self.event_by_id(event.id).await?.ok_or_else(event_not_found)
    }

    /// Takes an event off the calendar inside `tx`. False when it was gone.
    /// What it said and its cover stay on file until the retention is over
    /// (see `retention`).
    pub(crate) async fn remove_event(&self, tx: &mut Transaction<'_, Postgres>, event: &EventRow) -> AppResult<bool> {
        let removed =
            sqlx::query("UPDATE community_events SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL")
                .bind(event.id)
                .execute(&mut **tx)
                .await?;
        Ok(removed.rows_affected() > 0)
    }

    /// Removes an event: the organiser or the staff.
    pub(crate) async fn delete_event(&self, access: &Access, event: &EventRow) -> AppResult<()> {
        let actor = Self::require_manager(access, event)?;
        let community = &access.community;
        self.limit("event-edit", actor, 30, Duration::from_secs(3600)).await?;
        let mut tx = self.core.db.begin().await?;
        let removed = self.remove_event(&mut tx, event).await?;
        if removed && actor != event.organizer_id {
            let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_EVENT_REMOVED)
                .by(actor)
                .event(event.id)
                .target(event.organizer_id)
                .detail("title", &event.title)
                .detail("how", "deleted");
            modlog::record(&mut *tx, &entry).await?;
        }
        tx.commit().await?;
        if removed {
            self.push_event_changed(community, event.id).await;
        }
        Ok(())
    }

    /// The community's calendar: a time range, or what is coming up, or
    /// what is over.
    pub(crate) async fn list_events(
        &self,
        community_id: Uuid,
        range: Option<(Time, Time)>,
        past: bool,
        page: &Page,
    ) -> AppResult<(Vec<EventRow>, PageInfo)> {
        let after: Option<(Time, Uuid)> = page.decode()?;
        let (after_at, after_id) = (after.map(|(at, _)| at), after.map(|(_, id)| id));
        let db = &self.core.db;
        let mut rows: Vec<EventRow> = match (range, past) {
            (Some((from, to)), _) => {
                ensure!(
                    to > from && to - from <= MAX_RANGE,
                    invalid,
                    "RANGE_INVALID",
                    "a calendar range covers at most 92 days"
                );
                sqlx::query_as(concat!(
                    "SELECT ",
                    event_columns!(),
                    " FROM community_events e
                      WHERE e.community_id = $1 AND e.deleted_at IS NULL AND e.starts_at < $3 AND e.ends_at > $2
                        AND ($4::timestamptz IS NULL OR (e.starts_at, e.id) > ($4, $5))
                      ORDER BY e.starts_at, e.id LIMIT $6"
                ))
                .bind(community_id)
                .bind(from)
                .bind(to)
                .bind(after_at)
                .bind(after_id)
                .bind(page.fetch())
                .fetch_all(db)
                .await?
            }
            (None, false) => {
                sqlx::query_as(concat!(
                    "SELECT ",
                    event_columns!(),
                    " FROM community_events e
                      WHERE e.community_id = $1 AND e.deleted_at IS NULL AND e.ends_at > now()
                        AND ($2::timestamptz IS NULL OR (e.starts_at, e.id) > ($2, $3))
                      ORDER BY e.starts_at, e.id LIMIT $4"
                ))
                .bind(community_id)
                .bind(after_at)
                .bind(after_id)
                .bind(page.fetch())
                .fetch_all(db)
                .await?
            }
            (None, true) => {
                sqlx::query_as(concat!(
                    "SELECT ",
                    event_columns!(),
                    " FROM community_events e
                      WHERE e.community_id = $1 AND e.deleted_at IS NULL AND e.ends_at <= now()
                        AND ($2::timestamptz IS NULL OR (e.ends_at, e.id) < ($2, $3))
                      ORDER BY e.ends_at DESC, e.id DESC LIMIT $4"
                ))
                .bind(community_id)
                .bind(after_at)
                .bind(after_id)
                .bind(page.fetch())
                .fetch_all(db)
                .await?
            }
        };
        let info = page.finish(&mut rows, |row| match (range, past) {
            (None, true) => (row.ends_at, row.id),
            _ => (row.starts_at, row.id),
        });
        Ok((rows, info))
    }

    /// Events that are not over which `user_id` answered with anything but
    /// "not going", in communities they are still a member of.
    pub(crate) async fn my_events(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<EventRow>, PageInfo)> {
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<EventRow> = sqlx::query_as(concat!(
            "SELECT ",
            event_columns!(),
            " FROM community_event_attendees a
              JOIN community_events e ON e.id = a.event_id
              JOIN communities c ON c.id = e.community_id AND c.deleted_at IS NULL
              JOIN community_members m ON m.community_id = e.community_id AND m.user_id = a.user_id
              WHERE a.user_id = $1 AND a.status IN (1, 2, 3) AND e.deleted_at IS NULL AND e.ends_at > now()
                AND ($2::timestamptz IS NULL OR (e.starts_at, e.id) > ($2, $3))
              ORDER BY e.starts_at, e.id LIMIT $4"
        ))
        .bind(user_id)
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.starts_at, row.id));
        Ok((rows, info))
    }

    /// Moves people off the waitlist while there is room, first come first
    /// served, and tells them. The event row must be locked by the caller.
    async fn promote(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        event: &EventRow,
    ) -> AppResult<()> {
        let (capacity, going, waiting): (i32, i32, i32) =
            sqlx::query_as("SELECT capacity, going_count, waitlist_count FROM community_events WHERE id = $1")
                .bind(event.id)
                .fetch_one(&mut **tx)
                .await?;
        let free = if capacity == 0 { waiting } else { (capacity - going).min(waiting) };
        if free <= 0 {
            return Ok(());
        }
        let promoted: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE community_event_attendees a SET status = 1, queued_at = NULL
             WHERE a.event_id = $1 AND a.user_id IN (
                 SELECT user_id FROM community_event_attendees
                 WHERE event_id = $1 AND status = 2 ORDER BY queued_at, user_id LIMIT $2)
             RETURNING a.user_id",
        )
        .bind(event.id)
        .bind(i64::from(free))
        .fetch_all(&mut **tx)
        .await?;
        let moved = promoted.len() as i32;
        adjust_counters(tx, event.id, (moved, -moved, 0)).await?;
        for user_id in promoted {
            let notice = notices::event_update(user_id, community, event, &EventUpdate::Promoted);
            outbound::notify(&mut **tx, &notice).await?;
        }
        Ok(())
    }

    /// Answers an event for the caller (a member who is not timed out).
    pub(crate) async fn rsvp(&self, access: &Access, event: &EventRow, answer: RsvpAnswer) -> AppResult<EventRow> {
        let user_id = access.require_active()?.user_id;
        let community = &access.community;
        // Every change of mind moves the waitlist and tells someone.
        self.limit("rsvp", user_id, 30, Duration::from_secs(60)).await?;
        let mut tx = self.core.db.begin().await?;
        let locked = lock_event(&mut tx, event.id).await?;
        ensure!(!locked.is_cancelled(), precondition, "EVENT_CANCELLED", "this event was cancelled");
        ensure!(locked.ends_at > now(), precondition, "EVENT_ENDED", "this event is over");
        let current: Option<i32> =
            sqlx::query_scalar("SELECT status FROM community_event_attendees WHERE event_id = $1 AND user_id = $2")
                .bind(event.id)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;
        let wanted = resolve_answer(answer, current, locked.has_room())
            .ok_or_else(|| AppError::invalid("ANSWER_INVALID", "answer going, interested or not going"))?;
        if current != Some(wanted) {
            sqlx::query(
                "INSERT INTO community_event_attendees (event_id, user_id, status, queued_at)
                 VALUES ($1, $2, $3, CASE WHEN $3 = 2 THEN now() END)
                 ON CONFLICT (event_id, user_id) DO UPDATE
                     SET status = EXCLUDED.status, queued_at = EXCLUDED.queued_at, responded_at = now()",
            )
            .bind(event.id)
            .bind(user_id)
            .bind(wanted)
            .execute(&mut *tx)
            .await?;
            adjust_counters(&mut tx, event.id, counter_deltas(current, Some(wanted))).await?;
            if current == Some(attendance::GOING) {
                // A place freed: whoever waited longest moves up.
                self.promote(&mut tx, community, &locked).await?;
            }
        }
        tx.commit().await?;
        if current != Some(wanted) {
            self.push_event_changed(community, event.id).await;
        }
        self.event_by_id(event.id).await?.ok_or_else(event_not_found)
    }

    /// Takes back the answers of someone who left the community, for every
    /// event that is not over; places they held go to the waitlist. Returns
    /// the events that changed. Check-ins that happened stay.
    pub(crate) async fn withdraw_from_events(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        user_id: Uuid,
    ) -> AppResult<Vec<Uuid>> {
        // Always in ID order: two departures never wait on each other's events.
        let event_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT e.id FROM community_event_attendees a JOIN community_events e ON e.id = a.event_id
             WHERE a.user_id = $1 AND e.community_id = $2 AND e.deleted_at IS NULL AND e.ends_at > now()
               AND a.checked_in_at IS NULL
             ORDER BY e.id",
        )
        .bind(user_id)
        .bind(community.id)
        .fetch_all(&mut **tx)
        .await?;
        for event_id in &event_ids {
            let event = lock_event(tx, *event_id).await?;
            let status: Option<i32> = sqlx::query_scalar(
                "DELETE FROM community_event_attendees WHERE event_id = $1 AND user_id = $2 RETURNING status",
            )
            .bind(event_id)
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await?;
            adjust_counters(tx, *event_id, counter_deltas(status, None)).await?;
            if status == Some(attendance::GOING) && !event.is_cancelled() {
                self.promote(tx, community, &event).await?;
            }
        }
        Ok(event_ids)
    }

    /// Who answered an event with `status`. Members only; check-in times
    /// are for the organiser and the staff.
    pub(crate) async fn list_attendees(
        &self,
        access: &Access,
        event: &EventRow,
        status: i32,
        checked_in_only: bool,
        page: &Page,
    ) -> AppResult<(Vec<pb::Attendee>, PageInfo)> {
        if !access.is_member() && !access.site_moderator {
            return Err(not_a_member());
        }
        let manages = Self::manages_event(access, event) || access.site_moderator;
        if checked_in_only && !manages {
            return Err(permission_denied());
        }
        let status = if status == 0 { attendance::GOING } else { status };
        let blocked = self.blocked(access.user_id).await?;
        let after: Option<(Time, Uuid)> = page.decode()?;
        let mut rows: Vec<AttendeeRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ATTENDEE_COLUMNS} FROM community_event_attendees
             WHERE event_id = $1 AND status = $2 AND (NOT $3 OR checked_in_at IS NOT NULL)
               AND NOT (user_id = ANY($4))
               AND ($5::timestamptz IS NULL OR (coalesce(queued_at, responded_at), user_id) > ($5, $6))
             ORDER BY coalesce(queued_at, responded_at), user_id LIMIT $7"
        )))
        .bind(event.id)
        .bind(status)
        .bind(checked_in_only)
        .bind(&blocked)
        .bind(after.map(|(at, _)| at))
        .bind(after.map(|(_, id)| id))
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| (row.queued_at.unwrap_or(row.responded_at), row.user_id));
        let ids: Vec<Uuid> = rows.iter().map(|row| row.user_id).collect();
        let users = tokuly_identity::summaries(&self.core, &ids).await?;
        Ok((
            rows.iter()
                .map(|row| attendee_proto(row, tokuly_identity::users::pick(&users, row.user_id), manages))
                .collect(),
            info,
        ))
    }

    /// The number of the current check-in period.
    fn check_in_step(&self, at: Time) -> i64 {
        at.timestamp().div_euclid(self.cfg.check_in_code_seconds as i64)
    }

    fn ensure_check_in_open(event: &EventRow) -> AppResult<()> {
        ensure!(!event.is_cancelled(), precondition, "EVENT_CANCELLED", "this event was cancelled");
        ensure!(
            check_in_open(event, now()),
            precondition,
            "CHECK_IN_CLOSED",
            "check-in opens an hour before the event and closes six hours after it"
        );
        Ok(())
    }

    /// The code to show at the venue right now, and when it changes.
    pub(crate) fn current_code(&self, access: &Access, event: &EventRow) -> AppResult<(String, Time)> {
        Self::require_manager(access, event)?;
        Self::ensure_check_in_open(event)?;
        let period = self.cfg.check_in_code_seconds as i64;
        let step = self.check_in_step(now());
        let expires_at = chrono::DateTime::from_timestamp((step + 1) * period, 0).unwrap_or_else(now);
        Ok((check_in_code(&self.core.keys, event.id, step), expires_at))
    }

    /// Marks `user_id` as present inside `tx` and says so to the rest of
    /// the system. False when they were checked in already.
    async fn record_attendance(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        event: &EventRow,
        user_id: Uuid,
        by: Option<Uuid>,
    ) -> AppResult<bool> {
        let checked = sqlx::query(
            "UPDATE community_event_attendees SET checked_in_at = now(), checked_in_by = $3
             WHERE event_id = $1 AND user_id = $2 AND status = 1 AND checked_in_at IS NULL",
        )
        .bind(event.id)
        .bind(user_id)
        .bind(by)
        .execute(&mut **tx)
        .await?;
        if checked.rows_affected() == 0 {
            return Ok(false);
        }
        sqlx::query("UPDATE community_events SET attended_count = attended_count + 1 WHERE id = $1")
            .bind(event.id)
            .execute(&mut **tx)
            .await?;
        core_events::emit(
            &mut **tx,
            events::EVENT_ATTENDED,
            Some(user_id),
            &EventAttended { community_id: community.id, event_id: event.id, user_id, public: community.is_public() },
        )
        .await?;
        outbound::activity(&mut **tx, &notices::attended_activity(user_id, community, event)).await?;
        Ok(true)
    }

    /// Checks the caller in with the code on the organiser's screen.
    pub(crate) async fn check_in(&self, access: &Access, event: &EventRow, code: &str) -> AppResult<EventRow> {
        let user_id = access.require_member()?.user_id;
        Self::ensure_check_in_open(event)?;
        let community = &access.community;
        let status: Option<i32> =
            sqlx::query_scalar("SELECT status FROM community_event_attendees WHERE event_id = $1 AND user_id = $2")
                .bind(event.id)
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
        ensure!(
            status == Some(attendance::GOING),
            precondition,
            "NOT_GOING",
            "answer that you are going (and hold a place) before checking in"
        );
        // Six digits can be tried: keep the tries few.
        self.core
            .cache
            .rate_limit(&format!("community:check-in:{}:{user_id}", event.id), 10, Duration::from_secs(60))
            .await?;
        ensure!(
            code_matches(&self.core.keys, event.id, self.check_in_step(now()), code),
            invalid,
            "CODE_INVALID",
            "that is not the code on the organiser's screen"
        );
        let mut tx = self.core.db.begin().await?;
        // The event first, then the attendee, as the organiser's check-in
        // does: taken in opposite orders, the two deadlock when they meet.
        lock_event(&mut tx, event.id).await?;
        let checked = self.record_attendance(&mut tx, community, event, user_id, None).await?;
        tx.commit().await?;
        if checked {
            self.push_event_changed(community, event.id).await;
        }
        self.event_by_id(event.id).await?.ok_or_else(event_not_found)
    }

    /// Checks a member in by hand: the organiser or the staff. A member who
    /// had not answered "going" is put down as going, whatever the capacity.
    pub(crate) async fn check_in_attendee(
        &self,
        access: &Access,
        event: &EventRow,
        user_id: Uuid,
    ) -> AppResult<pb::Attendee> {
        let actor = Self::require_manager(access, event)?;
        Self::ensure_check_in_open(event)?;
        let community = &access.community;
        ensure!(
            self.member(community.id, user_id).await?.is_some(),
            not_found,
            "MEMBER_NOT_FOUND",
            "this user is not a member of the community"
        );
        self.limit("check-in-by-hand", actor, 600, Duration::from_secs(3600)).await?;
        let mut tx = self.core.db.begin().await?;
        lock_event(&mut tx, event.id).await?;
        let current: Option<i32> =
            sqlx::query_scalar("SELECT status FROM community_event_attendees WHERE event_id = $1 AND user_id = $2")
                .bind(event.id)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;
        // Being there is recorded for the member, with the event's place:
        // the organiser confirms an answer the member gave, and cannot
        // make one up for them (nor seat anyone past the capacity).
        ensure!(
            current == Some(attendance::GOING),
            precondition,
            "NOT_GOING",
            "only someone who answered that they are going (and holds a place) is checked in"
        );
        let checked = self.record_attendance(&mut tx, community, event, user_id, Some(actor)).await?;
        let row: AttendeeRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ATTENDEE_COLUMNS} FROM community_event_attendees WHERE event_id = $1 AND user_id = $2"
        )))
        .bind(event.id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        if checked {
            self.push_event_changed(community, event.id).await;
        }
        let user = tokuly_identity::summary(&self.core, user_id).await?;
        Ok(attendee_proto(&row, user, true))
    }

    /// Sends the reminders that are due: about a day and about an hour
    /// before an event starts, to everyone going or interested whose
    /// notification level is not "none". Returns how many events were
    /// reminded. Safe to run on several workers at once.
    pub async fn send_due_reminders(&self) -> AppResult<usize> {
        let mut reminded = 0;
        for reminder in [Reminder::Hour, Reminder::Day] {
            while reminded < REMINDERS_PER_PASS {
                let mut tx = self.core.db.begin().await?;
                // An event created after a reminder's moment gets none of that kind.
                let query = match reminder {
                    Reminder::Hour => concat!(
                        "SELECT ",
                        event_columns!(),
                        " FROM community_events e
                          WHERE e.deleted_at IS NULL AND e.cancelled_at IS NULL AND e.reminded_hour_at IS NULL
                            AND e.starts_at > now() AND e.starts_at <= now() + interval '1 hour'
                            AND e.created_at <= e.starts_at - interval '90 minutes'
                          ORDER BY e.starts_at LIMIT 1 FOR UPDATE SKIP LOCKED"
                    ),
                    Reminder::Day => concat!(
                        "SELECT ",
                        event_columns!(),
                        " FROM community_events e
                          WHERE e.deleted_at IS NULL AND e.cancelled_at IS NULL AND e.reminded_hour_at IS NULL
                            AND e.reminded_day_at IS NULL
                            AND e.starts_at > now() + interval '1 hour' AND e.starts_at <= now() + interval '24 hours'
                            AND e.created_at <= e.starts_at - interval '25 hours'
                          ORDER BY e.starts_at LIMIT 1 FOR UPDATE SKIP LOCKED"
                    ),
                };
                let Some(event): Option<EventRow> = sqlx::query_as(query).fetch_optional(&mut *tx).await? else {
                    break;
                };
                let mark = match reminder {
                    Reminder::Hour => "UPDATE community_events SET reminded_hour_at = now() WHERE id = $1",
                    Reminder::Day => "UPDATE community_events SET reminded_day_at = now() WHERE id = $1",
                };
                sqlx::query(mark).bind(event.id).execute(&mut *tx).await?;
                // On the transaction's own connection: a worker holding the
                // event's lock never asks the pool for a second one.
                let community: Option<CommunityRow> = sqlx::query_as(concat!(
                    "SELECT ",
                    community_columns!(),
                    " FROM communities c WHERE c.id = $1 AND c.deleted_at IS NULL"
                ))
                .bind(event.community_id)
                .fetch_optional(&mut *tx)
                .await?;
                if let Some(community) = community {
                    let recipients: Vec<Uuid> = sqlx::query_scalar(
                        "SELECT a.user_id FROM community_event_attendees a
                         JOIN community_members m ON m.community_id = $2 AND m.user_id = a.user_id
                         WHERE a.event_id = $1 AND a.status IN (1, 3) AND m.notification_level <> $3",
                    )
                    .bind(event.id)
                    .bind(community.id)
                    .bind(level::NONE)
                    .fetch_all(&mut *tx)
                    .await?;
                    for recipient in recipients {
                        outbound::notify(&mut *tx, &notices::event_reminder(recipient, &community, &event, reminder))
                            .await?;
                    }
                }
                tx.commit().await?;
                reminded += 1;
            }
        }
        Ok(reminded)
    }
}

fn attendee_proto(row: &AttendeeRow, user: tokuly_proto::common::UserSummary, with_check_in: bool) -> pb::Attendee {
    pb::Attendee {
        user: user.into(),
        status: row.status.into(),
        responded_at: ts(row.responded_at),
        checked_in_at: opt_ts(row.checked_in_at.filter(|_| with_check_in)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Keys {
        Keys::new("check-in-test-secret-check-in-test-secret")
    }

    #[test]
    fn codes_are_six_digits_and_rotate() {
        let event = Uuid::from_u128(7);
        let code = check_in_code(&keys(), event, 100);
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(code, check_in_code(&keys(), event, 100));
        // Over many periods the code keeps changing, and other events have other codes.
        let distinct: std::collections::HashSet<String> =
            (0..200).map(|step| check_in_code(&keys(), event, step)).collect();
        assert!(distinct.len() > 190);
        let same = (0..200)
            .filter(|step| check_in_code(&keys(), event, *step) == check_in_code(&keys(), Uuid::from_u128(8), *step))
            .count();
        assert!(same < 3);
    }

    #[test]
    fn the_current_and_the_previous_code_are_accepted() {
        let (keys, event) = (keys(), Uuid::from_u128(7));
        let current = check_in_code(&keys, event, 50);
        let previous = check_in_code(&keys, event, 49);
        let older = check_in_code(&keys, event, 48);
        assert!(code_matches(&keys, event, 50, &current));
        assert!(code_matches(&keys, event, 50, &format!(" {previous} ")));
        assert!(!code_matches(&keys, event, 50, &older) || older == current || older == previous);
        assert!(!code_matches(&keys, event, 50, ""));
        assert!(!code_matches(&keys, event, 50, "12345"));
    }

    #[test]
    fn going_takes_a_place_or_queues() {
        use RsvpAnswer::*;
        assert_eq!(resolve_answer(RSVP_ANSWER_GOING, None, true), Some(attendance::GOING));
        assert_eq!(resolve_answer(RSVP_ANSWER_GOING, None, false), Some(attendance::WAITLISTED));
        // Answering "going" again never gives up a place or a spot in the queue.
        assert_eq!(resolve_answer(RSVP_ANSWER_GOING, Some(attendance::GOING), false), Some(attendance::GOING));
        assert_eq!(resolve_answer(RSVP_ANSWER_GOING, Some(attendance::WAITLISTED), true), Some(attendance::WAITLISTED));
        assert_eq!(
            resolve_answer(RSVP_ANSWER_GOING, Some(attendance::INTERESTED), false),
            Some(attendance::WAITLISTED)
        );
        assert_eq!(resolve_answer(RSVP_ANSWER_INTERESTED, Some(attendance::GOING), true), Some(attendance::INTERESTED));
        assert_eq!(resolve_answer(RSVP_ANSWER_NOT_GOING, None, true), Some(attendance::NOT_GOING));
        assert_eq!(resolve_answer(RSVP_ANSWER_UNSPECIFIED, None, true), None);
    }

    #[test]
    fn counters_follow_status_changes() {
        assert_eq!(counter_deltas(None, Some(attendance::GOING)), (1, 0, 0));
        assert_eq!(counter_deltas(Some(attendance::GOING), Some(attendance::NOT_GOING)), (-1, 0, 0));
        assert_eq!(counter_deltas(Some(attendance::WAITLISTED), Some(attendance::GOING)), (1, -1, 0));
        assert_eq!(counter_deltas(Some(attendance::INTERESTED), Some(attendance::WAITLISTED)), (0, 1, -1));
        assert_eq!(counter_deltas(Some(attendance::INTERESTED), None), (0, 0, -1));
        assert_eq!(counter_deltas(Some(attendance::NOT_GOING), Some(attendance::NOT_GOING)), (0, 0, 0));
    }
}
