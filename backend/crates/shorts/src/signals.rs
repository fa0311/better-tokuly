//! Signals: what comes back from the swipe feed, and where it goes.
//!
//! A client reports one *stay* per short each time the viewer leaves it,
//! in batches ([`Shorts::record_playback`]). Every stay marks the short as
//! **seen** by the viewer. What a stay adds to the short's numbers is
//! bounded per viewer:
//!
//! - one **view** per viewer, short and dedupe window, for a stay in which
//!   enough of the short was played;
//! - one **sample** per viewer, short and dedupe window, the first stay:
//!   its watch-through ratio and its loops (replays past the end) are what
//!   the ranking and the creator's analytics take from that viewer.
//!
//! Likes, comments, saves and shares follow the same rule, with a week as
//! the period ([`Shorts::count_signal`]): **every signal counts at most
//! once per account, short and period**. Reporting more, or liking and
//! unliking in a loop, moves nothing; the weight of one account in a
//! short's ranking is that of one viewer.
//!
//! Nothing here writes to PostgreSQL per stay. Playback counters go into a
//! Valkey hash per short, and the flush worker ([`Shorts::flush_signals`])
//! moves what accumulated into `shorts` and `short_hourly_stats` every few
//! seconds, one statement for hundreds of shorts. The other signals are
//! discrete actions that already write a row; they add to the hourly
//! statistics directly ([`bump_hourly`]).
//!
//! | Valkey key | Type | Purpose |
//! |---|---|---|
//! | `shorts:pending` | set | shorts with buffered counters |
//! | `shorts:pending:<short>` | hash `views`, `loops`, `samples`, `ratio_milli` | the buffered counters |
//! | `shorts:viewed:<short>:<viewer>` | marker, TTL `view_dedupe_seconds` | this viewer's view was counted |
//! | `shorts:stayed:<short>:<viewer>` | marker, TTL `view_dedupe_seconds` | this viewer's sample was taken |
//! | `shorts:once:<signal>:<short>:<account>` | marker, TTL 7 days | this account's like / comment / save / share was counted |
//! | `shorts:seen:<user>` | sorted set by time, capped | shorts the user has played |
//! | `shorts:lastwatch:<user>:<channel>` | short ID, TTL 1 h | the short to credit if the user now follows the channel |
//!
//! Tests prefix the pending set and hashes with the test-run ID because the
//! test database is shared with other processes; a flush must not take their
//! buffered counters.
use std::{collections::HashSet, time::Duration};

use sqlx::PgExecutor;
use tokuly_core::{analytics::AnalyticsEvent, outbound, prelude::*};
use tokuly_proto::{common::SubjectType, shorts::PlaybackSource};

use crate::{
    Shorts,
    access::{self, Viewer},
    shorts::{self, ShortRow},
    text,
};

/// Longest stay taken at face value: an hour on one short is a phone left on.
const MAX_STAY_MS: i64 = 3_600_000;
/// Most full plays counted for one stay: ten loops say as much about a
/// short as a hundred.
const MAX_COMPLETIONS: i64 = 11;
/// How long a like, comment, save or share of one account keeps counting
/// as given: the period the ranking looks back on.
const SIGNAL_PERIOD_SECONDS: u64 = 7 * 86_400;
/// Shorts flushed per statement.
const FLUSH_BATCH: usize = 500;
/// How long a watched short may be credited with a new subscription.
const FOLLOW_CREDIT_SECONDS: u64 = 3600;
/// View counts the owner is congratulated on.
pub const MILESTONES: [i64; 3] = [1_000, 10_000, 100_000];

/// How long the for-you feed remembers having served a short to a viewer.
const SERVED_MEMORY_SECONDS: i64 = 6 * 3600;

const PENDING_SET: &str = "shorts:pending";

fn buffered_key(prefix: &str, key: &str) -> String {
    if prefix.is_empty() { key.to_owned() } else { format!("{prefix}:{key}") }
}

fn pending_set_key(prefix: &str) -> String {
    buffered_key(prefix, PENDING_SET)
}

fn signal_key(short_id: Uuid, signal: Engagement, account: Uuid) -> String {
    format!("shorts:once:{}:{}:{}", signal.name(), short_id.simple(), account.simple())
}

fn pending_key(short_id: Uuid) -> String {
    format!("shorts:pending:{}", short_id.simple())
}

fn pending_key_with_prefix(prefix: &str, short_id: Uuid) -> String {
    buffered_key(prefix, &pending_key(short_id))
}

