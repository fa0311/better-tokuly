//! Flames: the compliments a user received.
//!
//! A flame tells its receiver which question they were picked for and when.
//! The row type the receiver's queries use has no sender column at all, so
//! nothing here can pass the voter on by accident; the two places that do
//! read the sender are `hints.rs` (a hint the receiver pays for, if the
//! sender allows it) and the moderation service.
use std::collections::HashMap;

use chrono::{DateTime, Datelike as _, Duration, TimeZone as _, Utc};
use chrono_tz::Asia::Tokyo;
use tokuly_core::prelude::*;
use tokuly_proto::{
    ask::{Flame, FlameSummary, Hint, QuestionCount, TopFlameEntry},
    common::PageInfo,
};
use tokuly_social::graph;

use super::{bank::poll_question, hints::HintRow};
use crate::Ask;

/// "Most picked for…" questions in a summary.
const TOP_QUESTIONS: i64 = 5;
/// Lines of the weekly ranking.
const TOP_FLAMES: usize = 20;

/// SQL condition for "this flame is in its receiver's inbox": not dropped,
/// and not sent by a banned account.
macro_rules! in_inbox {
    () => {
        "NOT f.dropped AND NOT EXISTS (SELECT 1 FROM users banned WHERE banned.id = f.sender_id AND banned.banned_at IS NOT NULL)"
    };
}
pub(crate) use in_inbox;

/// Columns of [`FlameRow`], from `ask_flames f JOIN ask_poll_questions p`.
macro_rules! flame_columns {
    () => {
        "f.id, f.question_id, f.created_at, f.seen_at, p.text_ja, p.text_en, p.emoji"
    };
}
pub(crate) use flame_columns;

/// A flame as its receiver may see it. There is deliberately no sender here.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct FlameRow {
    pub id: Uuid,
    pub question_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub seen_at: Option<DateTime<Utc>>,
    pub text_ja: String,
    pub text_en: String,
    pub emoji: String,
}

impl FlameRow {
    /// `hints` are the ones bought so far, in order; `next_hint_price` is 0
    /// when no further hint is sold.
    pub(crate) fn to_proto(&self, hints: Vec<Hint>, next_hint_price: i64) -> Flame {
        Flame {
            id: self.id.to_string(),
            question: poll_question(self.question_id, &self.text_ja, &self.text_en, &self.emoji).into(),
            created_at: ts(self.created_at),
            seen: self.seen_at.is_some(),
            hints,
            next_hint_price,
            ..Default::default()
        }
    }
}

/// Monday 00:00 in Japan of the week `now` falls in.
pub(crate) fn week_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let local = now.with_timezone(&Tokyo);
    let monday = local.date_naive() - Duration::days(i64::from(local.weekday().num_days_from_monday()));
    // Japan has no daylight saving time, so midnight always exists once.
    Tokyo
        .from_local_datetime(&monday.and_hms_opt(0, 0, 0).unwrap_or_default())
        .single()
        .map(|start| start.with_timezone(&Utc))
        .unwrap_or(now)
}

impl Ask {
    /// The price of the hint after `bought` hints; 0 when none is left. It
    /// depends on the count alone, never on who sent the flame: a price that
    /// knew whether the sender allows hints would itself be a hint.
    pub(crate) fn next_hint_price(&self, bought: usize) -> i64 {
        self.hint_prices.get(bought).copied().unwrap_or(0)
    }

    /// Flames as their receiver sees them, with the hints they bought.
    pub(crate) async fn render_flames(&self, rows: &[FlameRow]) -> AppResult<Vec<Flame>> {
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let bought: Vec<HintRow> =
            sqlx::query_as("SELECT flame_id, kind, value FROM ask_flame_hints WHERE flame_id = ANY($1) AND entry_id IS NOT NULL ORDER BY flame_id, position")
                .bind(&ids)
                .fetch_all(&self.core.db)
                .await?;
        let mut by_flame: HashMap<Uuid, Vec<Hint>> = HashMap::new();
        for hint in bought {
            by_flame.entry(hint.flame_id).or_default().push(hint.to_proto());
        }
        Ok(rows
            .iter()
            .map(|row| {
                let hints = by_flame.remove(&row.id).unwrap_or_default();
                let price = self.next_hint_price(hints.len());
                row.to_proto(hints, price)
            })
            .collect())
    }

    /// A flame in `receiver`'s inbox, or `FLAME_NOT_FOUND`.
    pub(crate) async fn received_flame(&self, receiver: Uuid, flame_id: Uuid) -> AppResult<FlameRow> {
        let row: Option<FlameRow> = sqlx::query_as(concat!(
            "SELECT ",
            flame_columns!(),
            " FROM ask_flames f JOIN ask_poll_questions p ON p.id = f.question_id
              WHERE f.receiver_id = $1 AND f.id = $2 AND ",
            in_inbox!()
        ))
        .bind(receiver)
        .bind(flame_id)
        .fetch_optional(&self.core.db)
        .await?;
        row.or_not_found("FLAME_NOT_FOUND", "flame not found")
    }

