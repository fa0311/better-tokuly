//! Voice and video calls: one live call per conversation, with the media
//! on LiveKit.
//!
//! The database holds the truth about a call (`chat_calls`,
//! `chat_call_participants`); LiveKit only carries audio and video. Every
//! change to a call happens under a lock on its row. The rules that decide
//! when a call is over live in [`settle`]:
//!
//! - a direct call ends when either side leaves, when the callee declines,
//!   or as missed when nobody answers before the ringing time is up;
//! - a group call ends when nobody is connected any more, or when everyone
//!   who was rung declined; members who did not answer in time are marked
//!   missed but may still join while the call lasts.
//!
//! Two workers keep calls honest: [`Chat::expire_ringing_calls`] (ringing
//! times out) and [`Chat::reconcile_calls`] (participants who vanished from
//! LiveKit without hanging up).
use std::{collections::HashSet, time::Duration};

use sqlx::{Postgres, Transaction};
use tokuly_core::{analytics::AnalyticsEvent, events, outbound, prelude::*};
use tokuly_identity::{settings, users};
use tokuly_proto::{chat as pb, common::PageInfo, identity::Audience as PrivacyAudience};
use tokuly_social::{InteractionKind, graph};

use crate::{
    Chat,
    access::{self, Audience, Content, conversation_not_found},
    events::{CALL_ENDED, CALL_STARTED, CallEnded, CallStarted},
    livekit::{JOIN_TOKEN_TTL, LiveKit},
    model::{CALL, CallRow, ConversationRow, ParticipantRow, call, call_end, kind, msg, muted_now, party, request},
    notices,
    send::Draft,
};

/// Groups with more members than this are not rung: their members are told
/// about the call and join from the conversation.
pub const MAX_RINGING_GROUP_MEMBERS: i32 = 8;
/// Seconds a participant may be marked as joined without being connected to
/// LiveKit before they count as gone (time to connect, or to reconnect).
pub const CONNECT_GRACE_SECONDS: i64 = 60;
/// Participants of a group call whose time together is reported to the
/// best-friend ranking.
const MAX_RANKED_PARTICIPANTS: usize = 12;

fn calls_disabled() -> AppError {
    AppError::unimplemented("CALLS_DISABLED", "calls are not configured on this server")
}

fn call_not_found() -> AppError {
    AppError::not_found("CALL_NOT_FOUND", "call not found")
}

fn call_ended() -> AppError {
    AppError::precondition("CALL_ENDED", "this call has ended")
}

/// The LiveKit room of a call.
fn room_name(call_id: Uuid) -> String {
    format!("tokuly-call-{}", call_id.simple())
}

/// A call after something happened to it; drives the pushes after the commit.
#[derive(Debug, Clone)]
pub(crate) struct CallChange {
    pub call: CallRow,
    /// The call's timeline entry changed (answered, ended).
    pub summary_changed: bool,
}

/// What [`settle`] decides about a live call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The call goes on.
    Continue,
    /// The call is over, for this `CallEndReason`.
    End(i16),
}

/// The facts [`settle`] judges a call by.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CallFacts {
    pub direct: bool,
    /// Somebody other than the caller has joined at some point.
    pub answered: bool,
    /// The ringing time is up.
    pub ring_expired: bool,
    /// Participants connected right now.
    pub joined: usize,
    /// Whether the caller is among them.
    pub starter_joined: bool,
    /// Participants who were rung, how many of them declined, and how many
    /// are still being rung.
    pub rung: usize,
    pub declined: usize,
    pub ringing: usize,
}

/// Decides whether a live call is over (see the module documentation).
pub(crate) fn settle(facts: CallFacts) -> Verdict {
    let unanswered = if facts.ring_expired { call_end::MISSED } else { call_end::CANCELLED };
    if facts.direct {
        if facts.answered {
            return if facts.joined < 2 { Verdict::End(call_end::COMPLETED) } else { Verdict::Continue };
        }
        if facts.declined > 0 {
            return Verdict::End(call_end::DECLINED);
        }
        // It rings on while the caller waits and the callee can still answer.
        return if facts.starter_joined && facts.ringing > 0 { Verdict::Continue } else { Verdict::End(unanswered) };
    }
    if facts.joined == 0 {
        return Verdict::End(if facts.answered { call_end::COMPLETED } else { unanswered });
    }
    if !facts.answered && facts.rung > 0 && facts.declined == facts.rung {
        return Verdict::End(call_end::DECLINED);
    }
    Verdict::Continue
}