fn seen_key(user_id: Uuid) -> String {
    format!("shorts:seen:{}", user_id.simple())
}

fn last_watch_key(user_id: Uuid, channel_id: Uuid) -> String {
    format!("shorts:lastwatch:{}:{}", user_id.simple(), channel_id.simple())
}

/// Who a view is counted for: the account, or a keyed hash of the network
/// a visitor comes from.
pub fn viewer_key(core: &Core, user_id: Option<Uuid>, ip: &str) -> String {
    match user_id {
        Some(user_id) => user_id.to_string(),
        None => format!("ip:{}", &hex::encode(core.keys.mac("shorts/viewer-ip", network_of(ip).as_bytes()))[..24]),
    }
}

/// The network an address stands for when visitors are told apart: an IPv4
/// address itself, the /64 of an IPv6 address (one subscriber holds all
/// 2^64 addresses of theirs, so each of them is the same visitor).
pub fn network_of(ip: &str) -> String {
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V6(address)) => match address.to_ipv4_mapped() {
            Some(mapped) => mapped.to_string(),
            None => {
                let segments = address.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", segments[0], segments[1], segments[2], segments[3])
            }
        },
        Ok(std::net::IpAddr::V4(address)) => address.to_string(),
        Err(_) => ip.to_owned(),
    }
}

/// The source a client named, as its number; unknown and unspecified
/// sources count as OTHER.
pub fn source_number(source: buffa::EnumValue<PlaybackSource>) -> i32 {
    match source.as_known() {
        Some(PlaybackSource::PLAYBACK_SOURCE_UNSPECIFIED) | None => PlaybackSource::PLAYBACK_SOURCE_OTHER as i32,
        Some(known) => known as i32,
    }
}

/// What one stay on a short amounts to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stay {
    /// Milliseconds played, bounded.
    pub watched_ms: i64,
    /// Share of the short that was watched (1 once it played through).
    pub ratio: f64,
    /// Replays past the end.
    pub loops: i64,
    /// The short played through at least once.
    pub completed: bool,
    /// Enough was played for the stay to count as a view.
    pub is_view: bool,
}

/// Reads a reported stay, trusting the client only as far as its numbers
/// are possible: a short cannot have played through more often than the
/// reported watch time allows.
pub fn measure(duration_seconds: f64, watched_ms: i64, completions: i32, min_view_ms: i64) -> Stay {
    let duration_ms = ((duration_seconds * 1000.0).round() as i64).max(1);
    let watched_ms = watched_ms.clamp(0, MAX_STAY_MS);
    // A tenth of slack: players report a loop slightly before the last frame.
    let possible = ((watched_ms as f64 / (duration_ms as f64 * 0.9)).floor() as i64).min(MAX_COMPLETIONS);
    let completions = i64::from(completions.max(0)).min(possible);
    let ratio = if completions >= 1 { 1.0 } else { (watched_ms as f64 / duration_ms as f64).min(1.0) };
    Stay {
        watched_ms,
        ratio,
        loops: (completions - 1).max(0),
        completed: completions >= 1 || ratio >= 0.999,
        is_view: watched_ms >= min_view_ms.min(duration_ms),
    }
}

/// The highest milestone a view count has reached (0 = none yet).
pub fn milestone_reached(view_count: i64) -> i64 {
    MILESTONES.iter().copied().filter(|milestone| view_count >= *milestone).max().unwrap_or(0)
}

/// One stay as reported, with the short it is about.
#[derive(Debug, Clone)]
pub struct Report {
    pub short: ShortRow,
    /// `PlaybackSource` as its number.
    pub source: i32,
    pub watched_ms: i64,
    pub completions: i32,
}

/// What became of a batch of reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Recorded {
    pub accepted: i32,
    pub views: i32,
}

/// A hot counter of the hourly statistics that is written as it happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engagement {
    Like,
    Comment,
    Share,
    Save,
}

impl Engagement {
    fn name(self) -> &'static str {
        match self {
            Self::Like => "like",
            Self::Comment => "comment",
            Self::Share => "share",
            Self::Save => "save",
        }
    }
}

