//! Announcements: the editor, the public update log and the notify-everyone broadcast.
mod common;

use common::*;
use tokuly_admin::{AdminConfig, announcements::BroadcastJob};
use tokuly_core::outbound::NotificationRequest;
use tokuly_proto::{
    admin::*,
    common::{Image, PageRequest},
};
use tokuly_testkit::{TestApp, TestUser};
use uuid::Uuid;

fn text(ja: &str, en: &str) -> LocalizedText {
    LocalizedText { ja: ja.into(), en: en.into(), ..Default::default() }
}

fn draft(title: &str) -> CreateAnnouncementRequest {
    CreateAnnouncementRequest {
        title: text(title, "").into(),
        body: text("## 新機能\n\n**スタンプ**を自分で作れるようになりました!", "").into(),
        ..Default::default()
    }
}

async fn create(app: &TestApp, boss: &TestUser, request: CreateAnnouncementRequest) -> ManagedAnnouncement {
    let response: CreateAnnouncementResponse = app.call(&format!("{ANNOUNCEMENTS}/CreateAnnouncement"), Some(boss), &request).await.unwrap();
    response.announcement.into_option().unwrap()
}

async fn publish(app: &TestApp, boss: &TestUser, request: PublishAnnouncementRequest) -> ManagedAnnouncement {
    let response: PublishAnnouncementResponse = app.call(&format!("{ANNOUNCEMENTS}/PublishAnnouncement"), Some(boss), &request).await.unwrap();
    response.announcement.into_option().unwrap()
}

async fn publish_now(app: &TestApp, boss: &TestUser, id: &str) -> ManagedAnnouncement {
    publish(app, boss, PublishAnnouncementRequest { id: id.into(), ..Default::default() }).await
}

/// The public feed as an anonymous visitor sees it (first 50 entries).
async fn public_feed(app: &TestApp, request: ListAnnouncementsRequest) -> Vec<Announcement> {
    let request = ListAnnouncementsRequest { page: PageRequest { limit: 50, ..Default::default() }.into(), ..request };
    let response: ListAnnouncementsResponse = app.call(&format!("{ANNOUNCEMENTS}/ListAnnouncements"), None, &request).await.unwrap();
    response.announcements
}

async fn public_get(app: &TestApp, id: &str, locale: &str) -> Result<GetAnnouncementResponse, tokuly_testkit::RpcError> {
    app.call(&format!("{ANNOUNCEMENTS}/GetAnnouncement"), None, &GetAnnouncementRequest { id: id.into(), locale: locale.into(), ..Default::default() }).await
}

/// `announcement.published` requests in the outbox for one announcement.
async fn broadcast_recipients(app: &TestApp, announcement_id: &str) -> Vec<NotificationRequest> {
    app.events("notify.requested")
        .await
        .into_iter()
        .filter_map(|event| event.data::<NotificationRequest>().ok())
        .filter(|request| request.kind == "announcement.published" && request.data["announcement_id"] == announcement_id)
        .collect()
}

#[tokio::test]
async fn editing_is_for_admins_and_reading_is_public() {
    let (app, _admin) = start().await;
    let user = app.user("user").await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    for rpc in [
        "ListManagedAnnouncements",
        "GetManagedAnnouncement",
        "CreateAnnouncement",
        "UpdateAnnouncement",
        "PublishAnnouncement",
        "UnpublishAnnouncement",
        "DeleteAnnouncement",
    ] {
        assert_role_gated(&app, &format!("{ANNOUNCEMENTS}/{rpc}"), &[&user, &moderator]).await;
    }
    // The update log itself needs no account, and works with one.
    let _: ListAnnouncementsResponse = app.call(&format!("{ANNOUNCEMENTS}/ListAnnouncements"), None, &ListAnnouncementsRequest::default()).await.unwrap();
    let _: ListAnnouncementsResponse =
        app.call(&format!("{ANNOUNCEMENTS}/ListAnnouncements"), Some(&user), &ListAnnouncementsRequest::default()).await.unwrap();
}

