//! Use of emoji: what each user used recently, and the counters behind
//! "popular".
//!
//! A use is reported through `RecordUse` or [`crate::catalog::record_use`].
//! It does two things:
//!
//! - the user's "recently used" list is updated in the database right away
//!   (one statement), so the picker shows it at once;
//! - the emoji's use counter is incremented in Valkey. The usage worker
//!   ([`flush`]) moves the counters into `emoji_usage_daily` and
//!   `emoji_stats` in batches and recomputes the popularity scores
//!   ([`refresh_popularity`]), so counting costs no database write per use.
//!
//! Two rules keep the numbers about the crowd and not about a person. One
//! user counts at most [`DAILY_USE_CAP`] times per emoji and day, so
//! "popular" cannot be decided by a single account. And an emoji has no
//! popularity until [`POPULAR_MIN_USERS`] different people used it on one
//! day: emoji are used in private chats, and a list that moved when one
//! person used one would tell the others what that person does there. For
//! the same reason the exact use count is shown to moderators only.
use std::{collections::HashMap, sync::LazyLock};

use chrono::{Duration as ChronoDuration, Utc};
use tokuly_core::prelude::*;

use crate::{
    catalog::{self, Keys, Snapshot},
    model::shortcode,
    validate::{self, RECENTS_KEPT},
};

/// The time zone statistics days are counted in. Tokuly is a Japanese
/// product: "today" ends at midnight in Japan.
pub const STATS_TIME_ZONE: &str = "Asia/Tokyo";
/// Japan has no daylight saving time, so its day is UTC shifted by this.
const STATS_UTC_OFFSET_HOURS: i64 = 9;

/// Uses of one emoji by one user that count towards "popular" per day.
pub const DAILY_USE_CAP: u32 = 20;
/// Different people who must have used an emoji on one day before its use
/// shows in "popular". Below that, a number would not describe the crowd
/// but what one or two people do — in chats nobody else can see.
pub const POPULAR_MIN_USERS: i64 = 3;
/// How much a day of age takes off a use's weight in the popularity score.
const DAILY_DECAY: f64 = 0.85;
/// Days of daily counters kept.
const DAILY_RETENTION_DAYS: i32 = 400;

/// An emoji as it appears in decks, recently used lists and use reports.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Item {
    /// A custom emoji, by its (lowercase) name.
    Custom(String),
    /// One Unicode emoji.
    Unicode(String),
}

impl Item {
    /// Reads `:name:` (the colons may be missing) or a Unicode emoji.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if validate::is_unicode_emoji(text) {
            return Some(Self::Unicode(text.to_owned()));
        }
        validate::normalize_name(text).map(Self::Custom)
    }

    /// The form kept in decks and recently used lists, and shown in text:
    /// `:name:` or the emoji itself.
    pub fn text(&self) -> String {
        match self {
            Self::Custom(name) => shortcode(name),
            Self::Unicode(emoji) => emoji.clone(),
        }
    }
}

/// Counts uses against the user's daily allowance and adds the ones within
/// it to the counters waiting for the worker. The waiting counters are one
/// hash with two fields per emoji and day: `u:<day>:<emoji>` (uses) and
/// `p:<day>:<emoji>` (people: users for whom it was the first use that day).
static COUNT: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(
        "for i = 4, #ARGV do
             local mine = redis.call('HINCRBY', KEYS[1], ARGV[i], 1)
             if mine <= tonumber(ARGV[1]) then
                 redis.call('HINCRBY', KEYS[2], 'u:' .. ARGV[3] .. ':' .. ARGV[i], 1)
             end
             if mine == 1 then
                 redis.call('HINCRBY', KEYS[2], 'p:' .. ARGV[3] .. ':' .. ARGV[i], 1)
             end
         end
         redis.call('EXPIRE', KEYS[1], ARGV[2])
         return 1",
    )
});

/// Hands the worker a batch of counters: the batch it was interrupted on
/// if there is one, otherwise everything counted since the last batch. The
/// batch gets an ID so that it is applied to the database exactly once.
static TAKE_BATCH: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(
        "if redis.call('EXISTS', KEYS[2]) == 0 then
             if redis.call('EXISTS', KEYS[1]) == 0 then return {} end
             redis.call('RENAME', KEYS[1], KEYS[2])
             redis.call('HSET', KEYS[2], '_batch', ARGV[1])
         end
         return redis.call('HGETALL', KEYS[2])",
    )
});

