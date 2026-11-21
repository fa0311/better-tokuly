//! Reading news: lists, the recommended order with random picks, clicks,
//! feedback, interests, and moderation.
mod common;

use std::collections::HashSet;

use chrono::{Duration, Utc};
use common::{server::*, *};
use redis::AsyncCommands as _;
use tokuly_feed::{FeedConfig, RefreshOutcome};
use tokuly_proto::{common::SubjectType, feed::*};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

/// A source with a language of its own, holding `items`. Returns the source
/// and its language tag.
async fn publish(
    ctx: &Ctx,
    server: &FeedServer,
    admin: &TestUser,
    t: &str,
    items: &[Item],
) -> (ManagedNewsSource, String) {
    let language = format!("zz-{t}");
    server.publish(t, rss(&format!("Feed {t}"), &format!("https://site-{t}.example/"), items));
    let source = ctx.source_for(admin, &server.feed_url(t), &language).await;
    let outcome = ctx.feed.refresh_source(source.id.parse().unwrap()).await.unwrap();
    assert_eq!(outcome, RefreshOutcome::Fetched { new: items.len(), updated: 0 });
    (source, language)
}

/// `count` items, the n-th published n hours ago, tagged by `category(n)`.
fn items(t: &str, count: i64, category: impl Fn(i64) -> String) -> Vec<Item> {
    (1..=count)
        .map(|n| {
            Item::new(&format!("Article {n}"), &format!("https://news.example/{t}/{n}"))
                .categories(&[category(n).as_str()])
                .published(Utc::now() - Duration::hours(n))
        })
        .collect()
}

fn titles(articles: &[NewsArticle]) -> Vec<&str> {
    articles.iter().map(|article| article.title.as_str()).collect()
}

async fn article(ctx: &Ctx, user: Option<&TestUser>, id: &str) -> Result<NewsArticle, RpcError> {
    let response: GetNewsArticleResponse = ctx
        .call(NEWS, "GetNewsArticle", user, &GetNewsArticleRequest { article_id: id.to_owned(), ..Default::default() })
        .await?;
    Ok(response.article.into_option().expect("the article"))
}

async fn click(ctx: &Ctx, user: Option<&TestUser>, id: &str) -> Result<RecordNewsClickResponse, RpcError> {
    ctx.call(NEWS, "RecordNewsClick", user, &RecordNewsClickRequest { article_id: id.to_owned(), ..Default::default() })
        .await
}

async fn feedback(
    ctx: &Ctx,
    user: Option<&TestUser>,
    id: &str,
    feedback: NewsFeedback,
) -> Result<SendNewsFeedbackResponse, RpcError> {
    ctx.call(
        NEWS,
        "SendNewsFeedback",
        user,
        &SendNewsFeedbackRequest { article_id: id.to_owned(), feedback: feedback.into(), ..Default::default() },
    )
    .await
}

async fn tags(ctx: &Ctx, user: Option<&TestUser>, limit: i32) -> ListNewsTagsResponse {
    ctx.call(NEWS, "ListNewsTags", user, &ListNewsTagsRequest { limit, ..Default::default() }).await.expect("news tags")
}

/// The reader's current interest in `tag` (0 when not listed).
fn interest(response: &ListNewsTagsResponse, tag: &str) -> f64 {
    response.interests.iter().chain(response.avoided.iter()).find(|i| i.tag == tag).map_or(0.0, |i| i.score)
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 0.01
}

