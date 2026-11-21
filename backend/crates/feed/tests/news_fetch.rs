//! News sources and the fetcher, against a local HTTP server: admin RPCs,
//! RSS and Atom, conditional requests, failures, limits and SSRF protection.
mod common;

use chrono::{Duration, Utc};
use common::{server::*, *};
use tokuly_feed::{FeedConfig, RefreshOutcome};
use tokuly_proto::feed::*;
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

fn id_of(source: &ManagedNewsSource) -> Uuid {
    source.id.parse().expect("source id")
}

async fn create(
    ctx: &Ctx,
    user: Option<&TestUser>,
    request: CreateNewsSourceRequest,
) -> Result<ManagedNewsSource, RpcError> {
    let response: CreateNewsSourceResponse = ctx.call(NEWS_ADMIN, "CreateNewsSource", user, &request).await?;
    Ok(response.source.into_option().expect("the created source"))
}

async fn update(ctx: &Ctx, admin: &TestUser, request: UpdateNewsSourceRequest) -> Result<ManagedNewsSource, RpcError> {
    let response: UpdateNewsSourceResponse = ctx.call(NEWS_ADMIN, "UpdateNewsSource", Some(admin), &request).await?;
    Ok(response.source.into_option().expect("the updated source"))
}

fn failure(outcome: RefreshOutcome) -> String {
    match outcome {
        RefreshOutcome::Failed(reason) => reason,
        other => panic!("expected a failed fetch, got {other:?}"),
    }
}

