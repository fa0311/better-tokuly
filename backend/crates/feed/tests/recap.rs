//! Recaps: monthly and yearly numbers from the caller's own log, their
//! cache, and the month-end notice.
mod common;

use chrono::{DateTime, Datelike as _, Duration, NaiveDate, TimeZone as _, Utc};
use chrono_tz::Tz;
use common::*;
use tokuly_core::outbound::ActivityAudience;
use tokuly_feed::FeedConfig;
use tokuly_proto::{common::SubjectType, feed::*};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

const TOKYO: Tz = chrono_tz::Asia::Tokyo;

async fn recap(ctx: &Ctx, user: Option<&TestUser>, period: RecapPeriod, anchor: &str) -> Result<Recap, RpcError> {
    let response: GetRecapResponse = ctx
        .call(
            RECAP,
            "GetRecap",
            user,
            &GetRecapRequest { period: period.into(), anchor_date: anchor.to_owned(), ..Default::default() },
        )
        .await?;
    Ok(response.recap.into_option().expect("a recap"))
}

async fn month(ctx: &Ctx, user: &TestUser, anchor: NaiveDate) -> Recap {
    recap(ctx, Some(user), RecapPeriod::RECAP_PERIOD_MONTH, &anchor.to_string()).await.expect("month recap")
}

/// The instant of a local wall-clock time in a time zone.
fn at(tz: Tz, date: NaiveDate, hour: u32, minute: u32) -> DateTime<Utc> {
    tz.from_local_datetime(&date.and_hms_opt(hour, minute, 0).unwrap())
        .earliest()
        .expect("an existing local time")
        .with_timezone(&Utc)
}

/// The first day of the month before the one `today` is in.
fn previous_month(today: NaiveDate) -> NaiveDate {
    today.with_day(1).unwrap().pred_opt().unwrap().with_day(1).unwrap()
}

fn last_day(first: NaiveDate) -> NaiveDate {
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    };
    next.unwrap().pred_opt().unwrap()
}

fn texts(recap: &Recap) -> Vec<&str> {
    recap.highlights.iter().map(|line| line.text.as_str()).collect()
}

