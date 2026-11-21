//! `RecapService`: "your month / your year on Tokuly".
//!
//! A recap is computed on demand from the caller's own activity log
//! ([`aggregate`], a pure function over the period's entries) and cached in
//! Valkey: briefly while the period is still running, for hours once it is
//! over. What is cached is language- and viewer-independent numbers and IDs
//! ([`RecapData`]); names, labels and sentences are filled in per request, so
//! a block or a renamed friend shows immediately.
//!
//! Periods are calendar months and years in the caller's time zone.
//!
//! What producers put into `ActivityEntry.data` matters here (see
//! `docs/events/feed.md`): `place_name`, `region_name` and `region_code`
//! feed the places and regions of a recap.
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Duration,
};

use chrono::{Datelike as _, NaiveDate, TimeZone as _};
use chrono_tz::Tz;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use redis::AsyncCommands as _;
use serde::{Deserialize, Serialize};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{common::SubjectType, feed::*};

use crate::{Feed, activity::Reader, events::RECAP_READY_KIND, kinds};

/// Entries read for one recap (a year of a very active user fits).
const MAX_RECAP_ROWS: i64 = 100_000;
/// How long the recap of a finished period is cached.
const COMPLETE_CACHE: Duration = Duration::from_secs(6 * 3600);
const TOP_BUMP_FRIENDS: usize = 5;
const SHOWN_NEW_FRIENDS: usize = 12;
const SHOWN_PLACES: usize = 10;
const SHOWN_REGIONS: usize = 20;
/// Friends kept in the cached data, so that blocked ones can be skipped later.
const KEPT_FRIENDS: usize = 50;
/// Days after a month ended during which its recap notice is still sent.
const NOTICE_WINDOW_DAYS: u32 = 3;
const NOTICE_BATCH: i64 = 500;

/// A calendar month or year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    pub kind: RecapPeriod,
    /// First day.
    pub start: NaiveDate,
    /// Last day (inclusive).
    pub end: NaiveDate,
}

impl Period {
    /// The month or year containing `anchor`.
    pub fn containing(kind: RecapPeriod, anchor: NaiveDate) -> Option<Self> {
        let (start, next) = match kind {
            RecapPeriod::RECAP_PERIOD_MONTH => {
                let start = anchor.with_day(1)?;
                let next = if anchor.month() == 12 {
                    NaiveDate::from_ymd_opt(anchor.year() + 1, 1, 1)?
                } else {
                    NaiveDate::from_ymd_opt(anchor.year(), anchor.month() + 1, 1)?
                };
                (start, next)
            }
            RecapPeriod::RECAP_PERIOD_YEAR => {
                (NaiveDate::from_ymd_opt(anchor.year(), 1, 1)?, NaiveDate::from_ymd_opt(anchor.year() + 1, 1, 1)?)
            }
            RecapPeriod::RECAP_PERIOD_UNSPECIFIED => return None,
        };
        Some(Self { kind, start, end: next.pred_opt()? })
    }

    /// The instants at which the period starts and ends (exclusive) in `tz`.
    pub fn bounds(&self, tz: Tz) -> (Time, Time) {
        (local_midnight(self.start, tz), local_midnight(self.end.succ_opt().unwrap_or(self.end), tz))
    }

    pub fn title(&self) -> Localized {
        match self.kind {
            RecapPeriod::RECAP_PERIOD_YEAR => {
                Localized::new(format!("{}年のまとめ", self.start.year()), format!("{} recap", self.start.year()))
            }
            _ => Localized::new(
                format!("{}年{}月のまとめ", self.start.year(), self.start.month()),
                format!("{} {} recap", month_name(self.start.month()), self.start.year()),
            ),
        }
    }

    /// Path of the recap page in the app.
    pub fn path(&self) -> String {
        match self.kind {
            RecapPeriod::RECAP_PERIOD_YEAR => format!("/recap/{}", self.start.year()),
            _ => format!("/recap/{}-{:02}", self.start.year(), self.start.month()),
        }
    }
}

fn month_name(month: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    NAMES[(month.clamp(1, 12) - 1) as usize]
}

/// The instant a day starts in `tz`. Where midnight does not exist (a
/// daylight-saving jump), the first instant after it.
fn local_midnight(date: NaiveDate, tz: Tz) -> Time {
    let midnight = date.and_hms_opt(0, 0, 0).expect("midnight exists on every date");
    (0..=3)
        .find_map(|hours| tz.from_local_datetime(&(midnight + chrono::Duration::hours(hours))).earliest())
        .map(|local| local.with_timezone(&chrono::Utc))
        .unwrap_or_else(|| midnight.and_utc())
}