#[tokio::test]
async fn anyone_can_list_the_latest_news_with_filters() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    let special = format!("special-{t}");
    let (source, language) = publish(
        &ctx,
        &server,
        &admin,
        &t,
        &items(&t, 5, |n| if n % 2 == 1 { special.clone() } else { format!("plain-{t}") }),
    )
    .await;
    let by_language = || ListNewsRequest { language: language.clone(), ..Default::default() };

    // Anonymous readers get the newest first, never "recommended".
    let all = ctx.list_news(None, by_language()).await;
    assert!(!all.recommended);
    assert_eq!(titles(&all.articles), vec!["Article 1", "Article 2", "Article 3", "Article 4", "Article 5"]);
    assert!(all.articles.iter().all(|a| !a.random_pick && !a.clicked));

    // Pages of two.
    let first = ctx.list_news(None, ListNewsRequest { page: page(2, "").into(), ..by_language() }).await;
    assert_eq!(titles(&first.articles), vec!["Article 1", "Article 2"]);
    let cursor = first.page.as_option().unwrap().next_cursor.clone();
    let second = ctx.list_news(None, ListNewsRequest { page: page(2, &cursor).into(), ..by_language() }).await;
    assert_eq!(titles(&second.articles), vec!["Article 3", "Article 4"]);
    let cursor = second.page.as_option().unwrap().next_cursor.clone();
    let third = ctx.list_news(None, ListNewsRequest { page: page(2, &cursor).into(), ..by_language() }).await;
    assert_eq!(titles(&third.articles), vec!["Article 5"]);
    assert!(!third.page.as_option().unwrap().has_more);

    // By tag (however it is written) and by source.
    let tagged = ctx
        .list_news(None, ListNewsRequest { tag: format!(" #{} ", special.to_uppercase()), ..Default::default() })
        .await;
    assert_eq!(titles(&tagged.articles), vec!["Article 1", "Article 3", "Article 5"]);
    assert!(tagged.articles.iter().all(|a| a.tags == vec![special.clone()]));
    let of_source = ctx
        .list_news(
            None,
            ListNewsRequest { source_id: source.id.clone(), tag: format!("plain-{t}"), ..Default::default() },
        )
        .await;
    assert_eq!(titles(&of_source.articles), vec!["Article 2", "Article 4"]);
    assert!(
        ctx.list_news(None, ListNewsRequest { language: "yy".into(), tag: special.clone(), ..Default::default() })
            .await
            .articles
            .is_empty(),
        "another language"
    );
    // The primary language selects its regional variants.
    let primary = ctx
        .list_news(None, ListNewsRequest { language: "zz".into(), tag: special.clone(), ..Default::default() })
        .await;
    assert_eq!(primary.articles.len(), 3);

    // A signed-in reader may ask for the latest; filters always mean the latest.
    let reader = ctx.app.user("reader").await;
    let latest = ctx
        .list_news(Some(&reader), ListNewsRequest { order: NewsOrder::NEWS_ORDER_LATEST.into(), ..by_language() })
        .await;
    assert!(!latest.recommended);
    assert_eq!(titles(&latest.articles).len(), 5);
    assert!(
        !ctx.list_news(Some(&reader), ListNewsRequest { tag: special.clone(), ..Default::default() }).await.recommended
    );
    assert!(
        !ctx.list_news(Some(&reader), ListNewsRequest { source_id: source.id.clone(), ..Default::default() })
            .await
            .recommended
    );

    // Validation.
    let list = |request: ListNewsRequest| {
        let ctx = &ctx;
        async move { ctx.call::<_, ListNewsResponse>(NEWS, "ListNews", None, &request).await }
    };
    assert_eq!(
        list(ListNewsRequest { tag: "a,b".into(), ..Default::default() }).await.unwrap_err().reason,
        "INVALID_TAG"
    );
    assert_eq!(
        list(ListNewsRequest { source_id: "nope".into(), ..Default::default() }).await.unwrap_err().reason,
        "INVALID_ID"
    );
    assert_eq!(
        list(ListNewsRequest { language: "japanese".into(), ..Default::default() }).await.unwrap_err().reason,
        "INVALID_LANGUAGE"
    );
    assert_eq!(
        list(ListNewsRequest { page: page(2, "garbage").into(), ..Default::default() }).await.unwrap_err().reason,
        "INVALID_CURSOR"
    );
}