#[tokio::test]
async fn a_month_recap_is_built_from_the_callers_own_log() {
    let ctx = start().await;
    let (me, sakura, haru, yuki, other) = (
        ctx.app.user("me").await,
        ctx.app.user("さくら").await,
        ctx.app.user("はる").await,
        ctx.app.user("ゆき").await,
        ctx.app.user("other").await,
    );
    let first = previous_month(Utc::now().with_timezone(&TOKYO).date_naive());
    let last = last_day(first);
    let day = |n: u32| first.with_day(n).unwrap();
    let place = Uuid::now_v7();
    let visit = |name: &str| {
        entry(&me, "geo.visit", ActivityAudience::Private)
            .subject(SubjectType::SUBJECT_TYPE_PLACE, place)
            .at(35.6595, 139.7005)
            .data(serde_json::json!({ "place_name": name }))
    };

    // Day 2 is the busy one.
    ctx.record_at(bump(&me, &sakura, 35.6595, 139.7005), at(TOKYO, day(2), 10, 0)).await;
    ctx.record_at(bump(&me, &sakura, 35.6595, 139.7005), at(TOKYO, day(2), 15, 0)).await;
    ctx.record_at(visit("渋谷スクランブル交差点"), at(TOKYO, day(2), 16, 0)).await;
    ctx.record_at(note(&me), at(TOKYO, day(2), 23, 59)).await;
    ctx.record_at(bump(&me, &sakura, 35.6595, 139.7005), at(TOKYO, day(5), 12, 0)).await;
    ctx.record_at(bump(&me, &haru, 34.70, 135.50), at(TOKYO, day(6), 12, 0)).await;
    ctx.record_at(
        entry(&me, "social.friend_added", ActivityAudience::Friends)
            .subject(SubjectType::SUBJECT_TYPE_USER, yuki.id)
            .with([yuki.id]),
        at(TOKYO, day(7), 9, 0),
    )
    .await;
    ctx.record_at(visit(""), at(TOKYO, day(8), 9, 0)).await;
    ctx.record_at(
        entry(&me, "geo.new_region", ActivityAudience::Friends)
            .data(serde_json::json!({ "region_code": "JP-27", "region_name": "大阪府" }))
            .at(34.70, 135.50),
        at(TOKYO, day(8), 10, 0),
    )
    .await;
    // The very first and very last minutes of the month count...
    ctx.record_at(note(&me), at(TOKYO, first, 0, 0)).await;
    ctx.record_at(note(&me), at(TOKYO, last, 23, 59)).await;
    // ...what happened just before or after does not, nor does anyone else's log.
    ctx.record_at(note(&me), at(TOKYO, first, 0, 0) - Duration::minutes(1)).await;
    ctx.record_at(note(&me), at(TOKYO, last, 23, 59) + Duration::minutes(2)).await;
    ctx.record_at(bump(&other, &sakura, 35.0, 139.0), at(TOKYO, day(3), 12, 0)).await;

    let recap = month(&ctx, &me, day(17)).await;
    assert_eq!(recap.period, RecapPeriod::RECAP_PERIOD_MONTH);
    assert_eq!(recap.title, format!("{}年{}月のまとめ", first.year(), first.month()));
    assert_eq!(
        (recap.start_date.clone(), recap.end_date.clone(), recap.time_zone.as_str()),
        (first.to_string(), last.to_string(), "Asia/Tokyo")
    );
    assert!(recap.complete, "a month that is over");
    assert_eq!(recap.activity_count, 11);
    assert_eq!(recap.active_day_count, 7);

    // Kinds, most frequent first, with their labels.
    let kinds: Vec<(&str, &str, i32)> =
        recap.kinds.iter().map(|k| (k.kind.as_str(), k.label.as_str(), k.count)).collect();
    assert_eq!(kinds[0], ("social.bumped", "バンプ", 4));
    assert_eq!(kinds[1], ("note.posted", "ノート", 3));
    assert!(
        kinds.contains(&("geo.visit", "訪れた場所", 2))
            && kinds.contains(&("geo.new_region", "新しい地域", 1))
            && kinds.contains(&("social.friend_added", "友達追加", 1))
    );

    // Bumps and friends.
    assert_eq!(recap.bump_count, 4);
    let most = recap.most_bumped_friend.as_option().expect("a most-bumped friend");
    assert_eq!((most.user.as_option().unwrap().id.as_str(), most.count), (sakura.id.to_string().as_str(), 3));
    assert_eq!(
        recap
            .top_bump_friends
            .iter()
            .map(|f| (f.user.as_option().unwrap().display_name.as_str(), f.count))
            .collect::<Vec<_>>(),
        vec![("さくら", 3), ("はる", 1)]
    );
    assert_eq!(recap.new_friend_count, 1);
    assert_eq!(recap.new_friends.iter().map(|u| u.id.clone()).collect::<Vec<_>>(), vec![yuki.id.to_string()]);

    // Places, regions and areas come from what the entries carry.
    assert_eq!((recap.place_count, recap.place_names.clone()), (1, vec!["渋谷スクランブル交差点".to_owned()]));
    assert_eq!((recap.region_count, recap.region_names.clone()), (1, vec!["大阪府".to_owned()]));
    assert_eq!(recap.area_count, 2, "Shibuya and Osaka");

    // Days, for the calendar.
    let busiest = recap.busiest_day.as_option().expect("a busiest day");
    assert_eq!((busiest.date.clone(), busiest.count), (day(2).to_string(), 4));
    assert_eq!(recap.days.first().map(|d| (d.date.clone(), d.count)), Some((first.to_string(), 1)));
    assert_eq!(recap.days.iter().map(|d| d.count).sum::<i32>(), 11);
    assert!(recap.days.windows(2).all(|pair| pair[0].date < pair[1].date), "in calendar order");

    // The sentences, most interesting first.
    assert_eq!(
        texts(&recap),
        vec![
            "一番バンプした友達はさくらさん！(3回)".to_owned(),
            "新しい友達が1人できました！".to_owned(),
            "1つの新しい地域に足をのばしました！".to_owned(),
            "1か所の場所でログを残しました！".to_owned(),
            format!("一番アクティブだったのは{}月2日！(4件)", first.month()),
            "いちばん多かったのは「バンプ」！(4回)".to_owned(),
            "ぜんぶで11件のログが残りました！".to_owned(),
        ]
    );
    assert_eq!(recap.highlights[0].r#type, RecapHighlightType::RECAP_HIGHLIGHT_TYPE_MOST_BUMPED_FRIEND);
    assert_eq!(recap.highlights[0].value, 3);
    assert!(recap.generated_at.as_option().is_some());

    // In English for a reader who reads English.
    ctx.set_locale(&me, "en").await;
    let english = month(&ctx, &me, day(1)).await;
    assert!(
        english.title.ends_with(&format!("{} recap", first.year())) && !english.title.contains('年'),
        "{}",
        english.title
    );
    assert_eq!(english.kinds[0].label, "Bumps");
    assert_eq!(texts(&english)[0], "You bumped さくら the most (3 times)!");
    assert_eq!(texts(&english)[1], "You made a new friend!");

    // Somebody else's recap is their own.
    let theirs = month(&ctx, &other, day(17)).await;
    assert_eq!((theirs.activity_count, theirs.bump_count), (1, 1));
}

#[tokio::test]
async fn periods_are_validated_and_running_periods_are_marked() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    let today = Utc::now().with_timezone(&TOKYO).date_naive();

    // Nothing logged: zeroes, no sentences, and the running month is not complete.
    let empty = recap(&ctx, Some(&me), RecapPeriod::RECAP_PERIOD_MONTH, "").await.unwrap();
    assert_eq!((empty.activity_count, empty.active_day_count, empty.bump_count, empty.place_count), (0, 0, 0, 0));
    assert!(empty.highlights.is_empty() && empty.days.is_empty() && empty.kinds.is_empty());
    assert!(empty.busiest_day.as_option().is_none() && empty.most_bumped_friend.as_option().is_none());
    assert!(!empty.complete);
    assert_eq!(empty.start_date, today.with_day(1).unwrap().to_string(), "no anchor means today");

    // The running year: everything so far.
    ctx.record(note(&me), 5).await;
    let year = recap(&ctx, Some(&me), RecapPeriod::RECAP_PERIOD_YEAR, "").await.unwrap();
    assert_eq!(year.period, RecapPeriod::RECAP_PERIOD_YEAR);
    assert_eq!(
        (year.start_date.clone(), year.end_date.clone()),
        (format!("{}-01-01", today.year()), format!("{}-12-31", today.year()))
    );
    assert_eq!(year.title, format!("{}年のまとめ", today.year()));
    assert!(!year.complete);
    assert_eq!(year.activity_count, 1);
    // Last year is over (and was quiet).
    let last_year =
        recap(&ctx, Some(&me), RecapPeriod::RECAP_PERIOD_YEAR, &format!("{}-07-01", today.year() - 1)).await.unwrap();
    assert!(last_year.complete);
    assert_eq!(last_year.activity_count, 0);

    // Validation.
    let error = recap(&ctx, Some(&me), RecapPeriod::RECAP_PERIOD_UNSPECIFIED, "").await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("invalid_argument", "PERIOD_REQUIRED"));
    for bad in ["yesterday", "2026-13-01", "2026-02-30", "20261101", "2026/11/01"] {
        assert_eq!(
            recap(&ctx, Some(&me), RecapPeriod::RECAP_PERIOD_MONTH, bad).await.unwrap_err().reason,
            "INVALID_DATE",
            "{bad}"
        );
    }
    let next_year = format!("{}-01-15", today.year() + 1);
    assert_eq!(
        recap(&ctx, Some(&me), RecapPeriod::RECAP_PERIOD_MONTH, &next_year).await.unwrap_err().reason,
        "RECAP_IN_FUTURE"
    );
    assert_eq!(
        recap(&ctx, Some(&me), RecapPeriod::RECAP_PERIOD_YEAR, &next_year).await.unwrap_err().reason,
        "RECAP_IN_FUTURE"
    );
    // A later day of the running month is still this month.
    let end_of_month = last_day(today.with_day(1).unwrap());
    assert!(recap(&ctx, Some(&me), RecapPeriod::RECAP_PERIOD_MONTH, &end_of_month.to_string()).await.is_ok());
    assert_eq!(recap(&ctx, None, RecapPeriod::RECAP_PERIOD_MONTH, "").await.unwrap_err().code, "unauthenticated");
}