#[tokio::test]
async fn announcement_goes_from_draft_to_published_and_back() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let image = Image { url: "https://cdn.tokuly.example/media/news/stamps.webp".into(), width: 1200, height: 630, ..Default::default() };

    let created =
        create(&app, &boss, CreateAnnouncementRequest { image: image.clone().into(), pinned: true, ..draft("スタンプが作れるようになりました!") }).await;
    assert_eq!(created.status, AnnouncementStatus::ANNOUNCEMENT_STATUS_DRAFT);
    assert_eq!(created.category, AnnouncementCategory::ANNOUNCEMENT_CATEGORY_UPDATE, "the default category");
    assert_eq!(created.author.id, boss.id.to_string());
    assert!(created.published_at.is_unset() && created.pinned && !created.notify_users);
    assert_eq!(created.image.url, image.url);

    // Drafts are invisible to the public.
    assert_eq!(public_get(&app, &created.id, "").await.unwrap_err().reason, "ANNOUNCEMENT_NOT_FOUND");
    assert!(public_feed(&app, ListAnnouncementsRequest::default()).await.iter().all(|a| a.id != created.id));

    // Edit: add the English translation, change the category, unpin.
    let updated: UpdateAnnouncementResponse = app
        .call(
            &format!("{ANNOUNCEMENTS}/UpdateAnnouncement"),
            Some(&boss),
            &UpdateAnnouncementRequest {
                id: created.id.clone(),
                category: Some(AnnouncementCategory::ANNOUNCEMENT_CATEGORY_EVENT.into()),
                title: text("スタンプが作れるようになりました!", "Make your own stamps!").into(),
                body: text("**スタンプ**を自分で作れます。", "You can now **create** your own stamps.").into(),
                pinned: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.announcement.category, AnnouncementCategory::ANNOUNCEMENT_CATEGORY_EVENT);
    assert_eq!(updated.announcement.title.en, "Make your own stamps!");
    assert!(!updated.announcement.pinned);
    assert_eq!(updated.announcement.image.url, image.url, "fields that are not sent stay");

    let published = publish_now(&app, &boss, &created.id).await;
    assert_eq!(published.status, AnnouncementStatus::ANNOUNCEMENT_STATUS_PUBLISHED);
    assert!(published.published_at.is_set());

    // Public, in the reader's language. The test client sends `Accept-Language: ja`.
    let japanese = public_get(&app, &created.id, "").await.unwrap().announcement.into_option().unwrap();
    assert_eq!((japanese.locale.as_str(), japanese.title.as_str()), ("ja", "スタンプが作れるようになりました!"));
    assert_eq!(japanese.summary, "スタンプを自分で作れます。", "the summary falls back to the start of the body, without Markdown");
    assert_eq!(japanese.image.url, image.url);
    let english = public_get(&app, &created.id, "en-US").await.unwrap().announcement.into_option().unwrap();
    assert_eq!(
        (english.locale.as_str(), english.title.as_str(), english.body.as_str()),
        ("en", "Make your own stamps!", "You can now **create** your own stamps.")
    );
    assert_eq!(english.summary, "You can now create your own stamps.");
    let in_feed = public_feed(&app, ListAnnouncementsRequest { locale: "en".into(), ..Default::default() }).await;
    assert_eq!(in_feed.iter().find(|a| a.id == created.id).unwrap().title, "Make your own stamps!");

    let again = app
        .call::<_, PublishAnnouncementResponse>(
            &format!("{ANNOUNCEMENTS}/PublishAnnouncement"),
            Some(&boss),
            &PublishAnnouncementRequest { id: created.id.clone(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!((again.code.as_str(), again.reason.as_str()), ("failed_precondition", "ANNOUNCEMENT_ALREADY_PUBLISHED"));

    // Remove the image, then take it offline again.
    let cleared: UpdateAnnouncementResponse = app
        .call(
            &format!("{ANNOUNCEMENTS}/UpdateAnnouncement"),
            Some(&boss),
            &UpdateAnnouncementRequest { id: created.id.clone(), clear_image: true, ..Default::default() },
        )
        .await
        .unwrap();
    assert!(cleared.announcement.image.is_unset());
    let unpublished: UnpublishAnnouncementResponse = app
        .call(&format!("{ANNOUNCEMENTS}/UnpublishAnnouncement"), Some(&boss), &UnpublishAnnouncementRequest { id: created.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(unpublished.announcement.status, AnnouncementStatus::ANNOUNCEMENT_STATUS_DRAFT);
    assert_eq!(public_get(&app, &created.id, "").await.unwrap_err().reason, "ANNOUNCEMENT_NOT_FOUND");
    let twice = app
        .call::<_, UnpublishAnnouncementResponse>(
            &format!("{ANNOUNCEMENTS}/UnpublishAnnouncement"),
            Some(&boss),
            &UnpublishAnnouncementRequest { id: created.id.clone(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!(twice.reason, "ANNOUNCEMENT_NOT_PUBLISHED");

    let _: DeleteAnnouncementResponse = app
        .call(&format!("{ANNOUNCEMENTS}/DeleteAnnouncement"), Some(&boss), &DeleteAnnouncementRequest { id: created.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let gone = app
        .call::<_, GetManagedAnnouncementResponse>(
            &format!("{ANNOUNCEMENTS}/GetManagedAnnouncement"),
            Some(&boss),
            &GetManagedAnnouncementRequest { id: created.id.clone(), ..Default::default() },
        )
        .await
        .unwrap_err();
    assert_eq!((gone.code.as_str(), gone.reason.as_str()), ("not_found", "ANNOUNCEMENT_NOT_FOUND"));

    let audit: ListAuditLogResponse = app
        .call(
            &format!("{SYSTEM}/ListAuditLog"),
            Some(&boss),
            &ListAuditLogRequest { target_type: "announcement".into(), target_id: created.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(
        audit.entries.iter().map(|e| e.action.as_str()).collect::<Vec<_>>(),
        vec![
            "announcement.deleted",
            "announcement.unpublished",
            "announcement.updated",
            "announcement.published",
            "announcement.updated",
            "announcement.created"
        ]
    );
}

#[tokio::test]
async fn public_feed_puts_pinned_first_and_pages() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let tag = tokuly_core::ids::random_base62(8);

    let older = create(&app, &boss, draft(&format!("{tag} 1"))).await;
    let pinned = create(
        &app,
        &boss,
        CreateAnnouncementRequest { pinned: true, category: AnnouncementCategory::ANNOUNCEMENT_CATEGORY_IMPORTANT.into(), ..draft(&format!("{tag} 2")) },
    )
    .await;
    let newer = create(&app, &boss, draft(&format!("{tag} 3"))).await;
    for announcement in [&older, &pinned, &newer] {
        publish_now(&app, &boss, &announcement.id).await;
    }
    let mine = |feed: &[Announcement]| feed.iter().filter(|a| a.title.starts_with(&tag)).map(|a| a.id.clone()).collect::<Vec<_>>();

    // Pinned first, then newest first.
    let feed = public_feed(&app, ListAnnouncementsRequest::default()).await;
    assert_eq!(mine(&feed), vec![pinned.id.clone(), newer.id.clone(), older.id.clone()]);
    let important =
        public_feed(&app, ListAnnouncementsRequest { category: AnnouncementCategory::ANNOUNCEMENT_CATEGORY_IMPORTANT.into(), ..Default::default() }).await;
    assert_eq!(mine(&important), vec![pinned.id.clone()]);

    // Walking the whole feed one entry per page visits each announcement once, in the same order.
    let mut walked = Vec::new();
    let mut cursor = String::new();
    loop {
        let page: ListAnnouncementsResponse = app
            .call(
                &format!("{ANNOUNCEMENTS}/ListAnnouncements"),
                None,
                &ListAnnouncementsRequest { page: PageRequest { limit: 1, cursor: cursor.clone(), ..Default::default() }.into(), ..Default::default() },
            )
            .await
            .unwrap();
        assert!(page.announcements.len() <= 1);
        walked.extend(page.announcements);
        if !page.page.has_more {
            break;
        }
        cursor = page.page.next_cursor.clone();
    }
    assert_eq!(mine(&walked), vec![pinned.id.clone(), newer.id.clone(), older.id.clone()]);

    // The editor lists everything, filterable by state.
    let extra_draft = create(&app, &boss, draft(&format!("{tag} 4"))).await;
    let managed = |status: AnnouncementStatus| {
        let (app, boss, tag) = (&app, &boss, &tag);
        async move {
            let response: ListManagedAnnouncementsResponse = app
                .call(
                    &format!("{ANNOUNCEMENTS}/ListManagedAnnouncements"),
                    Some(boss),
                    &ListManagedAnnouncementsRequest {
                        status: status.into(),
                        page: PageRequest { limit: 100, ..Default::default() }.into(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            response.announcements.into_iter().filter(|a| a.title.ja.starts_with(tag.as_str())).map(|a| a.id.clone()).collect::<Vec<_>>()
        }
    };
    assert_eq!(
        managed(AnnouncementStatus::ANNOUNCEMENT_STATUS_UNSPECIFIED).await,
        vec![extra_draft.id.clone(), newer.id.clone(), pinned.id.clone(), older.id.clone()]
    );
    assert_eq!(managed(AnnouncementStatus::ANNOUNCEMENT_STATUS_DRAFT).await, vec![extra_draft.id.clone()]);
    assert_eq!(managed(AnnouncementStatus::ANNOUNCEMENT_STATUS_PUBLISHED).await.len(), 3);
    assert!(managed(AnnouncementStatus::ANNOUNCEMENT_STATUS_SCHEDULED).await.is_empty());
}

#[tokio::test]
async fn scheduled_announcements_stay_hidden_until_their_time() {
    let (app, admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let created = create(&app, &boss, draft("メンテナンスのお知らせ")).await;
    let in_an_hour = chrono::Utc::now() + chrono::Duration::hours(1);

    let scheduled = publish(
        &app,
        &boss,
        PublishAnnouncementRequest { id: created.id.clone(), publish_at: tokuly_core::time::ts(in_an_hour), notify_users: true, ..Default::default() },
    )
    .await;
    assert_eq!(scheduled.status, AnnouncementStatus::ANNOUNCEMENT_STATUS_SCHEDULED);
    assert!(scheduled.notify_users);
    assert_eq!(public_get(&app, &created.id, "").await.unwrap_err().reason, "ANNOUNCEMENT_NOT_FOUND");
    assert!(public_feed(&app, ListAnnouncementsRequest::default()).await.iter().all(|a| a.id != created.id));
    // Nothing is broadcast before the publish time, even if the job runs.
    admin.queue_due_broadcasts().await.unwrap();
    admin.run_broadcast(BroadcastJob { announcement_id: created.id.parse().unwrap() }).await.unwrap();
    assert!(broadcast_recipients(&app, &created.id).await.is_empty());

    // Rescheduling is publishing again; a time in the past publishes now.
    let published = publish(
        &app,
        &boss,
        PublishAnnouncementRequest {
            id: created.id.clone(),
            publish_at: tokuly_core::time::ts(chrono::Utc::now() - chrono::Duration::days(3)),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(published.status, AnnouncementStatus::ANNOUNCEMENT_STATUS_PUBLISHED);
    let published_at = tokuly_core::time::from_ts(published.published_at.as_option().unwrap());
    assert!((chrono::Utc::now() - published_at).num_seconds().abs() < 60, "not backdated");
    assert!(public_get(&app, &created.id, "").await.is_ok());

    let too_far = create(&app, &boss, draft("遠い未来")).await;
    let err = app
        .call::<_, PublishAnnouncementResponse>(
            &format!("{ANNOUNCEMENTS}/PublishAnnouncement"),
            Some(&boss),
            &PublishAnnouncementRequest {
                id: too_far.id.clone(),
                publish_at: tokuly_core::time::ts(chrono::Utc::now() + chrono::Duration::days(400)),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("invalid_argument", "PUBLISH_TIME_INVALID"));
}

#[tokio::test]
async fn announcement_input_is_validated() {
    let (app, _admin) = start().await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let bad_image = Image { url: "javascript:alert(1)".into(), ..Default::default() };

    let creations: Vec<(&str, CreateAnnouncementRequest)> = vec![
        ("TITLE_INVALID", CreateAnnouncementRequest { title: Default::default(), ..draft("x") }),
        ("TITLE_INVALID", CreateAnnouncementRequest { title: text("", "English only").into(), ..draft("x") }),
        ("TITLE_INVALID", draft(&"あ".repeat(121))),
        ("BODY_INVALID", CreateAnnouncementRequest { body: text(" ", "").into(), ..draft("x") }),
        ("BODY_INVALID", CreateAnnouncementRequest { body: text(&"x".repeat(20_001), "").into(), ..draft("x") }),
        ("SUMMARY_TOO_LONG", CreateAnnouncementRequest { summary: text(&"あ".repeat(301), "").into(), ..draft("x") }),
        ("IMAGE_INVALID", CreateAnnouncementRequest { image: bad_image.clone().into(), ..draft("x") }),
    ];
    for (reason, request) in creations {
        let err = app.call::<_, CreateAnnouncementResponse>(&format!("{ANNOUNCEMENTS}/CreateAnnouncement"), Some(&boss), &request).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), ("invalid_argument", reason), "{err}");
    }
    // An unknown category number is refused (sent as raw JSON: the typed request cannot express it).
    let err = app
        .call::<_, CreateAnnouncementResponse>(
            &format!("{ANNOUNCEMENTS}/CreateAnnouncement"),
            Some(&boss),
            &serde_json::json!({ "category": 99, "title": { "ja": "x" }, "body": { "ja": "x" } }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.reason, "CATEGORY_INVALID");

    let existing = create(&app, &boss, draft("検証用")).await;
    let updates: Vec<(&str, UpdateAnnouncementRequest)> = vec![
        ("ANNOUNCEMENT_NOT_FOUND", UpdateAnnouncementRequest { id: Uuid::now_v7().to_string(), pinned: Some(true), ..Default::default() }),
        ("INVALID_ID", UpdateAnnouncementRequest { id: "nope".into(), ..Default::default() }),
        ("TITLE_INVALID", UpdateAnnouncementRequest { id: existing.id.clone(), title: text("", "").into(), ..Default::default() }),
        ("BODY_INVALID", UpdateAnnouncementRequest { id: existing.id.clone(), body: text("", "only english").into(), ..Default::default() }),
        ("SUMMARY_TOO_LONG", UpdateAnnouncementRequest { id: existing.id.clone(), summary: text("ok", &"x".repeat(301)).into(), ..Default::default() }),
        ("IMAGE_INVALID", UpdateAnnouncementRequest { id: existing.id.clone(), image: bad_image.into(), ..Default::default() }),
        (
            "CATEGORY_INVALID",
            UpdateAnnouncementRequest {
                id: existing.id.clone(),
                category: Some(AnnouncementCategory::ANNOUNCEMENT_CATEGORY_UNSPECIFIED.into()),
                ..Default::default()
            },
        ),
    ];
    for (reason, request) in updates {
        let err = app.call::<_, UpdateAnnouncementResponse>(&format!("{ANNOUNCEMENTS}/UpdateAnnouncement"), Some(&boss), &request).await.unwrap_err();
        assert_eq!(err.reason, reason, "{err}");
    }
    for rpc in ["PublishAnnouncement", "UnpublishAnnouncement", "DeleteAnnouncement"] {
        let err =
            app.call::<_, serde_json::Value>(&format!("{ANNOUNCEMENTS}/{rpc}"), Some(&boss), &serde_json::json!({ "id": Uuid::now_v7() })).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), ("not_found", "ANNOUNCEMENT_NOT_FOUND"), "{rpc}");
    }
    let unchanged: GetManagedAnnouncementResponse = app
        .call(&format!("{ANNOUNCEMENTS}/GetManagedAnnouncement"), Some(&boss), &GetManagedAnnouncementRequest { id: existing.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(unchanged.announcement.title.ja, "検証用");
}

#[tokio::test]
async fn publishing_with_notify_reaches_every_active_user_exactly_once() {
    // A chunk of 2 forces the broadcast through several transactions.
    let (app, admin) = start_with(AdminConfig { announcement_notify_chunk: 2, ..Default::default() }).await;
    let boss = app.user_with_roles("boss", &["admin"]).await;
    let active: Vec<TestUser> = vec![app.user("a").await, app.user("b").await, app.user("c").await];
    let signed_out = app.user("signed_out").await;
    app.identity.revoke_all_sessions(signed_out.id, None).await.unwrap();
    let dormant = app.user("dormant").await;
    sqlx::query("UPDATE sessions SET last_active_at = now() - interval '90 days' WHERE user_id = $1").bind(dormant.id).execute(&app.core.db).await.unwrap();
    let banned = app.user("banned").await;
    app.identity.set_banned(banned.id, Some("test")).await.unwrap();

    let created = create(
        &app,
        &boss,
        CreateAnnouncementRequest {
            title: text("新機能のお知らせ!", "New feature!").into(),
            summary: text("スタンプを作ろう!", "Create stamps!").into(),
            body: text("本文", "Body").into(),
            image: Image { url: "https://cdn.tokuly.example/news.webp".into(), ..Default::default() }.into(),
            ..Default::default()
        },
    )
    .await;
    let announcement_id: Uuid = created.id.parse().unwrap();

    // Without the notify option nothing is sent.
    publish_now(&app, &boss, &created.id).await;
    admin.run_broadcast(BroadcastJob { announcement_id }).await.unwrap();
    assert!(broadcast_recipients(&app, &created.id).await.is_empty());

    // Republish with it; in production the queued job does what we run by hand here.
    let _: UnpublishAnnouncementResponse = app
        .call(&format!("{ANNOUNCEMENTS}/UnpublishAnnouncement"), Some(&boss), &UnpublishAnnouncementRequest { id: created.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let published = publish(&app, &boss, PublishAnnouncementRequest { id: created.id.clone(), notify_users: true, ..Default::default() }).await;
    assert!(published.notify_users && published.notified_at.is_unset());
    let queued_at = || async {
        let at: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar("SELECT broadcast_queued_at FROM announcements WHERE id = $1").bind(announcement_id).fetch_one(&app.core.db).await.unwrap();
        at
    };
    let first_queued = queued_at().await;
    assert!(first_queued.is_some(), "publishing queued the broadcast job");
    admin.queue_due_broadcasts().await.unwrap();
    assert_eq!(queued_at().await, first_queued, "the worker's next pass does not queue it a second time");

    admin.run_broadcast(BroadcastJob { announcement_id }).await.unwrap();
    let sent = broadcast_recipients(&app, &created.id).await;
    let count_for = |user: &TestUser| sent.iter().filter(|n| n.user_id == user.id).count();
    for user in &active {
        assert_eq!(count_for(user), 1, "{} is notified once", user.handle);
    }
    assert_eq!(count_for(&boss), 1, "the author is an active user too");
    for skipped in [&signed_out, &dormant, &banned] {
        assert_eq!(count_for(skipped), 0, "{} is not notified", skipped.handle);
    }
    let notice = sent.iter().find(|n| n.user_id == active[0].id).unwrap();
    assert_eq!((notice.title.ja.as_str(), notice.title.en.as_str()), ("新機能のお知らせ!", "New feature!"));
    assert_eq!((notice.body.ja.as_str(), notice.body.en.as_str()), ("スタンプを作ろう!", "Create stamps!"));
    assert_eq!(notice.path, format!("/announcements/{}", created.id));
    assert_eq!(notice.image_url.as_deref(), Some("https://cdn.tokuly.example/news.webp"));
    assert!(notice.inbox && notice.push);

    let finished: GetManagedAnnouncementResponse = app
        .call(&format!("{ANNOUNCEMENTS}/GetManagedAnnouncement"), Some(&boss), &GetManagedAnnouncementRequest { id: created.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(finished.announcement.notified_at.is_set());
    assert_eq!(finished.announcement.notified_count as usize, sent.len());

    // A duplicated or retried job, or publishing it again, never notifies twice.
    admin.run_broadcast(BroadcastJob { announcement_id }).await.unwrap();
    let _: UnpublishAnnouncementResponse = app
        .call(&format!("{ANNOUNCEMENTS}/UnpublishAnnouncement"), Some(&boss), &UnpublishAnnouncementRequest { id: created.id.clone(), ..Default::default() })
        .await
        .unwrap();
    publish(&app, &boss, PublishAnnouncementRequest { id: created.id.clone(), notify_users: true, ..Default::default() }).await;
    admin.run_broadcast(BroadcastJob { announcement_id }).await.unwrap();
    assert_eq!(broadcast_recipients(&app, &created.id).await.len(), sent.len());

    // A job for an announcement that was deleted meanwhile just ends.
    admin.run_broadcast(BroadcastJob { announcement_id: Uuid::now_v7() }).await.unwrap();
}
