//! The hot counters in Valkey.
//!
//! One consumer handles every domain's traffic, so an event must be cheap:
//! a deed costs one script call here and no PostgreSQL at all. What it
//! leaves behind is written to PostgreSQL a few seconds later, in one
//! transaction per player (`flush.rs`).
//!
//! | Key (after `quest:<namespace>:`) | Type, TTL | Purpose |
//! |---|---|---|
//! | `day:<user>:<date>` | hash, 3 days | A player's local day so far: `x:<source>` XP credited (never above the source's cap), `m:<metric>` counts, `h` bit mask of the local hours with activity, `v` a version that grows with every change |
//! | `seen:<id>` | marker, 2 days | A deed that was applied (event ID + rule + player): the same one is never applied twice |
//! | `dirty` | sorted set | `<user>|<date>` of the days with changes not yet in PostgreSQL, scored by when to write them |
//! | `flush:<user>` | lock, 30 s | One process writes a player's days at a time |
//! | `tw:<user>:<friend>:<date>` | marker, 3 days / 10 min | `1`: the player's part of that streak day towards that friend is recorded; `0`: the two are not friends (asked again later) |
//! | `drawn:<user>:<date>` | marker, 2 days | The day's missions exist |
//!
//! The values of a day hash are absolute, and PostgreSQL only ever takes
//! the difference to what it already holds, so writing a day twice (a
//! crash after the write, before the queue entry is removed) changes
//! nothing. A day whose hash is missing is loaded from PostgreSQL before
//! anything is added to it, so losing Valkey loses at most the few seconds
//! that were not written yet.
use std::collections::BTreeMap;

use chrono::NaiveDate;
use redis::AsyncCommands as _;
use tokuly_core::prelude::*;

use crate::{Quest, clock, metrics::Metric, rules::Source};

/// How long a day's counters are kept: past the last moment an event of
/// that day is still accepted.
const DAY_TTL_SECONDS: i64 = 3 * 24 * 3600;
/// How long an applied deed is remembered. Events older than this are not
/// counted at all (see `ingest.rs`), so nothing can be applied twice.
pub(crate) const SEEN_TTL_SECONDS: i64 = 48 * 3600;
const LOCK_TTL_MS: u64 = 30_000;

/// Adds one deed to a day. Returns `{status, xp credited}`: -1 the day is
/// not loaded, 0 the deed was applied before, 1 applied.
const APPLY: &str = r"
if redis.call('EXISTS', KEYS[2]) == 0 then return {-1, 0} end
if not redis.call('SET', KEYS[1], 1, 'NX', 'EX', ARGV[1]) then return {0, 0} end
local credited = 0
local xp = tonumber(ARGV[4])
if xp > 0 then
  local cap = tonumber(ARGV[5])
  if cap > 0 then
    local have = tonumber(redis.call('HGET', KEYS[2], ARGV[3]) or '0')
    if have + xp > cap then xp = math.max(cap - have, 0) end
  end
  if xp > 0 then
    redis.call('HINCRBY', KEYS[2], ARGV[3], xp)
    redis.call('HINCRBY', KEYS[2], 'm:xp', xp)
    credited = xp
  end
end
for i = 9, #ARGV, 2 do redis.call('HINCRBY', KEYS[2], ARGV[i], ARGV[i + 1]) end
redis.call('HSET', KEYS[2], 'm:active_days', 1)
if tonumber(ARGV[8]) >= 0 then
  local hours = tonumber(redis.call('HGET', KEYS[2], 'h') or '0')
  local bit = 2 ^ tonumber(ARGV[8])
  if math.floor(hours / bit) % 2 == 0 then redis.call('HSET', KEYS[2], 'h', string.format('%d', hours + bit)) end
end
redis.call('HINCRBY', KEYS[2], 'v', 1)
redis.call('EXPIRE', KEYS[2], ARGV[2])
redis.call('ZADD', KEYS[3], 'NX', ARGV[7], ARGV[6])
return {1, credited}
";

/// Loads a day from PostgreSQL unless it is there already.
const SEED: &str = r"
if redis.call('EXISTS', KEYS[1]) == 1 then return 0 end
redis.call('HSET', KEYS[1], 'v', 0)
for i = 2, #ARGV, 2 do redis.call('HSET', KEYS[1], ARGV[i], ARGV[i + 1]) end
redis.call('EXPIRE', KEYS[1], ARGV[1])
return 1
";