#[tokio::test]
async fn months_are_counted_in_the_callers_time_zone() {
    let ctx = start().await;
    let (tokyo, angeles) = (ctx.app.user("tokyo").await, ctx.app.user("angeles").await);
    ctx.set_time_zone(&angeles, "America/Los_Angeles").await;
    let year = Utc::now().year() - 1;
    // 1 November, 03:00 UTC: already noon in Tokyo, still 31 October in Los Angeles.
    let moment = Utc.with_ymd_and_hms(year, 11, 1, 3, 0, 0).unwrap();
    for user in [&tokyo, &angeles] {
        ctx.record_at(note(user), moment).await;
    }
    let october = NaiveDate::from_ymd_opt(year, 10, 15).unwrap();
    let november = NaiveDate::from_ymd_opt(year, 11, 15).unwrap();

    assert_eq!(
        (month(&ctx, &tokyo, october).await.activity_count, month(&ctx, &tokyo, november).await.activity_count),
        (0, 1)
    );
    let la_october = month(&ctx, &angeles, october).await;
    assert_eq!((la_october.activity_count, month(&ctx, &angeles, november).await.activity_count), (1, 0));
    assert_eq!(la_october.time_zone, "America/Los_Angeles");
    assert_eq!(
        la_october.days.iter().map(|d| d.date.clone()).collect::<Vec<_>>(),
        vec![format!("{year}-10-31")],
        "the day is the local day"
    );
}