/// Drops a batch once it is in the database, unless the key already holds
/// a newer one.
static DROP_BATCH: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(
        "if redis.call('HGET', KEYS[1], '_batch') == ARGV[1] then return redis.call('DEL', KEYS[1]) end
         return 0",
    )
});

/// The counters of one emoji on one day, as a batch carries them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Counter {
    emoji_id: Uuid,
    /// `YYYYMMDD`, in Japan.
    day: String,
    uses: i64,
    /// Users for whom a use in this batch was their first of the day.
    people: i64,
}

/// Reads a batch (see [`COUNT`] for its fields). Fields that are no
/// counters — the batch ID, anything malformed — are skipped.
fn parse_batch(batch: &HashMap<String, String>) -> Vec<Counter> {
    let mut counters: HashMap<(Uuid, &str), Counter> = HashMap::new();
    for (field, value) in batch {
        let mut parts = field.splitn(3, ':');
        let (Some(kind), Some(day), Some(emoji_id)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let (Ok(emoji_id), Ok(count)) = (Uuid::parse_str(emoji_id), value.parse::<i64>()) else { continue };
        if day.len() != 8 || !day.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let counter = counters.entry((emoji_id, day)).or_insert_with(|| Counter {
            emoji_id,
            day: day.to_owned(),
            uses: 0,
            people: 0,
        });
        match kind {
            "u" => counter.uses += count,
            "p" => counter.people += count,
            _ => {}
        }
    }
    let mut counters: Vec<Counter> = counters.into_values().collect();
    counters.sort_by(|a, b| (a.emoji_id, &a.day).cmp(&(b.emoji_id, &b.day)));
    counters
}

/// Today's date in Japan, as `YYYYMMDD`.
fn stats_day() -> String {
    (Utc::now() + ChronoDuration::hours(STATS_UTC_OFFSET_HOURS)).format("%Y%m%d").to_string()
}

/// Records one use of each item by `user_id`. Custom emoji that are not in
/// the catalogue are ignored; an item listed twice counts once.
pub(crate) async fn record(core: &Core, snapshot: &Snapshot, user_id: Uuid, items: &[Item]) -> AppResult<()> {
    let mut used: Vec<String> = Vec::with_capacity(items.len());
    let mut counted: Vec<&str> = Vec::new();
    for item in items {
        let text = match item {
            Item::Custom(name) => {
                let Some(emoji) = snapshot.get(name) else { continue };
                if !counted.contains(&emoji.id.as_str()) {
                    counted.push(&emoji.id);
                }
                shortcode(&emoji.name)
            }
            Item::Unicode(emoji) => emoji.clone(),
        };
        if !used.contains(&text) {
            used.push(text);
        }
    }
    if used.is_empty() {
        return Ok(());
    }
    remember_recent(core, user_id, &used).await?;
    if !counted.is_empty() {
        let keys = Keys::of(core).await?;
        let day = stats_day();
        let mut count = COUNT.prepare_invoke();
        count.key(keys.user_uses(&day, user_id)).key(keys.uses()).arg(DAILY_USE_CAP).arg(2 * 24 * 3600).arg(&day);
        for emoji_id in counted {
            count.arg(emoji_id);
        }
        count.invoke_async::<i64>(&mut core.cache.conn()).await?;
    }
    Ok(())
}

/// Moves `items` to the top of the user's recently used list (the last one
/// listed ends up first) and keeps the list at [`RECENTS_KEPT`] entries.
async fn remember_recent(core: &Core, user_id: Uuid, items: &[String]) -> AppResult<()> {
    let grew: bool = sqlx::query_scalar(
        "WITH used AS (
             INSERT INTO emoji_recents (user_id, item, used_at)
             SELECT $1, item, now() + ord * interval '1 microsecond'
             FROM unnest($2::text[]) WITH ORDINALITY AS listed (item, ord)
             ON CONFLICT (user_id, item) DO UPDATE
                 SET used_at = EXCLUDED.used_at, use_count = emoji_recents.use_count + 1
             RETURNING (xmax = 0) AS first_use
         )
         SELECT COALESCE(bool_or(first_use), false) FROM used",
    )
    .bind(user_id)
    .bind(items)
    .fetch_one(&core.db)
    .await?;
    // The list only grows when something is used for the first time, so that
    // is the only moment it may need trimming.
    if grew {
        sqlx::query(
            "DELETE FROM emoji_recents WHERE user_id = $1 AND item IN (
                 SELECT item FROM emoji_recents WHERE user_id = $1 ORDER BY used_at DESC OFFSET $2)",
        )
        .bind(user_id)
        .bind(RECENTS_KEPT)
        .execute(&core.db)
        .await?;
    }
    Ok(())
}