/// Adds `delta` to a short's statistics of the current hour. Pass the
/// transaction of the change. Callers go through [`Shorts::count_signal`]
/// first, so that one account counts once.
pub(crate) async fn bump_hourly<'e, E: PgExecutor<'e>>(
    executor: E,
    short_id: Uuid,
    engagement: Engagement,
    delta: i64,
) -> AppResult<()> {
    let sql = match engagement {
        Engagement::Like => {
            "INSERT INTO short_hourly_stats (short_id, hour, likes) VALUES ($1, date_trunc('hour', now()), $2)
             ON CONFLICT (short_id, hour) DO UPDATE SET likes = short_hourly_stats.likes + EXCLUDED.likes"
        }
        Engagement::Comment => {
            "INSERT INTO short_hourly_stats (short_id, hour, comments) VALUES ($1, date_trunc('hour', now()), $2)
             ON CONFLICT (short_id, hour) DO UPDATE SET comments = short_hourly_stats.comments + EXCLUDED.comments"
        }
        Engagement::Share => {
            "INSERT INTO short_hourly_stats (short_id, hour, shares) VALUES ($1, date_trunc('hour', now()), $2)
             ON CONFLICT (short_id, hour) DO UPDATE SET shares = short_hourly_stats.shares + EXCLUDED.shares"
        }
        Engagement::Save => {
            "INSERT INTO short_hourly_stats (short_id, hour, saves) VALUES ($1, date_trunc('hour', now()), $2)
             ON CONFLICT (short_id, hour) DO UPDATE SET saves = short_hourly_stats.saves + EXCLUDED.saves"
        }
    };
    sqlx::query(sql).bind(short_id).bind(delta).execute(executor).await?;
    Ok(())
}

/// Tracks an analytics event about a short, for its creator's dashboard.
/// `props` gets the channel added. Never fails the caller.
pub(crate) async fn track(
    core: &Core,
    name: &str,
    short: &ShortRow,
    user_id: Option<Uuid>,
    value: f64,
    mut props: serde_json::Value,
) {
    if let Some(map) = props.as_object_mut() {
        map.insert("channel_id".to_owned(), serde_json::Value::String(short.channel_id.to_string()));
    }
    core.analytics
        .track(
            AnalyticsEvent::new(name)
                .user(user_id)
                .subject("short", short.id)
                .owner(short.owner_id)
                .value(value)
                .props(props),
        )
        .await;
}

// ───────────────────────────── the seen list ─────────────────────────────

/// The shorts among `ids` that `user_id` has played.
pub async fn seen_among(core: &Core, user_id: Uuid, ids: &[Uuid]) -> AppResult<HashSet<Uuid>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let members: Vec<String> = ids.iter().map(|id| id.simple().to_string()).collect();
    let scores: Vec<Option<f64>> =
        redis::cmd("ZMSCORE").arg(seen_key(user_id)).arg(&members).query_async(&mut core.cache.conn()).await?;
    Ok(ids.iter().zip(scores).filter(|(_, score)| score.is_some()).map(|(id, _)| *id).collect())
}

/// Every short `user_id` is remembered to have played.
pub async fn seen_ids(core: &Core, user_id: Uuid) -> AppResult<HashSet<Uuid>> {
    let members: Vec<String> =
        redis::cmd("ZRANGE").arg(seen_key(user_id)).arg(0).arg(-1).query_async(&mut core.cache.conn()).await?;
    Ok(members.iter().filter_map(|member| Uuid::parse_str(member).ok()).collect())
}

fn served_key(user_id: Uuid) -> String {
    format!("shorts:served:{}", user_id.simple())
}

/// The shorts the for-you feed returned to `user_id` during the last
/// [`SERVED_MEMORY_SECONDS`], watched or not.
pub async fn served_ids(core: &Core, user_id: Uuid) -> AppResult<HashSet<Uuid>> {
    let since = now().timestamp_millis() - SERVED_MEMORY_SECONDS * 1000;
    let members: Vec<String> = redis::cmd("ZRANGEBYSCORE")
        .arg(served_key(user_id))
        .arg(since)
        .arg("+inf")
        .query_async(&mut core.cache.conn())
        .await?;
    Ok(members.iter().filter_map(|member| Uuid::parse_str(member).ok()).collect())
}

/// Records that the for-you feed returned `ids` to `user_id`. The list is
/// capped like the seen list and forgets entries after a few hours, so a
/// short that was prefetched and never watched gets another chance.
pub(crate) async fn mark_served(core: &Core, user_id: Uuid, ids: &[Uuid], cap: i64) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let key = served_key(user_id);
    let now_ms = now().timestamp_millis();
    let mut writes = redis::pipe();
    for id in ids {
        writes.cmd("ZADD").arg(&key).arg(now_ms).arg(id.simple().to_string()).ignore();
    }
    writes.cmd("ZREMRANGEBYSCORE").arg(&key).arg("-inf").arg(now_ms - SERVED_MEMORY_SECONDS * 1000).ignore();
    writes.cmd("ZREMRANGEBYRANK").arg(&key).arg(0).arg(-(cap + 1)).ignore();
    writes.cmd("EXPIRE").arg(&key).arg(SERVED_MEMORY_SECONDS).ignore();
    writes.query_async::<()>(&mut core.cache.conn()).await?;
    Ok(())
}

