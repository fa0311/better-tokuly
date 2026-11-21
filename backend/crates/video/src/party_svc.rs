//! `WatchPartyService`: handlers. The model and the membership rules are in
//! [`crate::party`].
//!
//! Every handler that changes a party locks its row first
//! ([`party::lock_open`]), decides and writes inside that transaction, and
//! pushes realtime events only after the commit.
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{outbound, prelude::*};
use tokuly_proto::{
    common::SubjectType,
    video::{video_event::Event as Push, *},
};

use crate::{
    Video,
    access::{self, Viewer},
    party::{self, Caller, MemberRow, PartyRow, ROLE_COHOST, ROLE_HOST, ROLE_MEMBER},
    playlists, validate,
    videos::{self, VideoRow},
};

/// Friends one `InviteToParty` call may name.
const MAX_INVITES: usize = 20;

fn not_host() -> AppError {
    AppError::denied("NOT_PARTY_HOST", "only the host can do this")
}

fn member_not_found() -> AppError {
    AppError::not_found("PARTY_MEMBER_NOT_FOUND", "no such member in this watch party")
}

fn playback_blocked(message: &'static str) -> AppError {
    AppError::denied("PLAYBACK_BLOCKED", message)
}

impl Video {
    /// Keyed hash of a network address, for guest bans.
    fn party_ip_hash(&self, ip: &str) -> String {
        if ip.is_empty() { String::new() } else { hex::encode(self.core.keys.mac("video/party-ip", ip.as_bytes()))[..32].to_owned() }
    }

    /// Fails unless `viewer` could press play on `video` right now.
    async fn ensure_playable(&self, viewer: &Viewer, video: &VideoRow) -> AppResult<()> {
        if !video.is_ready() {
            return Err(playback_blocked("the video is not ready to play"));
        }
        match access::age_block(&self.core, viewer, video.owner_id, video.age_restricted).await? {
            Some(_) => Err(playback_blocked("this video is age-restricted")),
            None => Ok(()),
        }
    }

    /// The party as `viewer` sees it. The video card is only filled in for
    /// viewers who may watch the video.
    async fn party_proto(&self, viewer: &Viewer, row: &PartyRow) -> AppResult<Party> {
        let core = &self.core;
        let video = match videos::find_by_id(core, row.video_id).await? {
            Some(video) if viewer.can_watch(&video.facts()) => Some(videos::summary(core, viewer, &video).await?),
            _ => None,
        };
        let playlist_id: Option<String> = match row.playlist_id {
            Some(playlist_id) => sqlx::query_scalar("SELECT short_id FROM playlists WHERE id = $1").bind(playlist_id).fetch_optional(&core.db).await?,
            None => None,
        };
        Ok(Party {
            id: row.id.to_string(),
            code: row.code.clone(),
            invite_url: core.web_url(&row.path()),
            video: video.into(),
            playlist_id: playlist_id.unwrap_or_default(),
            playback: row.playback().into(),
            allow_guests: row.allow_guests,
            member_count: party::member_count(core, row.id).await?,
            max_members: self.cfg.party_max_members as i32,
            created_at: ts(row.created_at),
            ended_at: opt_ts(row.ended_at),
            ..Default::default()
        })
    }

    /// The party as carried by realtime pushes, which everyone in it
    /// receives: the video card is included only when anyone with the link
    /// may watch the video (other members get theirs from `GetParty`).
    async fn party_push_proto(&self, row: &PartyRow) -> AppResult<Party> {
        self.party_proto(&Viewer::anonymous(), row).await
    }

    async fn party_session(&self, viewer: &Viewer, row: &PartyRow, me: &MemberRow, guest_token: Option<String>) -> AppResult<PartySession> {
        let core = &self.core;
        let seats = party::members(core, row.id).await?;
        Ok(PartySession {
            party: self.party_proto(viewer, row).await?.into(),
            self_: party::member_proto(core, me).await?.into(),
            members: party::member_protos(core, &seats).await?,
            recent_messages: party::recent_messages(core, row.id).await?,
            realtime_topic: me.topic(),
            server_time: ts(now()),
            guest_token: guest_token.unwrap_or_default(),
            ..Default::default()
        })
    }