/// The user's recently used emoji in stored form, most recent first.
pub(crate) async fn recent(core: &Core, user_id: Uuid, limit: i64) -> AppResult<Vec<String>> {
    Ok(sqlx::query_scalar("SELECT item FROM emoji_recents WHERE user_id = $1 ORDER BY used_at DESC, item LIMIT $2")
        .bind(user_id)
        .bind(limit)
        .fetch_all(&core.db)
        .await?)
}

pub(crate) async fn clear_recent(core: &Core, user_id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM emoji_recents WHERE user_id = $1").bind(user_id).execute(&core.db).await?;
    Ok(())
}

/// Moves the counters collected in Valkey into the database and returns
/// how many uses that were. Safe to run from several workers at once, and
/// to interrupt at any point: a batch is written exactly once.
pub async fn flush(core: &Core) -> AppResult<u64> {
    let keys = Keys::of(core).await?;
    let mut conn = core.cache.conn();
    let batch: HashMap<String, String> =
        TAKE_BATCH.key(keys.uses()).key(keys.uses_draining()).arg(new_id().to_string()).invoke_async(&mut conn).await?;
    let Some(batch_id) = batch.get("_batch") else { return Ok(0) };
    let counters = parse_batch(&batch);
    let emoji_ids: Vec<Uuid> = counters.iter().map(|counter| counter.emoji_id).collect();
    let days: Vec<&str> = counters.iter().map(|counter| counter.day.as_str()).collect();
    let uses: Vec<i64> = counters.iter().map(|counter| counter.uses).collect();
    let people: Vec<i64> = counters.iter().map(|counter| counter.people).collect();

    let mut tx = core.db.begin().await?;
    let first_time = sqlx::query("INSERT INTO emoji_usage_batches (batch_id) VALUES ($1::uuid) ON CONFLICT DO NOTHING")
        .bind(batch_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 1;
    let mut written = 0i64;
    if first_time {
        // Emoji deleted since they were used are dropped from the batch;
        // the key lock keeps the others from disappearing under the rows
        // written here.
        written = sqlx::query_scalar(
            "WITH counted AS (
                 SELECT e.id AS emoji_id, to_date(used.day, 'YYYYMMDD') AS day, used.uses, used.people
                 FROM unnest($1::uuid[], $2::text[], $3::bigint[], $4::bigint[]) AS used (emoji_id, day, uses, people)
                 JOIN emojis e ON e.id = used.emoji_id
                 FOR KEY SHARE OF e
             ), daily AS (
                 INSERT INTO emoji_usage_daily (emoji_id, day, uses, users)
                 SELECT emoji_id, day, uses, people FROM counted
                 ON CONFLICT (emoji_id, day) DO UPDATE
                     SET uses = emoji_usage_daily.uses + EXCLUDED.uses, users = emoji_usage_daily.users + EXCLUDED.users
             ), total AS (
                 INSERT INTO emoji_stats (emoji_id, use_count)
                 SELECT emoji_id, sum(uses) FROM counted GROUP BY emoji_id
                 ON CONFLICT (emoji_id) DO UPDATE SET use_count = emoji_stats.use_count + EXCLUDED.use_count
             )
             SELECT COALESCE(sum(uses), 0)::bigint FROM counted",
        )
        .bind(&emoji_ids)
        .bind(&days)
        .bind(&uses)
        .bind(&people)
        .fetch_one(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    DROP_BATCH.key(keys.uses_draining()).arg(batch_id).invoke_async::<i64>(&mut conn).await?;
    Ok(written as u64)
}

/// Recomputes every emoji's popularity: its uses of the last `days` days,
/// each day weighing [`DAILY_DECAY`] of the day after it — provided that on
/// one of those days at least [`POPULAR_MIN_USERS`] different people used
/// it. An emoji only a person or two use has no popularity at all.
pub async fn refresh_popularity(core: &Core, days: i32) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    // One recomputation at a time: every worker process runs this on the
    // same schedule, and two of them rewriting the same rows could wait on
    // each other. Whoever comes second has nothing to add.
    let alone: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended('emoji-popularity', 0))")
        .fetch_one(&mut *tx)
        .await?;
    if !alone {
        return Ok(());
    }
    sqlx::query(
        "WITH today AS (
             SELECT (now() AT TIME ZONE $1)::date AS day
         ), recent AS (
             SELECT d.emoji_id, sum(d.uses * power($2::float8, (today.day - d.day)::float8)) AS score
             FROM emoji_usage_daily d, today
             WHERE d.day > today.day - $3::int AND d.day <= today.day
             GROUP BY d.emoji_id
             HAVING max(d.users) >= $4
         ), scored AS (
             INSERT INTO emoji_stats (emoji_id, popularity)
             SELECT emoji_id, score FROM recent
             ON CONFLICT (emoji_id) DO UPDATE SET popularity = EXCLUDED.popularity
         )
         UPDATE emoji_stats s SET popularity = 0
         WHERE s.popularity > 0 AND NOT EXISTS (SELECT 1 FROM recent r WHERE r.emoji_id = s.emoji_id)",
    )
    .bind(STATS_TIME_ZONE)
    .bind(DAILY_DECAY)
    .bind(days)
    .bind(POPULAR_MIN_USERS)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    catalog::forget_popularity(&Keys::of(core).await?);
    Ok(())
}