/// After a day was written: takes it off the queue if nothing changed
/// since it was read, otherwise queues it again.
const WRITTEN: &str = r"
if (redis.call('HGET', KEYS[1], 'v') or '') == ARGV[2] then return redis.call('ZREM', KEYS[2], ARGV[1]) end
redis.call('ZADD', KEYS[2], ARGV[3], ARGV[1])
return 0
";

const RELEASE: &str =
    "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end";

pub(crate) struct Scripts {
    apply: redis::Script,
    seed: redis::Script,
    written: redis::Script,
    release: redis::Script,
}

impl Scripts {
    pub(crate) fn new() -> Self {
        Self {
            apply: redis::Script::new(APPLY),
            seed: redis::Script::new(SEED),
            written: redis::Script::new(WRITTEN),
            release: redis::Script::new(RELEASE),
        }
    }
}

/// Something to add to a player's day.
pub(crate) struct Credit<'a> {
    pub user: Uuid,
    pub day: NaiveDate,
    /// Local hour of the deed (0-23); `None` when the deed is no activity
    /// at an hour of the day (`Rule::timed`).
    pub hour: Option<u32>,
    /// Identifies the deed; a second credit with the same ID does nothing.
    pub dedupe: &'a str,
    pub source: Source,
    /// XP before the cap.
    pub xp: i64,
    pub counts: &'a [(Metric, i64)],
}

/// A day's counters as read from Valkey.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct DayCounters {
    pub xp: BTreeMap<Source, i64>,
    pub metrics: BTreeMap<Metric, i64>,
    /// Bit `h` set = something was done during local hour `h`.
    pub hours: u32,
    pub version: String,
}

impl DayCounters {
    pub(crate) fn from_fields(fields: Vec<(String, String)>) -> Self {
        let mut day = Self::default();
        for (field, value) in fields {
            if field == "v" {
                day.version = value;
                continue;
            }
            let Ok(number) = value.parse::<i64>() else { continue };
            if field == "h" {
                day.hours = number as u32;
            } else if let Some(source) = field.strip_prefix("x:").and_then(Source::from_key) {
                day.xp.insert(source, number);
            } else if let Some(metric) = field.strip_prefix("m:").and_then(Metric::from_key) {
                day.metrics.insert(metric, number);
            }
        }
        day
    }

    /// Whether anything was done between two local hours (`from` inclusive, `to` exclusive).
    pub(crate) fn active_between(&self, from: u32, to: u32) -> bool {
        (from..to.min(24)).any(|hour| self.hours & (1 << hour) != 0)
    }
}

/// A queue entry: one player's local day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DirtyDay {
    pub user: Uuid,
    pub day: NaiveDate,
}

impl DirtyDay {
    fn member(&self) -> String {
        format!("{}|{}", self.user, clock::date_text(self.day))
    }

    fn parse(member: &str) -> Option<Self> {
        let (user, day) = member.split_once('|')?;
        Some(Self { user: Uuid::parse_str(user).ok()?, day: clock::parse_date(day)? })
    }
}

/// The lock on a player's days while they are written.
pub(crate) struct FlushLock {
    key: String,
    token: String,
}

impl Quest {
    pub(crate) fn key(&self, rest: std::fmt::Arguments<'_>) -> String {
        format!("{}{rest}", self.keyspace)
    }

    fn day_key(&self, user: Uuid, day: NaiveDate) -> String {
        self.key(format_args!("day:{user}:{}", clock::date_text(day)))
    }

    fn dirty_key(&self) -> String {
        self.key(format_args!("dirty"))
    }

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    /// Adds a deed to the player's day. Returns the XP credited (after the
    /// cap), or `None` when the same deed was credited before.
    pub(crate) async fn credit(&self, credit: &Credit<'_>) -> AppResult<Option<i64>> {
        let day_key = self.day_key(credit.user, credit.day);
        let member = DirtyDay { user: credit.user, day: credit.day }.member();
        let due = Self::now_ms() + self.cfg.flush_delay_ms as i64;
        let def = credit.source.def();
        let xp_field = format!("x:{}", def.key);
        for attempt in 0..3 {
            let mut invocation = self.scripts.apply.prepare_invoke();
            invocation
                .key(self.key(format_args!("seen:{}", credit.dedupe)))
                .key(&day_key)
                .key(self.dirty_key())
                .arg(SEEN_TTL_SECONDS)
                .arg(DAY_TTL_SECONDS)
                .arg(&xp_field)
                .arg(credit.xp.max(0))
                .arg(def.daily_cap)
                .arg(&member)
                .arg(due)
                .arg(credit.hour.map_or(-1, |hour| i64::from(hour.min(23))));
            for (metric, amount) in credit.counts {
                invocation.arg(format!("m:{}", metric.key())).arg(*amount);
            }
            let (status, credited): (i64, i64) = invocation.invoke_async(&mut self.core.cache.conn()).await?;
            match status {
                -1 if attempt < 2 => self.load_day(credit.user, credit.day).await?,
                -1 => break,
                0 => return Ok(None),
                _ => return Ok(Some(credited)),
            }
        }
        Err(AppError::unavailable("QUEST_COUNTERS_UNAVAILABLE", "the day's counters could not be loaded"))
    }

