//! What readers get of an article: its fields, its pictures (from this
//! instance's image proxy) and its search document.
mod common;

use std::collections::{HashMap, HashSet};

use common::*;
use tokuly_core::time::now;
use tokuly_feed::news::articles::{ArticleRow, ReaderMarks, article_path};
use tokuly_media::link_preview::image_proxy::{Kind, proxy_url};
use tokuly_proto::feed::NewsFeedback;
use uuid::Uuid;

fn row() -> ArticleRow {
    ArticleRow {
        id: Uuid::now_v7(),
        source_id: Uuid::now_v7(),
        url: "https://news.example/a?utm_source=rss".into(),
        title: "見出し".into(),
        summary: "要約".into(),
        image_url: "https://cdn.example/a.jpg".into(),
        tags: vec!["ゲーム".into(), "tech".into()],
        published_at: now(),
        click_count: 12,
        like_count: 3,
        comment_count: 1,
        source_name: "Example News".into(),
        source_site_url: "https://news.example/".into(),
        source_favicon_url: "https://news.example/favicon.ico".into(),
        source_language: "ja".into(),
    }
}

#[tokio::test]
async fn articles_carry_what_the_info_sheet_needs() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let row = row();
    let marks = ReaderMarks { clicked: HashSet::from([row.id]), feedback: HashMap::from([(row.id, 2)]) };
    let article = row.to_proto(core, &marks, true);
    assert_eq!(article.tags, vec!["ゲーム", "tech"]);
    let source = article.source.as_option().unwrap();
    assert_eq!(
        (source.name.as_str(), source.site_url.as_str(), source.language.as_str()),
        ("Example News", "https://news.example/", "ja")
    );
    assert!(article.random_pick && article.clicked);
    // Pictures come from this instance's image proxy, never from the site.
    assert_eq!(article.image_url, proxy_url(core, Kind::Image, &row.image_url));
    assert!(
        article.image_url.starts_with("http://localhost:8080/api/media/link-image/image/"),
        "{}",
        article.image_url
    );
    assert_eq!(source.favicon_url, proxy_url(core, Kind::Icon, &row.source_favicon_url));
    assert!(source.favicon_url.contains("/media/link-image/icon/"), "{}", source.favicon_url);
    assert_eq!(article.feedback, NewsFeedback::NEWS_FEEDBACK_LESS);
    assert_eq!(article.path, format!("/news/articles/{}", row.id));
    assert_eq!((article.click_count, article.like_count, article.comment_count), (12, 3, 1));

    let untouched = row.to_proto(core, &ReaderMarks::default(), false);
    assert!(!untouched.random_pick && !untouched.clicked);
    assert_eq!(untouched.feedback, NewsFeedback::NEWS_FEEDBACK_UNSPECIFIED);
}

#[tokio::test]
async fn search_documents_are_public_ownerless_news() {
    let ctx = start().await;
    let core = &ctx.app.core;
    let row = row();
    let doc = row.doc().search_doc(core);
    assert_eq!((doc.kind.as_str(), doc.id, doc.public, doc.owner_id), ("news", row.id, true, None));
    assert_eq!((doc.title.as_str(), doc.text.as_str()), ("見出し", "要約"));
    assert_eq!(doc.tags, row.tags);
    assert_eq!(doc.path, article_path(row.id));
    assert_eq!(doc.image_url, Some(proxy_url(core, Kind::Image, &row.image_url)));
    assert_eq!(doc.created_at, row.published_at.timestamp());
    assert_eq!(doc.extra["source_name"], "Example News");
    assert_eq!(doc.extra["url"], row.url.as_str());

    let mut bare = row;
    bare.image_url = String::new();
    bare.source_favicon_url = String::new();
    assert_eq!(bare.doc().search_doc(core).image_url, None);
    let article = bare.to_proto(core, &ReaderMarks::default(), false);
    assert_eq!((article.image_url.as_str(), article.source.as_option().unwrap().favicon_url.as_str()), ("", ""));
}