async fn lock_call(tx: &mut Transaction<'_, Postgres>, call_id: Uuid) -> AppResult<CallRow> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {CALL} FROM chat_calls k WHERE k.id = $1 FOR UPDATE")))
        .bind(call_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(call_not_found)
}

async fn participants_in_tx(tx: &mut Transaction<'_, Postgres>, call_id: Uuid) -> AppResult<Vec<(Uuid, i16, bool)>> {
    Ok(sqlx::query_as("SELECT user_id, state, rung FROM chat_call_participants WHERE call_id = $1").bind(call_id).fetch_all(&mut **tx).await?)
}

/// Marks a connected participant as having left, adding the time they were
/// connected. Returns whether they were connected.
async fn mark_left(tx: &mut Transaction<'_, Postgres>, call_id: Uuid, user_id: Uuid) -> AppResult<bool> {
    let left = sqlx::query(
        "UPDATE chat_call_participants SET
             state = $3, left_at = now(), seconds = seconds + GREATEST(extract(epoch FROM now() - joined_at)::int, 0)
         WHERE call_id = $1 AND user_id = $2 AND state = $4",
    )
    .bind(call_id)
    .bind(user_id)
    .bind(party::LEFT)
    .bind(party::JOINED)
    .execute(&mut **tx)
    .await?;
    Ok(left.rows_affected() > 0)
}

impl Chat {
    /// The LiveKit deployment, or `CALLS_DISABLED`.
    fn require_livekit(&self) -> AppResult<&LiveKit> {
        self.livekit.as_ref().ok_or_else(calls_disabled)
    }

    fn credentials(&self, livekit: &LiveKit, call: &CallRow, user_id: Uuid, name: &str) -> AppResult<pb::CallCredentials> {
        Ok(pb::CallCredentials {
            url: livekit.public_url().to_owned(),
            token: livekit.join_token(&call.room, &user_id.to_string(), name)?,
            room: call.room.clone(),
            expires_at: ts(now() + chrono::Duration::seconds(JOIN_TOKEN_TTL.as_secs() as i64)),
            ..Default::default()
        })
    }

    /// The call ringing or running in a conversation, if any.
    pub(crate) async fn live_call(&self, conversation_id: Uuid) -> AppResult<Option<CallRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {CALL} FROM chat_calls k WHERE k.conversation_id = $1 AND k.state <> $2")))
            .bind(conversation_id)
            .bind(call::ENDED)
            .fetch_optional(&self.core.db)
            .await?)
    }

    /// A call the caller may know about: they took part in it, or they are
    /// a member of its conversation.
    pub(crate) async fn call_for(&self, user_id: Uuid, call_id: Uuid) -> AppResult<CallRow> {
        let call: CallRow = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {CALL} FROM chat_calls k WHERE k.id = $1")))
            .bind(call_id)
            .fetch_optional(&self.core.db)
            .await?
            .ok_or_else(call_not_found)?;
        let visible: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_call_participants WHERE call_id = $1 AND user_id = $2)
                 OR EXISTS (SELECT 1 FROM chat_members WHERE conversation_id = $3 AND user_id = $2 AND request_state = $4)",
        )
        .bind(call_id)
        .bind(user_id)
        .bind(call.conversation_id)
        .bind(request::ACCEPTED)
        .fetch_one(&self.core.db)
        .await?;
        if visible { Ok(call) } else { Err(call_not_found()) }
    }

    pub(crate) async fn call_to_proto(&self, call: &CallRow) -> AppResult<pb::Call> {
        let mut calls = self.calls_to_proto(std::slice::from_ref(call)).await?;
        calls.pop().ok_or_else(call_not_found)
    }

    pub(crate) async fn calls_to_proto(&self, calls: &[CallRow]) -> AppResult<Vec<pb::Call>> {
        if calls.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = calls.iter().map(|call| call.id).collect();
        let participants: Vec<ParticipantRow> = sqlx::query_as(
            "SELECT call_id, user_id, state, joined_at, left_at FROM chat_call_participants
             WHERE call_id = ANY($1) ORDER BY call_id, joined_at NULLS LAST, user_id",
        )
        .bind(&ids)
        .fetch_all(&self.core.db)
        .await?;
        let user_ids: Vec<Uuid> = participants.iter().map(|p| p.user_id).chain(calls.iter().filter_map(|call| call.started_by)).collect::<HashSet<_>>().into_iter().collect();
        let summaries = tokuly_identity::summaries(&self.core, &user_ids).await?;
        Ok(calls
            .iter()
            .map(|call| pb::Call {
                id: call.id.to_string(),
                conversation_id: call.conversation_id.to_string(),
                started_by: call.started_by.map(|id| users::pick(&summaries, id)).into(),
                video: call.video,
                state: i32::from(call.state).into(),
                end_reason: i32::from(call.end_reason).into(),
                started_at: ts(call.created_at),
                answered_at: opt_ts(call.answered_at),
                ended_at: opt_ts(call.ended_at),
                duration_seconds: call.duration_seconds(),
                ring_expires_at: ts(call.ring_expires_at),
                participants: participants
                    .iter()
                    .filter(|participant| participant.call_id == call.id)
                    .map(|participant| pb::CallParticipant {
                        user: users::pick(&summaries, participant.user_id).into(),
                        state: i32::from(participant.state).into(),
                        joined_at: opt_ts(participant.joined_at),
                        left_at: opt_ts(participant.left_at),
                        ..Default::default()
                    })
                    .collect(),
                message_id: call.message_id.map(|id| id.to_string()).unwrap_or_default(),
                ..Default::default()
            })
            .collect())
    }

    /// The caller's call history, newest first.
    pub(crate) async fn list_calls(&self, user_id: Uuid, missed_only: bool, page: &Page) -> AppResult<(Vec<CallRow>, PageInfo)> {
        let before: Option<Uuid> = page.decode()?;
        let mut rows: Vec<CallRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CALL} FROM chat_call_participants p JOIN chat_calls k ON k.id = p.call_id
             WHERE p.user_id = $1 AND (NOT $2 OR p.state = $3) AND ($4::uuid IS NULL OR p.call_id < $4)
             ORDER BY p.call_id DESC LIMIT $5"
        )))
        .bind(user_id)
        .bind(missed_only)
        .bind(party::MISSED)
        .bind(before)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |call| call.id);
        Ok((rows, info))
    }

    // ───────────────────────────── starting ─────────────────────────────

    pub(crate) async fn start_call(&self, user_id: Uuid, conversation_id: Uuid, video: bool) -> AppResult<(CallRow, pb::CallCredentials)> {
        let core = &self.core;
        let livekit = self.require_livekit()?;
        let access = self.access(user_id, conversation_id).await?;
        self.ensure_accepted(&access)?;
        // The same rules as for sending something that is not plain text:
        // a read-only conversation and an unanswered request both stop a call.
        self.ensure_can_send(&access, Content::Rich).await?;
        if let Some(callee) = access.other_user() {
            let privacy = settings::privacy_of(core, callee).await?;
            let audience = privacy.calls_from.as_known().unwrap_or(PrivacyAudience::AUDIENCE_UNSPECIFIED);
            ensure!(
                graph::allows(core, callee, Some(user_id), audience).await?,
                denied,
                "CALL_NOT_ALLOWED",
                "this user does not accept calls from you"
            );
        }
        let in_progress = || AppError::precondition("CALL_IN_PROGRESS", "this conversation already has a call; join it instead");
        if self.live_call(conversation_id).await?.is_some() {
            return Err(in_progress());
        }
        core.cache.rate_limit(&format!("chat-call:{user_id}"), 20, Duration::from_secs(600)).await?;
        let caller = tokuly_identity::summary(core, user_id).await?;

        let call_id = new_id();
        let room = room_name(call_id);
        livekit.create_room(&room, access.conv.member_count.max(2) as u32).await.map_err(|e| {
            tracing::error!(error = %e, "LiveKit room could not be created");
            AppError::unavailable("CALL_UNAVAILABLE", "calls are unavailable right now")
        })?;

        let started: AppResult<(CallRow, crate::send::Posted)> = async {
            let mut tx = core.db.begin().await?;
            let mut conv = access::lock_conversation(&mut tx, conversation_id).await?;
            let live: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM chat_calls WHERE conversation_id = $1 AND state <> $2)")
                .bind(conversation_id)
                .bind(call::ENDED)
                .fetch_one(&mut *tx)
                .await?;
            if live {
                return Err(in_progress());
            }
            // Who the call is for: the accepted members, without anyone in a
            // block with the caller. Small conversations ring.
            let hidden = graph::blocked_ids_either(core, user_id).await?;
            let members: Vec<(Uuid, bool, Option<Time>)> = sqlx::query_as(
                "SELECT user_id, muted, muted_until FROM chat_members WHERE conversation_id = $1 AND request_state = $2 AND user_id <> $3",
            )
            .bind(conversation_id)
            .bind(request::ACCEPTED)
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
            let callees: Vec<(Uuid, bool)> =
                members.into_iter().filter(|(member, _, _)| !hidden.contains(member)).map(|(member, muted, until)| (member, muted_now(muted, until))).collect();
            let rings = conv.is_direct() || conv.member_count <= MAX_RINGING_GROUP_MEMBERS;

            sqlx::query(
                "INSERT INTO chat_calls (id, conversation_id, started_by, video, state, room, ring_expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, now() + make_interval(secs => $7))",
            )
            .bind(call_id)
            .bind(conversation_id)
            .bind(user_id)
            .bind(video)
            .bind(call::RINGING)
            .bind(&room)
            .bind(self.cfg.ring_seconds as f64)
            .execute(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO chat_call_participants (call_id, user_id, state, joined_at, rung) VALUES ($1, $2, $3, now(), false)")
                .bind(call_id)
                .bind(user_id)
                .bind(party::JOINED)
                .execute(&mut *tx)
                .await?;
            if rings {
                sqlx::query("INSERT INTO chat_call_participants (call_id, user_id, state, rung) SELECT $1, callee, $3, true FROM unnest($2::uuid[]) AS callee")
                    .bind(call_id)
                    .bind(callees.iter().map(|(callee, _)| *callee).collect::<Vec<_>>())
                    .bind(party::RINGING)
                    .execute(&mut *tx)
                    .await?;
            }
            // The call's entry in the conversation; the ringing itself is the notification.
            let draft = Draft { call_id: Some(call_id), ..Draft::from_user(msg::CALL, user_id) }.silent();
            let posted = self.post(&mut tx, &mut conv, draft).await?;
            let call: CallRow = sqlx::query_as(sqlx::AssertSqlSafe(format!("UPDATE chat_calls k SET message_id = $2 WHERE k.id = $1 RETURNING {CALL}")))
                .bind(call_id)
                .bind(posted.row.id)
                .fetch_one(&mut *tx)
                .await?;
            events::emit(&mut *tx, CALL_STARTED, Some(user_id), &CallStarted { call_id, conversation_id, started_by: user_id, video }).await?;
            for (callee, muted) in &callees {
                // A direct call always rings through. In a group, members who
                // muted it see the call in the app but get no push.
                if conv.is_group() && *muted {
                    continue;
                }
                outbound::notify(&mut *tx, &notices::incoming_call(*callee, &caller, user_id, &conv, call_id, video, rings)).await?;
            }
            tx.commit().await?;
            Ok((call, posted))
        }
        .await;
        let (call, posted) = match started {
            Ok(started) => started,
            Err(e) => {
                // Nobody will ever join the room that was prepared.
                if let Err(e) = livekit.delete_room(&room).await {
                    tracing::warn!(error = %e, "unused LiveKit room could not be removed");
                }
                return Err(e);
            }
        };
        self.announce(&posted).await;
        self.push_call(&call).await;
        let credentials = self.credentials(livekit, &call, user_id, &notices::name_of(&caller))?;
        Ok((call, credentials))
    }

    // ───────────────────────────── taking part ─────────────────────────────

    /// Accepts a ringing call, joins a running group call, or reconnects.
    pub(crate) async fn join_call(&self, user_id: Uuid, call_id: Uuid) -> AppResult<(CallRow, pb::CallCredentials)> {
        let livekit = self.require_livekit()?;
        let call = self.call_for(user_id, call_id).await?;
        if !call.is_live() {
            return Err(call_ended());
        }
        // Only members who accepted the conversation take part in its calls.
        let access = self.access(user_id, call.conversation_id).await.map_err(|_| call_not_found())?;
        if !access.me.is_accepted() {
            return Err(call_not_found());
        }
        self.ensure_writable(&access).await?;
        let me = tokuly_identity::summary(&self.core, user_id).await?;

        let mut tx = self.core.db.begin().await?;
        let call = lock_call(&mut tx, call_id).await?;
        if !call.is_live() {
            return Err(call_ended());
        }
        sqlx::query(
            "INSERT INTO chat_call_participants AS p (call_id, user_id, state, joined_at, rung) VALUES ($1, $2, $3, now(), false)
             ON CONFLICT (call_id, user_id) DO UPDATE SET
                 seconds = p.seconds + CASE WHEN p.state = $3 THEN GREATEST(extract(epoch FROM now() - p.joined_at)::int, 0) ELSE 0 END,
                 state = $3, joined_at = now(), left_at = NULL",
        )
        .bind(call_id)
        .bind(user_id)
        .bind(party::JOINED)
        .execute(&mut *tx)
        .await?;
        let answers = call.state == call::RINGING && call.started_by != Some(user_id);
        let call = if answers {
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "UPDATE chat_calls k SET state = $2, answered_at = now() WHERE k.id = $1 RETURNING {CALL}"
            )))
            .bind(call_id)
            .bind(call::ACTIVE)
            .fetch_one(&mut *tx)
            .await?
        } else {
            call
        };
        tx.commit().await?;
        self.after_call_change(&CallChange { call: call.clone(), summary_changed: answers }).await;
        let credentials = self.credentials(livekit, &call, user_id, &notices::name_of(&me))?;
        Ok((call, credentials))
    }

    pub(crate) async fn decline_call(&self, user_id: Uuid, call_id: Uuid) -> AppResult<CallRow> {
        self.require_livekit()?;
        self.call_for(user_id, call_id).await?;
        let mut tx = self.core.db.begin().await?;
        let call = lock_call(&mut tx, call_id).await?;
        if !call.is_live() {
            return Err(call_ended());
        }
        let state: Option<i16> = sqlx::query_scalar("SELECT state FROM chat_call_participants WHERE call_id = $1 AND user_id = $2")
            .bind(call_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
        match state {
            Some(party::JOINED) => bail!(precondition, "ALREADY_JOINED", "you are in this call; leave it instead"),
            Some(party::RINGING | party::MISSED) => {
                sqlx::query("UPDATE chat_call_participants SET state = $3 WHERE call_id = $1 AND user_id = $2")
                    .bind(call_id)
                    .bind(user_id)
                    .bind(party::DECLINED)
                    .execute(&mut *tx)
                    .await?;
            }
            // Declined before, or never rung: nothing to decline.
            _ => return Ok(call),
        }
        let change = self.settle_call(&mut tx, call).await?;
        tx.commit().await?;
        self.after_call_change(&change).await;
        Ok(change.call)
    }

    /// Hangs up. Succeeds when the call already ended.
    pub(crate) async fn leave_call(&self, user_id: Uuid, call_id: Uuid) -> AppResult<CallRow> {
        self.require_livekit()?;
        self.call_for(user_id, call_id).await?;
        let mut tx = self.core.db.begin().await?;
        let call = lock_call(&mut tx, call_id).await?;
        if !call.is_live() {
            return Ok(call);
        }
        if !mark_left(&mut tx, call_id, user_id).await? {
            // Hanging up on a call that is still ringing for you declines it.
            sqlx::query("UPDATE chat_call_participants SET state = $3 WHERE call_id = $1 AND user_id = $2 AND state = $4")
                .bind(call_id)
                .bind(user_id)
                .bind(party::DECLINED)
                .bind(party::RINGING)
                .execute(&mut *tx)
                .await?;
        }
        let change = self.settle_call(&mut tx, call).await?;
        tx.commit().await?;
        self.after_call_change(&change).await;
        Ok(change.call)
    }

    // ───────────────────────────── ending ─────────────────────────────

    /// Applies the end rules to a live call after its participants changed.
    async fn settle_call(&self, tx: &mut Transaction<'_, Postgres>, call: CallRow) -> AppResult<CallChange> {
        let conversation_kind: i16 =
            sqlx::query_scalar("SELECT kind FROM chat_conversations WHERE id = $1").bind(call.conversation_id).fetch_one(&mut **tx).await?;
        let participants = participants_in_tx(tx, call.id).await?;
        let facts = CallFacts {
            direct: conversation_kind == kind::DIRECT,
            answered: call.answered_at.is_some(),
            ring_expired: call.ring_expires_at <= now(),
            joined: participants.iter().filter(|(_, state, _)| *state == party::JOINED).count(),
            starter_joined: participants.iter().any(|(user_id, state, _)| Some(*user_id) == call.started_by && *state == party::JOINED),
            rung: participants.iter().filter(|(_, _, rung)| *rung).count(),
            declined: participants.iter().filter(|(_, state, rung)| *rung && *state == party::DECLINED).count(),
            ringing: participants.iter().filter(|(_, state, _)| *state == party::RINGING).count(),
        };
        match settle(facts) {
            Verdict::Continue => Ok(CallChange { call, summary_changed: false }),
            Verdict::End(reason) => self.finish_call(tx, &call, reason).await,
        }
    }

    /// Ends a call: everyone still connected has left, everyone still being
    /// rung has missed it (and is told so).
    async fn finish_call(&self, tx: &mut Transaction<'_, Postgres>, call: &CallRow, reason: i16) -> AppResult<CallChange> {
        sqlx::query(
            "UPDATE chat_call_participants SET
                 state = $2, left_at = now(), seconds = seconds + GREATEST(extract(epoch FROM now() - joined_at)::int, 0)
             WHERE call_id = $1 AND state = $3",
        )
        .bind(call.id)
        .bind(party::LEFT)
        .bind(party::JOINED)
        .execute(&mut **tx)
        .await?;
        self.mark_missed(tx, call).await?;
        let ended: CallRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE chat_calls k SET state = $2, end_reason = $3, ended_at = now() WHERE k.id = $1 RETURNING {CALL}"
        )))
        .bind(call.id)
        .bind(call::ENDED)
        .bind(reason)
        .fetch_one(&mut **tx)
        .await?;
        let participant_ids: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM chat_call_participants WHERE call_id = $1 AND joined_at IS NOT NULL")
            .bind(call.id)
            .fetch_all(&mut **tx)
            .await?;
        let payload = CallEnded {
            call_id: call.id,
            conversation_id: call.conversation_id,
            end_reason: i32::from(reason),
            duration_seconds: ended.duration_seconds(),
            participant_ids,
        };
        events::emit(&mut **tx, CALL_ENDED, call.started_by, &payload).await?;
        Ok(CallChange { call: ended, summary_changed: true })
    }

    /// Turns everyone still being rung into "missed" and requests their
    /// missed-call notifications. Returns how many there were.
    async fn mark_missed(&self, tx: &mut Transaction<'_, Postgres>, call: &CallRow) -> AppResult<usize> {
        let missed: Vec<Uuid> = sqlx::query_scalar("UPDATE chat_call_participants SET state = $2 WHERE call_id = $1 AND state = $3 RETURNING user_id")
            .bind(call.id)
            .bind(party::MISSED)
            .bind(party::RINGING)
            .fetch_all(&mut **tx)
            .await?;
        let Some(caller_id) = call.started_by.filter(|_| !missed.is_empty()) else { return Ok(missed.len()) };
        let conv: Option<ConversationRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {} FROM chat_conversations c WHERE c.id = $1", crate::model::CONV)))
                .bind(call.conversation_id)
                .fetch_optional(&mut **tx)
                .await?;
        let conv = conv.ok_or_else(conversation_not_found)?;
        let caller = tokuly_identity::summary(&self.core, caller_id).await?;
        for user_id in &missed {
            outbound::notify(&mut **tx, &notices::missed_call(*user_id, &caller, caller_id, &conv, call.id, call.video)).await?;
        }
        Ok(missed.len())
    }

    /// Takes a user who is no longer a member out of the live call of a
    /// conversation (which the caller has locked). Returns what to announce.
    pub(crate) async fn drop_from_live_call(&self, tx: &mut Transaction<'_, Postgres>, conv: &ConversationRow, user_id: Uuid) -> AppResult<Option<CallChange>> {
        let Some(call) = self.lock_live_call(tx, conv.id).await? else { return Ok(None) };
        if !mark_left(tx, call.id, user_id).await? {
            let withdrawn = sqlx::query("UPDATE chat_call_participants SET state = $3, rung = false WHERE call_id = $1 AND user_id = $2 AND state = $4")
                .bind(call.id)
                .bind(user_id)
                .bind(party::LEFT)
                .bind(party::RINGING)
                .execute(&mut **tx)
                .await?;
            if withdrawn.rows_affected() == 0 {
                return Ok(None);
            }
        }
        Ok(Some(self.settle_call(tx, call).await?))
    }

    /// Ends the live call of a conversation nobody can talk in any more.
    pub(crate) async fn fail_live_call(&self, tx: &mut Transaction<'_, Postgres>, conversation_id: Uuid) -> AppResult<Option<CallChange>> {
        match self.lock_live_call(tx, conversation_id).await? {
            Some(call) => Ok(Some(self.finish_call(tx, &call, call_end::FAILED).await?)),
            None => Ok(None),
        }
    }

    async fn lock_live_call(&self, tx: &mut Transaction<'_, Postgres>, conversation_id: Uuid) -> AppResult<Option<CallRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CALL} FROM chat_calls k WHERE k.conversation_id = $1 AND k.state <> $2 FOR UPDATE"
        )))
        .bind(conversation_id)
        .bind(call::ENDED)
        .fetch_optional(&mut **tx)
        .await?)
    }

    // ───────────────────────────── announcing ─────────────────────────────

    async fn push_call(&self, call: &CallRow) {
        match self.call_to_proto(call).await {
            Ok(proto) => self.push_members(call.conversation_id, Audience::Accepted, pb::CallChanged { call: proto.into(), ..Default::default() }).await,
            Err(e) => tracing::warn!(call_id = %call.id, error = %e, "call push failed"),
        }
    }

    /// Pushes a changed call and, when it ended, closes its room and
    /// reports the time people spent talking.
    pub(crate) async fn after_call_change(&self, change: &CallChange) {
        let call = &change.call;
        self.push_call(call).await;
        if let Some(message_id) = call.message_id.filter(|_| change.summary_changed) {
            self.push_message_updated_by_id(message_id, pb::MessageUpdateReason::MESSAGE_UPDATE_REASON_CALL).await;
        }
        if call.is_live() {
            return;
        }
        if let Some(livekit) = &self.livekit {
            if let Err(e) = livekit.delete_room(&call.room).await {
                tracing::warn!(call_id = %call.id, error = %e, "LiveKit room could not be closed");
            }
        }
        if let Err(e) = self.report_call(call).await {
            tracing::warn!(call_id = %call.id, error = %e, "reporting a call to other domains failed");
        }
    }

    /// Reports a finished call: minutes talked feed the best-friend ranking.
    async fn report_call(&self, call: &CallRow) -> AppResult<()> {
        let core = &self.core;
        let mut talkers: Vec<(Uuid, i32)> =
            sqlx::query_as("SELECT user_id, seconds FROM chat_call_participants WHERE call_id = $1 AND seconds > 0 ORDER BY seconds DESC, user_id LIMIT $2")
                .bind(call.id)
                .bind(MAX_RANKED_PARTICIPANTS as i64)
                .fetch_all(&core.db)
                .await?;
        if call.answered_at.is_some() {
            talkers.sort_by_key(|(user_id, _)| *user_id);
            for (index, (a, a_seconds)) in talkers.iter().enumerate() {
                for (b, b_seconds) in &talkers[index + 1..] {
                    // Two people were on the call together for at most as long as the shorter stay.
                    let minutes = (*a_seconds.min(b_seconds) + 59) / 60;
                    graph::record_interaction(core, *a, *b, InteractionKind::Call, minutes).await?;
                }
            }
        }
        let event = AnalyticsEvent::new("chat.call.end")
            .user(call.started_by)
            .subject("conversation", call.conversation_id)
            .value(f64::from(call.duration_seconds()))
            .props(serde_json::json!({ "video": call.video, "end_reason": call.end_reason, "participants": talkers.len() }));
        core.analytics.track(event).await;
        Ok(())
    }

    // ───────────────────────────── workers ─────────────────────────────

    /// Stops the ringing of calls whose ringing time is up: an unanswered
    /// direct call ends as missed; in a group the members who did not
    /// answer are marked missed and the call goes on. Returns how many
    /// calls were touched.
    pub async fn expire_ringing_calls(&self) -> AppResult<usize> {
        let due: Vec<Uuid> = sqlx::query_scalar(
            "SELECT k.id FROM chat_calls k
             WHERE k.state <> $1 AND k.ring_expires_at <= now()
               AND EXISTS (SELECT 1 FROM chat_call_participants p WHERE p.call_id = k.id AND p.state = $2)
             ORDER BY k.ring_expires_at LIMIT 200",
        )
        .bind(call::ENDED)
        .bind(party::RINGING)
        .fetch_all(&self.core.db)
        .await?;
        let mut touched = 0;
        for call_id in due {
            let mut tx = self.core.db.begin().await?;
            let call = lock_call(&mut tx, call_id).await?;
            if !call.is_live() || call.ring_expires_at > now() {
                continue;
            }
            if self.mark_missed(&mut tx, &call).await? == 0 {
                continue;
            }
            let change = self.settle_call(&mut tx, call).await?;
            tx.commit().await?;
            self.after_call_change(&change).await;
            touched += 1;
        }
        Ok(touched)
    }

    /// Compares live calls with LiveKit: a participant marked as joined who
    /// has not been connected for [`CONNECT_GRACE_SECONDS`] has left (their
    /// app crashed or lost its connection), which may end the call. Returns
    /// how many calls changed. Does nothing when LiveKit cannot be reached.
    pub async fn reconcile_calls(&self) -> AppResult<usize> {
        let Some(livekit) = &self.livekit else { return Ok(0) };
        let live: Vec<CallRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CALL} FROM chat_calls k WHERE k.state <> $1 AND k.created_at <= now() - make_interval(secs => $2) ORDER BY k.id LIMIT 500"
        )))
        .bind(call::ENDED)
        .bind(CONNECT_GRACE_SECONDS as f64)
        .fetch_all(&self.core.db)
        .await?;
        let mut changed = 0;
        for call in live {
            let connected: Vec<Uuid> = match livekit.list_participants(&call.room).await {
                Ok(identities) => identities.unwrap_or_default().iter().filter_map(|identity| Uuid::parse_str(identity).ok()).collect(),
                Err(e) => {
                    tracing::warn!(call_id = %call.id, error = %e, "call could not be reconciled with LiveKit");
                    continue;
                }
            };
            let mut tx = self.core.db.begin().await?;
            let call = lock_call(&mut tx, call.id).await?;
            if !call.is_live() {
                continue;
            }
            let gone = sqlx::query(
                "UPDATE chat_call_participants SET
                     state = $2, left_at = now(), seconds = seconds + GREATEST(extract(epoch FROM now() - joined_at)::int, 0)
                 WHERE call_id = $1 AND state = $3 AND joined_at <= now() - make_interval(secs => $4) AND user_id <> ALL($5)",
            )
            .bind(call.id)
            .bind(party::LEFT)
            .bind(party::JOINED)
            .bind(CONNECT_GRACE_SECONDS as f64)
            .bind(&connected)
            .execute(&mut *tx)
            .await?;
            if gone.rows_affected() == 0 {
                continue;
            }
            let change = self.settle_call(&mut tx, call).await?;
            tx.commit().await?;
            self.after_call_change(&change).await;
            changed += 1;
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct() -> CallFacts {
        CallFacts { direct: true, answered: false, ring_expired: false, joined: 1, starter_joined: true, rung: 1, declined: 0, ringing: 1 }
    }

    fn group() -> CallFacts {
        CallFacts { direct: false, answered: false, ring_expired: false, joined: 1, starter_joined: true, rung: 3, declined: 0, ringing: 3 }
    }

    #[test]
    fn a_ringing_direct_call_waits_for_the_callee() {
        assert_eq!(settle(direct()), Verdict::Continue);
        assert_eq!(settle(CallFacts { declined: 1, ringing: 0, ..direct() }), Verdict::End(call_end::DECLINED));
        // The caller gives up.
        assert_eq!(settle(CallFacts { joined: 0, starter_joined: false, ..direct() }), Verdict::End(call_end::CANCELLED));
        // Nobody answered in time: the callee is no longer being rung.
        assert_eq!(settle(CallFacts { ring_expired: true, ringing: 0, ..direct() }), Verdict::End(call_end::MISSED));
        assert_eq!(settle(CallFacts { joined: 0, starter_joined: false, ring_expired: true, ringing: 0, ..direct() }), Verdict::End(call_end::MISSED));
    }

    #[test]
    fn an_answered_direct_call_ends_when_either_side_leaves() {
        let talking = CallFacts { answered: true, joined: 2, ..direct() };
        assert_eq!(settle(talking), Verdict::Continue);
        assert_eq!(settle(CallFacts { joined: 1, ..talking }), Verdict::End(call_end::COMPLETED));
        assert_eq!(settle(CallFacts { joined: 1, starter_joined: false, ..talking }), Verdict::End(call_end::COMPLETED));
    }

    #[test]
    fn a_group_call_lasts_while_someone_is_connected() {
        assert_eq!(settle(group()), Verdict::Continue);
        // Some declined, some may still answer.
        assert_eq!(settle(CallFacts { declined: 2, ..group() }), Verdict::Continue);
        assert_eq!(settle(CallFacts { declined: 3, ringing: 0, ..group() }), Verdict::End(call_end::DECLINED));
        // The ringing time passing does not end it: members may still join.
        assert_eq!(settle(CallFacts { ring_expired: true, ringing: 0, ..group() }), Verdict::Continue);
        let talking = CallFacts { answered: true, joined: 3, ..group() };
        assert_eq!(settle(CallFacts { joined: 1, starter_joined: false, ..talking }), Verdict::Continue, "the caller may leave a group call");
        assert_eq!(settle(CallFacts { joined: 0, starter_joined: false, ..talking }), Verdict::End(call_end::COMPLETED));
    }

    #[test]
    fn an_unanswered_group_call_ends_with_its_caller() {
        let alone = CallFacts { joined: 0, starter_joined: false, ..group() };
        assert_eq!(settle(alone), Verdict::End(call_end::CANCELLED));
        assert_eq!(settle(CallFacts { ring_expired: true, ..alone }), Verdict::End(call_end::MISSED));
        // A big group nobody was rung in.
        assert_eq!(settle(CallFacts { rung: 0, ringing: 0, ..group() }), Verdict::Continue);
    }

    #[test]
    fn rooms_are_named_after_their_call() {
        let call_id = Uuid::now_v7();
        assert_eq!(room_name(call_id), format!("tokuly-call-{}", call_id.simple()));
    }
}