    /// Puts what PostgreSQL knows of a day into Valkey, unless the day is
    /// there already (somebody else loaded it, or it never left).
    async fn load_day(&self, user: Uuid, day: NaiveDate) -> AppResult<()> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT 'x:' || source, xp::bigint FROM quest_xp_days WHERE user_id = $1 AND day = $2
             UNION ALL
             SELECT 'm:' || metric, value FROM quest_metric_days WHERE user_id = $1 AND day = $2",
        )
        .bind(user)
        .bind(day)
        .fetch_all(&self.core.db)
        .await?;
        let mut invocation = self.scripts.seed.prepare_invoke();
        invocation.key(self.day_key(user, day)).arg(DAY_TTL_SECONDS);
        for (field, value) in rows {
            invocation.arg(field).arg(value);
        }
        let _: i64 = invocation.invoke_async(&mut self.core.cache.conn()).await?;
        Ok(())
    }

    /// The counters of a day as they stand in Valkey.
    pub(crate) async fn read_day(&self, entry: DirtyDay) -> AppResult<DayCounters> {
        let fields: Vec<(String, String)> = self.core.cache.conn().hgetall(self.day_key(entry.user, entry.day)).await?;
        Ok(DayCounters::from_fields(fields))
    }

    /// Days waiting to be written whose time has come (all of them with `everything`).
    pub(crate) async fn due_days(&self, everything: bool, limit: isize) -> AppResult<Vec<DirtyDay>> {
        let max = if everything { "+inf".to_owned() } else { Self::now_ms().to_string() };
        let members: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(self.dirty_key())
            .arg("-inf")
            .arg(max)
            .arg("LIMIT")
            .arg(0)
            .arg(limit)
            .query_async(&mut self.core.cache.conn())
            .await?;
        Ok(members.iter().filter_map(|member| DirtyDay::parse(member)).collect())
    }

    /// The days of one player that wait to be written.
    pub(crate) async fn dirty_days_of(&self, user: Uuid) -> AppResult<Vec<DirtyDay>> {
        // A player's unwritten days lie around "now" in some time zone: one
        // round trip asks about all of them.
        let today = chrono::Utc::now().date_naive();
        let candidates: Vec<DirtyDay> =
            (-3..=2).map(|offset| DirtyDay { user, day: today + chrono::Duration::days(offset) }).collect();
        let dirty_key = self.dirty_key();
        let mut pipe = redis::pipe();
        for entry in &candidates {
            pipe.zscore(&dirty_key, entry.member());
        }
        let scores: Vec<Option<f64>> = pipe.query_async(&mut self.core.cache.conn()).await?;
        Ok(candidates.into_iter().zip(scores).filter_map(|(entry, score)| score.map(|_| entry)).collect())
    }

    /// Takes a day off the queue once it is written, unless it changed
    /// since `version` was read: then it stays, due again after the delay.
    pub(crate) async fn mark_written(&self, entry: DirtyDay, version: &str) -> AppResult<()> {
        let _: i64 = self
            .scripts
            .written
            .prepare_invoke()
            .key(self.day_key(entry.user, entry.day))
            .key(self.dirty_key())
            .arg(entry.member())
            .arg(version)
            .arg(Self::now_ms() + self.cfg.flush_delay_ms as i64)
            .invoke_async(&mut self.core.cache.conn())
            .await?;
        Ok(())
    }

    /// Drops a queue entry that can never be written (the account is gone).
    pub(crate) async fn discard_day(&self, entry: DirtyDay) -> AppResult<()> {
        let mut conn = self.core.cache.conn();
        conn.zrem::<_, _, ()>(self.dirty_key(), entry.member()).await?;
        conn.del::<_, ()>(self.day_key(entry.user, entry.day)).await?;
        Ok(())
    }

    /// Takes the lock on a player's days; `None` when another process holds it.
    pub(crate) async fn try_lock_flush(&self, user: Uuid) -> AppResult<Option<FlushLock>> {
        let key = self.key(format_args!("flush:{user}"));
        let token = Uuid::now_v7().to_string();
        let taken: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg(&token)
            .arg("NX")
            .arg("PX")
            .arg(LOCK_TTL_MS)
            .query_async(&mut self.core.cache.conn())
            .await?;
        Ok(taken.map(|_| FlushLock { key, token }))
    }

    pub(crate) async fn unlock_flush(&self, lock: FlushLock) {
        let released: Result<i64, _> = self
            .scripts
            .release
            .prepare_invoke()
            .key(&lock.key)
            .arg(&lock.token)
            .invoke_async(&mut self.core.cache.conn())
            .await;
        if let Err(e) = released {
            tracing::warn!(error = %e, "quest: flush lock not released; it expires by itself");
        }
    }

    /// What is known about the player's part of a streak day towards a
    /// friend: `Some(true)` recorded, `Some(false)` the two are not friends.
    pub(crate) async fn towards_marker(&self, user: Uuid, other: Uuid, day: NaiveDate) -> AppResult<Option<bool>> {
        let value: Option<String> = self.core.cache.get_string(&self.towards_key(user, other, day)).await?;
        Ok(value.map(|value| value == "1"))
    }

    /// Remembers the answer. Returns whether this call was the first to
    /// record the part (the caller then counts the friend as reached).
    pub(crate) async fn mark_towards(&self, user: Uuid, other: Uuid, day: NaiveDate, friends: bool) -> AppResult<bool> {
        let key = self.towards_key(user, other, day);
        if friends {
            // Overwrites a stale "not friends" answer; only a fresh marker counts as first.
            let previous: Option<String> = redis::cmd("SET")
                .arg(&key)
                .arg("1")
                .arg("EX")
                .arg(DAY_TTL_SECONDS)
                .arg("GET")
                .query_async(&mut self.core.cache.conn())
                .await?;
            Ok(previous.as_deref() != Some("1"))
        } else {
            self.core.cache.set_string(&key, "0", Some(std::time::Duration::from_secs(600))).await?;
            Ok(false)
        }
    }

    /// Forgets what was recorded between two players for the days around
    /// today (their friendship ended; should it resume, it starts clean).
    pub(crate) async fn forget_towards(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        let today = chrono::Utc::now().date_naive();
        let mut conn = self.core.cache.conn();
        for offset in -2..=2 {
            let day = today + chrono::Duration::days(offset);
            conn.del::<_, ()>(&[self.towards_key(a, b, day), self.towards_key(b, a, day)]).await?;
        }
        Ok(())
    }

    fn towards_key(&self, user: Uuid, other: Uuid, day: NaiveDate) -> String {
        self.key(format_args!("tw:{user}:{other}:{}", clock::date_text(day)))
    }

    /// Returns true the first time it is asked about a player's day: the
    /// caller then makes sure the day's missions exist.
    pub(crate) async fn first_sight_of_day(&self, user: Uuid, day: NaiveDate) -> AppResult<bool> {
        let key = self.key(format_args!("drawn:{user}:{}", clock::date_text(day)));
        self.core.cache.set_nx(&key, "1", std::time::Duration::from_secs(2 * 24 * 3600)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_fields_are_sorted_into_xp_metrics_hours_and_version() {
        let day = DayCounters::from_fields(vec![
            ("v".into(), "7".into()),
            ("x:chat".into(), "12".into()),
            ("m:messages".into(), "6".into()),
            ("m:xp".into(), "12".into()),
            ("h".into(), "1048608".into()),
            ("x:retired_source".into(), "5".into()),
            ("m:messages_bogus".into(), "x".into()),
        ]);
        assert_eq!(day.version, "7");
        assert_eq!(day.xp, BTreeMap::from([(Source::Chat, 12)]));
        assert_eq!(day.metrics, BTreeMap::from([(Metric::Messages, 6), (Metric::Xp, 12)]));
        // Bits 5 and 20: active at 05:xx and 20:xx.
        assert!(day.active_between(5, 7) && day.active_between(20, 21));
        assert!(!day.active_between(6, 20) && !day.active_between(21, 24));
    }

    #[test]
    fn queue_members_round_trip() {
        let entry = DirtyDay { user: Uuid::now_v7(), day: NaiveDate::from_ymd_opt(2026, 11, 22).unwrap() };
        assert_eq!(DirtyDay::parse(&entry.member()), Some(entry));
        assert_eq!(DirtyDay::parse("garbage"), None);
        assert_eq!(DirtyDay::parse("not-a-uuid|2026-11-22"), None);
    }
}