/// What a recap needs from one activity entry.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RecapRow {
    pub kind: String,
    pub subject_type: Option<i32>,
    pub subject_id: Option<Uuid>,
    pub with_user_ids: Vec<Uuid>,
    pub lat: Option<f64>,
    pub lng: Option<f64>,
    pub place_name: Option<String>,
    pub region_name: Option<String>,
    pub region_code: Option<String>,
    pub occurred_at: Time,
}

/// The numbers of a recap, without anything that depends on who reads it
/// or in which language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecapData {
    pub activity_count: i32,
    /// Entries per kind, most frequent first.
    pub kinds: Vec<(String, i32)>,
    pub bump_count: i32,
    /// Friends by number of bumps, most bumped first.
    pub bump_friends: Vec<(Uuid, i32)>,
    pub new_friend_count: i32,
    /// New friends, most recent first.
    pub new_friends: Vec<Uuid>,
    pub place_count: i32,
    /// Names of places, most frequent first.
    pub place_names: Vec<String>,
    pub region_count: i32,
    pub region_names: Vec<String>,
    pub area_count: i32,
    /// Every day with at least one entry, in order.
    pub days: Vec<(NaiveDate, i32)>,
    pub generated_at: Time,
}

/// The people an entry is about: the user it points at and the users it names.
fn partners(row: &RecapRow) -> Vec<Uuid> {
    let subject = (row.subject_type == Some(SubjectType::SUBJECT_TYPE_USER as i32)).then_some(row.subject_id).flatten();
    let mut people: Vec<Uuid> = row.with_user_ids.clone();
    if let Some(subject) = subject
        && !people.contains(&subject)
    {
        people.push(subject);
    }
    people
}

fn non_empty(text: &Option<String>) -> Option<&str> {
    text.as_deref().map(str::trim).filter(|text| !text.is_empty())
}

/// Counts by key, most frequent first; ties keep first-seen order.
fn ranked<K: Clone + Eq + std::hash::Hash>(keys: impl Iterator<Item = K>) -> Vec<(K, i32)> {
    let mut order: Vec<K> = Vec::new();
    let mut counts: HashMap<K, i32> = HashMap::new();
    for key in keys {
        let count = counts.entry(key.clone()).or_insert(0);
        if *count == 0 {
            order.push(key);
        }
        *count += 1;
    }
    let mut ranked: Vec<(K, i32)> = order
        .into_iter()
        .map(|key| {
            let count = counts[&key];
            (key, count)
        })
        .collect();
    // Stable: equal counts stay in first-seen order.
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    ranked
}

/// Aggregates the entries of one period (in any order) into recap numbers.
/// Days are counted in `tz`.
pub fn aggregate(rows: &[RecapRow], tz: Tz, generated_at: Time) -> RecapData {
    let mut days: BTreeMap<NaiveDate, i32> = BTreeMap::new();
    for row in rows {
        *days.entry(row.occurred_at.with_timezone(&tz).date_naive()).or_insert(0) += 1;
    }

    let bumps: Vec<&RecapRow> = rows.iter().filter(|row| row.kind == kinds::BUMPED).collect();
    let mut bump_friends = ranked(bumps.iter().flat_map(|row| partners(row)));
    bump_friends.truncate(KEPT_FRIENDS);

    // New friends, most recent first, each once.
    let mut friendships: Vec<&RecapRow> = rows.iter().filter(|row| row.kind == kinds::FRIEND_ADDED).collect();
    friendships.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
    let mut seen = HashSet::new();
    let all_new_friends: Vec<Uuid> =
        friendships.iter().flat_map(|row| partners(row)).filter(|friend| seen.insert(*friend)).collect();

    // A place is identified by the subject it points at, else by its name.
    let place_subject = SubjectType::SUBJECT_TYPE_PLACE as i32;
    let place_key = |row: &RecapRow| match (row.subject_type, row.subject_id, non_empty(&row.place_name)) {
        (Some(subject_type), Some(id), _) if subject_type == place_subject => Some(id.to_string()),
        (_, _, Some(name)) => Some(format!("name:{}", name.to_lowercase())),
        _ => None,
    };
    let places = ranked(rows.iter().filter_map(place_key));
    let mut place_names: Vec<String> = Vec::new();
    for (key, _) in &places {
        // The name of a place: the first one any of its entries carries.
        let name = rows
            .iter()
            .filter(|row| place_key(row).as_deref() == Some(key.as_str()))
            .find_map(|row| non_empty(&row.place_name));
        if let Some(name) = name
            && !place_names.iter().any(|known| known == name)
        {
            place_names.push(name.to_owned());
        }
        if place_names.len() >= SHOWN_PLACES {
            break;
        }
    }

    // Regions come from entries of the "new region" family.
    let region_key = |row: &RecapRow| {
        let regional = kinds::matches(&row.kind, "geo.new_region") || kinds::matches(&row.kind, "geo.region");
        regional.then(|| non_empty(&row.region_code).or(non_empty(&row.region_name)).map(str::to_lowercase)).flatten()
    };
    let mut region_keys = HashSet::new();
    let mut region_names: Vec<String> = Vec::new();
    for row in rows {
        if let Some(key) = region_key(row)
            && region_keys.insert(key)
            && let Some(name) = non_empty(&row.region_name).or(non_empty(&row.region_code))
            && region_names.len() < SHOWN_REGIONS
        {
            region_names.push(name.to_owned());
        }
    }

    // Roughly one-kilometre cells: two decimals of a degree.
    let areas: HashSet<(i64, i64)> = rows
        .iter()
        .filter_map(|row| Some(((row.lat? * 100.0).round() as i64, (row.lng? * 100.0).round() as i64)))
        .collect();

    RecapData {
        activity_count: rows.len() as i32,
        kinds: ranked(rows.iter().map(|row| row.kind.clone())),
        bump_count: bumps.len() as i32,
        bump_friends,
        new_friend_count: all_new_friends.len() as i32,
        new_friends: all_new_friends.into_iter().take(KEPT_FRIENDS).collect(),
        place_count: places.len() as i32,
        place_names,
        region_count: region_keys.len() as i32,
        region_names,
        area_count: areas.len() as i32,
        days: days.into_iter().collect(),
        generated_at,
    }
}