#[tokio::test]
async fn recaps_are_cached_but_deletions_and_blocks_show_at_once() {
    let ctx = start().await;
    let (me, sakura, haru) = (ctx.app.user("me").await, ctx.app.user("さくら").await, ctx.app.user("はる").await);
    let first = ctx.record(bump(&me, &sakura, 35.0, 139.0), 30).await;
    ctx.record(bump(&me, &sakura, 35.0, 139.0), 20).await;
    ctx.record(bump(&me, &haru, 35.0, 139.0), 10).await;
    let today = Utc::now().with_timezone(&TOKYO).date_naive();
    // (Around midnight the three entries may straddle two months; a year never does in practice.)
    let now_recap = || {
        let (ctx, me) = (&ctx, &me);
        async move { recap(ctx, Some(me), RecapPeriod::RECAP_PERIOD_YEAR, &today.to_string()).await.unwrap() }
    };

    let before = now_recap().await;
    assert_eq!(before.bump_count, 3);
    assert_eq!(before.most_bumped_friend.as_option().unwrap().user.as_option().unwrap().id, sakura.id.to_string());

    // New entries wait for the cache to expire (five minutes by default)...
    ctx.record(note(&me), 2).await;
    ctx.record(note(&me), 1).await;
    let cached = now_recap().await;
    assert_eq!(cached.activity_count, 3);
    assert_eq!(cached.generated_at, before.generated_at, "served from the cache");

    // ...but deleting something is reflected immediately.
    let _: DeleteActivityResponse = ctx
        .call(
            ACTIVITY,
            "DeleteActivity",
            Some(&me),
            &DeleteActivityRequest { activity_id: first.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let after_delete = now_recap().await;
    assert_eq!((after_delete.activity_count, after_delete.bump_count), (4, 2));
    // Sakura and Haru are tied now; the friend bumped first leads.
    assert_eq!(after_delete.top_bump_friends.len(), 2);

    // A block hides the person from the (cached) recap right away: names are resolved per request.
    ctx.block(&me, &sakura).await;
    let after_block = now_recap().await;
    assert_eq!(after_block.generated_at, after_delete.generated_at, "the numbers are still the cached ones");
    assert_eq!(
        after_block.top_bump_friends.iter().map(|f| f.user.as_option().unwrap().id.clone()).collect::<Vec<_>>(),
        vec![haru.id.to_string()]
    );
    assert_eq!(after_block.most_bumped_friend.as_option().unwrap().user.as_option().unwrap().id, haru.id.to_string());
    assert!(texts(&after_block)[0].contains("はる"), "{:?}", texts(&after_block));
    assert_eq!(after_block.bump_count, 2, "the count itself is the caller's own history");

    // Clearing the log empties the recap.
    let _: ClearActivityResponse = ctx
        .call(ACTIVITY, "ClearActivity", Some(&me), &ClearActivityRequest { everything: true, ..Default::default() })
        .await
        .unwrap();
    assert_eq!(now_recap().await.activity_count, 0);
}

#[tokio::test]
async fn a_running_period_is_recomputed_after_the_configured_time() {
    let ctx = start_with(FeedConfig { recap_cache_seconds: 1, ..test_config() }).await;
    let me = ctx.app.user("me").await;
    ctx.record(note(&me), 3).await;
    let first = recap(&ctx, Some(&me), RecapPeriod::RECAP_PERIOD_YEAR, "").await.unwrap();
    assert_eq!(first.activity_count, 1);
    ctx.record(note(&me), 2).await;
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    let second = recap(&ctx, Some(&me), RecapPeriod::RECAP_PERIOD_YEAR, "").await.unwrap();
    assert_eq!(second.activity_count, 2);
    assert_ne!(second.generated_at, first.generated_at);
}

/// `notify.requested` events of kind `feed.recap_ready` addressed to `user`.
async fn recap_notices(ctx: &Ctx, user: &TestUser) -> Vec<tokuly_core::events::Event> {
    ctx.app
        .events("notify.requested")
        .await
        .into_iter()
        .filter(|e| e.data["user_id"] == serde_json::json!(user.id) && e.data["kind"] == "feed.recap_ready")
        .collect()
}

#[tokio::test]
async fn a_recap_notice_goes_out_once_when_a_month_has_ended() {
    let ctx = start().await;
    let (active, quiet, banned, angeles) = (
        ctx.app.user("active").await,
        ctx.app.user("quiet").await,
        ctx.app.user("banned").await,
        ctx.app.user("angeles").await,
    );
    ctx.set_time_zone(&angeles, "America/Los_Angeles").await;
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(banned.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    // A June two years ago, so the test does not depend on today's date.
    let year = Utc::now().year() - 2;
    let june = |day: u32, hour: u32| at(TOKYO, NaiveDate::from_ymd_opt(year, 6, day).unwrap(), hour, 0);
    // (Mid-month days: the same day in Tokyo and in Los Angeles.)
    for user in [&active, &banned, &angeles] {
        for day in 10..=14 {
            ctx.record_at(note(user), june(day, 12)).await;
        }
    }
    for day in 10..=13 {
        ctx.record_at(note(&quiet), june(day, 12)).await;
    }

    // Mid-June: nothing has ended.
    ctx.feed.send_recap_notices(june(20, 12)).await.unwrap();
    assert!(recap_notices(&ctx, &active).await.is_empty());

    // 1 July, 09:00 in Tokyo (still 30 June in Los Angeles).
    let july_first = at(TOKYO, NaiveDate::from_ymd_opt(year, 7, 1).unwrap(), 9, 0);
    assert!(ctx.feed.send_recap_notices(july_first).await.unwrap() >= 1);
    let notices = recap_notices(&ctx, &active).await;
    assert_eq!(notices.len(), 1);
    let notice = &notices[0].data;
    assert_eq!(notice["title"]["ja"], "6月のまとめができました！");
    assert_eq!(notice["title"]["en"], "Your June recap is ready!");
    assert_eq!(notice["path"], format!("/recap/{year}-06"));
    assert_eq!((notice["inbox"].as_bool(), notice["push"].as_bool()), (Some(true), Some(false)), "inbox only");
    assert_eq!(notice["collapse_key"], format!("feed.recap:{year}-06-01"));
    assert_eq!(notice["data"]["period"], "month");
    assert_eq!(
        (notice["data"]["start_date"].as_str(), notice["data"]["end_date"].as_str()),
        (Some(format!("{year}-06-01").as_str()), Some(format!("{year}-06-30").as_str()))
    );
    assert!(notice["actor_id"].is_null(), "a system notice");
    // Too few entries, a banned account, a time zone where the month is not over: no notice.
    assert!(recap_notices(&ctx, &quiet).await.is_empty());
    assert!(recap_notices(&ctx, &banned).await.is_empty());
    assert!(recap_notices(&ctx, &angeles).await.is_empty());

    // Running again (another worker, the next hour) sends nothing twice.
    ctx.feed.send_recap_notices(july_first + Duration::hours(1)).await.unwrap();
    assert_eq!(recap_notices(&ctx, &active).await.len(), 1);
    // Later that day the month has ended in Los Angeles as well.
    ctx.feed.send_recap_notices(july_first + Duration::hours(12)).await.unwrap();
    assert_eq!(recap_notices(&ctx, &angeles).await.len(), 1);
    assert_eq!(recap_notices(&ctx, &active).await.len(), 1);

    // The notice is only sent in the first days of the next month.
    let late = ctx.app.user("late").await;
    for day in 10..=14 {
        ctx.record_at(note(&late), june(day, 12)).await;
    }
    ctx.feed.send_recap_notices(at(TOKYO, NaiveDate::from_ymd_opt(year, 7, 10).unwrap(), 9, 0)).await.unwrap();
    assert!(recap_notices(&ctx, &late).await.is_empty());
}

#[tokio::test]
async fn decembers_notice_announces_the_year() {
    let ctx = start().await;
    let user = ctx.app.user("yearly").await;
    let year = Utc::now().year() - 3;
    for day in 1..=5 {
        ctx.record_at(note(&user), at(TOKYO, NaiveDate::from_ymd_opt(year, 12, day).unwrap(), 12, 0)).await;
    }
    ctx.feed.send_recap_notices(at(TOKYO, NaiveDate::from_ymd_opt(year + 1, 1, 2).unwrap(), 9, 0)).await.unwrap();
    let notices = recap_notices(&ctx, &user).await;
    assert_eq!(notices.len(), 1);
    let notice = &notices[0].data;
    assert_eq!(notice["title"]["ja"], format!("{year}年のまとめができました！"));
    assert_eq!(notice["path"], format!("/recap/{year}"));
    assert_eq!(notice["data"]["period"], "year");
    assert_eq!(notice["data"]["start_date"], format!("{year}-12-01"));

    // The page the notice links to exists.
    let recap = recap(&ctx, Some(&user), RecapPeriod::RECAP_PERIOD_YEAR, &format!("{year}-12-31")).await.unwrap();
    assert_eq!((recap.activity_count, recap.complete), (5, true));
}