#[tokio::test]
async fn switching_a_source_off_hides_its_articles_everywhere() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    let (source, language) = publish(&ctx, &server, &admin, &t, &items(&t, 2, |_| format!("tag-{t}"))).await;
    let listed =
        ctx.list_news(None, ListNewsRequest { language: language.clone(), ..Default::default() }).await.articles;
    let ids: Vec<String> = listed.iter().map(|a| a.id.clone()).collect();
    assert_eq!(ids.len(), 2);

    let switch = |enabled: bool| {
        let (ctx, admin, source_id) = (&ctx, &admin, source.id.clone());
        async move {
            ctx.call::<_, UpdateNewsSourceResponse>(
                NEWS_ADMIN,
                "UpdateNewsSource",
                Some(admin),
                &UpdateNewsSourceRequest { source_id, enabled: Some(enabled), ..Default::default() },
            )
            .await
            .unwrap()
        }
    };
    switch(false).await;
    assert!(
        ctx.list_news(None, ListNewsRequest { language: language.clone(), ..Default::default() })
            .await
            .articles
            .is_empty()
    );
    assert_eq!(article(&ctx, None, &ids[0]).await.unwrap_err().reason, "NEWS_ARTICLE_NOT_FOUND");
    assert_eq!(click(&ctx, None, &ids[0]).await.unwrap_err().reason, "NEWS_ARTICLE_NOT_FOUND");
    for id in &ids {
        let changes = ctx.search_changes(id).await;
        assert_eq!(changes.len(), 2, "indexed when fetched, removed when the source was switched off");
        assert!(changes[1].data["doc"].is_null());
    }
    // The admin still sees how many articles the source holds.
    assert_eq!(ctx.managed_source(&admin, &source.id).await.article_count, 2);

    // Switched back on, everything returns, search included.
    switch(true).await;
    assert_eq!(
        ctx.list_news(None, ListNewsRequest { language: language.clone(), ..Default::default() }).await.articles.len(),
        2
    );
    assert_eq!(article(&ctx, None, &ids[0]).await.unwrap().id, ids[0]);
    let changes = ctx.search_changes(&ids[0]).await;
    assert_eq!(changes.len(), 3);
    assert_eq!(changes[2].data["doc"]["kind"], "news");
}