/// Forgets what the for-you feed served to a viewer.
pub async fn forget_served(core: &Core, user_id: Uuid) -> AppResult<()> {
    core.cache.del(&served_key(user_id)).await
}

/// Drops everything Valkey remembers about a viewer (account deletion).
pub(crate) async fn forget_viewer(core: &Core, user_id: Uuid) -> AppResult<()> {
    core.cache.del(&seen_key(user_id)).await?;
    forget_served(core, user_id).await?;
    crate::feeds::drop_batch(core, Some(user_id)).await
}

// ───────────────────────────── recording ─────────────────────────────

impl Shorts {
    /// Whether `account` did `signal` on a short for the first time this
    /// week, and the action therefore counts towards the short's ranking.
    /// The owner's own actions never count. A signal is claimed with a
    /// marker in Valkey, so the same event delivered twice, or a like taken
    /// back and given again, counts once.
    pub(crate) async fn count_signal(&self, short: &ShortRow, signal: Engagement, account: Uuid) -> AppResult<bool> {
        if account == short.owner_id || !self.carries_weight(account).await? {
            return Ok(false);
        }
        self.claim_signal(short, signal, account).await
    }

    /// The claim alone, for an account already known to carry weight
    /// ([`Self::carries_weight`]). It talks to Valkey only, so it may be
    /// made while a transaction is open.
    pub(crate) async fn claim_signal(&self, short: &ShortRow, signal: Engagement, account: Uuid) -> AppResult<bool> {
        let key = signal_key(short.id, signal, account);
        self.core.cache.set_nx(&key, "1", Duration::from_secs(SIGNAL_PERIOD_SECONDS)).await
    }

    /// Gives a claimed signal back: the write it stood for did not happen,
    /// and the account's next attempt must be able to count.
    pub(crate) async fn release_signal(&self, short: &ShortRow, signal: Engagement, account: Uuid) {
        if let Err(error) = self.core.cache.del(&signal_key(short.id, signal, account)).await {
            tracing::warn!(%error, short_id = %short.id, "a ranking signal could not be released");
        }
    }

    /// Whether what `user_id` does moves a short's ranking at all: only an
    /// account that has existed for a while does. Visitors without an
    /// account never do, and an account made a minute ago to like, share
    /// and watch one short weighs as much as a visitor.
    pub async fn carries_weight(&self, user_id: Uuid) -> AppResult<bool> {
        let old_enough: Option<bool> = sqlx::query_scalar(
            "SELECT created_at <= now() - make_interval(hours => $2) AND deleted_at IS NULL AND banned_at IS NULL
             FROM users WHERE id = $1",
        )
        .bind(user_id)
        .bind(self.cfg.signal_min_account_age_hours)
        .fetch_optional(&self.core.db)
        .await?;
        Ok(old_enough.unwrap_or(false))
    }

