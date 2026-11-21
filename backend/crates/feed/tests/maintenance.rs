//! Background work: account deletion, content takedowns, the retention
//! sweep, the personal data export and the limits on news sources.
mod common;

use chrono::{Duration, Utc};
use common::*;
use tokuly_core::outbound::ActivityAudience;
use tokuly_feed::FeedConfig;
use tokuly_proto::{common::SubjectType, feed::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// A news source and one article of it, written straight to the database.
async fn article_row(ctx: &Ctx, t: &str, tags: &[&str]) -> (Uuid, Uuid) {
    let db = &ctx.app.core.db;
    let source: Uuid = sqlx::query_scalar("INSERT INTO news_sources (name, feed_url, last_success_at, document_at) VALUES ('Direct', $1, now(), now()) RETURNING id")
        .bind(format!("https://direct-{t}.example/feed.xml"))
        .fetch_one(db)
        .await
        .unwrap();
    let article: Uuid = sqlx::query_scalar(
        "INSERT INTO news_articles (source_id, canonical_url, url, title, tags, published_at) VALUES ($1, $2, $2, 'Direct article', $3, now()) RETURNING id",
    )
    .bind(source)
    .bind(format!("https://direct-{t}.example/article"))
    .bind(tags.iter().map(|tag| (*tag).to_owned()).collect::<Vec<_>>())
    .fetch_one(db)
    .await
    .unwrap();
    (source, article)
}

async fn count(ctx: &Ctx, sql: &str, user: Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned())).bind(user).fetch_one(&ctx.app.core.db).await.unwrap()
}

async fn hide_kind(ctx: &Ctx, user: &TestUser, kind: &str) {
    let _: SetActivityKindVisibilityResponse = ctx
        .call(
            ACTIVITY,
            "SetActivityKindVisibility",
            Some(user),
            &SetActivityKindVisibilityRequest { kind: kind.to_owned(), visible_to_others: false, ..Default::default() },
        )
        .await
        .unwrap();
}

async fn bump_count_this_year(ctx: &Ctx, user: &TestUser) -> i32 {
    let response: GetRecapResponse = ctx
        .call(
            RECAP,
            "GetRecap",
            Some(user),
            &GetRecapRequest { period: RecapPeriod::RECAP_PERIOD_YEAR.into(), ..Default::default() },
        )
        .await
        .unwrap();
    response.recap.as_option().unwrap().bump_count
}