#[tokio::test]
async fn signed_in_readers_get_their_interests_with_random_picks_mixed_in() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin, reader) = (token(), ctx.admin().await, ctx.app.user("reader").await);
    let (dogs, cats) = (format!("dogs-{t}"), format!("cats-{t}"));
    // 25 articles, one per hour; odd ones are about dogs, even ones about cats.
    let (_, language) =
        publish(&ctx, &server, &admin, &t, &items(&t, 25, |n| if n % 2 == 1 { dogs.clone() } else { cats.clone() }))
            .await;
    let latest = ctx
        .list_news(
            None,
            ListNewsRequest { language: language.clone(), page: page(50, "").into(), ..Default::default() },
        )
        .await
        .articles;
    let id_of =
        |title: &str| latest.iter().find(|a| a.title == title).unwrap_or_else(|| panic!("{title} exists")).id.clone();

    // The reader opens two old dog articles and asks for more like one of them.
    click(&ctx, Some(&reader), &id_of("Article 23")).await.unwrap();
    click(&ctx, Some(&reader), &id_of("Article 25")).await.unwrap();
    feedback(&ctx, Some(&reader), &id_of("Article 25"), NewsFeedback::NEWS_FEEDBACK_MORE).await.unwrap();
    assert!(close(interest(&tags(&ctx, Some(&reader), 0).await, &dogs), 5.0));

    let request = |cursor: &str| ListNewsRequest {
        language: language.clone(),
        page: page(10, cursor).into(),
        ..Default::default()
    };
    let first = ctx.list_news(Some(&reader), request("")).await;
    assert!(first.recommended);
    assert_eq!(first.articles.len(), 10);
    // A fifth of the positions, evenly spaced, are random picks — and say so.
    let random_positions: Vec<usize> =
        first.articles.iter().enumerate().filter(|(_, a)| a.random_pick).map(|(i, _)| i).collect();
    assert_eq!(random_positions, vec![4, 9]);
    // Everything else on the first page is what the reader cares about: dogs they have not read yet.
    for (position, shown) in first.articles.iter().enumerate().filter(|(_, a)| !a.random_pick) {
        assert_eq!(shown.tags, vec![dogs.clone()], "position {position}: {}", shown.title);
        assert!(!shown.clicked);
    }
    assert_eq!(first.articles[0].title, "Article 1", "the freshest dog article leads");
    // Random picks carry what the info sheet needs like any other article.
    let pick = &first.articles[4];
    assert!(!pick.tags.is_empty() && pick.source.as_option().is_some_and(|s| !s.name.is_empty()));

    // Paging: every article exactly once, however the reader got there.
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    let second = ctx.list_news(Some(&reader), request(&info.next_cursor)).await;
    let third = ctx.list_news(Some(&reader), request(&second.page.as_option().unwrap().next_cursor)).await;
    assert!(second.recommended && third.recommended);
    assert_eq!((second.articles.len(), third.articles.len()), (10, 5));
    assert!(!third.page.as_option().unwrap().has_more);
    let everything: Vec<&NewsArticle> = first.articles.iter().chain(&second.articles).chain(&third.articles).collect();
    assert_eq!(everything.iter().map(|a| a.id.as_str()).collect::<HashSet<_>>().len(), 25);
    assert_eq!(everything.iter().filter(|a| a.random_pick).count(), 5);
    // What the reader already opened sinks to the end, marked as read.
    let read: Vec<&&NewsArticle> = everything.iter().filter(|a| a.clicked).collect();
    assert_eq!(
        read.iter().map(|a| a.title.as_str()).collect::<HashSet<_>>(),
        HashSet::from(["Article 23", "Article 25"])
    );
    assert!(read.iter().all(|a| !a.random_pick), "an opened article is never offered as a random pick");
    assert_eq!(everything.iter().find(|a| a.title == "Article 25").unwrap().feedback, NewsFeedback::NEWS_FEEDBACK_MORE);

    // The order of a reading session survives the loss of its snapshot:
    // the cursor alone is enough to rebuild it.
    let mut valkey = ctx.app.core.cache.conn();
    let snapshots: Vec<String> = valkey.keys(format!("feed:news:rec:{}:*", reader.id)).await.unwrap();
    assert_eq!(snapshots.len(), 1);
    let _: () = valkey.del(&snapshots).await.unwrap();
    let rebuilt = ctx.list_news(Some(&reader), request(&info.next_cursor)).await;
    assert_eq!(
        rebuilt.articles.iter().map(|a| (&a.id, a.random_pick)).collect::<Vec<_>>(),
        second.articles.iter().map(|a| (&a.id, a.random_pick)).collect::<Vec<_>>()
    );

    // Cursors belong to the list they came from.
    let keyset = ctx
        .list_news(None, ListNewsRequest { language: language.clone(), page: page(2, "").into(), ..Default::default() })
        .await;
    let keyset_cursor = keyset.page.as_option().unwrap().next_cursor.clone();
    let wrong = ctx.call::<_, ListNewsResponse>(NEWS, "ListNews", Some(&reader), &request(&keyset_cursor)).await;
    assert_eq!(wrong.unwrap_err().reason, "INVALID_CURSOR");
    let wrong = ctx.call::<_, ListNewsResponse>(NEWS, "ListNews", None, &request(&info.next_cursor)).await;
    assert_eq!(wrong.unwrap_err().reason, "INVALID_CURSOR");
    let wrong = ctx.call::<_, ListNewsResponse>(NEWS, "ListNews", Some(&reader), &request("garbage")).await;
    assert_eq!(wrong.unwrap_err().reason, "INVALID_CURSOR");

    // A reader without any history gets the freshest articles, random picks included.
    let newcomer = ctx.app.user("newcomer").await;
    let fresh = ctx.list_news(Some(&newcomer), request("")).await;
    assert!(fresh.recommended);
    assert_eq!(fresh.articles.iter().filter(|a| a.random_pick).count(), 2);
    assert_eq!(fresh.articles[0].title, "Article 1");
}

#[tokio::test]
async fn the_share_of_random_picks_is_configurable() {
    let server = FeedServer::start().await;
    let t = token();
    for (percent, expected_random) in [(0, 0), (50, 5), (100, 10)] {
        let ctx = start_with(FeedConfig { news_random_share_percent: percent, ..test_config() }).await;
        let (admin, reader) = (ctx.admin().await, ctx.app.user("reader").await);
        let name = format!("{t}-{percent}");
        let (_, language) = publish(&ctx, &server, &admin, &name, &items(&name, 12, |_| format!("tag-{name}"))).await;
        let response = ctx
            .list_news(Some(&reader), ListNewsRequest { language, page: page(10, "").into(), ..Default::default() })
            .await;
        assert!(response.recommended);
        assert_eq!(response.articles.iter().filter(|a| a.random_pick).count(), expected_random, "{percent}% random");
    }
}