    /// An open party and the caller's seat in it.
    async fn party_seat(&self, party_id: &str, caller: &Caller) -> AppResult<(PartyRow, MemberRow)> {
        let party_id = Uuid::parse_str(party_id.trim()).map_err(|_| party::not_found())?;
        let row = party::find(&self.core, party_id).await?.filter(PartyRow::is_open).ok_or_else(party::not_found)?;
        let seat = party::seat(&self.core, party_id, caller).await?.ok_or_else(party::not_a_member)?;
        Ok((row, seat))
    }

    /// Reloads a party and pushes it to everyone in it.
    async fn push_party_updated(&self, party_id: Uuid) -> AppResult<Party> {
        let row = party::find(&self.core, party_id).await?.ok_or_else(party::not_found)?;
        let pushed = self.party_push_proto(&row).await?;
        party::push_to_party(&self.core, party_id, Push::PartyUpdated(Box::new(pushed))).await;
        Ok(self.party_push_proto(&row).await?)
    }
}

impl WatchPartyService for Video {
    async fn create_party(&self, ctx: RequestContext, request: ServiceRequest<'_, CreatePartyRequest>) -> ServiceResult<CreatePartyResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let req = request.to_owned_message();
            let video = videos::find_watchable(core, &viewer, &req.video_id).await?;
            self.ensure_playable(&viewer, &video).await?;
            let playlist_id = match req.playlist_id.trim() {
                "" => None,
                reference => Some(playlists::resolve(core, &viewer, reference).await?.id),
            };
            ensure!(
                party::valid_playback(req.position_seconds, 1.0, video.duration_seconds),
                invalid,
                "PLAYBACK_INVALID",
                "the start position is outside the video"
            );
            core.cache.rate_limit(&format!("video-party-create:{user_id}"), 20, Duration::from_secs(3600)).await?;

            let mut tx = core.db.begin().await?;
            // One at a time per host, so the limit holds.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!("video:parties:{}", user_id.simple()))
                .execute(&mut *tx)
                .await?;
            let hosting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM watch_party_members m JOIN watch_parties p ON p.id = m.party_id
                 WHERE m.user_id = $1 AND m.role = 1 AND p.ended_at IS NULL",
            )
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            let max = self.cfg.party_max_hosted;
            ensure!(hosting < max, precondition, "PARTY_LIMIT_REACHED", "you can host at most {max} watch parties at a time");
            let party_id = new_id();
            // Codes are random; a collision just draws again.
            let mut created = false;
            for _ in 0..5 {
                let inserted = sqlx::query(
                    "INSERT INTO watch_parties (id, code, video_id, playlist_id, allow_guests, position_seconds, created_by)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT (code) DO NOTHING",
                )
                .bind(party_id)
                .bind(tokuly_core::ids::random_base62(10))
                .bind(video.id)
                .bind(playlist_id)
                .bind(req.allow_guests.unwrap_or(true))
                .bind(req.position_seconds)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
                if inserted.rows_affected() == 1 {
                    created = true;
                    break;
                }
            }
            ensure!(created, unavailable, "PARTY_CODE_UNAVAILABLE", "could not allocate a party code; try again");
            let seat_id = new_id();
            sqlx::query("INSERT INTO watch_party_members (id, party_id, user_id, role) VALUES ($1, $2, $3, $4)")
                .bind(seat_id)
                .bind(party_id)
                .bind(user_id)
                .bind(ROLE_HOST)
                .execute(&mut *tx)
                .await?;
            let seat = party::member_in(&mut tx, party_id, seat_id).await?.ok_or_else(party::not_a_member)?;
            tx.commit().await?;
            let row = party::find(core, party_id).await?.ok_or_else(party::not_found)?;
            ok(CreatePartyResponse { session: self.party_session(&viewer, &row, &seat, None).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn preview_party(&self, ctx: RequestContext, request: ServiceRequest<'_, PreviewPartyRequest>) -> ServiceResult<PreviewPartyResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let limiter = viewer.user_id.map(|id| id.to_string()).unwrap_or_else(|| ctx.client().ip);
            core.cache.rate_limit(&format!("video-party-preview:{limiter}"), 60, Duration::from_secs(60)).await?;
            let row = party::find_by_code(core, request.code.trim()).await?.filter(PartyRow::is_open).ok_or_else(party::not_found)?;
            let seats = party::members(core, row.id).await?;
            let host = seats.iter().find(|seat| seat.is_host());
            // A host who is hidden from the viewer hosts no party for them.
            if host.and_then(|host| host.user_id).is_some_and(|host| viewer.hides(host)) {
                return Err(party::not_found());
            }
            let host = match host {
                Some(host) => Some(party::member_proto(core, host).await?),
                None => None,
            };
            ok(PreviewPartyResponse { party: self.party_proto(&viewer, &row).await?.into(), host: host.into(), ..Default::default() })
        })
        .await
    }