    /// Takes in a batch of stays reported by `viewer`. Stays on shorts the
    /// viewer may not see, on drafts and on the viewer's own shorts are
    /// ignored. Every other stay marks its short as seen; what else it
    /// counts for is decided by two markers per viewer and short (see the
    /// module documentation), and by who the viewer is: a visitor, or an
    /// account too new to carry weight, adds to the view count that is
    /// shown and to nothing the ranking reads.
    pub async fn record_playback(&self, viewer: &Viewer, ip: &str, reports: Vec<Report>) -> AppResult<Recorded> {
        let core = &self.core;
        let who = viewer_key(core, viewer.user_id, ip);
        let weighs = match viewer.user_id {
            Some(user_id) => self.carries_weight(user_id).await?,
            None => false,
        };
        let stays: Vec<(Report, Stay)> = reports
            .into_iter()
            .filter(|report| {
                report.short.is_published()
                    && !viewer.is(report.short.owner_id)
                    && access::can_watch(viewer, &report.short.facts())
            })
            .map(|report| {
                let stay =
                    measure(report.short.duration_seconds, report.watched_ms, report.completions, self.cfg.min_view_ms);
                (report, stay)
            })
            .collect();
        if stays.is_empty() {
            return Ok(Recorded::default());
        }
        let mut conn = core.cache.conn();

        // Claim, per stay, the viewer's sample of the short and (for a stay
        // long enough) their view of it. `SET NX` is the arbiter: of two
        // reports racing for the same marker, exactly one gets it.
        let ttl = self.cfg.view_dedupe_seconds.max(1);
        let mut claims = redis::pipe();
        for (report, stay) in &stays {
            let short = report.short.id.simple();
            claims.cmd("SET").arg(format!("shorts:stayed:{short}:{who}")).arg(1).arg("NX").arg("EX").arg(ttl);
            if stay.is_view {
                claims.cmd("SET").arg(format!("shorts:viewed:{short}:{who}")).arg(1).arg("NX").arg("EX").arg(ttl);
            }
        }
        let mut claimed = claims.query_async::<Vec<Option<String>>>(&mut conn).await?.into_iter();
        let mut granted = || claimed.next().flatten().is_some();
        // (first stay of the period, counted view)
        let counted: Vec<(bool, bool)> = stays.iter().map(|(_, stay)| (granted(), stay.is_view && granted())).collect();

        let now_ms = now().timestamp_millis();
        let mut writes = redis::pipe();
        for ((report, stay), (sample, view)) in stays.iter().zip(&counted) {
            let key = pending_key_with_prefix(&self.signal_buffer_prefix, report.short.id);
            if *view {
                writes.cmd("HINCRBY").arg(&key).arg(if weighs { "views" } else { "guest_views" }).arg(1).ignore();
            }
            let sample = &(*sample && weighs);
            if *sample {
                if stay.loops > 0 {
                    writes.cmd("HINCRBY").arg(&key).arg("loops").arg(stay.loops).ignore();
                }
                writes.cmd("HINCRBY").arg(&key).arg("samples").arg(1).ignore();
                writes.cmd("HINCRBY").arg(&key).arg("ratio_milli").arg((stay.ratio * 1000.0).round() as i64).ignore();
            }
            if *view || *sample {
                writes
                    .cmd("SADD")
                    .arg(pending_set_key(&self.signal_buffer_prefix))
                    .arg(report.short.id.simple().to_string())
                    .ignore();
            }
            if let Some(user_id) = viewer.user_id {
                writes
                    .cmd("ZADD")
                    .arg(seen_key(user_id))
                    .arg(now_ms)
                    .arg(report.short.id.simple().to_string())
                    .ignore();
                writes
                    .cmd("SET")
                    .arg(last_watch_key(user_id, report.short.channel_id))
                    .arg(report.short.id.to_string())
                    .arg("EX")
                    .arg(FOLLOW_CREDIT_SECONDS)
                    .ignore();
            }
        }
        if let Some(user_id) = viewer.user_id {
            // Keep the newest `seen_cap` entries; drop the list of a viewer who stays away.
            let key = seen_key(user_id);
            writes.cmd("ZREMRANGEBYRANK").arg(&key).arg(0).arg(-(self.cfg.seen_cap + 1)).ignore();
            writes.cmd("EXPIRE").arg(&key).arg(self.cfg.seen_ttl_days.max(1) * 86_400).ignore();
        }
        // A guest's repeated stay leaves nothing to write, and an empty
        // pipeline is an error to the client library.
        if !writes.is_empty() {
            writes.query_async::<()>(&mut conn).await?;
        }

        for ((report, stay), (sample, view)) in stays.iter().zip(&counted) {
            if *sample && weighs {
                let props = serde_json::json!({
                    "viewer": who,
                    "source": report.source,
                    "ratio": stay.ratio,
                    "loops": stay.loops,
                    "completed": stay.completed,
                });
                track(core, "short.watch", &report.short, viewer.user_id, stay.watched_ms as f64 / 1000.0, props).await;
            }
            if *view {
                let props = serde_json::json!({ "viewer": who, "source": report.source, "counted": weighs });
                track(core, "short.view", &report.short, viewer.user_id, 1.0, props).await;
            }
        }
        Ok(Recorded { accepted: stays.len() as i32, views: counted.iter().filter(|(_, view)| *view).count() as i32 })
    }

    /// A user subscribed to a channel: if they watched one of its shorts
    /// within the last hour, that short is credited with the new follower
    /// (`short.follow` in the creator analytics).
    pub async fn credit_follow(&self, user_id: Uuid, channel_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let watched: Option<String> =
            redis::cmd("GETDEL").arg(last_watch_key(user_id, channel_id)).query_async(&mut core.cache.conn()).await?;
        let Some(short_id) = watched.and_then(|id| Uuid::parse_str(&id).ok()) else { return Ok(()) };
        if let Some(short) = shorts::find_by_id(core, short_id).await?.filter(|short| short.channel_id == channel_id) {
            track(core, "short.follow", &short, Some(user_id), 1.0, serde_json::json!({})).await;
        }
        Ok(())
    }
}