/// The day with the most entries (the earliest one on a tie).
fn busiest_day(days: &[(NaiveDate, i32)]) -> Option<(NaiveDate, i32)> {
    days.iter().copied().reduce(|best, day| if day.1 > best.1 { day } else { best })
}

fn highlight(kind: RecapHighlightType, text: Localized, value: i32, locale: &str) -> RecapHighlight {
    RecapHighlight { r#type: kind.into(), text: text.pick(locale).to_owned(), value, ..Default::default() }
}

/// The ready-made sentences of a recap, most interesting first.
fn highlights(data: &RecapData, most_bumped: Option<(&str, i32)>, locale: &str) -> Vec<RecapHighlight> {
    use RecapHighlightType as H;
    let mut lines = Vec::new();
    if let Some((name, count)) = most_bumped {
        let text = Localized::new(
            format!("一番バンプした友達は{name}さん！({count}回)"),
            format!("You bumped {name} the most ({count} times)!"),
        );
        lines.push(highlight(H::RECAP_HIGHLIGHT_TYPE_MOST_BUMPED_FRIEND, text, count, locale));
    }
    if data.new_friend_count > 0 {
        let n = data.new_friend_count;
        let text = Localized::new(
            format!("新しい友達が{n}人できました！"),
            if n == 1 { "You made a new friend!".to_owned() } else { format!("You made {n} new friends!") },
        );
        lines.push(highlight(H::RECAP_HIGHLIGHT_TYPE_NEW_FRIENDS, text, n, locale));
    }
    if data.region_count > 0 {
        let n = data.region_count;
        let text = Localized::new(
            format!("{n}つの新しい地域に足をのばしました！"),
            if n == 1 { "You explored a new region!".to_owned() } else { format!("You explored {n} new regions!") },
        );
        lines.push(highlight(H::RECAP_HIGHLIGHT_TYPE_REGIONS, text, n, locale));
    }
    if data.place_count > 0 {
        let n = data.place_count;
        let text = Localized::new(
            format!("{n}か所の場所でログを残しました！"),
            if n == 1 {
                "You left your mark on one place!".to_owned()
            } else {
                format!("You left your mark on {n} places!")
            },
        );
        lines.push(highlight(H::RECAP_HIGHLIGHT_TYPE_PLACES, text, n, locale));
    }
    if let Some((day, count)) = busiest_day(&data.days) {
        let text = Localized::new(
            format!("一番アクティブだったのは{}月{}日！({count}件)", day.month(), day.day()),
            format!("Your busiest day was {} {} ({count} activities)!", month_name(day.month()), day.day()),
        );
        lines.push(highlight(H::RECAP_HIGHLIGHT_TYPE_BUSIEST_DAY, text, count, locale));
    }
    if let Some((kind, count)) = data.kinds.first() {
        let label = kinds::label(kind);
        let text = Localized::new(
            format!("いちばん多かったのは「{}」！({count}回)", label.ja),
            format!("Your top activity: {} ({count} times)", label.en),
        );
        lines.push(highlight(H::RECAP_HIGHLIGHT_TYPE_TOP_KIND, text, *count, locale));
    }
    if data.activity_count > 0 {
        let n = data.activity_count;
        let text = Localized::new(
            format!("ぜんぶで{n}件のログが残りました！"),
            if n == 1 { "You logged one moment!".to_owned() } else { format!("You logged {n} moments!") },
        );
        lines.push(highlight(H::RECAP_HIGHLIGHT_TYPE_TOTAL_ACTIVITY, text, n, locale));
    }
    lines
}

fn recap_epoch_key(user_id: Uuid) -> String {
    format!("feed:recap:epoch:{user_id}")
}

impl Feed {
    /// Makes every cached recap of a user stale (their log changed in a way
    /// a recap must reflect at once: entries were deleted).
    pub(crate) async fn invalidate_recaps(&self, user_id: Uuid) {
        let bumped: Result<i64, _> = self.core.cache.conn().incr(recap_epoch_key(user_id), 1).await;
        if let Err(error) = bumped {
            tracing::warn!(user = %user_id, %error, "could not invalidate cached recaps");
        }
    }

    /// The numbers of a period, from the cache or freshly computed.
    async fn recap_data(&self, user_id: Uuid, period: &Period, tz: Tz, complete: bool) -> AppResult<RecapData> {
        let epoch = self.core.cache.get_string(&recap_epoch_key(user_id)).await?.unwrap_or_else(|| "0".to_owned());
        let key = format!("feed:recap:{user_id}:{epoch}:{}:{}:{}", period.kind as i32, period.start, tz.name());
        if let Some(cached) = self.core.cache.get_json::<RecapData>(&key).await? {
            return Ok(cached);
        }
        let (from, to) = period.bounds(tz);
        let rows: Vec<RecapRow> = sqlx::query_as(
            "SELECT kind, subject_type, subject_id, with_user_ids, lat, lng,
                    COALESCE(data->>'place_name', CASE WHEN subject_type = $4 THEN data->>'name' END) AS place_name,
                    data->>'region_name' AS region_name, data->>'region_code' AS region_code, occurred_at
             FROM activities WHERE user_id = $1 AND occurred_at >= $2 AND occurred_at < $3
             ORDER BY occurred_at LIMIT $5",
        )
        .bind(user_id)
        .bind(from)
        .bind(to)
        .bind(SubjectType::SUBJECT_TYPE_PLACE as i32)
        .bind(MAX_RECAP_ROWS)
        .fetch_all(&self.core.db)
        .await?;
        let data = aggregate(&rows, tz, now());
        let ttl = if complete { COMPLETE_CACHE } else { Duration::from_secs(self.cfg.recap_cache_seconds.max(1)) };
        self.core.cache.set_json(&key, &data, Some(ttl)).await?;
        Ok(data)
    }

    /// Recap numbers as the API message: names resolved, people hidden from
    /// the reader left out, sentences in the reader's language.
    async fn recap_to_proto(
        &self,
        reader: &Reader,
        period: &Period,
        tz: Tz,
        complete: bool,
        data: &RecapData,
    ) -> AppResult<Recap> {
        let friend_ids: Vec<Uuid> =
            data.bump_friends.iter().map(|(id, _)| *id).chain(data.new_friends.iter().copied()).collect();
        let people = tokuly_identity::summaries(&self.core, &friend_ids).await?;
        // Deleted accounts and people behind a block are not shown by name.
        let shown = |id: &Uuid| people.get(id).filter(|summary| !summary.deleted && !reader.is_hidden(*id)).cloned();

        let bump_friends: Vec<RecapFriend> = data
            .bump_friends
            .iter()
            .filter_map(|(id, count)| {
                shown(id).map(|user| RecapFriend { user: user.into(), count: *count, ..Default::default() })
            })
            .take(TOP_BUMP_FRIENDS)
            .collect();
        let most_bumped = bump_friends.first().cloned();
        let most_bumped_name = most_bumped.as_ref().and_then(|friend| {
            friend.user.as_option().map(|user| {
                (
                    if user.display_name.is_empty() { user.handle.clone() } else { user.display_name.clone() },
                    friend.count,
                )
            })
        });
        let busiest = busiest_day(&data.days);
        let day = |(date, count): (NaiveDate, i32)| RecapDay { date: date.to_string(), count, ..Default::default() };

        Ok(Recap {
            period: period.kind.into(),
            title: period.title().pick(&reader.locale).to_owned(),
            start_date: period.start.to_string(),
            end_date: period.end.to_string(),
            time_zone: tz.name().to_owned(),
            complete,
            activity_count: data.activity_count,
            active_day_count: data.days.len() as i32,
            kinds: data
                .kinds
                .iter()
                .map(|(kind, count)| RecapKindCount {
                    kind: kind.clone(),
                    label: kinds::label(kind).pick(&reader.locale).to_owned(),
                    count: *count,
                    ..Default::default()
                })
                .collect(),
            bump_count: data.bump_count,
            most_bumped_friend: most_bumped.into(),
            top_bump_friends: bump_friends,
            new_friend_count: data.new_friend_count,
            new_friends: data.new_friends.iter().filter_map(shown).take(SHOWN_NEW_FRIENDS).collect(),
            place_count: data.place_count,
            place_names: data.place_names.clone(),
            region_count: data.region_count,
            region_names: data.region_names.clone(),
            area_count: data.area_count,
            busiest_day: busiest.map(day).into(),
            days: data.days.iter().copied().map(day).collect(),
            highlights: highlights(
                data,
                most_bumped_name.as_ref().map(|(name, count)| (name.as_str(), *count)),
                &reader.locale,
            ),
            generated_at: ts(data.generated_at),
            ..Default::default()
        })
    }

    /// Tells users whose month just ended (in their time zone) that its
    /// recap is ready: one inbox notice per user and month, for users with
    /// at least `TOKULY_FEED_RECAP_MIN_ACTIVITIES` entries in that month.
    /// Returns how many notices were sent.
    pub async fn send_recap_notices(&self, now: Time) -> AppResult<u64> {
        let zones: Vec<String> = sqlx::query_scalar("SELECT DISTINCT time_zone FROM users WHERE deleted_at IS NULL")
            .fetch_all(&self.core.db)
            .await?;
        let mut sent = 0;
        for zone in zones {
            // Accounts with a time zone we do not know are treated as Tokyo's.
            let tz: Tz = zone.parse().unwrap_or(chrono_tz::Asia::Tokyo);
            let today = now.with_timezone(&tz).date_naive();
            if today.day() > NOTICE_WINDOW_DAYS {
                continue;
            }
            let Some(last_day) = today.with_day(1).and_then(|first| first.pred_opt()) else { continue };
            let Some(period) = Period::containing(RecapPeriod::RECAP_PERIOD_MONTH, last_day) else { continue };
            sent += self.send_recap_notices_for(&zone, &period, tz).await?;
        }
        Ok(sent)
    }

    async fn send_recap_notices_for(&self, zone: &str, period: &Period, tz: Tz) -> AppResult<u64> {
        let (from, to) = period.bounds(tz);
        // December's notice also announces the recap of the whole year.
        let year_end = period.start.month() == 12;
        let (title, body, path) = if year_end {
            let year = Period::containing(RecapPeriod::RECAP_PERIOD_YEAR, period.start).unwrap_or(*period);
            (
                Localized::new(
                    format!("{}年のまとめができました！", period.start.year()),
                    format!("Your {} recap is ready!", period.start.year()),
                ),
                Localized::new("この1年のTokulyをふりかえってみよう！", "Look back on your year on Tokuly!"),
                year.path(),
            )
        } else {
            (
                Localized::new(
                    format!("{}月のまとめができました！", period.start.month()),
                    format!("Your {} recap is ready!", month_name(period.start.month())),
                ),
                Localized::new("この1か月のTokulyをふりかえってみよう！", "Look back on your month on Tokuly!"),
                period.path(),
            )
        };
        let mut sent = 0;
        loop {
            let mut tx = self.core.db.begin().await?;
            // The notice row and the notification commit together: at most
            // one notice per user and month, whoever runs this.
            let users: Vec<Uuid> = sqlx::query_scalar(
                "INSERT INTO feed_recap_notices (user_id, period_start)
                 SELECT u.id, $2 FROM users u
                 WHERE u.time_zone = $1 AND u.deleted_at IS NULL AND u.banned_at IS NULL
                   AND NOT EXISTS (SELECT 1 FROM feed_recap_notices n WHERE n.user_id = u.id AND n.period_start = $2)
                   AND (SELECT count(*) FROM activities a WHERE a.user_id = u.id AND a.occurred_at >= $3 AND a.occurred_at < $4) >= $5
                 LIMIT $6
                 ON CONFLICT DO NOTHING
                 RETURNING user_id",
            )
            .bind(zone)
            .bind(period.start)
            .bind(from)
            .bind(to)
            .bind(i64::from(self.cfg.recap_min_activities.max(1)))
            .bind(NOTICE_BATCH)
            .fetch_all(&mut *tx)
            .await?;
            for user_id in &users {
                let notice =
                    NotificationRequest::new(*user_id, RECAP_READY_KIND, title.clone(), body.clone(), path.clone())
                        .inbox_only()
                        .collapse(format!("feed.recap:{}", period.start))
                        .data(serde_json::json!({
                            "period": if year_end { "year" } else { "month" },
                            "start_date": period.start.to_string(),
                            "end_date": period.end.to_string(),
                        }));
                outbound::notify(&mut *tx, &notice).await?;
            }
            tx.commit().await?;
            sent += users.len() as u64;
            if (users.len() as i64) < NOTICE_BATCH {
                return Ok(sent);
            }
        }
    }
}

impl RecapService for Feed {
    async fn get_recap(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetRecapRequest>,
    ) -> ServiceResult<GetRecapResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let kind = match request.period.as_known() {
                Some(kind) if kind != RecapPeriod::RECAP_PERIOD_UNSPECIFIED => kind,
                _ => bail!(invalid, "PERIOD_REQUIRED", "choose a month or a year"),
            };
            let time_zone: Option<String> = sqlx::query_scalar("SELECT time_zone FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&self.core.db)
                .await?;
            let tz: Tz = time_zone.and_then(|zone| zone.parse().ok()).unwrap_or(chrono_tz::Asia::Tokyo);
            let today = now().with_timezone(&tz).date_naive();
            let anchor = match request.anchor_date.trim() {
                "" => today,
                text => match NaiveDate::parse_from_str(text, "%Y-%m-%d") {
                    Ok(date) => date,
                    Err(_) => bail!(invalid, "INVALID_DATE", "the anchor date must look like 2026-10-31"),
                },
            };
            let Some(period) = Period::containing(kind, anchor) else {
                bail!(invalid, "INVALID_DATE", "the anchor date is out of range")
            };
            ensure!(period.start <= today, invalid, "RECAP_IN_FUTURE", "this period has not started yet");
            let complete = period.end < today;

            let data = self.recap_data(user_id, &period, tz, complete).await?;
            let reader = self.reader_for(user_id).await?;
            ok(GetRecapResponse {
                recap: self.recap_to_proto(&reader, &period, tz, complete, &data).await?.into(),
                ..Default::default()
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn utc(year: i32, month: u32, day: u32, hour: u32) -> Time {
        chrono::Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    #[test]
    fn periods_are_calendar_months_and_years() {
        let month = Period::containing(RecapPeriod::RECAP_PERIOD_MONTH, date(2026, 10, 17)).unwrap();
        assert_eq!((month.start, month.end), (date(2026, 10, 1), date(2026, 10, 31)));
        let february = Period::containing(RecapPeriod::RECAP_PERIOD_MONTH, date(2028, 2, 29)).unwrap();
        assert_eq!((february.start, february.end), (date(2028, 2, 1), date(2028, 2, 29)), "a leap year");
        let december = Period::containing(RecapPeriod::RECAP_PERIOD_MONTH, date(2026, 12, 31)).unwrap();
        assert_eq!((december.start, december.end), (date(2026, 12, 1), date(2026, 12, 31)));
        let year = Period::containing(RecapPeriod::RECAP_PERIOD_YEAR, date(2026, 6, 15)).unwrap();
        assert_eq!((year.start, year.end), (date(2026, 1, 1), date(2026, 12, 31)));
        assert!(Period::containing(RecapPeriod::RECAP_PERIOD_UNSPECIFIED, date(2026, 6, 15)).is_none());
    }

    #[test]
    fn period_bounds_follow_the_time_zone() {
        let october = Period::containing(RecapPeriod::RECAP_PERIOD_MONTH, date(2026, 10, 10)).unwrap();
        // Tokyo is nine hours ahead: its October starts on 30 September, 15:00 UTC.
        assert_eq!(october.bounds(chrono_tz::Asia::Tokyo), (utc(2026, 9, 30, 15), utc(2026, 10, 31, 15)));
        assert_eq!(october.bounds(chrono_tz::UTC), (utc(2026, 10, 1, 0), utc(2026, 11, 1, 0)));
        // New York: daylight time (UTC-4) at the start, standard time (UTC-5) at the end.
        assert_eq!(october.bounds(chrono_tz::America::New_York), (utc(2026, 10, 1, 4), utc(2026, 11, 1, 4)));
        let november = Period::containing(RecapPeriod::RECAP_PERIOD_MONTH, date(2026, 11, 10)).unwrap();
        assert_eq!(november.bounds(chrono_tz::America::New_York), (utc(2026, 11, 1, 4), utc(2026, 12, 1, 5)));
    }

    #[test]
    fn a_day_whose_midnight_does_not_exist_starts_when_the_clock_resumes() {
        // Cairo skipped midnight on 24 April 2026 (00:00 → 01:00).
        let start = local_midnight(date(2026, 4, 24), chrono_tz::Africa::Cairo);
        let before = local_midnight(date(2026, 4, 23), chrono_tz::Africa::Cairo);
        assert!(start > before);
        assert!((start - before).num_hours() <= 24);
    }

    #[test]
    fn titles_and_paths_name_the_period() {
        let month = Period::containing(RecapPeriod::RECAP_PERIOD_MONTH, date(2026, 10, 3)).unwrap();
        assert_eq!(
            (month.title().ja.as_str(), month.title().en.as_str()),
            ("2026年10月のまとめ", "October 2026 recap")
        );
        assert_eq!(month.path(), "/recap/2026-10");
        let year = Period::containing(RecapPeriod::RECAP_PERIOD_YEAR, date(2026, 10, 3)).unwrap();
        assert_eq!((year.title().ja.as_str(), year.title().en.as_str()), ("2026年のまとめ", "2026 recap"));
        assert_eq!(year.path(), "/recap/2026");
        assert_eq!(
            Period::containing(RecapPeriod::RECAP_PERIOD_MONTH, date(2026, 3, 3)).unwrap().path(),
            "/recap/2026-03"
        );
    }

    fn row(kind: &str, at: Time) -> RecapRow {
        RecapRow {
            kind: kind.to_owned(),
            subject_type: None,
            subject_id: None,
            with_user_ids: Vec::new(),
            lat: None,
            lng: None,
            place_name: None,
            region_name: None,
            region_code: None,
            occurred_at: at,
        }
    }

    fn bump(with: Uuid, at: Time) -> RecapRow {
        RecapRow {
            subject_type: Some(SubjectType::SUBJECT_TYPE_USER as i32),
            subject_id: Some(with),
            with_user_ids: vec![with],
            lat: Some(35.6595),
            lng: Some(139.7005),
            ..row(kinds::BUMPED, at)
        }
    }

    fn friendship(with: Uuid, at: Time) -> RecapRow {
        RecapRow {
            subject_type: Some(SubjectType::SUBJECT_TYPE_USER as i32),
            subject_id: Some(with),
            with_user_ids: vec![with],
            ..row(kinds::FRIEND_ADDED, at)
        }
    }

    fn place(id: Option<Uuid>, name: &str, at: Time) -> RecapRow {
        RecapRow {
            subject_type: id.map(|_| SubjectType::SUBJECT_TYPE_PLACE as i32),
            subject_id: id,
            place_name: (!name.is_empty()).then(|| name.to_owned()),
            lat: Some(34.70),
            lng: Some(135.50),
            ..row("geo.visit", at)
        }
    }

    fn region(code: &str, name: &str, at: Time) -> RecapRow {
        RecapRow {
            region_code: (!code.is_empty()).then(|| code.to_owned()),
            region_name: (!name.is_empty()).then(|| name.to_owned()),
            ..row("geo.new_region", at)
        }
    }

    #[test]
    fn an_empty_period_is_all_zeroes() {
        let data = aggregate(&[], chrono_tz::Asia::Tokyo, utc(2026, 11, 1, 0));
        assert_eq!(
            (
                data.activity_count,
                data.bump_count,
                data.new_friend_count,
                data.place_count,
                data.region_count,
                data.area_count
            ),
            (0, 0, 0, 0, 0, 0)
        );
        assert!(data.kinds.is_empty() && data.days.is_empty() && data.bump_friends.is_empty());
        assert!(busiest_day(&data.days).is_none());
        assert!(highlights(&data, None, "ja").is_empty());
    }

    #[test]
    fn kinds_days_and_the_busiest_day_are_counted_in_the_users_time_zone() {
        let rows = vec![
            row("note.posted", utc(2026, 10, 1, 16)),  // 2 October, 01:00 in Tokyo
            row("note.posted", utc(2026, 10, 2, 3)),   // 2 October, 12:00
            row("story.posted", utc(2026, 10, 2, 14)), // 2 October, 23:00
            row("note.posted", utc(2026, 10, 2, 15)),  // 3 October, 00:00
            row("live.started", utc(2026, 10, 20, 0)),
        ];
        let data = aggregate(&rows, chrono_tz::Asia::Tokyo, utc(2026, 11, 1, 0));
        assert_eq!(data.activity_count, 5);
        assert_eq!(
            data.kinds,
            vec![("note.posted".to_owned(), 3), ("story.posted".to_owned(), 1), ("live.started".to_owned(), 1)]
        );
        assert_eq!(data.days, vec![(date(2026, 10, 2), 3), (date(2026, 10, 3), 1), (date(2026, 10, 20), 1)]);
        assert_eq!(busiest_day(&data.days), Some((date(2026, 10, 2), 3)));
        // The same entries seen from UTC fall on other days.
        let in_utc = aggregate(&rows, chrono_tz::UTC, utc(2026, 11, 1, 0));
        assert_eq!(in_utc.days, vec![(date(2026, 10, 1), 1), (date(2026, 10, 2), 3), (date(2026, 10, 20), 1)]);
    }

    #[test]
    fn bumps_and_new_friends_are_counted_per_person() {
        let (sakura, haru, yuki) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let rows = vec![
            bump(haru, utc(2026, 10, 1, 1)),
            bump(sakura, utc(2026, 10, 2, 1)),
            bump(sakura, utc(2026, 10, 3, 1)),
            bump(sakura, utc(2026, 10, 9, 1)),
            bump(haru, utc(2026, 10, 12, 1)),
            friendship(yuki, utc(2026, 10, 5, 1)),
            friendship(haru, utc(2026, 10, 20, 1)),
            friendship(yuki, utc(2026, 10, 25, 1)), // re-added: still one new friend
        ];
        let data = aggregate(&rows, chrono_tz::UTC, utc(2026, 11, 1, 0));
        assert_eq!(data.bump_count, 5);
        assert_eq!(data.bump_friends, vec![(sakura, 3), (haru, 2)]);
        assert_eq!(data.new_friend_count, 2);
        assert_eq!(data.new_friends, vec![yuki, haru], "most recent first");
        assert_eq!(data.area_count, 1, "all bumps happened in the same cell");
    }

    #[test]
    fn places_are_counted_by_subject_or_name_and_regions_by_code() {
        let cafe = Uuid::from_u128(10);
        let rows = vec![
            place(Some(cafe), "喫茶トクリー", utc(2026, 10, 1, 1)),
            place(Some(cafe), "", utc(2026, 10, 2, 1)),
            place(Some(cafe), "喫茶トクリー", utc(2026, 10, 3, 1)),
            place(None, "大阪城公園", utc(2026, 10, 4, 1)),
            place(None, "大阪城公園 ", utc(2026, 10, 5, 1)),
            place(Some(Uuid::from_u128(11)), "", utc(2026, 10, 6, 1)), // a place nobody named
            region("JP-27", "大阪府", utc(2026, 10, 4, 2)),
            region("jp-27", "Osaka", utc(2026, 10, 5, 2)), // the same region again
            region("", "京都府", utc(2026, 10, 6, 2)),
            region("", "", utc(2026, 10, 7, 2)), // nothing to go by
            RecapRow { region_name: Some("東京都".into()), ..row("note.posted", utc(2026, 10, 8, 2)) }, // not a region entry
        ];
        let data = aggregate(&rows, chrono_tz::UTC, utc(2026, 11, 1, 0));
        assert_eq!(data.place_count, 3);
        assert_eq!(data.place_names, vec!["喫茶トクリー", "大阪城公園"]);
        assert_eq!(data.region_count, 2);
        assert_eq!(data.region_names, vec!["大阪府", "京都府"]);
        assert_eq!(data.area_count, 1);
    }

    #[test]
    fn areas_are_roughly_kilometre_cells() {
        let at = utc(2026, 10, 1, 1);
        let located = |lat: f64, lng: f64| RecapRow { lat: Some(lat), lng: Some(lng), ..row("note.posted", at) };
        let rows = vec![
            located(35.6595, 139.7005),
            located(35.6601, 139.7011),
            located(35.6895, 139.6917),
            located(-33.8688, 151.2093),
            row("note.posted", at),
        ];
        assert_eq!(aggregate(&rows, chrono_tz::UTC, at).area_count, 3);
    }

    #[test]
    fn highlights_read_naturally_in_both_languages() {
        let sakura = Uuid::from_u128(1);
        let mut rows = vec![
            bump(sakura, utc(2026, 10, 2, 1)),
            bump(sakura, utc(2026, 10, 2, 2)),
            friendship(sakura, utc(2026, 10, 1, 1)),
        ];
        rows.push(place(None, "大阪城公園", utc(2026, 10, 2, 3)));
        rows.push(region("JP-27", "大阪府", utc(2026, 10, 2, 4)));
        let data = aggregate(&rows, chrono_tz::UTC, utc(2026, 11, 1, 0));

        let ja = highlights(&data, Some(("さくら", 2)), "ja");
        let texts: Vec<&str> = ja.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "一番バンプした友達はさくらさん！(2回)",
                "新しい友達が1人できました！",
                "1つの新しい地域に足をのばしました！",
                "1か所の場所でログを残しました！",
                "一番アクティブだったのは10月2日！(4件)",
                "いちばん多かったのは「バンプ」！(2回)",
                "ぜんぶで5件のログが残りました！",
            ]
        );
        assert_eq!(ja[0].r#type, RecapHighlightType::RECAP_HIGHLIGHT_TYPE_MOST_BUMPED_FRIEND);
        assert_eq!(ja.iter().map(|line| line.value).collect::<Vec<_>>(), vec![2, 1, 1, 1, 4, 2, 5]);

        let en = highlights(&data, Some(("Sakura", 2)), "en");
        assert_eq!(en[0].text, "You bumped Sakura the most (2 times)!");
        assert_eq!(en[1].text, "You made a new friend!");
        assert_eq!(en[4].text, "Your busiest day was October 2 (4 activities)!");
        assert_eq!(en[6].text, "You logged 5 moments!");
        // Without a nameable friend the bump line is left out.
        assert_eq!(highlights(&data, None, "ja").len(), 6);
    }

    #[test]
    fn recap_numbers_survive_the_cache() {
        let rows = vec![bump(Uuid::from_u128(1), utc(2026, 10, 2, 1)), place(None, "駅", utc(2026, 10, 3, 1))];
        let data = aggregate(&rows, chrono_tz::Asia::Tokyo, utc(2026, 11, 1, 0));
        let cached: RecapData = serde_json::from_slice(&serde_json::to_vec(&data).unwrap()).unwrap();
        assert_eq!(cached, data);
    }
}