#[tokio::test]
async fn deleting_an_account_removes_every_trace_of_it() {
    let ctx = start().await;
    let t = token();
    let (gone, friend, bystander) =
        (ctx.app.user("gone").await, ctx.app.user("friend").await, ctx.app.user("bystander").await);
    ctx.befriend(&gone, &friend).await;

    // What the account itself left behind.
    ctx.record(note(&gone), 50).await;
    let gone_story = ctx.record(entry(&gone, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), 40).await;
    hide_kind(&ctx, &gone, "wallet").await;
    let friend_story = ctx.record(entry(&friend, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), 30).await;
    let _: MarkMapStorySeenResponse = ctx
        .call(
            STORIES,
            "MarkMapStorySeen",
            Some(&gone),
            &MarkMapStorySeenRequest { activity_ids: vec![friend_story.to_string()], ..Default::default() },
        )
        .await
        .unwrap();
    let _: MarkMapStorySeenResponse = ctx
        .call(
            STORIES,
            "MarkMapStorySeen",
            Some(&friend),
            &MarkMapStorySeenRequest { activity_ids: vec![gone_story.to_string()], ..Default::default() },
        )
        .await
        .unwrap();
    let (_, article) = article_row(&ctx, &t, &["tag-a", "tag-b"]).await;
    let _: RecordNewsClickResponse = ctx
        .call(
            NEWS,
            "RecordNewsClick",
            Some(&gone),
            &RecordNewsClickRequest { article_id: article.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let _: SendNewsFeedbackResponse = ctx
        .call(
            NEWS,
            "SendNewsFeedback",
            Some(&gone),
            &SendNewsFeedbackRequest {
                article_id: article.to_string(),
                feedback: NewsFeedback::NEWS_FEEDBACK_MORE.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO feed_recap_notices (user_id, period_start) VALUES ($1, '2026-01-01')")
        .bind(gone.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    // What other people's logs say about the account: its name is in those summaries.
    ctx.record(bump(&friend, &gone, 35.0, 139.0), 20).await;
    ctx.record(
        entry(&friend, "social.friend_added", ActivityAudience::Friends)
            .subject(SubjectType::SUBJECT_TYPE_USER, gone.id)
            .with([gone.id]),
        15,
    )
    .await;
    let friend_note = ctx.record(note(&friend), 10).await;
    let bystander_note = ctx.record(note(&bystander), 5).await;
    assert_eq!(bump_count_this_year(&ctx, &friend).await, 1, "cached for the friend before the deletion");

    let deleted = event("identity.user.deleted", Some(gone.id), serde_json::json!({ "user_id": gone.id }));
    ctx.feed.handle_event(deleted.clone()).await.unwrap();
    ctx.feed.handle_event(deleted).await.unwrap();

    for (table, sql) in [
        (
            "activities",
            "SELECT count(*) FROM activities WHERE user_id = $1 OR with_user_ids @> ARRAY[$1]::uuid[] OR subject_id = $1",
        ),
        ("hidden kinds", "SELECT count(*) FROM activity_hidden_kinds WHERE user_id = $1"),
        ("story views", "SELECT count(*) FROM map_story_views WHERE viewer_id = $1 OR author_id = $1"),
        ("news clicks", "SELECT count(*) FROM news_clicks WHERE user_id = $1"),
        ("news feedback", "SELECT count(*) FROM news_feedback WHERE user_id = $1"),
        ("news interests", "SELECT count(*) FROM news_tag_affinity WHERE user_id = $1"),
        ("recap notices", "SELECT count(*) FROM feed_recap_notices WHERE user_id = $1"),
    ] {
        assert_eq!(count(&ctx, sql, gone.id).await, 0, "{table}");
    }
    // Other people keep what does not involve the account; their recaps are recomputed.
    assert_eq!(ids(&ctx.my_log(&friend).await), [friend_note, friend_story].map(|id| id.to_string()));
    assert_eq!(ids(&ctx.my_log(&bystander).await), vec![bystander_note.to_string()]);
    assert_eq!(bump_count_this_year(&ctx, &friend).await, 0);
    // The article itself is untouched.
    let still: i64 = sqlx::query_scalar("SELECT click_count FROM news_articles WHERE id = $1")
        .bind(article)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(still, 1);
}

#[tokio::test]
async fn content_taken_down_by_moderators_leaves_every_log_and_feed() {
    let ctx = start().await;
    let (me, anna, ben) = (ctx.app.user("me").await, ctx.app.user("anna").await, ctx.app.user("ben").await);
    ctx.befriend(&me, &anna).await;
    ctx.befriend(&me, &ben).await;
    let (removed_video, other_video) = (Uuid::now_v7(), Uuid::now_v7());
    let about = |user: &TestUser, kind: &str, video: Uuid| {
        entry(user, kind, ActivityAudience::Friends).subject(SubjectType::SUBJECT_TYPE_VIDEO, video)
    };
    // (Hours apart from Anna's other video, so the two are separate cards.)
    ctx.record(about(&anna, "video.published", removed_video), 400).await;
    ctx.record(about(&ben, "video.shared", removed_video), 30).await;
    let kept = ctx.record(about(&anna, "video.published", other_video), 20).await;
    // The same ID under another subject type is a different thing.
    let note_with_same_id = ctx
        .record(
            entry(&ben, "note.posted", ActivityAudience::Friends)
                .subject(SubjectType::SUBJECT_TYPE_NOTE, removed_video),
            10,
        )
        .await;
    assert_eq!(ctx.friends_feed(&me).await.len(), 4);

    let moderator = Uuid::now_v7();
    let takedown = |subject_type: SubjectType, id: Uuid| {
        event(
            "admin.content.removed",
            Some(moderator),
            serde_json::json!({ "subject_type": subject_type as i32, "subject_id": id, "reason": "規約違反", "moderator_id": moderator }),
        )
    };
    ctx.feed.handle_event(takedown(SubjectType::SUBJECT_TYPE_VIDEO, removed_video)).await.unwrap();
    ctx.feed.handle_event(takedown(SubjectType::SUBJECT_TYPE_VIDEO, removed_video)).await.unwrap();

    assert_eq!(
        ctx.friends_feed(&me).await.iter().map(head_of).collect::<Vec<_>>(),
        [note_with_same_id, kept].map(|id| id.to_string())
    );
    assert_eq!(ctx.home_feed(&me).await.len(), 2);
    assert_eq!(ids(&ctx.my_log(&anna).await), vec![kept.to_string()], "the owner's own log loses the card too");
    assert_eq!(ctx.my_log(&ben).await.len(), 1);

    // A profile takedown does not erase what people did together.
    let bumped = ctx.record(bump(&anna, &ben, 35.0, 139.0), 5).await;
    ctx.feed.handle_event(takedown(SubjectType::SUBJECT_TYPE_USER, ben.id)).await.unwrap();
    assert!(ctx.my_log(&anna).await.iter().any(|activity| activity.id == bumped.to_string()));
    // Takedowns of things nobody logged are fine.
    ctx.feed.handle_event(takedown(SubjectType::SUBJECT_TYPE_SHORT, Uuid::now_v7())).await.unwrap();
}

#[tokio::test]
async fn the_retention_sweep_removes_only_what_is_past_its_time() {
    let ctx = start().await;
    let t = token();
    let db = &ctx.app.core.db;
    let (user, viewer) = (ctx.app.user("user").await, ctx.app.user("viewer").await);
    let now = Utc::now();

    // Activities: one past the default 400 days, one just inside.
    let ancient = ctx.record_at(note(&user), now - Duration::days(401)).await;
    let old = ctx.record_at(note(&user), now - Duration::days(399)).await;
    let story = ctx.record(entry(&user, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), 5).await;
    let older_story = ctx.record(entry(&user, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), 6).await;
    // Seen marks: the story window (48 h) plus a day of grace.
    sqlx::query("INSERT INTO map_story_views (viewer_id, activity_id, author_id, seen_at) VALUES ($1, $2, $3, now() - interval '73 hours'), ($1, $4, $3, now() - interval '71 hours')")
        .bind(viewer.id)
        .bind(older_story)
        .bind(user.id)
        .bind(story)
        .execute(db)
        .await
        .unwrap();
    // Markers of handled events: kept a day longer than the stream keeps events.
    let (stale_event, fresh_event) = (Uuid::now_v7(), Uuid::now_v7());
    sqlx::query("INSERT INTO feed_processed_events (event_id, processed_at) VALUES ($1, now() - interval '9 days'), ($2, now() - interval '7 days')").bind(stale_event).bind(fresh_event).execute(db).await.unwrap();
    sqlx::query("INSERT INTO feed_recap_notices (user_id, period_start, created_at) VALUES ($1, '2024-01-01', now() - interval '401 days'), ($1, '2026-01-01', now())").bind(user.id).execute(db).await.unwrap();

    // News articles, 40 days old (retention: 30) unless said otherwise.
    let source = |name: &str, enabled: bool, success_days_ago: i32, document_days_ago: i32| {
        let feed_url = format!("https://{name}-{t}.example/feed.xml");
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO news_sources (name, feed_url, enabled, last_success_at, document_at)
                 VALUES ('s', $1, $2, now() - make_interval(days => $3), now() - make_interval(days => $4)) RETURNING id",
            )
            .bind(feed_url)
            .bind(enabled)
            .bind(success_days_ago)
            .bind(document_days_ago)
            .fetch_one(db)
            .await
            .unwrap()
        }
    };
    let article = |source: Uuid, name: &str, published_days_ago: i32, seen_hours_ago: i32| {
        let url = format!("https://articles-{t}.example/{name}");
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO news_articles (source_id, canonical_url, url, title, published_at, last_seen_at)
                 VALUES ($1, $2, $2, 'old news', now() - make_interval(days => $3), now() - make_interval(hours => $4)) RETURNING id",
            )
            .bind(source)
            .bind(url)
            .bind(published_days_ago)
            .bind(seen_hours_ago)
            .fetch_one(db)
            .await
            .unwrap()
        }
    };
    let healthy = source("healthy", true, 0, 0).await;
    let left_feed = article(healthy, "left-feed", 40, 48).await; // not in the document read today
    let still_in_feed = article(healthy, "still-in-feed", 40, 1).await; // old, but the feed still carries it
    let recent = article(healthy, "recent", 3, 48).await; // left the feed, but young
    let switched_off = source("off", false, 0, 0).await;
    let of_disabled = article(switched_off, "disabled", 40, 1).await;
    let dead = source("dead", true, 8, 8).await; // has not answered for over a week
    let of_dead = article(dead, "dead", 40, 8 * 24).await;
    let unchanged = source("unchanged", true, 0, 5).await; // only "304 Not Modified" for five days
    let of_unchanged = article(unchanged, "unchanged", 40, 5 * 24).await; // was in the last document read

    let exists = |table: &str, column: &str, id: Uuid| {
        let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {column} = $1)");
        async move { sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(sql)).bind(id).fetch_one(db).await.unwrap() }
    };

    // With activity retention switched off (0), old entries stay; the rest is swept.
    let keep_all = start_with(FeedConfig { activity_retention_days: 0, ..test_config() }).await;
    let report = keep_all.feed.run_retention().await.unwrap();
    assert_eq!(report.activities, 0);
    assert!(exists("activities", "id", ancient).await);
    assert!(
        report.story_views >= 1 && report.processed_events >= 1 && report.recap_notices >= 1 && report.articles >= 3,
        "{report:?}"
    );

    // Seen marks, event markers, notices.
    let views: Vec<Uuid> = sqlx::query_scalar("SELECT activity_id FROM map_story_views WHERE viewer_id = $1")
        .bind(viewer.id)
        .fetch_all(db)
        .await
        .unwrap();
    assert_eq!(views, vec![story]);
    assert!(
        !exists("feed_processed_events", "event_id", stale_event).await
            && exists("feed_processed_events", "event_id", fresh_event).await
    );
    let notices: i64 = sqlx::query_scalar("SELECT count(*) FROM feed_recap_notices WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(notices, 1);

    // Articles: old and gone from their feed, or of a source that cannot vouch for them.
    for (name, id, kept) in [
        ("left the feed", left_feed, false),
        ("still in the feed", still_in_feed, true),
        ("recent", recent, true),
        ("disabled source", of_disabled, false),
        ("dead source", of_dead, false),
        ("unchanged feed", of_unchanged, true),
    ] {
        assert_eq!(exists("news_articles", "id", id).await, kept, "{name}");
    }
    // Pruned articles leave the search index.
    let removals = ctx.search_changes(&left_feed.to_string()).await;
    assert_eq!(removals.len(), 1);
    assert!(removals[0].data["doc"].is_null());
    assert!(ctx.search_changes(&still_in_feed.to_string()).await.is_empty());

    // The default: activities older than 400 days go.
    let report = ctx.feed.run_retention().await.unwrap();
    assert!(report.activities >= 1, "{report:?}");
    assert!(!exists("activities", "id", ancient).await);
    assert!(exists("activities", "id", old).await);
    // A second sweep finds nothing of this test's left to remove.
    ctx.feed.run_retention().await.unwrap();
    assert!(exists("activities", "id", old).await && exists("news_articles", "id", still_in_feed).await);
}