// ───────────────────────────── flushing ─────────────────────────────

/// Reads and clears the buffered counters of the given keys in one step,
/// so a stay recorded meanwhile lands in the next flush instead of nowhere.
const TAKE_PENDING: &str = r"
local out = {}
for i, key in ipairs(KEYS) do
  out[i] = redis.call('HGETALL', key)
  redis.call('DEL', key)
end
return out";

/// The buffered counters of one short.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Pending {
    views: i64,
    /// Views of visitors and of accounts too new to carry weight: shown in
    /// the view count, never part of the hourly statistics.
    guest_views: i64,
    loops: i64,
    samples: i64,
    ratio_milli: i64,
}

impl Pending {
    fn from_fields(fields: &[String]) -> Self {
        let mut pending = Self::default();
        for pair in fields.chunks_exact(2) {
            let value: i64 = pair[1].parse().unwrap_or(0);
            match pair[0].as_str() {
                "views" => pending.views = value,
                "guest_views" => pending.guest_views = value,
                "loops" => pending.loops = value,
                "samples" => pending.samples = value,
                "ratio_milli" => pending.ratio_milli = value,
                _ => {}
            }
        }
        pending
    }

    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl Shorts {
    /// Moves the buffered playback counters into PostgreSQL: totals on the
    /// shorts, the current hour's statistics, and the view milestones that
    /// were crossed (the owner is told). Returns how many shorts were
    /// written; call again while it returns a full batch.
    pub async fn flush_signals(&self) -> AppResult<u64> {
        let core = &self.core;
        let mut conn = core.cache.conn();
        let members: Vec<String> = redis::cmd("SPOP")
            .arg(pending_set_key(&self.signal_buffer_prefix))
            .arg(FLUSH_BATCH)
            .query_async(&mut conn)
            .await?;
        let mut ids: Vec<Uuid> = members.iter().filter_map(|member| Uuid::parse_str(member).ok()).collect();
        if ids.is_empty() {
            return Ok(0);
        }
        // In ID order, so two flushing processes never lock rows crosswise.
        ids.sort_unstable();
        let script = redis::Script::new(TAKE_PENDING);
        let mut take = script.prepare_invoke();
        for id in &ids {
            take.key(pending_key_with_prefix(&self.signal_buffer_prefix, *id));
        }
        let taken: Vec<Vec<String>> = take.invoke_async(&mut conn).await?;
        let batch: Vec<(Uuid, Pending)> = ids
            .into_iter()
            .zip(taken.iter().map(|fields| Pending::from_fields(fields)))
            .filter(|(_, pending)| !pending.is_empty())
            .collect();
        if batch.is_empty() {
            return Ok(0);
        }
        match self.write_pending(&batch).await {
            Ok(written) => Ok(written),
            Err(error) => {
                // Put the counters back; the next flush tries again.
                let mut restore = redis::pipe();
                for (id, pending) in &batch {
                    let key = pending_key_with_prefix(&self.signal_buffer_prefix, *id);
                    for (field, value) in [
                        ("views", pending.views),
                        ("guest_views", pending.guest_views),
                        ("loops", pending.loops),
                        ("samples", pending.samples),
                        ("ratio_milli", pending.ratio_milli),
                    ] {
                        if value != 0 {
                            restore.cmd("HINCRBY").arg(&key).arg(field).arg(value).ignore();
                        }
                    }
                    restore
                        .cmd("SADD")
                        .arg(pending_set_key(&self.signal_buffer_prefix))
                        .arg(id.simple().to_string())
                        .ignore();
                }
                if let Err(lost) = restore.query_async::<()>(&mut conn).await {
                    tracing::error!(error = %lost, shorts = batch.len(), "buffered playback counters were lost");
                }
                Err(error)
            }
        }
    }

    async fn write_pending(&self, batch: &[(Uuid, Pending)]) -> AppResult<u64> {
        let ids: Vec<Uuid> = batch.iter().map(|(id, _)| *id).collect();
        let views: Vec<i64> = batch.iter().map(|(_, pending)| pending.views).collect();
        let guest_views: Vec<i64> = batch.iter().map(|(_, pending)| pending.guest_views).collect();
        let loops: Vec<i64> = batch.iter().map(|(_, pending)| pending.loops).collect();
        let samples: Vec<i64> = batch.iter().map(|(_, pending)| pending.samples).collect();
        let ratios: Vec<f64> = batch.iter().map(|(_, pending)| pending.ratio_milli as f64 / 1000.0).collect();
        let mut tx = self.core.db.begin().await?;
        // Shorts deleted since the stays were recorded simply drop out.
        let written: Vec<(Uuid, i64, i64)> = sqlx::query_as(
            "WITH data AS (
                 SELECT * FROM unnest($1::uuid[], $2::bigint[], $3::bigint[], $4::bigint[], $5::float8[], $6::bigint[])
                      AS d(short_id, views, loops, samples, ratio_sum, guest_views)
             ), updated AS (
                 UPDATE shorts s
                 SET view_count = s.view_count + d.views + d.guest_views, loop_count = s.loop_count + d.loops,
                     watch_samples = s.watch_samples + d.samples, watch_ratio_sum = s.watch_ratio_sum + d.ratio_sum
                 FROM data d WHERE s.id = d.short_id
                 RETURNING s.id, s.view_count, s.milestone
             ), hourly AS (
                 INSERT INTO short_hourly_stats (short_id, hour, views, loops)
                 SELECT d.short_id, date_trunc('hour', now()), d.views, d.loops
                 FROM data d JOIN updated u ON u.id = d.short_id
                 WHERE d.views > 0 OR d.loops > 0
                 ON CONFLICT (short_id, hour) DO UPDATE
                     SET views = short_hourly_stats.views + EXCLUDED.views,
                         loops = short_hourly_stats.loops + EXCLUDED.loops
             )
             SELECT id, view_count, milestone FROM updated",
        )
        .bind(&ids)
        .bind(&views)
        .bind(&loops)
        .bind(&samples)
        .bind(&ratios)
        .bind(&guest_views)
        .fetch_all(&mut *tx)
        .await?;
        for (short_id, view_count, told) in &written {
            let reached = milestone_reached(*view_count);
            if reached > *told {
                self.celebrate_in(&mut tx, *short_id, reached, *view_count).await?;
            }
        }
        tx.commit().await?;
        Ok(written.len() as u64)
    }