#[tokio::test]
async fn clicks_and_feedback_shape_a_readers_interests() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    let (reader, other) = (ctx.app.user("reader").await, ctx.app.user("other").await);
    let (x, y) = (format!("x-{t}"), format!("y-{t}"));
    let both = Item::new("Both tags", &format!("https://news.example/{t}/both"))
        .categories(&[x.as_str(), y.as_str()])
        .published(Utc::now() - Duration::hours(1));
    let only_y = Item::new("Only y", &format!("https://news.example/{t}/y"))
        .categories(&[y.as_str()])
        .published(Utc::now() - Duration::hours(2));
    let (_, language) = publish(&ctx, &server, &admin, &t, &[both, only_y]).await;
    let listed = ctx.list_news(None, ListNewsRequest { language, ..Default::default() }).await.articles;
    let (a, b) = (listed[0].id.clone(), listed[1].id.clone());

    // Opening an article counts once per reader and raises their interest in its tags.
    click(&ctx, Some(&reader), &a).await.unwrap();
    let after = tags(&ctx, Some(&reader), 0).await;
    assert!(close(interest(&after, &x), 1.0) && close(interest(&after, &y), 1.0), "{after:?}");
    assert!(after.avoided.is_empty());
    let seen = article(&ctx, Some(&reader), &a).await.unwrap();
    assert!(seen.clicked);
    assert_eq!(seen.click_count, 1);
    assert!(
        !article(&ctx, Some(&other), &a).await.unwrap().clicked,
        "what one reader opened is not marked for another"
    );

    click(&ctx, Some(&reader), &a).await.unwrap();
    assert_eq!(
        article(&ctx, None, &a).await.unwrap().click_count,
        1,
        "a second click by the same reader does not count again"
    );
    assert!(close(interest(&tags(&ctx, Some(&reader), 0).await, &x), 1.0), "nor does it raise the interest again");
    let own_clicks: i32 =
        sqlx::query_scalar("SELECT click_count FROM news_clicks WHERE user_id = $1 AND article_id = $2::uuid")
            .bind(reader.id)
            .bind(&a)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(own_clicks, 2);

    click(&ctx, Some(&other), &a).await.unwrap();
    click(&ctx, None, &a).await.unwrap();
    click(&ctx, None, &a).await.unwrap();
    assert_eq!(article(&ctx, None, &a).await.unwrap().click_count, 3, "another reader and one anonymous address");

    // "More like this" weighs more than a click; changing one's mind takes
    // the earlier feedback back before applying the new one.
    feedback(&ctx, Some(&reader), &b, NewsFeedback::NEWS_FEEDBACK_MORE).await.unwrap();
    assert!(close(interest(&tags(&ctx, Some(&reader), 0).await, &y), 4.0));
    feedback(&ctx, Some(&reader), &b, NewsFeedback::NEWS_FEEDBACK_MORE).await.unwrap();
    assert!(close(interest(&tags(&ctx, Some(&reader), 0).await, &y), 4.0), "repeating feedback adds nothing");
    assert_eq!(article(&ctx, Some(&reader), &b).await.unwrap().feedback, NewsFeedback::NEWS_FEEDBACK_MORE);

    feedback(&ctx, Some(&reader), &b, NewsFeedback::NEWS_FEEDBACK_LESS).await.unwrap();
    let disliked = tags(&ctx, Some(&reader), 0).await;
    assert!(close(interest(&disliked, &y), -2.0), "{disliked:?}");
    assert_eq!(disliked.avoided.iter().map(|i| i.tag.as_str()).collect::<Vec<_>>(), vec![y.as_str()]);
    assert_eq!(disliked.interests.iter().map(|i| i.tag.as_str()).collect::<Vec<_>>(), vec![x.as_str()]);
    assert_eq!(article(&ctx, Some(&reader), &b).await.unwrap().feedback, NewsFeedback::NEWS_FEEDBACK_LESS);

    feedback(&ctx, Some(&reader), &b, NewsFeedback::NEWS_FEEDBACK_UNSPECIFIED).await.unwrap();
    assert!(close(interest(&tags(&ctx, Some(&reader), 0).await, &y), 1.0), "taking feedback back leaves the click");
    assert_eq!(article(&ctx, Some(&reader), &b).await.unwrap().feedback, NewsFeedback::NEWS_FEEDBACK_UNSPECIFIED);
    feedback(&ctx, Some(&reader), &b, NewsFeedback::NEWS_FEEDBACK_UNSPECIFIED).await.unwrap();

    // The tag lists: mine, and what is common right now (for everyone).
    feedback(&ctx, Some(&reader), &b, NewsFeedback::NEWS_FEEDBACK_MORE).await.unwrap();
    let mine = tags(&ctx, Some(&reader), 1).await;
    assert_eq!(
        mine.interests.iter().map(|i| i.tag.as_str()).collect::<Vec<_>>(),
        vec![y.as_str()],
        "strongest first, as many as asked for"
    );
    ctx.app.core.cache.del("feed:news:popular-tags").await.unwrap();
    let anonymous = tags(&ctx, None, 100).await;
    assert!(anonymous.interests.is_empty() && anonymous.avoided.is_empty());
    let popular = |tag: &str| anonymous.popular.iter().find(|p| p.tag == tag).map(|p| p.article_count);
    assert_eq!((popular(&y), popular(&x)), (Some(2), Some(1)));
    let bystander = ctx.app.user("bystander").await;
    let nothing = tags(&ctx, Some(&bystander), 0).await;
    assert!(nothing.interests.is_empty() && nothing.avoided.is_empty(), "interests are personal");

    // Starting over: interests and feedback go; the reading history only when asked.
    let reset = |user: Option<&TestUser>, clear_history: bool| {
        let (ctx, user) = (&ctx, user.cloned());
        async move {
            ctx.call::<_, ResetNewsInterestsResponse>(
                NEWS,
                "ResetNewsInterests",
                user.as_ref(),
                &ResetNewsInterestsRequest { clear_history, ..Default::default() },
            )
            .await
        }
    };
    reset(Some(&reader), false).await.unwrap();
    let cleared = tags(&ctx, Some(&reader), 0).await;
    assert!(cleared.interests.is_empty() && cleared.avoided.is_empty());
    let kept = article(&ctx, Some(&reader), &b).await.unwrap();
    assert_eq!(kept.feedback, NewsFeedback::NEWS_FEEDBACK_UNSPECIFIED);
    assert!(article(&ctx, Some(&reader), &a).await.unwrap().clicked, "the history stays");
    reset(Some(&reader), true).await.unwrap();
    assert!(!article(&ctx, Some(&reader), &a).await.unwrap().clicked);
    assert!(article(&ctx, Some(&other), &a).await.unwrap().clicked, "only the caller's history is cleared");

    // Errors.
    let unknown = Uuid::now_v7().to_string();
    assert_eq!(click(&ctx, Some(&reader), &unknown).await.unwrap_err().reason, "NEWS_ARTICLE_NOT_FOUND");
    assert_eq!(click(&ctx, None, "nope").await.unwrap_err().reason, "INVALID_ID");
    assert_eq!(
        feedback(&ctx, Some(&reader), &unknown, NewsFeedback::NEWS_FEEDBACK_MORE).await.unwrap_err().reason,
        "NEWS_ARTICLE_NOT_FOUND"
    );
    assert_eq!(
        feedback(&ctx, Some(&reader), "nope", NewsFeedback::NEWS_FEEDBACK_MORE).await.unwrap_err().reason,
        "INVALID_ID"
    );
    assert_eq!(feedback(&ctx, None, &a, NewsFeedback::NEWS_FEEDBACK_MORE).await.unwrap_err().code, "unauthenticated");
    assert_eq!(reset(None, false).await.unwrap_err().code, "unauthenticated");

    // Clicks are tracked for analytics.
    assert!(ctx.app.flush_analytics().await >= 4);
}