    async fn join_party(&self, ctx: RequestContext, request: ServiceRequest<'_, JoinPartyRequest>) -> ServiceResult<JoinPartyResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.maybe_user_id()?;
            let client = ctx.client();
            let viewer = Viewer::load(core, user_id).await?;
            let req = request.to_owned_message();
            let limiter = user_id.map(|id| id.to_string()).unwrap_or_else(|| client.ip.clone());
            core.cache.rate_limit(&format!("video-party-join:{limiter}"), 30, Duration::from_secs(60)).await?;
            let found = party::find_by_code(core, req.code.trim()).await?.filter(PartyRow::is_open).ok_or_else(party::not_found)?;

            let mut tx = core.db.begin().await?;
            let row = party::lock_open(&mut tx, found.id).await?;
            // Joining again (a reload, a second device) returns the seat one has.
            let returning = match (user_id, req.guest_token.trim()) {
                (Some(user_id), _) => Some(Caller::User(user_id)),
                (None, "") => None,
                (None, token) => Some(Caller::Guest(token.to_owned())),
            };
            if let Some(caller) = &returning {
                if let Some(seat) = party::seat_in(&mut tx, core, row.id, caller).await? {
                    sqlx::query("UPDATE watch_party_members SET last_seen_at = now() WHERE id = $1").bind(seat.id).execute(&mut *tx).await?;
                    tx.commit().await?;
                    let token = user_id.is_none().then(|| req.guest_token.trim().to_owned());
                    return ok(JoinPartyResponse { session: self.party_session(&viewer, &row, &seat, token).await?.into(), ..Default::default() });
                }
            }

            let video = videos::find_by_id(core, row.video_id).await?.ok_or_else(|| AppError::not_found("VIDEO_NOT_FOUND", "video not found"))?;
            let ip_hash = self.party_ip_hash(&client.ip);
            let seat_id = new_id();
            let mut guest_token = None;
            match user_id {
                Some(user_id) => {
                    let (banned, host): (bool, Option<Uuid>) = sqlx::query_as(
                        "SELECT EXISTS (SELECT 1 FROM watch_party_bans WHERE party_id = $1 AND user_id = $2),
                                (SELECT user_id FROM watch_party_members WHERE party_id = $1 AND role = 1)",
                    )
                    .bind(row.id)
                    .bind(user_id)
                    .fetch_one(&mut *tx)
                    .await?;
                    if host.is_some_and(|host| viewer.hides(host)) {
                        return Err(party::not_found());
                    }
                    ensure!(!banned, denied, "PARTY_BANNED", "you were removed from this watch party");
                    ensure!(viewer.can_watch(&video.facts()), not_found, "VIDEO_NOT_FOUND", "video not found");
                    self.ensure_playable(&viewer, &video).await?;
                }
                None => {
                    ensure!(row.allow_guests, denied, "PARTY_GUESTS_NOT_ALLOWED", "sign in to join this watch party");
                    let banned: bool = sqlx::query_scalar(
                        "SELECT $2 <> '' AND EXISTS (SELECT 1 FROM watch_party_bans WHERE party_id = $1 AND ip_hash = $2)",
                    )
                    .bind(row.id)
                    .bind(&ip_hash)
                    .fetch_one(&mut *tx)
                    .await?;
                    ensure!(!banned, denied, "PARTY_BANNED", "you were removed from this watch party");
                    ensure!(viewer.can_watch(&video.facts()), not_found, "VIDEO_NOT_FOUND", "video not found");
                    self.ensure_playable(&viewer, &video).await?;
                }
            }
            let guest_name = if user_id.is_none() { validate::guest_name(&req.guest_name)? } else { String::new() };
            let seated: i64 = sqlx::query_scalar("SELECT count(*) FROM watch_party_members WHERE party_id = $1").bind(row.id).fetch_one(&mut *tx).await?;
            ensure!(seated < self.cfg.party_max_members, precondition, "PARTY_FULL", "this watch party is full");
            match user_id {
                Some(user_id) => {
                    sqlx::query("INSERT INTO watch_party_members (id, party_id, user_id, role) VALUES ($1, $2, $3, $4)")
                        .bind(seat_id)
                        .bind(row.id)
                        .bind(user_id)
                        .bind(ROLE_MEMBER)
                        .execute(&mut *tx)
                        .await?;
                }
                None => {
                    // The token proves the seat in later calls; only its keyed hash is stored.
                    let token = tokuly_core::keys::random_token(24);
                    sqlx::query(
                        "INSERT INTO watch_party_members (id, party_id, guest_name, guest_token_hash, topic_key, ip_hash, role)
                         VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    )
                    .bind(seat_id)
                    .bind(row.id)
                    .bind(&guest_name)
                    .bind(core.keys.token_hash(&token))
                    .bind(tokuly_core::ids::random_base62(22))
                    .bind(&ip_hash)
                    .bind(ROLE_MEMBER)
                    .execute(&mut *tx)
                    .await?;
                    guest_token = Some(token);
                }
            }
            let seat = party::member_in(&mut tx, row.id, seat_id).await?.ok_or_else(party::not_a_member)?;
            tx.commit().await?;
            let joined = party::member_proto(core, &seat).await?;
            party::push_to_party(core, row.id, Push::PartyMemberJoined(Box::new(joined))).await;
            ok(JoinPartyResponse { session: self.party_session(&viewer, &row, &seat, guest_token).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn get_party(&self, ctx: RequestContext, request: ServiceRequest<'_, GetPartyRequest>) -> ServiceResult<GetPartyResponse> {
        handle(async {
            let core = &self.core;
            let viewer = Viewer::load(core, ctx.maybe_user_id()?).await?;
            let caller = Caller::new(viewer.user_id, request.guest_token)?;
            // A party that just ended is still shown to those who were in it.
            let party_id = Uuid::parse_str(request.party_id.trim()).map_err(|_| party::not_found())?;
            let row = party::find(core, party_id).await?.ok_or_else(party::not_found)?;
            let seat = party::seat(core, party_id, &caller).await?.ok_or_else(party::not_a_member)?;
            ok(GetPartyResponse { session: self.party_session(&viewer, &row, &seat, None).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn heartbeat_party(&self, ctx: RequestContext, request: ServiceRequest<'_, HeartbeatPartyRequest>) -> ServiceResult<HeartbeatPartyResponse> {
        handle(async {
            let core = &self.core;
            let caller = Caller::new(ctx.maybe_user_id()?, request.guest_token)?;
            let (row, seat) = self.party_seat(request.party_id, &caller).await?;
            sqlx::query("UPDATE watch_party_members SET last_seen_at = now() WHERE id = $1").bind(seat.id).execute(&core.db).await?;
            ok(HeartbeatPartyResponse {
                playback: row.playback().into(),
                server_time: ts(now()),
                member_count: party::member_count(core, row.id).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn leave_party(&self, ctx: RequestContext, request: ServiceRequest<'_, LeavePartyRequest>) -> ServiceResult<LeavePartyResponse> {
        handle(async {
            let core = &self.core;
            let caller = Caller::new(ctx.maybe_user_id()?, request.guest_token)?;
            let party_id = Uuid::parse_str(request.party_id.trim()).map_err(|_| party::not_found())?;
            let mut tx = core.db.begin().await?;
            let row = party::lock_open(&mut tx, party_id).await?;
            // Leaving twice is fine.
            let Some(seat) = party::seat_in(&mut tx, core, party_id, &caller).await? else { return ok(LeavePartyResponse::default()) };
            let departure = party::remove_member_in(&mut tx, &row, &seat).await?;
            tx.commit().await?;
            party::announce_departure(core, &departure, PartyLeaveReason::PARTY_LEAVE_REASON_LEFT).await;
            ok(LeavePartyResponse::default())
        })
        .await
    }

    async fn set_party_playback(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetPartyPlaybackRequest>,
    ) -> ServiceResult<SetPartyPlaybackResponse> {
        handle(async {
            let core = &self.core;
            let caller = Caller::User(ctx.user_id()?);
            let party_id = Uuid::parse_str(request.party_id.trim()).map_err(|_| party::not_found())?;
            let mut tx = core.db.begin().await?;
            let row = party::lock_open(&mut tx, party_id).await?;
            let seat = party::seat_in(&mut tx, core, party_id, &caller).await?.ok_or_else(party::not_a_member)?;
            ensure!(seat.can_control(), denied, "NOT_PARTY_HOST", "only the host and co-hosts control playback");
            let duration: f64 = sqlx::query_scalar("SELECT duration_seconds FROM videos WHERE id = $1").bind(row.video_id).fetch_one(&mut *tx).await?;
            let rate = if request.rate == 0.0 { row.rate } else { request.rate };
            ensure!(
                party::valid_playback(request.position_seconds, rate, duration),
                invalid,
                "PLAYBACK_INVALID",
                "the position is outside the video or the speed is not between 0.25 and 2"
            );
            core.cache.rate_limit(&format!("video-party-control:{}", seat.id), 240, Duration::from_secs(60)).await?;
            let updated: PartyRow = sqlx::query_as(concat!(
                "UPDATE watch_parties p SET playing = $2, position_seconds = $3, rate = $4, state_at = now(),
                        state_seq = p.state_seq + 1, state_by = $5
                 WHERE p.id = $1
                 RETURNING ",
                party::party_columns!()
            ))
            .bind(party_id)
            .bind(request.playing)
            .bind(request.position_seconds.min(duration))
            .bind(rate)
            .bind(seat.id)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            party::push_to_party(core, party_id, Push::PartyPlayback(Box::new(updated.playback()))).await;
            ok(SetPartyPlaybackResponse { playback: updated.playback().into(), server_time: ts(now()), ..Default::default() })
        })
        .await
    }

    async fn set_party_video(&self, ctx: RequestContext, request: ServiceRequest<'_, SetPartyVideoRequest>) -> ServiceResult<SetPartyVideoResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let req = request.to_owned_message();
            let party_id = Uuid::parse_str(req.party_id.trim()).map_err(|_| party::not_found())?;
            let video = videos::find_watchable(core, &viewer, &req.video_id).await?;
            self.ensure_playable(&viewer, &video).await?;
            // Unset keeps the playlist, an empty string clears it.
            let playlist_change = match req.playlist_id.as_deref().map(str::trim) {
                None => None,
                Some("") => Some(None),
                Some(reference) => Some(Some(playlists::resolve(core, &viewer, reference).await?.id)),
            };

            let mut tx = core.db.begin().await?;
            party::lock_open(&mut tx, party_id).await?;
            let seat = party::seat_in(&mut tx, core, party_id, &Caller::User(user_id)).await?.ok_or_else(party::not_a_member)?;
            ensure!(seat.can_control(), denied, "NOT_PARTY_HOST", "only the host and co-hosts change the video");
            // The new video starts paused at the beginning; the host presses play.
            sqlx::query(
                "UPDATE watch_parties SET video_id = $2, playlist_id = CASE WHEN $3 THEN $4 ELSE playlist_id END,
                        playing = false, position_seconds = 0, rate = 1, state_at = now(), state_seq = state_seq + 1, state_by = $5
                 WHERE id = $1",
            )
            .bind(party_id)
            .bind(video.id)
            .bind(playlist_change.is_some())
            .bind(playlist_change.flatten())
            .bind(seat.id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            self.push_party_updated(party_id).await?;
            let row = party::find(core, party_id).await?.ok_or_else(party::not_found)?;
            ok(SetPartyVideoResponse { party: self.party_proto(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn set_party_member_role(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetPartyMemberRoleRequest>,
    ) -> ServiceResult<SetPartyMemberRoleResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let party_id = Uuid::parse_str(request.party_id.trim()).map_err(|_| party::not_found())?;
            let role = match request.role.as_known() {
                Some(PartyRole::PARTY_ROLE_HOST) => ROLE_HOST,
                Some(PartyRole::PARTY_ROLE_COHOST) => ROLE_COHOST,
                Some(PartyRole::PARTY_ROLE_MEMBER) => ROLE_MEMBER,
                _ => bail!(invalid, "ROLE_INVALID", "the role must be HOST, COHOST or MEMBER"),
            };
            let target_id = Uuid::parse_str(request.member_id.trim()).map_err(|_| member_not_found())?;

            let mut tx = core.db.begin().await?;
            party::lock_open(&mut tx, party_id).await?;
            let me = party::seat_in(&mut tx, core, party_id, &Caller::User(user_id)).await?.ok_or_else(party::not_a_member)?;
            if !me.is_host() {
                return Err(not_host());
            }
            let target = party::member_in(&mut tx, party_id, target_id).await?.ok_or_else(member_not_found)?;
            // The host changes their own role only by handing the party over.
            ensure!(target.id != me.id, invalid, "ROLE_INVALID", "hand the party over to change your own role");
            ensure!(!target.is_guest() || role == ROLE_MEMBER, precondition, "GUESTS_CANNOT_HOST", "guests cannot host or co-host");
            let mut changed = Vec::new();
            if role == ROLE_HOST {
                // Step down first: a party has exactly one host at any moment.
                sqlx::query("UPDATE watch_party_members SET role = $2 WHERE id = $1").bind(me.id).bind(ROLE_COHOST).execute(&mut *tx).await?;
                changed.push(me.id);
            }
            if target.role != role {
                sqlx::query("UPDATE watch_party_members SET role = $2 WHERE id = $1").bind(target.id).bind(role).execute(&mut *tx).await?;
                changed.push(target.id);
            }
            tx.commit().await?;

            let seats = party::members(core, party_id).await?;
            let members = party::member_protos(core, &seats).await?;
            for (seat, member) in seats.iter().zip(&members) {
                if changed.contains(&seat.id) {
                    party::push_to_party(core, party_id, Push::PartyMemberUpdated(Box::new(member.clone()))).await;
                }
            }
            ok(SetPartyMemberRoleResponse { members, ..Default::default() })
        })
        .await
    }

    async fn remove_party_member(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemovePartyMemberRequest>,
    ) -> ServiceResult<RemovePartyMemberResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let party_id = Uuid::parse_str(request.party_id.trim()).map_err(|_| party::not_found())?;
            let target_id = Uuid::parse_str(request.member_id.trim()).map_err(|_| member_not_found())?;
            let mut tx = core.db.begin().await?;
            let row = party::lock_open(&mut tx, party_id).await?;
            let me = party::seat_in(&mut tx, core, party_id, &Caller::User(user_id)).await?.ok_or_else(party::not_a_member)?;
            if !me.can_control() {
                return Err(not_host());
            }
            let target = party::member_in(&mut tx, party_id, target_id).await?.ok_or_else(member_not_found)?;
            ensure!(target.id != me.id, invalid, "CANNOT_REMOVE_SELF", "leave the party instead of removing yourself");
            // Co-hosts keep order among members; only the host removes co-hosts.
            if !me.is_host() && target.can_control() {
                return Err(not_host());
            }
            match target.user_id {
                Some(removed) => {
                    sqlx::query(
                        "INSERT INTO watch_party_bans (party_id, user_id) VALUES ($1, $2)
                         ON CONFLICT (party_id, user_id) WHERE user_id IS NOT NULL DO NOTHING",
                    )
                    .bind(party_id)
                    .bind(removed)
                    .execute(&mut *tx)
                    .await?;
                }
                None if !target.ip_hash.is_empty() => {
                    sqlx::query(
                        "INSERT INTO watch_party_bans (party_id, ip_hash) VALUES ($1, $2)
                         ON CONFLICT (party_id, ip_hash) WHERE ip_hash IS NOT NULL DO NOTHING",
                    )
                    .bind(party_id)
                    .bind(&target.ip_hash)
                    .execute(&mut *tx)
                    .await?;
                }
                None => {}
            }
            let departure = party::remove_member_in(&mut tx, &row, &target).await?;
            tx.commit().await?;
            party::announce_departure(core, &departure, PartyLeaveReason::PARTY_LEAVE_REASON_REMOVED).await;
            ok(RemovePartyMemberResponse::default())
        })
        .await
    }

    async fn update_party(&self, ctx: RequestContext, request: ServiceRequest<'_, UpdatePartyRequest>) -> ServiceResult<UpdatePartyResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let party_id = Uuid::parse_str(request.party_id.trim()).map_err(|_| party::not_found())?;
            let mut tx = core.db.begin().await?;
            party::lock_open(&mut tx, party_id).await?;
            let me = party::seat_in(&mut tx, core, party_id, &Caller::User(user_id)).await?.ok_or_else(party::not_a_member)?;
            if !me.is_host() {
                return Err(not_host());
            }
            let changed = match request.allow_guests {
                Some(allow_guests) => {
                    sqlx::query("UPDATE watch_parties SET allow_guests = $2 WHERE id = $1 AND allow_guests <> $2")
                        .bind(party_id)
                        .bind(allow_guests)
                        .execute(&mut *tx)
                        .await?
                        .rows_affected()
                        == 1
                }
                None => false,
            };
            tx.commit().await?;
            if changed {
                self.push_party_updated(party_id).await?;
            }
            let row = party::find(core, party_id).await?.ok_or_else(party::not_found)?;
            ok(UpdatePartyResponse { party: self.party_proto(&viewer, &row).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn send_party_message(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SendPartyMessageRequest>,
    ) -> ServiceResult<SendPartyMessageResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.maybe_user_id()?;
            let caller = Caller::new(user_id, request.guest_token)?;
            let (row, seat) = self.party_seat(request.party_id, &caller).await?;
            let text = validate::party_text(request.text)?;
            let stamp_id = match request.stamp_id.trim() {
                "" => None,
                id => Some(Uuid::parse_str(id).map_err(|_| AppError::precondition("STAMP_NOT_USABLE", "you cannot send this stamp"))?),
            };
            ensure!(!text.is_empty() || stamp_id.is_some(), invalid, "MESSAGE_EMPTY", "write something or pick a stamp");
            core.cache.rate_limit(&format!("video-party-chat:{}", seat.id), 20, Duration::from_secs(10)).await?;
            let stamp = match (stamp_id, user_id, self.party_stamps()) {
                (None, _, _) => None,
                // Checks the sender may use the stamp and records the send.
                (Some(stamp_id), Some(user_id), Some(stamps)) => Some(stamps.send_stamp(user_id, stamp_id).await?),
                (Some(_), _, _) => bail!(precondition, "STAMP_NOT_USABLE", "stamps cannot be sent here"),
            };
            let sender = party::member_proto(core, &seat).await?;
            let stamp_json = stamp
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|e| AppError::internal(anyhow::anyhow!("stamp encode: {e}")))?;
            let (message_id, sent_at): (Uuid, Time) = sqlx::query_as(
                "INSERT INTO watch_party_messages (party_id, member_id, user_id, name, role, text, stamp)
                 SELECT $1, $2, $3, $4, $5, $6, $7 WHERE EXISTS (SELECT 1 FROM watch_parties WHERE id = $1 AND ended_at IS NULL)
                 RETURNING id, created_at",
            )
            .bind(row.id)
            .bind(seat.id)
            .bind(seat.user_id)
            .bind(&sender.name)
            .bind(seat.role)
            .bind(&text)
            .bind(stamp_json)
            .fetch_optional(&core.db)
            .await?
            .ok_or_else(party::not_found)?;
            // Chatting is a sign of life, too.
            sqlx::query("UPDATE watch_party_members SET last_seen_at = now() WHERE id = $1").bind(seat.id).execute(&core.db).await?;
            let message = PartyMessage { id: message_id.to_string(), sender: sender.into(), text, stamp: stamp.into(), sent_at: ts(sent_at), ..Default::default() };
            party::push_to_party(core, row.id, Push::PartyMessage(Box::new(message.clone()))).await;
            ok(SendPartyMessageResponse { message: message.into(), ..Default::default() })
        })
        .await
    }

    async fn invite_to_party(&self, ctx: RequestContext, request: ServiceRequest<'_, InviteToPartyRequest>) -> ServiceResult<InviteToPartyResponse> {
        handle(async {
            let core = &self.core;
            let user_id = ctx.user_id()?;
            let viewer = Viewer::load(core, Some(user_id)).await?;
            let (row, _seat) = self.party_seat(request.party_id, &Caller::User(user_id)).await?;
            ensure!(request.user_ids.len() <= MAX_INVITES, invalid, "TOO_MANY_INVITES", "invite at most {MAX_INVITES} friends at a time");
            let mut invited: Vec<Uuid> = request.user_ids.iter().map(|id| parse_id(id)).collect::<AppResult<_>>()?;
            invited.sort_unstable();
            invited.dedup();
            ensure!(invited.iter().all(|id| viewer.friends.contains(id)), denied, "NOT_FRIENDS", "you can only invite friends");
            core.cache.rate_limit(&format!("video-party-invite:{user_id}"), 100, Duration::from_secs(3600)).await?;

            let me = tokuly_identity::summary(core, user_id).await?;
            let video = videos::find_by_id(core, row.video_id).await?;
            // The title is shared only when it is no secret.
            let title = video.as_ref().filter(|video| Viewer::anonymous().can_watch(&video.facts())).map(|video| video.title.clone());
            let (ja, en) = match &title {
                Some(title) => (format!("「{title}」を一緒に見よう!"), format!("Let's watch \"{title}\" together!")),
                None => ("動画を一緒に見よう!".to_owned(), "Let's watch a video together!".to_owned()),
            };
            let already: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM watch_party_members WHERE party_id = $1 AND user_id = ANY($2)")
                .bind(row.id)
                .bind(&invited)
                .fetch_all(&core.db)
                .await?;
            let mut tx = core.db.begin().await?;
            for friend in invited.into_iter().filter(|id| !already.contains(id)) {
                let invitation = outbound::NotificationRequest::new(
                    friend,
                    "video.party_invite",
                    outbound::Localized::new(
                        format!("{}さんからウォッチパーティーのお誘い!", me.display_name),
                        format!("{} invited you to a watch party", me.display_name),
                    ),
                    outbound::Localized::new(ja.clone(), en.clone()),
                    row.path(),
                )
                .actor(user_id)
                .subject(SubjectType::SUBJECT_TYPE_VIDEO, row.video_id)
                .collapse(format!("video.party:{}", row.id))
                .data(serde_json::json!({ "party_id": row.id, "code": row.code }));
                outbound::notify(&mut *tx, &invitation).await?;
            }
            tx.commit().await?;
            ok(InviteToPartyResponse::default())
        })
        .await
    }

    async fn end_party(&self, ctx: RequestContext, request: ServiceRequest<'_, EndPartyRequest>) -> ServiceResult<EndPartyResponse> {
        handle(async {
            let core = &self.core;
            let (row, seat) = self.party_seat(request.party_id, &Caller::User(ctx.user_id()?)).await?;
            if !seat.is_host() {
                return Err(not_host());
            }
            party::end(core, row.id, PartyEndReason::PARTY_END_REASON_ENDED_BY_HOST).await?;
            ok(EndPartyResponse::default())
        })
        .await
    }
}