#[tokio::test]
async fn the_personal_data_export_has_a_feed_section() {
    let ctx = start().await;
    let t = token();
    let (me, friend, nobody) = (ctx.app.user("me").await, ctx.app.user("friend").await, ctx.app.user("nobody").await);
    ctx.befriend(&me, &friend).await;
    ctx.record(bump(&me, &friend, 35.0, 139.0), 20).await;
    ctx.record(
        entry(&me, "geo.visit", ActivityAudience::Private)
            .at(35.0, 139.0)
            .data(serde_json::json!({ "place_name": "駅" })),
        10,
    )
    .await;
    hide_kind(&ctx, &me, "geo").await;
    let friend_story = ctx.record(entry(&friend, "live.started", ActivityAudience::Friends).at(35.0, 139.0), 5).await;
    let _: MarkMapStorySeenResponse = ctx
        .call(
            STORIES,
            "MarkMapStorySeen",
            Some(&me),
            &MarkMapStorySeenRequest { activity_ids: vec![friend_story.to_string()], ..Default::default() },
        )
        .await
        .unwrap();
    let (_, article) = article_row(&ctx, &t, &["export-tag"]).await;
    let _: RecordNewsClickResponse = ctx
        .call(
            NEWS,
            "RecordNewsClick",
            Some(&me),
            &RecordNewsClickRequest { article_id: article.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let _: SendNewsFeedbackResponse = ctx
        .call(
            NEWS,
            "SendNewsFeedback",
            Some(&me),
            &SendNewsFeedbackRequest {
                article_id: article.to_string(),
                feedback: NewsFeedback::NEWS_FEEDBACK_LESS.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO feed_recap_notices (user_id, period_start) VALUES ($1, '2026-10-01')")
        .bind(me.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|exporter| exporter.name() == "feed")
        .expect("the feed exporter is registered");
    let export = exporter.export(me.id).await.unwrap();
    let activities = export["activities"].as_array().unwrap();
    assert_eq!(activities.len(), 2);
    assert_eq!(activities[0]["kind"], "geo.visit", "newest first, private entries included");
    assert_eq!(activities[0]["data"]["place_name"], "駅");
    assert_eq!(activities[1]["with_user_ids"], serde_json::json!([friend.id]));
    assert_eq!(export["hidden_activity_kinds"], serde_json::json!(["geo"]));
    assert_eq!(export["map_story_views"].as_array().unwrap().len(), 1);
    assert_eq!(export["map_story_views"][0]["activity_id"], serde_json::json!(friend_story));
    assert_eq!(export["news_clicks"][0]["title"], "Direct article");
    assert_eq!(export["news_feedback"][0]["feedback"], "less");
    let interests = export["news_interests"].as_array().unwrap();
    assert_eq!(interests.len(), 1);
    assert_eq!(interests[0]["tag"], "export-tag");
    assert_eq!(export["recap_notices"], serde_json::json!(["2026-10-01"]));

    // Someone with no data gets empty lists, not errors — and never someone else's data.
    let empty = exporter.export(nobody.id).await.unwrap();
    for section in [
        "activities",
        "hidden_activity_kinds",
        "map_story_views",
        "news_clicks",
        "news_feedback",
        "news_interests",
        "recap_notices",
    ] {
        assert_eq!(empty[section], serde_json::json!([]), "{section}");
    }
    let friends = exporter.export(friend.id).await.unwrap();
    assert_eq!(friends["activities"].as_array().unwrap().len(), 1);
    assert!(friends["map_story_views"].as_array().unwrap().is_empty(), "who watched my stories is not my data");
}

#[tokio::test]
async fn all_sources_can_be_refreshed_at_once_and_their_number_is_capped() {
    let ctx = start().await;
    let (t, admin) = (token(), ctx.admin().await);
    let db = &ctx.app.core.db;
    let create = |name: &str, disabled: bool| {
        let request = CreateNewsSourceRequest {
            feed_url: format!("https://{name}-{t}.example/feed.xml"),
            disabled,
            ..Default::default()
        };
        let (ctx, admin) = (&ctx, &admin);
        async move {
            ctx.call::<_, CreateNewsSourceResponse>(NEWS_ADMIN, "CreateNewsSource", Some(admin), &request)
                .await
                .map(|response| response.source.into_option().unwrap())
        }
    };
    let (first, second, off) =
        (create("one", false).await.unwrap(), create("two", false).await.unwrap(), create("off", true).await.unwrap());
    // Pretend the scheduler had already planned their next fetch.
    sqlx::query("UPDATE news_sources SET next_fetch_at = now() + interval '10 minutes' WHERE id = ANY($1::uuid[])")
        .bind(vec![first.id.clone(), second.id.clone(), off.id.clone()])
        .execute(db)
        .await
        .unwrap();

    let refreshed: RefreshNewsSourcesResponse =
        ctx.call(NEWS_ADMIN, "RefreshNewsSources", Some(&admin), &RefreshNewsSourcesRequest::default()).await.unwrap();
    assert!(refreshed.queued_count >= 2, "every enabled source: {}", refreshed.queued_count);
    let due = |source: ManagedNewsSource| {
        tokuly_core::time::from_ts(source.next_fetch_at.as_option().unwrap()) <= Utc::now() + Duration::seconds(5)
    };
    assert!(
        due(ctx.managed_source(&admin, &first.id).await) && due(ctx.managed_source(&admin, &second.id).await),
        "due right away"
    );
    assert!(!due(ctx.managed_source(&admin, &off.id).await), "a source that is switched off is left alone");

    // Fill the table up to the limit.
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM news_sources").fetch_one(db).await.unwrap();
    sqlx::query("INSERT INTO news_sources (name, feed_url, enabled) SELECT 'bulk', 'https://bulk-' || $1 || '.example/' || n, false FROM generate_series(1, $2) AS n")
        .bind(&t)
        .bind((500 - existing) as i32)
        .execute(db)
        .await
        .unwrap();
    let error = create("one-too-many", true).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("failed_precondition", "TOO_MANY_NEWS_SOURCES"));
    let all: ListManagedNewsSourcesResponse = ctx
        .call(NEWS_ADMIN, "ListManagedNewsSources", Some(&admin), &ListManagedNewsSourcesRequest::default())
        .await
        .unwrap();
    assert_eq!(all.sources.len(), 500);

    // Room again once sources are removed.
    sqlx::query("DELETE FROM news_sources WHERE name = 'bulk' AND feed_url LIKE 'https://bulk-' || $1 || '.example/%'")
        .bind(&t)
        .execute(db)
        .await
        .unwrap();
    create("one-too-many", true).await.unwrap();
}