#[tokio::test]
async fn interests_fade_with_time() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin, reader) = (token(), ctx.admin().await, ctx.app.user("reader").await);
    let tag = format!("fading-{t}");
    let (_, language) = publish(&ctx, &server, &admin, &t, &items(&t, 3, |_| tag.clone())).await;
    let listed = ctx.list_news(None, ListNewsRequest { language, ..Default::default() }).await.articles;
    click(&ctx, Some(&reader), &listed[0].id).await.unwrap();
    assert!(close(interest(&tags(&ctx, Some(&reader), 0).await, &tag), 1.0));

    let age = |days: i32| {
        let (db, reader_id) = (&ctx.app.core.db, reader.id);
        async move {
            sqlx::query(
                "UPDATE news_tag_affinity SET updated_at = now() - make_interval(days => $2) WHERE user_id = $1",
            )
            .bind(reader_id)
            .bind(days)
            .execute(db)
            .await
            .unwrap()
        }
    };
    // One half-life (30 days by default) later the interest is half as strong.
    age(30).await;
    assert!(close(interest(&tags(&ctx, Some(&reader), 0).await, &tag), 0.5));
    // A new click adds to what is left of the old interest.
    click(&ctx, Some(&reader), &listed[1].id).await.unwrap();
    assert!(close(interest(&tags(&ctx, Some(&reader), 0).await, &tag), 1.5));
    // After a year nothing worth listing remains, and the sweep removes the row.
    age(365).await;
    assert!(tags(&ctx, Some(&reader), 0).await.interests.is_empty());
    ctx.feed.run_retention().await.unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM news_tag_affinity WHERE user_id = $1")
        .bind(reader.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);

    // Interests cannot grow without bound.
    sqlx::query("INSERT INTO news_tag_affinity (user_id, tag, score) VALUES ($1, $2, 19.5)")
        .bind(reader.id)
        .bind(&tag)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    click(&ctx, Some(&reader), &listed[2].id).await.unwrap();
    assert!(close(interest(&tags(&ctx, Some(&reader), 0).await, &tag), 20.0));
}