/// Deletes daily counters past their retention and the batch markers that
/// are no longer needed. Returns the rows deleted.
pub(crate) async fn prune(core: &Core) -> AppResult<u64> {
    let daily = sqlx::query("DELETE FROM emoji_usage_daily WHERE day < (now() AT TIME ZONE $1)::date - $2::int")
        .bind(STATS_TIME_ZONE)
        .bind(DAILY_RETENTION_DAYS)
        .execute(&core.db)
        .await?
        .rows_affected();
    // A batch is retried within minutes; a day is a wide margin.
    let batches = sqlx::query("DELETE FROM emoji_usage_batches WHERE applied_at < now() - interval '1 day'")
        .execute(&core.db)
        .await?
        .rows_affected();
    Ok(daily + batches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_are_custom_names_or_unicode_emoji() {
        assert_eq!(Item::parse(":BlobCat:"), Some(Item::Custom("blobcat".into())));
        assert_eq!(Item::parse("blobcat"), Some(Item::Custom("blobcat".into())));
        assert_eq!(Item::parse(" 👍 "), Some(Item::Unicode("👍".into())));
        assert_eq!(Item::parse("❤️"), Some(Item::Unicode("❤️".into())));
        for junk in ["", ":", "::", "a", "hello world", "<script>", "👍👍"] {
            assert_eq!(Item::parse(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn items_are_written_as_they_appear_in_text() {
        assert_eq!(Item::Custom("blobcat".into()).text(), ":blobcat:");
        assert_eq!(Item::Unicode("🎉".into()).text(), "🎉");
        // The stored form reads back to the same item.
        for item in [Item::Custom("neko_2".into()), Item::Unicode("👨‍👩‍👧".into())] {
            assert_eq!(Item::parse(&item.text()), Some(item));
        }
    }

    #[test]
    fn a_batch_is_read_into_counters_per_emoji_and_day() {
        let (cat, dog) = (Uuid::now_v7(), Uuid::now_v7());
        let batch: HashMap<String, String> = [
            (format!("u:20261122:{cat}"), "7"),
            (format!("p:20261122:{cat}"), "3"),
            (format!("u:20261123:{cat}"), "1"),
            (format!("p:20261122:{dog}"), "1"),
            ("_batch".to_owned(), "0190a000-0000-7000-8000-000000000001"),
            (format!("x:20261122:{cat}"), "9"),
            (format!("u:today:{cat}"), "9"),
            ("u:20261122:not-an-id".to_owned(), "9"),
            (format!("u:20261122:{dog}:extra"), "9"),
            (format!("p:20261123:{dog}"), "many"),
        ]
        .into_iter()
        .map(|(field, value)| (field, value.to_owned()))
        .collect();
        let mut expected = vec![
            Counter { emoji_id: cat, day: "20261122".into(), uses: 7, people: 3 },
            Counter { emoji_id: cat, day: "20261123".into(), uses: 1, people: 0 },
            Counter { emoji_id: dog, day: "20261122".into(), uses: 0, people: 1 },
        ];
        expected.sort_by(|a, b| (a.emoji_id, &a.day).cmp(&(b.emoji_id, &b.day)));
        assert_eq!(parse_batch(&batch), expected);
        assert!(parse_batch(&HashMap::new()).is_empty());
    }

    #[test]
    fn the_statistics_day_is_a_date() {
        let day = stats_day();
        assert_eq!(day.len(), 8);
        assert!(day.bytes().all(|b| b.is_ascii_digit()));
    }
}