    /// `receiver`'s flames, newest first.
    pub(crate) async fn flames_page(&self, receiver: Uuid, page: &Page) -> AppResult<(Vec<FlameRow>, PageInfo)> {
        let cursor: Option<Uuid> = page.decode()?;
        let mut rows: Vec<FlameRow> = sqlx::query_as(concat!(
            "SELECT ",
            flame_columns!(),
            " FROM ask_flames f JOIN ask_poll_questions p ON p.id = f.question_id
              WHERE f.receiver_id = $1 AND ",
            in_inbox!(),
            " AND ($2::uuid IS NULL OR f.id < $2)
              ORDER BY f.id DESC LIMIT $3"
        ))
        .bind(receiver)
        .bind(cursor)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// How many of `receiver`'s flames are unseen.
    pub(crate) async fn unseen_flames(&self, receiver: Uuid) -> AppResult<i32> {
        let unseen: i64 = sqlx::query_scalar(concat!(
            "SELECT count(*) FROM ask_flames f WHERE f.receiver_id = $1 AND f.seen_at IS NULL AND ",
            in_inbox!()
        ))
        .bind(receiver)
        .fetch_one(&self.core.db)
        .await?;
        Ok(unseen as i32)
    }

    /// `receiver`'s flames in numbers.
    pub(crate) async fn flame_summary(&self, receiver: Uuid) -> AppResult<FlameSummary> {
        let (total, unseen, this_week): (i64, i64, i64) = sqlx::query_as(concat!(
            "SELECT count(*), count(*) FILTER (WHERE f.seen_at IS NULL), count(*) FILTER (WHERE f.created_at >= $2)
             FROM ask_flames f WHERE f.receiver_id = $1 AND ",
            in_inbox!()
        ))
        .bind(receiver)
        .bind(week_start(Utc::now()))
        .fetch_one(&self.core.db)
        .await?;
        let top: Vec<(Uuid, String, String, String, i64)> = sqlx::query_as(concat!(
            "SELECT p.id, p.text_ja, p.text_en, p.emoji, count(*) AS picked
             FROM ask_flames f JOIN ask_poll_questions p ON p.id = f.question_id
             WHERE f.receiver_id = $1 AND ",
            in_inbox!(),
            " GROUP BY p.id ORDER BY picked DESC, p.id LIMIT $2"
        ))
        .bind(receiver)
        .bind(TOP_QUESTIONS)
        .fetch_all(&self.core.db)
        .await?;
        Ok(FlameSummary {
            total: total as i32,
            unseen: unseen as i32,
            this_week: this_week as i32,
            top_questions: top
                .into_iter()
                .map(|(id, ja, en, emoji, picked)| QuestionCount {
                    question: poll_question(id, &ja, &en, &emoji).into(),
                    count: picked as i32,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
    }

    /// Marks flames of `receiver` as seen: `ids`, or all of them.
    pub(crate) async fn mark_seen(&self, receiver: Uuid, ids: &[Uuid], all: bool) -> AppResult<i32> {
        sqlx::query(
            "UPDATE ask_flames SET seen_at = now() WHERE receiver_id = $1 AND seen_at IS NULL AND ($2 OR id = ANY($3))",
        )
        .bind(receiver)
        .bind(all)
        .bind(ids)
        .execute(&self.core.db)
        .await?;
        self.unseen_flames(receiver).await
    }

    /// This week's flame counts among `viewer` and their friends.
    ///
    /// The two counts are not the same number on purpose:
    /// - **friends** are counted with dropped flames included. A voter whom
    ///   a friend blocked as an anonymous sender expects that friend's count
    ///   to go up after voting for them; if it did not, the ranking would
    ///   tell the voter about the block.
    /// - **the viewer's own** count leaves dropped flames out, like their
    ///   inbox: flames they will never see would otherwise show up as a
    ///   number they cannot explain.
    pub(crate) async fn top_flames(&self, viewer: Uuid) -> AppResult<(Vec<TopFlameEntry>, DateTime<Utc>, i32)> {
        let since = week_start(Utc::now());
        let hidden = graph::blocked_ids_either(&self.core, viewer).await?;
        let friends: Vec<Uuid> =
            graph::friend_ids(&self.core, viewer).await?.into_iter().filter(|id| !hidden.contains(id)).collect();
        let friend_counts: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT f.receiver_id, count(*)
             FROM ask_flames f JOIN users u ON u.id = f.receiver_id
             WHERE f.receiver_id = ANY($1) AND f.created_at >= $2
               AND u.deleted_at IS NULL AND u.banned_at IS NULL
               AND NOT EXISTS (SELECT 1 FROM ask_settings s WHERE s.user_id = f.receiver_id AND NOT s.in_polls)
               AND NOT EXISTS (SELECT 1 FROM users banned WHERE banned.id = f.sender_id AND banned.banned_at IS NOT NULL)
             GROUP BY f.receiver_id",
        )
        .bind(&friends)
        .bind(since)
        .fetch_all(&self.core.db)
        .await?;
        let my_count: i64 = sqlx::query_scalar(concat!(
            "SELECT count(*) FROM ask_flames f WHERE f.receiver_id = $1 AND f.created_at >= $2 AND ",
            in_inbox!()
        ))
        .bind(viewer)
        .bind(since)
        .fetch_one(&self.core.db)
        .await?;

        let mut counts = friend_counts;
        if my_count > 0 {
            counts.push((viewer, my_count));
        }
        let ranked = rank(counts, TOP_FLAMES);
        let ids: Vec<Uuid> = ranked.iter().map(|(id, _, _)| *id).collect();
        let users = tokuly_identity::summaries(&self.core, &ids).await?;
        let entries = ranked
            .into_iter()
            .map(|(id, count, rank)| TopFlameEntry {
                user: tokuly_identity::users::pick(&users, id).into(),
                count,
                rank,
                ..Default::default()
            })
            .collect();
        Ok((entries, since, my_count as i32))
    }
}

/// Orders counts most first (ties by ID, so the order is stable) and numbers
/// them; equal counts share a rank. Keeps the first `limit`.
fn rank(mut counts: Vec<(Uuid, i64)>, limit: usize) -> Vec<(Uuid, i32, i32)> {
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut ranked = Vec::with_capacity(counts.len().min(limit));
    let mut previous: Option<(i64, i32)> = None;
    for (index, (id, count)) in counts.into_iter().enumerate().take(limit) {
        let rank = match previous {
            Some((same, rank)) if same == count => rank,
            _ => index as i32 + 1,
        };
        previous = Some((count, rank));
        ranked.push((id, count as i32, rank));
    }
    ranked
}

#[cfg(test)]
mod tests {
    use chrono::Weekday;

    use super::*;

    #[test]
    fn the_week_starts_on_monday_midnight_in_japan() {
        // Wednesday 2026-11-18 03:00 UTC is Wednesday 12:00 in Japan.
        let now = Utc.with_ymd_and_hms(2026, 11, 18, 3, 0, 0).unwrap();
        let start = week_start(now);
        // Monday 2026-11-16 00:00 JST = Sunday 2026-11-15 15:00 UTC.
        assert_eq!(start, Utc.with_ymd_and_hms(2026, 11, 15, 15, 0, 0).unwrap());
        assert_eq!(start.with_timezone(&Tokyo).weekday(), Weekday::Mon);
    }

    #[test]
    fn sunday_evening_utc_is_already_monday_in_japan() {
        // Sunday 2026-11-22 16:00 UTC is Monday 01:00 in Japan: a new week.
        let now = Utc.with_ymd_and_hms(2026, 11, 22, 16, 0, 0).unwrap();
        assert_eq!(week_start(now), Utc.with_ymd_and_hms(2026, 11, 22, 15, 0, 0).unwrap());
        // One hour earlier it is still Sunday there: last week.
        let earlier = Utc.with_ymd_and_hms(2026, 11, 22, 14, 59, 0).unwrap();
        assert_eq!(week_start(earlier), Utc.with_ymd_and_hms(2026, 11, 15, 15, 0, 0).unwrap());
    }

    #[test]
    fn the_week_start_is_never_in_the_future() {
        let now = Utc::now();
        let start = week_start(now);
        assert!(start <= now && now - start < Duration::days(7));
    }

    #[test]
    fn ranking_orders_by_count_and_shares_ranks_on_ties() {
        let ids: Vec<Uuid> = {
            let mut ids: Vec<Uuid> = (0..4).map(|_| Uuid::now_v7()).collect();
            ids.sort_unstable();
            ids
        };
        let ranked = rank(vec![(ids[0], 2), (ids[1], 5), (ids[2], 5), (ids[3], 1)], 20);
        assert_eq!(ranked, vec![(ids[1], 5, 1), (ids[2], 5, 1), (ids[0], 2, 3), (ids[3], 1, 4)]);
    }

    #[test]
    fn ranking_keeps_the_top_and_handles_nobody() {
        let ids: Vec<Uuid> = (0..5).map(|_| Uuid::now_v7()).collect();
        let counts: Vec<(Uuid, i64)> = ids.iter().enumerate().map(|(i, id)| (*id, 10 - i as i64)).collect();
        let ranked = rank(counts, 3);
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked.iter().map(|(_, count, _)| *count).collect::<Vec<_>>(), vec![10, 9, 8]);
        assert!(rank(Vec::new(), 3).is_empty());
    }

    #[test]
    fn a_flame_shows_its_question_and_never_a_sender() {
        let row = FlameRow {
            id: new_id(),
            question_id: new_id(),
            created_at: now(),
            seen_at: None,
            text_ja: "いつも笑顔な人は?".into(),
            text_en: "Who is always smiling?".into(),
            emoji: "😊".into(),
        };
        let flame = row.to_proto(Vec::new(), 30);
        assert!(!flame.seen && flame.hints.is_empty());
        assert_eq!(flame.next_hint_price, 30);
        // Everything the receiver gets, as it goes over the wire.
        let json = serde_json::to_value(&flame).unwrap();
        let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        for key in &keys {
            assert!(
                ["id", "question", "createdAt", "nextHintPrice"].contains(key),
                "unexpected field {key} on a flame"
            );
        }
    }
}