    /// Records that a short crossed a view milestone and tells its owner.
    /// Milestones of one short share a collapse key: a short that jumps
    /// from 900 to 12 000 views leaves one notification, about 10 000.
    async fn celebrate_in(
        &self,
        tx: &mut shorts::Tx<'_>,
        short_id: Uuid,
        milestone: i64,
        view_count: i64,
    ) -> AppResult<()> {
        // Conditional, so that two flushes crossing the same line tell once.
        let crossed = sqlx::query("UPDATE shorts SET milestone = $2 WHERE id = $1 AND milestone < $2")
            .bind(short_id)
            .bind(milestone)
            .execute(&mut **tx)
            .await?
            .rows_affected()
            == 1;
        if !crossed {
            return Ok(());
        }
        let Some(short) = shorts::find_in(tx, short_id).await? else { return Ok(()) };
        let count = group_digits(milestone);
        let caption = text::excerpt(&short.caption, 60);
        let mut request = outbound::NotificationRequest::new(
            short.owner_id,
            "shorts.milestone",
            outbound::Localized::new(
                format!("ショートが{count}回再生されたよ!"),
                format!("Your short reached {count} views!"),
            ),
            outbound::Localized::same(caption),
            short.path(),
        )
        .subject(SubjectType::SUBJECT_TYPE_SHORT, short.id)
        .collapse(format!("shorts.milestone:{}", short.id))
        .data(serde_json::json!({
            "id": short.id,
            "short_id": short.short_id,
            "milestone": milestone,
            "view_count": view_count,
        }));
        if let Some(url) = short.cover_url() {
            request = request.image(url);
        }
        outbound::notify(&mut **tx, &request).await
    }

    /// Runs the flush until the buffer is empty. Used by the worker's timer
    /// and by tests.
    pub async fn flush_all_signals(&self) -> AppResult<u64> {
        let mut total = 0;
        loop {
            let written = self.flush_signals().await?;
            total += written;
            if written < FLUSH_BATCH as u64 {
                return Ok(total);
            }
        }
    }
}

/// `12345` as `12,345`.
fn group_digits(number: i64) -> String {
    let digits = number.abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if number < 0 { format!("-{out}") } else { out }
}