#[tokio::test]
async fn articles_can_be_taken_down_and_stay_down() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    let (moderator, reader) = (ctx.app.user_with_roles("mod", &["moderator"]).await, ctx.app.user("reader").await);
    let feed_items = items(&t, 3, |_| format!("tag-{t}"));
    let (source, language) = publish(&ctx, &server, &admin, &t, &feed_items).await;
    let listed =
        ctx.list_news(None, ListNewsRequest { language: language.clone(), ..Default::default() }).await.articles;
    let (first, second, third) = (listed[0].id.clone(), listed[1].id.clone(), listed[2].id.clone());

    let remove = |user: Option<&TestUser>, id: &str| {
        let (ctx, user, article_id) = (&ctx, user.cloned(), id.to_owned());
        async move {
            ctx.call::<_, RemoveNewsArticleResponse>(
                NEWS_ADMIN,
                "RemoveNewsArticle",
                user.as_ref(),
                &RemoveNewsArticleRequest { article_id, ..Default::default() },
            )
            .await
        }
    };
    // Moderators (and admins) only.
    assert_eq!(remove(Some(&reader), &first).await.unwrap_err().reason, "FORBIDDEN");
    assert_eq!(remove(None, &first).await.unwrap_err().code, "unauthenticated");
    assert_eq!(
        remove(Some(&moderator), &Uuid::now_v7().to_string()).await.unwrap_err().reason,
        "NEWS_ARTICLE_NOT_FOUND"
    );
    assert_eq!(remove(Some(&moderator), "nope").await.unwrap_err().reason, "INVALID_ID");
    remove(Some(&moderator), &first).await.unwrap();
    remove(Some(&admin), &first).await.unwrap();

    // Gone from lists, by ID, from the recommended order, and from search.
    let remaining =
        ctx.list_news(None, ListNewsRequest { language: language.clone(), ..Default::default() }).await.articles;
    assert_eq!(remaining.iter().map(|a| a.id.clone()).collect::<Vec<_>>(), vec![second.clone(), third.clone()]);
    assert_eq!(article(&ctx, None, &first).await.unwrap_err().reason, "NEWS_ARTICLE_NOT_FOUND");
    let recommended =
        ctx.list_news(Some(&reader), ListNewsRequest { language: language.clone(), ..Default::default() }).await;
    assert!(recommended.recommended && recommended.articles.iter().all(|a| a.id != first));
    let changes = ctx.search_changes(&first).await;
    assert_eq!(changes.len(), 2, "indexed once, removed once");
    assert!(changes[1].data["doc"].is_null());

    // The feed still carries it (even edited): it is not brought back.
    let mut edited = feed_items.clone();
    edited[0].title = "Article 1, now with a new title".into();
    server.publish(&t, rss(&format!("Feed {t}"), &format!("https://site-{t}.example/"), &edited));
    assert_eq!(
        ctx.feed.refresh_source(source.id.parse().unwrap()).await.unwrap(),
        RefreshOutcome::Fetched { new: 0, updated: 0 }
    );
    assert_eq!(
        ctx.list_news(None, ListNewsRequest { language: language.clone(), ..Default::default() }).await.articles.len(),
        2
    );
    assert_eq!(ctx.search_changes(&first).await.len(), 2);

    // A takedown through the moderation queue arrives as an event.
    let removed = serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_NEWS_ARTICLE as i32, "subject_id": second, "reason": "規約違反", "moderator_id": moderator.id });
    ctx.feed.handle_event(event("admin.content.removed", Some(moderator.id), removed.clone())).await.unwrap();
    ctx.feed.handle_event(event("admin.content.removed", Some(moderator.id), removed)).await.unwrap();
    assert_eq!(article(&ctx, None, &second).await.unwrap_err().reason, "NEWS_ARTICLE_NOT_FOUND");
    assert_eq!(ctx.search_changes(&second).await.len(), 2);

    // Comments, likes and reports attach to articles through the subject registry.
    let subjects = &ctx.app.core.subjects;
    assert!(subjects.is_registered(SubjectType::SUBJECT_TYPE_NEWS_ARTICLE));
    let info = subjects.resolve(SubjectType::SUBJECT_TYPE_NEWS_ARTICLE, &third, Some(reader.id)).await.unwrap();
    assert_eq!(
        (info.id.to_string(), info.title.as_str(), info.path.as_str()),
        (third.clone(), "Article 3", format!("/news/articles/{third}").as_str())
    );
    assert!(info.owner_id.is_nil(), "articles are official content: nobody to notify");
    assert!(info.interactions_enabled);
    for hidden in [&first, &second, &Uuid::now_v7().to_string(), &"nope".to_owned()] {
        let error = subjects.resolve(SubjectType::SUBJECT_TYPE_NEWS_ARTICLE, hidden, None).await.unwrap_err();
        assert_eq!(error.reason, "SUBJECT_NOT_FOUND");
    }
    let third_id: Uuid = third.parse().unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_NEWS_ARTICLE, third_id, "comment_count", 2).await.unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_NEWS_ARTICLE, third_id, "like_count", 1).await.unwrap();
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_NEWS_ARTICLE, third_id, "view_count", 9).await.unwrap();
    let counted = article(&ctx, None, &third).await.unwrap();
    assert_eq!((counted.comment_count, counted.like_count), (2, 1));
    subjects.counter_changed(SubjectType::SUBJECT_TYPE_NEWS_ARTICLE, third_id, "like_count", -5).await.unwrap();
    assert_eq!(article(&ctx, None, &third).await.unwrap().like_count, 0, "counters never go below zero");

    // Deleting the source removes what is left, from search too.
    let _: DeleteNewsSourceResponse = ctx
        .call(
            NEWS_ADMIN,
            "DeleteNewsSource",
            Some(&admin),
            &DeleteNewsSourceRequest { source_id: source.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(article(&ctx, None, &third).await.unwrap_err().reason, "NEWS_ARTICLE_NOT_FOUND");
    let changes = ctx.search_changes(&third).await;
    assert!(changes.last().unwrap().data["doc"].is_null());
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM news_articles WHERE source_id = $1::uuid")
        .bind(&source.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}