#[tokio::test]
async fn sources_are_managed_by_admins_only() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let t = token();
    let (admin, moderator, user) =
        (ctx.admin().await, ctx.app.user_with_roles("mod", &["moderator"]).await, ctx.app.user("user").await);
    let request =
        || CreateNewsSourceRequest { feed_url: server.feed_url(&format!("managed-{t}")), ..Default::default() };

    // Everyone but admins is turned away, on every RPC.
    for (who, code, reason) in [
        (Some(&user), "permission_denied", "FORBIDDEN"),
        (Some(&moderator), "permission_denied", "FORBIDDEN"),
        (None, "unauthenticated", "SIGN_IN_REQUIRED"),
    ] {
        let error = create(&ctx, who, request()).await.unwrap_err();
        assert_eq!((error.code.as_str(), error.reason.as_str()), (code, reason));
        let error = ctx
            .call::<_, ListManagedNewsSourcesResponse>(
                NEWS_ADMIN,
                "ListManagedNewsSources",
                who,
                &ListManagedNewsSourcesRequest::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.reason, reason);
        let any_id = Uuid::now_v7().to_string();
        let error = ctx
            .call::<_, UpdateNewsSourceResponse>(
                NEWS_ADMIN,
                "UpdateNewsSource",
                who,
                &UpdateNewsSourceRequest { source_id: any_id.clone(), ..Default::default() },
            )
            .await
            .unwrap_err();
        assert_eq!(error.reason, reason);
        let error = ctx
            .call::<_, DeleteNewsSourceResponse>(
                NEWS_ADMIN,
                "DeleteNewsSource",
                who,
                &DeleteNewsSourceRequest { source_id: any_id.clone(), ..Default::default() },
            )
            .await
            .unwrap_err();
        assert_eq!(error.reason, reason);
        let error = ctx
            .call::<_, RefreshNewsSourcesResponse>(
                NEWS_ADMIN,
                "RefreshNewsSources",
                who,
                &RefreshNewsSourcesRequest { source_id: any_id, ..Default::default() },
            )
            .await
            .unwrap_err();
        assert_eq!(error.reason, reason);
    }

    // Validation, field by field.
    for (change, reason) in [
        (CreateNewsSourceRequest { feed_url: String::new(), ..request() }, "INVALID_FEED_URL"),
        (CreateNewsSourceRequest { feed_url: "not a url".into(), ..request() }, "INVALID_FEED_URL"),
        (
            CreateNewsSourceRequest { feed_url: "ftp://news.example/rss.xml".into(), ..request() },
            "FEED_URL_NOT_ALLOWED",
        ),
        (
            CreateNewsSourceRequest { feed_url: "https://user:secret@news.example/rss.xml".into(), ..request() },
            "FEED_URL_NOT_ALLOWED",
        ),
        (CreateNewsSourceRequest { feed_url: "http://10.0.0.8/rss.xml".into(), ..request() }, "FEED_URL_NOT_ALLOWED"),
        (
            CreateNewsSourceRequest { feed_url: "http://169.254.169.254/latest/meta-data/".into(), ..request() },
            "FEED_URL_NOT_ALLOWED",
        ),
        (CreateNewsSourceRequest { feed_url: "http://[fd00::1]/rss.xml".into(), ..request() }, "FEED_URL_NOT_ALLOWED"),
        (CreateNewsSourceRequest { name: "n".repeat(101), ..request() }, "INVALID_NAME"),
        (CreateNewsSourceRequest { site_url: "news.example".into(), ..request() }, "INVALID_SITE_URL"),
        (CreateNewsSourceRequest { language: "japanese".into(), ..request() }, "INVALID_LANGUAGE"),
        (CreateNewsSourceRequest { default_tags: vec!["ok".into(), "a,b".into()], ..request() }, "INVALID_TAG"),
        (
            CreateNewsSourceRequest { default_tags: (0..11).map(|i| format!("tag{i}")).collect(), ..request() },
            "TOO_MANY_TAGS",
        ),
    ] {
        let error = create(&ctx, Some(&admin), change).await.unwrap_err();
        assert_eq!((error.code.as_str(), error.reason.as_str()), ("invalid_argument", reason));
    }

    // A source created switched off, with defaults filled in.
    let source = create(
        &ctx,
        Some(&admin),
        CreateNewsSourceRequest {
            default_tags: vec![" Tech News ".into(), "tech news".into(), "ゲーム".into()],
            disabled: true,
            ..request()
        },
    )
    .await
    .unwrap();
    assert_eq!(source.feed_url, server.feed_url(&format!("managed-{t}")));
    assert_eq!(source.name, "127.0.0.1", "the host name until the feed says what it is called");
    assert_eq!(
        (source.language.as_str(), source.enabled, source.from_config, source.article_count),
        ("ja", false, false, 0)
    );
    assert_eq!(source.default_tags, vec!["tech news", "ゲーム"]);
    assert!(source.last_fetched_at.as_option().is_none() && source.last_error.is_empty());
    let duplicate = create(&ctx, Some(&admin), request()).await.unwrap_err();
    assert_eq!((duplicate.code.as_str(), duplicate.reason.as_str()), ("already_exists", "NEWS_SOURCE_EXISTS"));
    assert_eq!(ctx.managed_source(&admin, &source.id).await.feed_url, source.feed_url);

    // Switched off: not listed for readers, and not refreshable.
    let listed: ListNewsSourcesResponse =
        ctx.call(NEWS, "ListNewsSources", None, &ListNewsSourcesRequest::default()).await.unwrap();
    assert!(!listed.sources.iter().any(|s| s.id == source.id));
    let refresh = |source_id: String| {
        let (ctx, admin) = (&ctx, &admin);
        async move {
            ctx.call::<_, RefreshNewsSourcesResponse>(
                NEWS_ADMIN,
                "RefreshNewsSources",
                Some(admin),
                &RefreshNewsSourcesRequest { source_id, ..Default::default() },
            )
            .await
        }
    };
    let error = refresh(source.id.clone()).await.unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("failed_precondition", "NEWS_SOURCE_DISABLED"));
    assert_eq!(refresh(Uuid::now_v7().to_string()).await.unwrap_err().reason, "NEWS_SOURCE_NOT_FOUND");
    assert_eq!(refresh("nope".into()).await.unwrap_err().reason, "INVALID_ID");

    // Updating: only what is set changes.
    let updated = update(
        &ctx,
        &admin,
        UpdateNewsSourceRequest {
            source_id: source.id.clone(),
            name: Some("トクリーニュース".into()),
            site_url: Some("https://news.example/".into()),
            language: Some("EN-us".into()),
            default_tags: NewsTagList { tags: vec!["World".into()], ..Default::default() }.into(),
            enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (updated.name.as_str(), updated.site_url.as_str(), updated.language.as_str(), updated.enabled),
        ("トクリーニュース", "https://news.example/", "en-us", true)
    );
    assert_eq!(updated.default_tags, vec!["world"]);
    let untouched =
        update(&ctx, &admin, UpdateNewsSourceRequest { source_id: source.id.clone(), ..Default::default() })
            .await
            .unwrap();
    assert_eq!(
        (untouched.name.as_str(), untouched.default_tags.clone(), untouched.enabled),
        ("トクリーニュース", vec!["world".to_owned()], true)
    );
    let listed: ListNewsSourcesResponse =
        ctx.call(NEWS, "ListNewsSources", None, &ListNewsSourcesRequest::default()).await.unwrap();
    let public = listed.sources.iter().find(|s| s.id == source.id).expect("an enabled source is listed for readers");
    assert_eq!(
        (public.name.as_str(), public.language.as_str(), public.site_url.as_str()),
        ("トクリーニュース", "en-us", "https://news.example/")
    );
    assert_eq!(refresh(source.id.clone()).await.unwrap().queued_count, 1);

    // Update errors.
    let error =
        update(&ctx, &admin, UpdateNewsSourceRequest { source_id: Uuid::now_v7().to_string(), ..Default::default() })
            .await
            .unwrap_err();
    assert_eq!((error.code.as_str(), error.reason.as_str()), ("not_found", "NEWS_SOURCE_NOT_FOUND"));
    let error = update(
        &ctx,
        &admin,
        UpdateNewsSourceRequest { source_id: source.id.clone(), language: Some("x".into()), ..Default::default() },
    )
    .await
    .unwrap_err();
    assert_eq!(error.reason, "INVALID_LANGUAGE");
    let error = update(
        &ctx,
        &admin,
        UpdateNewsSourceRequest {
            source_id: source.id.clone(),
            feed_url: Some("http://192.168.1.1/rss".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.reason, "FEED_URL_NOT_ALLOWED");
    let other = create(
        &ctx,
        Some(&admin),
        CreateNewsSourceRequest {
            feed_url: server.feed_url(&format!("other-{t}")),
            disabled: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let error = update(
        &ctx,
        &admin,
        UpdateNewsSourceRequest {
            source_id: other.id.clone(),
            feed_url: Some(source.feed_url.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.reason, "NEWS_SOURCE_EXISTS");

    // Deleting.
    let delete = |source_id: String| {
        let (ctx, admin) = (&ctx, &admin);
        async move {
            ctx.call::<_, DeleteNewsSourceResponse>(
                NEWS_ADMIN,
                "DeleteNewsSource",
                Some(admin),
                &DeleteNewsSourceRequest { source_id, ..Default::default() },
            )
            .await
        }
    };
    delete(source.id.clone()).await.unwrap();
    assert_eq!(delete(source.id.clone()).await.unwrap_err().reason, "NEWS_SOURCE_NOT_FOUND");
    assert_eq!(delete("nope".into()).await.unwrap_err().reason, "INVALID_ID");
    // The URL is free again.
    create(&ctx, Some(&admin), CreateNewsSourceRequest { disabled: true, ..request() }).await.unwrap();
}

#[tokio::test]
async fn an_rss_feed_becomes_tagged_articles_with_search_documents() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    let first_link = format!("https://news.example/{t}/a1?utm_source=rss&id=7#top");
    server.publish(
        &t,
        rss(
            "Example & ニュース",
            &format!("https://site-{t}.example/"),
            &[
                Item::new("任天堂が<b>新作ゲーム</b>を発表", &first_link)
                    .description(
                        "<p>Switch向けの<a href=\"https://x.example\">新作</a>です。</p><script>alert(1)</script>",
                    )
                    .categories(&["エンタメ"])
                    .published(Utc::now() - Duration::hours(1))
                    .image(&format!("https://cdn.example/{t}/a1.jpg")),
                Item::new("Plain story", &format!("https://news.example/{t}/a2")),
            ],
        ),
    );
    let source = ctx
        .create_source(
            &admin,
            CreateNewsSourceRequest {
                feed_url: server.feed_url(&t),
                language: "ja".into(),
                default_tags: vec!["Tech News".into()],
                ..Default::default()
            },
        )
        .await;

    assert_eq!(ctx.feed.refresh_source(id_of(&source)).await.unwrap(), RefreshOutcome::Fetched { new: 2, updated: 0 });

    let articles = ctx.articles_of(&source.id).await;
    assert_eq!(
        articles.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        vec!["Plain story", "任天堂が新作ゲームを発表"],
        "an item without a date counts as published when first seen"
    );
    let article = &articles[1];
    assert_eq!(article.url, first_link, "readers open the link as the feed gave it");
    assert_eq!(article.summary, "Switch向けの新作です。", "markup and scripts are gone");
    assert_eq!(article.image_url, format!("https://cdn.example/{t}/a1.jpg"));
    assert_eq!(article.tags, vec!["tech news", "エンタメ", "ゲーム"], "source tags, feed categories, keyword matches");
    assert_eq!(article.path, format!("/news/articles/{}", article.id));
    assert!(!article.random_pick && !article.clicked);
    assert_eq!((article.click_count, article.like_count, article.comment_count), (0, 0, 0));
    assert!(article.published_at.as_option().is_some());
    // What the info sheet shows about the source comes from the feed.
    let shown = article.source.as_option().unwrap();
    assert_eq!(shown.id, source.id);
    assert_eq!(shown.name, "Example & ニュース");
    assert_eq!(shown.site_url, format!("https://site-{t}.example/"));
    assert_eq!(shown.favicon_url, format!("https://site-{t}.example/favicon.ico"));
    assert_eq!(shown.language, "ja");
    assert_eq!(articles[0].tags, vec!["tech news"]);
    assert_eq!(articles[0].summary, "");

    // One article, publicly readable by ID.
    let one: GetNewsArticleResponse = ctx
        .call(
            NEWS,
            "GetNewsArticle",
            None,
            &GetNewsArticleRequest { article_id: article.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(one.article.as_option().unwrap().title, "任天堂が新作ゲームを発表");
    let missing = ctx
        .call::<_, GetNewsArticleResponse>(
            NEWS,
            "GetNewsArticle",
            None,
            &GetNewsArticleRequest { article_id: Uuid::now_v7().to_string(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!((missing.code.as_str(), missing.reason.as_str()), ("not_found", "NEWS_ARTICLE_NOT_FOUND"));
    let bad = ctx
        .call::<_, GetNewsArticleResponse>(
            NEWS,
            "GetNewsArticle",
            None,
            &GetNewsArticleRequest { article_id: "nope".into(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(bad.reason, "INVALID_ID");

    // Each article went to the search index as `news`.
    let changes = ctx.search_changes(&article.id).await;
    assert_eq!(changes.len(), 1);
    let doc = &changes[0].data["doc"];
    assert_eq!(changes[0].data["kind"], "news");
    assert_eq!(doc["title"], "任天堂が新作ゲームを発表");
    assert_eq!(doc["text"], "Switch向けの新作です。");
    assert_eq!(doc["tags"], serde_json::json!(["tech news", "エンタメ", "ゲーム"]));
    assert_eq!(doc["path"], format!("/news/articles/{}", article.id));
    assert_eq!(doc["public"], true);
    assert_eq!(doc["owner_id"], serde_json::Value::Null);
    assert_eq!(doc["extra"]["source_name"], "Example & ニュース");

    // The source records the success.
    let managed = ctx.managed_source(&admin, &source.id).await;
    assert_eq!(managed.name, "Example & ニュース");
    assert_eq!((managed.article_count, managed.consecutive_failures, managed.last_error.as_str()), (2, 0, ""));
    assert!(managed.last_success_at.as_option().is_some() && managed.last_fetched_at.as_option().is_some());
}

#[tokio::test]
async fn an_atom_feed_becomes_articles_too() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    let published = Utc::now() - Duration::hours(3);
    server.publish(
        &t,
        atom(
            "Atom ブログ",
            &format!("https://blog-{t}.example/"),
            &[Item::new("新しい記事", &format!("https://blog.example/{t}/posts/1"))
                .description("<p>要約です &amp; more</p>")
                .categories(&["Diary", "日記"])
                .published(published)
                .image(&format!("https://blog.example/{t}/cover.jpg"))],
        ),
    );
    // A name chosen by the admin is not replaced by the feed's title.
    let source = ctx
        .create_source(
            &admin,
            CreateNewsSourceRequest {
                feed_url: server.feed_url(&t),
                name: "My Blog".into(),
                language: "ja".into(),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(ctx.feed.refresh_source(id_of(&source)).await.unwrap(), RefreshOutcome::Fetched { new: 1, updated: 0 });

    let articles = ctx.articles_of(&source.id).await;
    assert_eq!(articles.len(), 1);
    let article = &articles[0];
    assert_eq!(article.title, "新しい記事");
    assert_eq!(article.url, format!("https://blog.example/{t}/posts/1"));
    assert_eq!(article.summary, "要約です & more");
    assert_eq!(article.image_url, format!("https://blog.example/{t}/cover.jpg"));
    assert_eq!(article.tags, vec!["diary", "日記"]);
    let stored = tokuly_core::time::from_ts(article.published_at.as_option().unwrap());
    assert_eq!(stored.timestamp(), published.timestamp());
    let shown = article.source.as_option().unwrap();
    assert_eq!(
        (shown.name.as_str(), shown.site_url.as_str()),
        ("My Blog", format!("https://blog-{t}.example/").as_str())
    );
}

#[tokio::test]
async fn fetches_are_conditional_and_changes_update_articles_in_place() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    let link = |n: u32| format!("https://news.example/{t}/a{n}");
    let hour_ago = Utc::now() - Duration::hours(1);
    server.publish(
        &t,
        rss(
            "Feed",
            "https://news.example/",
            &[Item::new("First title", &link(1)).description("First body").published(hour_ago)],
        ),
    );
    let source = ctx.source_for(&admin, &server.feed_url(&t), "ja").await;
    let source_id = id_of(&source);

    // First fetch: nothing to be conditional about.
    assert_eq!(ctx.feed.refresh_source(source_id).await.unwrap(), RefreshOutcome::Fetched { new: 1, updated: 0 });
    let hits = server.hits(&t);
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].if_none_match.clone(), hits[0].if_modified_since.clone(), hits[0].status), (None, None, 200));
    let original = ctx.articles_of(&source.id).await.remove(0);

    // Second fetch: the validators of the first response are sent back, and "304" is a success.
    assert_eq!(ctx.feed.refresh_source(source_id).await.unwrap(), RefreshOutcome::NotModified);
    let hits = server.hits(&t);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[1].status, 304);
    assert!(hits[1].if_none_match.as_deref().is_some_and(|etag| etag.starts_with("\"v")), "{:?}", hits[1]);
    assert!(hits[1].if_modified_since.as_deref().is_some_and(|date| date.ends_with("GMT")), "{:?}", hits[1]);
    let managed = ctx.managed_source(&admin, &source.id).await;
    assert_eq!((managed.consecutive_failures, managed.last_error.as_str(), managed.article_count), (0, "", 1));

    // The feed changes: the first article is edited, a second one appears.
    server.publish(
        &t,
        rss(
            "Feed",
            "https://news.example/",
            &[
                Item::new("Second article", &link(2)).published(Utc::now()),
                Item::new("First title, corrected", &link(1)).description("First body").published(hour_ago),
            ],
        ),
    );
    assert_eq!(ctx.feed.refresh_source(source_id).await.unwrap(), RefreshOutcome::Fetched { new: 1, updated: 1 });
    assert_eq!(server.hits(&t)[2].status, 200);
    let articles = ctx.articles_of(&source.id).await;
    assert_eq!(
        articles.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        vec!["Second article", "First title, corrected"]
    );
    assert_eq!(articles[1].id, original.id, "an edited article keeps its identity");
    assert_eq!(articles[1].published_at, original.published_at);
    // The correction reached the search index as well.
    let changes = ctx.search_changes(&original.id).await;
    assert_eq!(
        changes.iter().map(|c| c.data["doc"]["title"].as_str().unwrap().to_owned()).collect::<Vec<_>>(),
        vec!["First title", "First title, corrected"]
    );

    // Nothing changed since: 304 again, with the new validators.
    assert_eq!(ctx.feed.refresh_source(source_id).await.unwrap(), RefreshOutcome::NotModified);
    let hits = server.hits(&t);
    assert_ne!(hits[3].if_none_match, hits[1].if_none_match);
    assert_eq!(ctx.search_changes(&original.id).await.len(), 2, "an unchanged article is not indexed again");

    // Pointing the source at another feed forgets the validators.
    let other = format!("{t}-moved");
    server.publish(&other, rss("Feed", "https://news.example/", &[]));
    update(
        &ctx,
        &admin,
        UpdateNewsSourceRequest {
            source_id: source.id.clone(),
            feed_url: Some(server.feed_url(&other)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(ctx.feed.refresh_source(source_id).await.unwrap(), RefreshOutcome::Fetched { new: 0, updated: 0 });
    let moved = server.hits(&other);
    assert_eq!((moved.len(), moved[0].if_none_match.clone()), (1, None));
}

#[tokio::test]
async fn failing_sources_are_recorded_and_retried_with_backoff() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    server.publish(&t, "<rss version=\"2.0\"><channel><title>cut off in the mid");
    let source = ctx.source_for(&admin, &server.feed_url(&t), "ja").await;
    let source_id = id_of(&source);

    // Malformed XML: an outcome, not an error; nothing is stored.
    let reason = failure(ctx.feed.refresh_source(source_id).await.unwrap());
    assert!(reason.starts_with("the document is not a feed"), "{reason}");
    let failed = ctx.managed_source(&admin, &source.id).await;
    assert_eq!(
        (failed.consecutive_failures, failed.last_error.as_str(), failed.article_count),
        (1, reason.as_str(), 0)
    );
    assert!(failed.last_success_at.as_option().is_none() && failed.last_fetched_at.as_option().is_some());
    let wait = |source: &ManagedNewsSource| {
        (tokuly_core::time::from_ts(source.next_fetch_at.as_option().unwrap()) - Utc::now()).num_seconds()
    };
    assert!((1700..=1800).contains(&wait(&failed)), "twice the interval after one failure, got {}s", wait(&failed));

    // Again: the pause doubles.
    failure(ctx.feed.refresh_source(source_id).await.unwrap());
    let failed = ctx.managed_source(&admin, &source.id).await;
    assert_eq!(failed.consecutive_failures, 2);
    assert!(
        (3500..=3600).contains(&wait(&failed)),
        "four times the interval after two failures, got {}s",
        wait(&failed)
    );

    // The feed is repaired: the failure state is forgotten.
    server.publish(
        &t,
        rss("Repaired", "https://news.example/", &[Item::new("Back", &format!("https://news.example/{t}/back"))]),
    );
    assert_eq!(ctx.feed.refresh_source(source_id).await.unwrap(), RefreshOutcome::Fetched { new: 1, updated: 0 });
    let repaired = ctx.managed_source(&admin, &source.id).await;
    assert_eq!((repaired.consecutive_failures, repaired.last_error.as_str(), repaired.article_count), (0, "", 1));
    assert!((800..=900).contains(&wait(&repaired)), "back to the normal interval, got {}s", wait(&repaired));

    // Everything else that can go wrong with a document.
    let big = format!("{t}-big");
    server.publish(
        &big,
        rss(
            "Big",
            "https://news.example/",
            &[Item::new("Huge", &format!("https://news.example/{t}/huge")).description(&"x".repeat(100_000))],
        ),
    );
    let not_xml = format!("{t}-html");
    server.publish(&not_xml, "<html><body><h1>Not a feed</h1></body></html>");
    for (path, expected) in [
        (format!("feeds/{big}"), "larger than 65536 bytes"),
        ("stream-big".to_owned(), "larger than 65536 bytes"),
        (format!("feeds/{not_xml}"), "the document is not a feed"),
        (format!("feeds/{t}-missing"), "HTTP 404"),
        ("status/500".to_owned(), "HTTP 500"),
        ("status/403".to_owned(), "HTTP 403"),
        ("redirect/loop".to_owned(), "more than 5 redirects"),
        ("slow".to_owned(), "took too long"),
    ] {
        let source = ctx.source_for(&admin, &format!("{}?case={t}", server.url(&path)), "ja").await;
        let reason = failure(ctx.feed.refresh_source(id_of(&source)).await.unwrap());
        assert!(reason.contains(expected), "{path}: expected {expected:?} in {reason:?}");
        assert_eq!(ctx.managed_source(&admin, &source.id).await.article_count, 0, "{path}");
    }
}

#[tokio::test]
async fn redirects_are_followed_only_to_places_the_fetcher_may_reach() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    server.publish(
        &t,
        rss("Moved", "https://news.example/", &[Item::new("Found", &format!("https://news.example/{t}/found"))]),
    );

    // A redirect to another document on a permitted host is followed.
    let moved = ctx.source_for(&admin, &server.url(&format!("redirect/to/{t}")), "ja").await;
    assert_eq!(ctx.feed.refresh_source(id_of(&moved)).await.unwrap(), RefreshOutcome::Fetched { new: 1, updated: 0 });
    assert_eq!(server.hits(&t).len(), 1);

    // Redirects into private networks, to the cloud metadata service, or to
    // another scheme are refused before any connection is made.
    for (path, expected) in [
        ("redirect/private", "refused: 10.11.12.13 is not a public address"),
        ("redirect/metadata", "refused: 169.254.169.254 is not a public address"),
        ("redirect/file", "refused: only http and https URLs are fetched"),
    ] {
        let source = ctx.source_for(&admin, &format!("{}?case={t}", server.url(path)), "ja").await;
        let reason = failure(ctx.feed.refresh_source(id_of(&source)).await.unwrap());
        assert_eq!(reason, expected, "{path}");
        let managed = ctx.managed_source(&admin, &source.id).await;
        assert_eq!(
            (managed.consecutive_failures, managed.last_error.as_str(), managed.article_count),
            (1, expected, 0)
        );
    }
}

#[tokio::test]
async fn without_the_development_flag_the_fetcher_never_touches_loopback() {
    let ctx = start_with(FeedConfig { news_allow_loopback: false, ..test_config() }).await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    server.publish(
        &t,
        rss("Local", "https://news.example/", &[Item::new("Secret", &format!("https://news.example/{t}/secret"))]),
    );

    // The API refuses such a source outright...
    let error =
        create(&ctx, Some(&admin), CreateNewsSourceRequest { feed_url: server.feed_url(&t), ..Default::default() })
            .await
            .unwrap_err();
    assert_eq!(error.reason, "FEED_URL_NOT_ALLOWED");
    let error = create(
        &ctx,
        Some(&admin),
        CreateNewsSourceRequest { feed_url: "http://127.0.0.1/rss.xml".into(), ..Default::default() },
    )
    .await
    .unwrap_err();
    assert_eq!(error.reason, "FEED_URL_NOT_ALLOWED");

    // ...and should one exist anyway (an old row, a name that now resolves
    // to this machine), the fetcher refuses it when fetching.
    let insert = |feed_url: String| {
        let db = &ctx.app.core.db;
        async move {
            sqlx::query_scalar::<_, Uuid>("INSERT INTO news_sources (name, feed_url) VALUES ('local', $1) RETURNING id")
                .bind(feed_url)
                .fetch_one(db)
                .await
                .unwrap()
        }
    };
    let by_port = insert(server.feed_url(&t)).await;
    assert_eq!(failure(ctx.feed.refresh_source(by_port).await.unwrap()), "refused: only the default ports are fetched");
    let by_address = insert(format!("http://127.0.0.1/{t}.xml")).await;
    assert_eq!(
        failure(ctx.feed.refresh_source(by_address).await.unwrap()),
        "refused: 127.0.0.1 is not a public address"
    );
    let by_name = insert(format!("http://localhost/{t}.xml")).await;
    let reason = failure(ctx.feed.refresh_source(by_name).await.unwrap());
    assert!(
        reason.starts_with("refused: ") && reason.ends_with("is not a public address"),
        "a name resolving to loopback is refused when resolved: {reason}"
    );

    assert!(server.hits(&t).is_empty(), "the local server was never contacted");
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM news_articles WHERE source_id = ANY($1)")
        .bind(vec![by_port, by_address, by_name])
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn an_article_carried_by_two_feeds_is_stored_once() {
    let ctx = start().await;
    let server = FeedServer::start().await;
    let (t, admin) = (token(), ctx.admin().await);
    let shared = format!("https://news.example/{t}/shared");
    let (first, second) = (format!("{t}-one"), format!("{t}-two"));
    // The same article with different tracking decorations, and twice in one feed.
    server.publish(
        &first,
        rss(
            "One",
            "https://one.example/",
            &[
                Item::new("Shared story", &format!("{shared}?utm_source=one")),
                Item::new("Shared story (again)", &format!("{shared}?utm_medium=feed#comments")),
            ],
        ),
    );
    server.publish(
        &second,
        rss(
            "Two",
            "https://two.example/",
            &[
                Item::new("Shared story, retitled", &format!("{shared}?fbclid=abc")),
                Item::new("Only in two", &format!("https://news.example/{t}/two-only")),
            ],
        ),
    );
    let one = ctx.source_for(&admin, &server.feed_url(&first), "ja").await;
    let two = ctx.source_for(&admin, &server.feed_url(&second), "ja").await;

    assert_eq!(ctx.feed.refresh_source(id_of(&one)).await.unwrap(), RefreshOutcome::Fetched { new: 1, updated: 0 });
    assert_eq!(
        ctx.feed.refresh_source(id_of(&two)).await.unwrap(),
        RefreshOutcome::Fetched { new: 1, updated: 0 },
        "the shared article is not stored again, nor retitled by the second feed"
    );

    let from_one = ctx.articles_of(&one.id).await;
    assert_eq!(
        from_one.iter().map(|a| (a.title.as_str(), a.url.as_str())).collect::<Vec<_>>(),
        vec![("Shared story", format!("{shared}?utm_source=one").as_str())]
    );
    let from_two = ctx.articles_of(&two.id).await;
    assert_eq!(from_two.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), vec!["Only in two"]);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM news_articles WHERE canonical_url = $1")
        .bind(&shared)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(rows, 1);

    // Fetching the first feed again changes nothing.
    server.publish(
        &first,
        rss("One", "https://one.example/", &[Item::new("Shared story", &format!("{shared}?utm_source=one"))]),
    );
    assert_eq!(ctx.feed.refresh_source(id_of(&one)).await.unwrap(), RefreshOutcome::Fetched { new: 0, updated: 0 });
}

#[tokio::test]
async fn sources_are_seeded_from_configuration_and_fetched_on_schedule() {
    let server = FeedServer::start().await;
    let t = token();
    let (good, other) = (server.feed_url(&format!("{t}-seed-a")), server.feed_url(&format!("{t}-seed-b")));
    let ctx = start_with(FeedConfig {
        news_feeds: format!(" {good} , not-a-url,ftp://nope.example/feed, http://10.0.0.1/private.xml ,{other},{good}"),
        ..test_config()
    })
    .await;
    let admin = ctx.admin().await;

    // Only the fetchable URLs become sources, once each, however often seeding runs.
    assert_eq!(ctx.feed.seed_config_sources().await.unwrap(), 2);
    assert_eq!(ctx.feed.seed_config_sources().await.unwrap(), 0);
    let all: ListManagedNewsSourcesResponse = ctx
        .call(NEWS_ADMIN, "ListManagedNewsSources", Some(&admin), &ListManagedNewsSourcesRequest::default())
        .await
        .unwrap();
    let seeded: Vec<&ManagedNewsSource> =
        all.sources.iter().filter(|s| s.feed_url == good || s.feed_url == other).collect();
    assert_eq!(seeded.len(), 2);
    assert!(seeded.iter().all(|s| s.from_config && s.enabled && s.language == "ja" && s.name == "127.0.0.1"));
    assert!(!all.sources.iter().any(|s| s.feed_url.contains("10.0.0.1") || s.feed_url.contains("nope.example")));

    // A new source is due at once: the scheduler claims it (pushing its next
    // fetch one interval ahead) and queues a job; claimed sources are not
    // claimed again.
    let source = seeded[0].clone();
    let due_at = |source: ManagedNewsSource| tokuly_core::time::from_ts(source.next_fetch_at.as_option().unwrap());
    assert!(due_at(source.clone()) <= Utc::now());
    ctx.feed.enqueue_due_fetches().await.unwrap();
    let claimed = ctx.managed_source(&admin, &source.id).await;
    let wait = (due_at(claimed) - Utc::now()).num_seconds();
    assert!((800..=900).contains(&wait), "next fetch one interval from now, got {wait}s");

    // The job fetches the source. (No worker runs in tests: call it directly.)
    server.publish(
        &format!("{t}-seed-a"),
        rss(
            "Seeded",
            "https://seed.example/",
            &[Item::new("From config", &format!("https://news.example/{t}/seeded"))],
        ),
    );
    server.publish(&format!("{t}-seed-b"), rss("Seeded B", "https://seed.example/", &[]));
    for source in &seeded {
        assert!(matches!(ctx.feed.refresh_source(id_of(source)).await.unwrap(), RefreshOutcome::Fetched { .. }));
    }
    let mut names = Vec::new();
    for source in &seeded {
        names.push(ctx.managed_source(&admin, &source.id).await.name);
    }
    names.sort();
    assert_eq!(names, vec!["Seeded", "Seeded B"], "seeded sources take their names from their feeds");

    // A source that is switched off, or unknown, is skipped.
    update(
        &ctx,
        &admin,
        UpdateNewsSourceRequest { source_id: seeded[0].id.clone(), enabled: Some(false), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(ctx.feed.refresh_source(id_of(seeded[0])).await.unwrap(), RefreshOutcome::Skipped);
    assert_eq!(ctx.feed.refresh_source(Uuid::now_v7()).await.unwrap(), RefreshOutcome::Skipped);
    // And one that is being fetched right now is not fetched twice.
    let busy = id_of(seeded[1]);
    assert!(
        ctx.app
            .core
            .cache
            .set_nx(&format!("feed:news:fetching:{busy}"), "1", std::time::Duration::from_secs(30))
            .await
            .unwrap()
    );
    assert_eq!(ctx.feed.refresh_source(busy).await.unwrap(), RefreshOutcome::Skipped);
    ctx.app.core.cache.del(&format!("feed:news:fetching:{busy}")).await.unwrap();
    assert!(matches!(
        ctx.feed.refresh_source(busy).await.unwrap(),
        RefreshOutcome::NotModified | RefreshOutcome::Fetched { .. }
    ));
}