/// How long the worker waits between flushes.
pub(crate) fn flush_interval(seconds: u64) -> Duration {
    Duration::from_secs(seconds.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stay_counts_as_a_view_after_a_second_or_the_whole_short() {
        assert!(!measure(15.0, 400, 0, 1000).is_view);
        assert!(measure(15.0, 1000, 0, 1000).is_view);
        // A half-second short cannot be watched for a second.
        assert!(measure(0.5, 500, 1, 1000).is_view);
        assert!(!measure(0.5, 200, 0, 1000).is_view);
        assert!(measure(15.0, 0, 0, 0).is_view, "a zero threshold counts every stay");
    }

    #[test]
    fn the_watch_ratio_is_the_share_watched_until_it_played_through() {
        let half = measure(10.0, 5000, 0, 1000);
        assert_eq!((half.ratio, half.loops, half.completed), (0.5, 0, false));
        let through = measure(10.0, 10_000, 1, 1000);
        assert_eq!((through.ratio, through.loops, through.completed), (1.0, 0, true));
        let twice_and_a_bit = measure(10.0, 26_000, 2, 1000);
        assert_eq!((twice_and_a_bit.ratio, twice_and_a_bit.loops), (1.0, 1));
        // Watched to the end without the client saying so.
        assert!(measure(10.0, 10_000, 0, 1000).completed);
    }

    #[test]
    fn impossible_reports_are_cut_down_to_what_the_watch_time_allows() {
        // 3 seconds cannot hold 40 plays of a 10 second short.
        let liar = measure(10.0, 3000, 40, 1000);
        assert_eq!((liar.ratio, liar.loops, liar.completed), (0.3, 0, false));
        // An hour at most, and eleven plays (ten loops) at most.
        let forever = measure(1.0, i64::MAX, i32::MAX, 1000);
        assert_eq!((forever.watched_ms, forever.loops), (MAX_STAY_MS, MAX_COMPLETIONS - 1));
        let negative = measure(10.0, -5, -3, 1000);
        assert_eq!((negative.watched_ms, negative.ratio, negative.loops, negative.is_view), (0, 0.0, 0, false));
        // A loop reported a little early still counts.
        assert_eq!(measure(10.0, 18_500, 2, 1000).loops, 1);
    }

    #[test]
    fn milestones_are_the_highest_threshold_reached() {
        assert_eq!(milestone_reached(999), 0);
        assert_eq!(milestone_reached(1_000), 1_000);
        assert_eq!(milestone_reached(12_345), 10_000);
        assert_eq!(milestone_reached(5_000_000), 100_000);
        assert_eq!(group_digits(1_000), "1,000");
        assert_eq!(group_digits(100_000), "100,000");
        assert_eq!(group_digits(999), "999");
    }

    #[test]
    fn buffered_counters_are_read_from_field_value_pairs() {
        let fields: Vec<String> = ["views", "3", "samples", "5", "ratio_milli", "4200", "unknown", "9"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let pending = Pending::from_fields(&fields);
        assert_eq!(pending, Pending { views: 3, guest_views: 0, loops: 0, samples: 5, ratio_milli: 4200 });
        assert_eq!(network_of("2001:db8:1:2:aaaa:bbbb:cccc:dddd"), network_of("2001:db8:1:2::1"));
        assert_ne!(network_of("2001:db8:1:2::1"), network_of("2001:db8:1:3::1"));
        assert_eq!(network_of("::ffff:203.0.113.7"), "203.0.113.7");
        assert_eq!(network_of("203.0.113.7"), "203.0.113.7");
        assert!(Pending::from_fields(&[]).is_empty());
    }

    #[test]
    fn signal_buffer_keys_can_be_scoped_per_test_run() {
        let id = Uuid::nil();
        assert_eq!(pending_set_key(""), "shorts:pending");
        assert_eq!(pending_key_with_prefix("", id), pending_key(id));
        assert_eq!(pending_set_key("tokuly_test_1_ab12cd"), "tokuly_test_1_ab12cd:shorts:pending");
        assert_eq!(
            pending_key_with_prefix("tokuly_test_1_ab12cd", id),
            "tokuly_test_1_ab12cd:shorts:pending:00000000000000000000000000000000"
        );
    }

    #[test]
    fn unknown_sources_count_as_other() {
        let other = PlaybackSource::PLAYBACK_SOURCE_OTHER as i32;
        assert_eq!(source_number(PlaybackSource::PLAYBACK_SOURCE_UNSPECIFIED.into()), other);
        assert_eq!(source_number(buffa::EnumValue::from(99)), other);
        assert_eq!(source_number(PlaybackSource::PLAYBACK_SOURCE_FOR_YOU.into()), 1);
    }
}
