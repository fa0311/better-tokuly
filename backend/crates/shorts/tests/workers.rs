//! Event consumers, the announcement and reindex jobs, housekeeping and the
//! data export: what happens to shorts when something happens elsewhere.
mod common;

use common::*;
use tokuly_media::assets;
use tokuly_proto::{
    common::{SubjectType, Visibility},
    shorts::*,
    video::{DeleteChannelRequest, DeleteChannelResponse, NotificationLevel, UnsubscribeRequest, UnsubscribeResponse},
};
use tokuly_shorts::{
    ShortsConfig,
    events::{AnnounceJob, ReindexJob},
};
use tokuly_testkit::TestUser;

async fn save(ctx: &Ctx, user: &TestUser, short_id: &str) {
    let request = SaveShortRequest { short_id: short_id.to_owned(), ..Default::default() };
    let _: SaveShortResponse = ctx.call(SHORTS, "SaveShort", Some(user), &request).await.expect("save");
}

async fn saved(ctx: &Ctx, user: &TestUser) -> Vec<String> {
    let response: ListSavedShortsResponse =
        ctx.call(SHORTS, "ListSavedShorts", Some(user), &ListSavedShortsRequest::default()).await.unwrap();
    response.shorts.into_iter().map(|short| short.caption).collect()
}

async fn hashtag_count(ctx: &Ctx, tag: &str) -> i64 {
    let request = GetHashtagRequest { tag: tag.to_owned(), ..Default::default() };
    ctx.call::<_, GetHashtagResponse>(DISCOVERY, "GetHashtag", None, &request)
        .await
        .ok()
        .and_then(|response| response.hashtag.as_option().map(|hashtag| hashtag.short_count))
        .unwrap_or(0)
}

async fn set_banned(ctx: &Ctx, user: &TestUser, banned: bool) {
    sqlx::query("UPDATE users SET banned_at = CASE WHEN $2 THEN now() END WHERE id = $1")
        .bind(user.id)
        .bind(banned)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

fn user_event(user: &TestUser) -> serde_json::Value {
    serde_json::json!({ "user_id": user.id })
}

/// The `shorts.published` notifications requested so far, as `(recipient, push)`.
async fn announcements(ctx: &Ctx, short_id: &str) -> Vec<(uuid::Uuid, bool)> {
    ctx.app
        .events("notify.requested")
        .await
        .into_iter()
        .filter(|event| event.data["kind"] == "shorts.published" && event.data["data"]["id"] == short_id)
        .map(|event| (event.data["user_id"].as_str().unwrap().parse().unwrap(), event.data["push"].as_bool().unwrap()))
        .collect()
}

#[tokio::test]
async fn an_erased_account_takes_its_shorts_and_its_traces() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let a = ctx.channel(&alice, "a").await;
    let b = ctx.channel(&bob, "b").await;
    let tag = unique("erase");
    let hers = ctx.short(&alice, &a, &format!("ありすの #{tag}")).await;
    let his = ctx.short(&bob, &b, &format!("ボブの @{} へ", alice.handle)).await;
    assert_eq!(his.mentions.len(), 1);
    save(&ctx, &alice, &his.id).await;
    save(&ctx, &bob, &hers.id).await;
    ctx.like(&alice, &his.id).await;
    ctx.mark_not_interested(&alice, &his.id).await;
    ctx.subscribe(&alice, &b, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    {
        let _lock = playback_lock().await;
        ctx.watch(&alice, &his.id).await;
        ctx.flush().await;
    }
    let asset_id = hers.studio.as_option().unwrap().asset_id.clone();

    // The identity domain erased alice. The event may well arrive twice.
    for _ in 0..2 {
        ctx.deliver_event("identity.user.deleted", None, user_event(&alice)).await;
    }
    assert_eq!(ctx.get(None, &hers.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert_eq!(hashtag_count(&ctx, &tag).await, 0);
    assert!(assets::find(&ctx.app.core, id(&asset_id)).await.unwrap().is_none());
    assert!(saved(&ctx, &bob).await.is_empty());
    let deleted = ctx
        .app
        .events("shorts.short.deleted")
        .await
        .into_iter()
        .filter(|e| e.data["id"] == hers.id)
        .collect::<Vec<_>>();
    assert_eq!(deleted.len(), 1);
    assert_eq!((deleted[0].data["reason"].as_str(), deleted[0].actor_id), (Some("account_deleted"), None));
    // Bob's short loses her save (once), her mention and nothing else.
    let his_now = ctx.get(Some(&bob), &his.id).await.unwrap();
    assert_eq!((his_now.save_count, his_now.mentions.len()), (0, 0));
    assert!(his_now.caption.contains(&alice.handle), "the text stays, the link goes");
    let left: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM short_saves WHERE user_id = $1), (SELECT count(*) FROM short_likes WHERE user_id = $1),
                (SELECT count(*) FROM short_hidden WHERE user_id = $1), (SELECT count(*) FROM short_follows WHERE user_id = $1)",
    )
    .bind(alice.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(left, (0, 0, 0, 0));
    assert!(tokuly_shorts::signals::seen_ids(&ctx.app.core, alice.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_suspended_account_disappears_and_returns_whatever_order_the_events_come_in() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let tag = unique("ban");
    let short = ctx.short(&alice, &channel, &format!("停止される人の #{tag}")).await;
    let sound_id = short.sound.as_option().unwrap().id.clone();
    let visible = |viewer: Option<&TestUser>| {
        let ctx = &ctx;
        let viewer = viewer.cloned();
        let short_id = short.id.clone();
        async move { ctx.get(viewer.as_ref(), &short_id).await.is_ok() }
    };
    let search_state = || async {
        ctx.app
            .events("search.changed")
            .await
            .into_iter()
            .filter(|event| event.data["kind"] == "short" && event.data["id"] == short.id)
            .map(|event| !event.data["doc"].is_null())
            .next_back()
    };

    set_banned(&ctx, &alice, true).await;
    ctx.deliver_event("identity.user.banned", None, user_event(&alice)).await;
    assert!(!visible(Some(&bob)).await && !visible(None).await);
    assert!(visible(Some(&alice)).await, "the owner still sees their own");
    assert_eq!(hashtag_count(&ctx, &tag).await, 0);
    assert_eq!(search_state().await, Some(false));
    let request = GetSoundRequest { sound_id: sound_id.clone(), ..Default::default() };
    let sound: GetSoundResponse = ctx.call(DISCOVERY, "GetSound", Some(&bob), &request).await.unwrap();
    let sound = sound.sound.as_option().unwrap();
    assert!(sound.origin_short_id.is_empty() && sound.use_count == 0 && !sound.reusable);

    // A stale "unbanned" (from an earlier suspension) arrives late: the
    // account is still banned, and so its shorts stay hidden.
    ctx.deliver_event("identity.user.unbanned", None, user_event(&alice)).await;
    assert!(!visible(Some(&bob)).await);

    // Reinstated; the two events of that arrive the wrong way round.
    set_banned(&ctx, &alice, false).await;
    ctx.deliver_event("identity.user.unbanned", None, user_event(&alice)).await;
    ctx.deliver_event("identity.user.banned", None, user_event(&alice)).await;
    assert!(visible(Some(&bob)).await && visible(None).await);
    assert_eq!(hashtag_count(&ctx, &tag).await, 1);
    assert_eq!(search_state().await, Some(true));
    // Nothing was announced a second time.
    assert_eq!(ctx.app.events_by("activity.recorded", alice.id).await.len(), 1);
    // An event about an account that no longer exists is nothing.
    ctx.deliver_event("identity.user.banned", None, serde_json::json!({ "user_id": uuid::Uuid::now_v7() })).await;
}

#[tokio::test]
async fn a_block_removes_what_the_two_kept_of_each_other() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let a = ctx.channel(&alice, "a").await;
    let b = ctx.channel(&bob, "b").await;
    let hers = ctx.short(&alice, &a, "ありすの").await;
    let his = ctx.short(&bob, &b, "ボブの").await;
    for (user, short) in [(&bob, &hers), (&carol, &hers), (&alice, &his)] {
        save(&ctx, user, &short.id).await;
        ctx.like(user, &short.id).await;
    }
    let kept = |user: &TestUser| {
        let ctx = &ctx;
        let user_id = user.id;
        async move {
            let rows: (i64, i64) = sqlx::query_as(
                "SELECT (SELECT count(*) FROM short_saves WHERE user_id = $1),
                        (SELECT count(*) FROM short_likes WHERE user_id = $1 AND liked)",
            )
            .bind(user_id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
            rows
        }
    };

    // An event about a block that is not in place (lifted again before the
    // event arrived) takes nothing away.
    let stale = serde_json::json!({ "user_id": alice.id, "blocked_id": bob.id });
    ctx.deliver_event("social.user.blocked", Some(alice.id), stale.clone()).await;
    assert_eq!((kept(&bob).await, kept(&alice).await), ((1, 1), (1, 1)));

    ctx.block(&alice, &bob).await;
    assert_eq!(ctx.deliver("social.user.blocked", alice.id).await, 1);
    ctx.deliver_event("social.user.blocked", Some(alice.id), stale).await;
    assert_eq!((kept(&bob).await, kept(&alice).await, kept(&carol).await), ((0, 0), (0, 0), (1, 1)));
    // The counts lose exactly those saves.
    assert_eq!((ctx.row(&hers.id).await.save_count, ctx.row(&his.id).await.save_count), (1, 0));
}

#[tokio::test]
async fn takedowns_deleted_channels_and_deleted_uploads_remove_shorts() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "main").await;
    let doomed_channel = ctx.channel(&alice, "side").await;
    let follower = ctx.app.user("follower").await;
    ctx.subscribe(&follower, &doomed_channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    let taken_down = ctx.short(&alice, &channel, "規約違反").await;
    let with_channel = ctx.short(&alice, &doomed_channel, "チャンネルごと").await;
    let with_upload = ctx.short(&alice, &channel, "ファイルごと").await;
    let stays = ctx.short(&alice, &channel, "残る").await;
    let reason_of = |short_id: String| {
        let ctx = &ctx;
        async move {
            let events = ctx.app.events("shorts.short.deleted").await;
            let mine: Vec<_> = events.iter().filter(|event| event.data["id"] == short_id).collect();
            assert_eq!(mine.len(), 1, "one deletion event per short");
            mine[0].data["reason"].as_str().unwrap().to_owned()
        }
    };

    // A moderator's takedown: the upload goes even though something else holds it.
    let upload = id(&taken_down.studio.as_option().unwrap().asset_id);
    assets::attach(&ctx.app.core.db, upload, "test:another-holder").await.unwrap();
    let removed = serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32, "subject_id": taken_down.id, "reason": "spam", "moderator_id": uuid::Uuid::now_v7() });
    for _ in 0..2 {
        ctx.deliver_event("admin.content.removed", None, removed.clone()).await;
    }
    assert_eq!(ctx.get(Some(&alice), &taken_down.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert!(assets::find(&ctx.app.core, upload).await.unwrap().is_none());
    assert_eq!(reason_of(taken_down.id.clone()).await, "moderation");
    // Takedowns of other kinds of content are someone else's.
    let other = serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_VIDEO as i32, "subject_id": stays.id, "reason": "", "moderator_id": null });
    ctx.deliver_event("admin.content.removed", None, other).await;

    // The channel is deleted in the video domain.
    let request = DeleteChannelRequest { channel_id: doomed_channel.summary.id.clone(), ..Default::default() };
    let _: DeleteChannelResponse = ctx.call(CHANNELS, "DeleteChannel", Some(&alice), &request).await.unwrap();
    // Until the event arrives the short has no channel to show: it reads as gone.
    assert_eq!(ctx.get(Some(&alice), &with_channel.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert_eq!(ctx.deliver("video.channel.deleted", alice.id).await, 1);
    assert_eq!(reason_of(with_channel.id.clone()).await, "channel_deleted");
    let follows: i64 = sqlx::query_scalar("SELECT count(*) FROM short_follows WHERE channel_id = $1")
        .bind(id(&doomed_channel.summary.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(follows, 0);

    // The upload is deleted under the short (a takedown of the file).
    let upload = id(&with_upload.studio.as_option().unwrap().asset_id);
    assets::delete(&ctx.app.core, upload, None).await.unwrap();
    let gone = serde_json::json!({ "asset_id": upload, "owner_id": alice.id, "purpose": 13, "kind": 2 });
    ctx.deliver_event("media.asset.deleted", None, gone.clone()).await;
    ctx.deliver_event("media.asset.deleted", None, gone).await;
    assert_eq!(reason_of(with_upload.id.clone()).await, "asset_deleted");

    assert!(ctx.get(None, &stays.id).await.is_ok());
}

#[tokio::test]
async fn subscribers_are_told_per_their_level_and_only_once() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ありすのチャンネル").await;
    let everything = ctx.app.user("everything").await;
    let personalized = ctx.app.user("personalized").await;
    let engaged = ctx.app.user("engaged").await;
    let silent = ctx.app.user("silent").await;
    let blocked = ctx.app.user("blocked").await;
    ctx.subscribe(&everything, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.subscribe(&personalized, &channel, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;
    ctx.subscribe(&engaged, &channel, NotificationLevel::NOTIFICATION_LEVEL_PERSONALIZED).await;
    ctx.subscribe(&silent, &channel, NotificationLevel::NOTIFICATION_LEVEL_NONE).await;
    ctx.subscribe(&blocked, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    ctx.block(&alice, &blocked).await;
    ctx.befriend(&alice, &personalized).await;
    // `engaged` liked an earlier short of the channel, which nobody was told about.
    let earlier = ctx.short_with(&alice, &channel, "前の", |request| request.notify_subscribers = Some(false)).await;
    ctx.like(&engaged, &earlier.id).await;
    assert_eq!(ctx.shorts.announce(AnnounceJob { short_id: id(&earlier.id), after: None }).await.unwrap(), 0);

    let short = ctx.short(&alice, &channel, "新作だよ #見てね").await;
    // The published event is what queues the job.
    let published = ctx.app.events_by("shorts.short.published", alice.id).await;
    assert!(published.iter().any(|event| event.data["id"] == short.id && event.data["first_publication"] == true));
    let job = AnnounceJob { short_id: id(&short.id), after: None };
    assert_eq!(ctx.shorts.announce(job.clone()).await.unwrap(), 3);
    let mut told = announcements(&ctx, &short.id).await;
    told.sort();
    let mut expected = vec![(everything.id, true), (personalized.id, false), (engaged.id, true)];
    expected.sort();
    assert_eq!(
        told, expected,
        "ALL is pushed; PERSONALIZED is pushed to those who keep watching; NONE and blocked hear nothing"
    );
    let (_, request) = ctx.notifications(everything.id).await.pop().unwrap();
    assert_eq!(
        (request["title"]["ja"].as_str(), request["path"].as_str()),
        (Some("ありすのチャンネル"), Some(format!("/shorts/{}", short.short_id).as_str()))
    );
    assert!(request["actor_id"].is_null(), "the channel speaks, not the account");
    assert_eq!(request["collapse_key"], format!("shorts.published:{}", short.id));

    // The job runs again (a retry, a duplicate, twice at once): nobody is told twice.
    let (again, at_once) = tokio::join!(ctx.shorts.announce(job.clone()), ctx.shorts.announce(job.clone()));
    assert_eq!((again.unwrap(), at_once.unwrap()), (0, 0));
    assert_eq!(announcements(&ctx, &short.id).await.len(), 3);

    // A friends-only short is announced to subscribers who are friends.
    let private = ctx
        .short_with(&alice, &channel, "友達だけに", |request| {
            request.visibility = Visibility::VISIBILITY_FRIENDS.into()
        })
        .await;
    assert_eq!(ctx.shorts.announce(AnnounceJob { short_id: id(&private.id), after: None }).await.unwrap(), 1);
    assert_eq!(announcements(&ctx, &private.id).await, [(personalized.id, false)]);
    // A short nobody but the owner sees is announced to nobody.
    let hidden = ctx
        .short_with(&alice, &channel, "非公開", |request| request.visibility = Visibility::VISIBILITY_PRIVATE.into())
        .await;
    assert_eq!(ctx.shorts.announce(AnnounceJob { short_id: id(&hidden.id), after: None }).await.unwrap(), 0);
    assert_eq!(ctx.shorts.announce(AnnounceJob { short_id: uuid::Uuid::now_v7(), after: None }).await.unwrap(), 0);
}

#[tokio::test]
async fn a_channel_announces_a_few_shorts_a_day_in_small_batches() {
    let cfg = ShortsConfig { announcements_per_day: 2, announce_batch_size: 1, ..test_config() };
    let ctx = start_with(cfg).await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let mut fans = Vec::new();
    for n in 0..3 {
        let fan = ctx.app.user(&format!("fan{n}")).await;
        ctx.subscribe(&fan, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
        fans.push(fan);
    }
    let mut shorts = Vec::new();
    for n in 1..=3 {
        shorts.push(ctx.short(&alice, &channel, &format!("今日の{n}本目")).await);
    }
    // Batches of one: every subscriber exactly once all the same.
    for short in &shorts[..2] {
        assert_eq!(ctx.shorts.announce(AnnounceJob { short_id: id(&short.id), after: None }).await.unwrap(), 3);
        assert_eq!(announcements(&ctx, &short.id).await.len(), 3);
    }
    // The third short of the day is published, and not announced.
    assert_eq!(ctx.shorts.announce(AnnounceJob { short_id: id(&shorts[2].id), after: None }).await.unwrap(), 0);
    assert!(announcements(&ctx, &shorts[2].id).await.is_empty());
    let request = GetFollowingFeedRequest::default();
    let feed: GetFollowingFeedResponse = ctx.call(FEEDS, "GetFollowingFeed", Some(&fans[0]), &request).await.unwrap();
    assert_eq!(feed.items.len(), 3, "subscribers still find it in their feed");
    // Tomorrow there is room again.
    sqlx::query("UPDATE shorts SET announced_at = now() - interval '25 hours' WHERE id = ANY($1)")
        .bind(vec![id(&shorts[0].id), id(&shorts[1].id)])
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let next = ctx.short(&alice, &channel, "明日の1本目").await;
    assert_eq!(ctx.shorts.announce(AnnounceJob { short_id: id(&next.id), after: None }).await.unwrap(), 3);
}

#[tokio::test]
async fn the_copy_of_subscriptions_follows_the_video_domain_not_the_order_of_events() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let vera = ctx.app.user("vera").await;
    let channel = ctx.channel(&alice, "ch").await;
    ctx.short(&alice, &channel, "一本").await;
    let following = || async {
        let request = GetFollowingFeedRequest::default();
        let feed: GetFollowingFeedResponse = ctx.call(FEEDS, "GetFollowingFeed", Some(&vera), &request).await.unwrap();
        feed.items.len()
    };
    // Subscribed and unsubscribed again before either event was delivered.
    ctx.subscribe(&vera, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    assert_eq!(following().await, 1);
    let leave = UnsubscribeRequest { channel_id: channel.summary.id.clone(), ..Default::default() };
    let _: UnsubscribeResponse = ctx.call(CHANNELS, "Unsubscribe", Some(&vera), &leave).await.unwrap();
    // The two events arrive the wrong way round.
    assert_eq!(ctx.deliver("video.channel.unsubscribed", vera.id).await, 1);
    assert_eq!(ctx.deliver("video.channel.subscribed", vera.id).await, 1);
    assert_eq!(following().await, 0, "she is not subscribed, whatever came last");
}

#[tokio::test]
async fn a_mention_tells_a_person_once_however_the_caption_is_edited() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ch").await;
    let short = ctx.short(&alice, &channel, &format!("@{} 見てね", bob.handle)).await;
    assert_eq!(ctx.notified(bob.id).await, ["shorts.mention"]);
    let caption =
        |text: String| UpdateShortRequest { short_id: short.id.clone(), caption: Some(text), ..Default::default() };
    // Taking the mention out and putting it back, as often as one likes.
    for _ in 0..3 {
        ctx.update(&alice, &caption("メンションなし".into())).await.unwrap();
        let edited = ctx.update(&alice, &caption(format!("@{} もう一度", bob.handle))).await.unwrap();
        assert_eq!(edited.mentions.len(), 1);
    }
    assert_eq!(ctx.notified(bob.id).await, ["shorts.mention"], "bob heard of this short once");

    // Pinging someone and deleting the short does not put it out of reach:
    // whoever was notified can still point at it, to report it.
    let subjects = &ctx.app.core.subjects;
    ctx.update(&alice, &caption("もう名前は出さない".into())).await.unwrap();
    ctx.delete(&alice, &short.id).await;
    assert_eq!(ctx.get(Some(&bob), &short.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    let reportable =
        subjects.resolve(SubjectType::SUBJECT_TYPE_SHORT, &short.id, Some(bob.id)).await.expect("bob can report it");
    assert_eq!((reportable.owner_id, reportable.interactions_enabled), (alice.id, false));
    let outsider = ctx.app.user("outsider").await;
    assert!(subjects.resolve(SubjectType::SUBJECT_TYPE_SHORT, &short.id, Some(outsider.id)).await.is_err());
    assert!(subjects.resolve(SubjectType::SUBJECT_TYPE_SHORT, &short.id, None).await.is_err());
    assert_eq!(ctx.row(&short.id).await.status, tokuly_shorts::access::WITHDRAWN, "kept for the retention period");
    // A block made after the caption was written holds too.
    let carol = ctx.app.user("carol").await;
    let hidden = ctx
        .short_with(&alice, &channel, &format!("@{} へ", carol.handle), |request| {
            request.visibility = Visibility::VISIBILITY_PRIVATE.into()
        })
        .await;
    ctx.block(&carol, &alice).await;
    ctx.set_visibility(&alice, &hidden.id, Visibility::VISIBILITY_PUBLIC).await;
    assert!(ctx.notified(carol.id).await.is_empty());
}

#[tokio::test]
async fn a_reported_short_is_withdrawn_not_destroyed() {
    let ctx = start_with(ShortsConfig { reported_retention_days: 30, ..test_config() }).await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let moderator = ctx.app.user_with_roles("moderator", &["moderator"]).await;
    let channel = ctx.channel(&alice, "ch").await;
    let tag = unique("reported");
    let short = ctx.short(&alice, &channel, &format!("ひどい動画 #{tag}")).await;
    let unreported = ctx.short(&alice, &channel, "ふつうの動画").await;
    let upload = id(&short.studio.as_option().unwrap().asset_id);
    // An old short: its age must not matter for how long the evidence is kept.
    sqlx::query("UPDATE shorts SET created_at = now() - interval '2 years', published_at = now() - interval '2 years' WHERE id = $1")
        .bind(id(&short.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    // Bob reports it (the engagement domain says so); alice deletes it at once.
    let report = serde_json::json!({
        "report_id": uuid::Uuid::now_v7(), "reporter_id": bob.id,
        "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32, "subject_id": short.id, "subject_owner_id": alice.id,
        "category": "HARASSMENT", "text": "", "subject_title": "ひどい動画", "subject_path": format!("/shorts/{}", short.short_id),
    });
    ctx.deliver_event("engagement.report.created", Some(bob.id), report.clone()).await;
    ctx.deliver_event("engagement.report.created", Some(bob.id), report).await;
    ctx.delete(&alice, &short.id).await;
    ctx.delete(&alice, &unreported.id).await;

    // For everyone, alice included, it is gone, exactly like a deleted short.
    for viewer in [Some(&alice), Some(&bob), None] {
        assert_eq!(ctx.get(viewer, &short.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    }
    let again = DeleteShortRequest { short_id: short.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SHORTS, "DeleteShort", Some(&alice), &again).await, "SHORT_NOT_FOUND");
    let edit =
        UpdateShortRequest { short_id: short.id.clone(), caption: Some("書き換え".into()), ..Default::default() };
    assert_eq!(ctx.update(&alice, &edit).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    let mine: ListMyShortsResponse =
        ctx.call(SHORTS, "ListMyShorts", Some(&alice), &ListMyShortsRequest::default()).await.unwrap();
    assert!(mine.shorts.is_empty());
    assert_eq!(hashtag_count(&ctx, &tag).await, 0);
    let deleted: Vec<_> = ctx.app.events_by("shorts.short.deleted", alice.id).await;
    assert_eq!(deleted.len(), 2, "other domains hear of a deletion either way");
    // A stay reported on it counts for nothing.
    {
        let _lock = playback_lock().await;
        assert_eq!(ctx.watch(&bob, &short.id).await.accepted, 0);
    }

    // The moderator can still look at what was reported; the unreported
    // short really is gone, with its upload.
    let reviewed = ctx.get(Some(&moderator), &short.id).await.expect("moderators review withdrawn shorts");
    assert_eq!(reviewed.status, ShortStatus::SHORT_STATUS_WITHDRAWN);
    // What was written on a withdrawn short stays with it until it is purged.
    assert!(!ctx.app.subject_deleted(tokuly_proto::common::SubjectType::SUBJECT_TYPE_SHORT, &short.id).await);
    assert!(reviewed.playback.as_option().is_some() && reviewed.caption.starts_with("ひどい動画"));
    assert!(assets::find(&ctx.app.core, upload).await.unwrap().is_some());
    assert_eq!(ctx.get(Some(&moderator), &unreported.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    // A moderator's role opens what there is something to review about,
    // not any short whose ID they know.
    let private = ctx
        .short_with(&alice, &channel, "非公開の動画", |request| {
            request.visibility = Visibility::VISIBILITY_PRIVATE.into()
        })
        .await;
    assert_eq!(ctx.get(Some(&moderator), &private.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    let reported =
        serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32, "subject_id": private.id });
    ctx.deliver_event("engagement.report.created", Some(bob.id), reported).await;
    assert_eq!(ctx.get(Some(&moderator), &private.id).await.unwrap().caption, "非公開の動画");
    // Staff powers are not for one's own content.
    let own_channel = ctx.channel(&moderator, "mod").await;
    let own = ctx.short(&moderator, &own_channel, "モデレーター自身の").await;
    let report = serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32, "subject_id": own.id });
    ctx.deliver_event("engagement.report.created", Some(bob.id), report).await;
    ctx.delete(&moderator, &own.id).await;
    assert_eq!(ctx.get(Some(&moderator), &own.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");

    // The retention clock starts at the deletion, not at the short's creation.
    assert_eq!(ctx.shorts.housekeeping().await.unwrap(), 0);
    assert!(ctx.get(Some(&moderator), &short.id).await.is_ok(), "a two-year-old short, withdrawn a moment ago");
    sqlx::query("UPDATE shorts SET withdrawn_at = now() - interval '31 days' WHERE id = $1")
        .bind(id(&short.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.shorts.housekeeping().await.unwrap(), 1);
    assert_eq!(ctx.get(Some(&moderator), &short.id).await.unwrap_err().reason, "SHORT_NOT_FOUND");
    assert!(assets::find(&ctx.app.core, upload).await.unwrap().is_none());
    assert_eq!(
        ctx.app.events_by("shorts.short.deleted", alice.id).await.len(),
        2,
        "the purge is not a second deletion"
    );
    assert!(ctx.app.subject_deleted(tokuly_proto::common::SubjectType::SUBJECT_TYPE_SHORT, &short.id).await);

    // A takedown ends the retention at once.
    let own_upload = id(&ctx.row(&own.id).await.asset_id.to_string());
    let removed = serde_json::json!({ "subject_type": SubjectType::SUBJECT_TYPE_SHORT as i32, "subject_id": own.id });
    ctx.deliver_event("admin.content.removed", None, removed).await;
    assert!(assets::find(&ctx.app.core, own_upload).await.unwrap().is_none());
}

#[tokio::test]
async fn the_search_index_can_be_rebuilt() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let channel = ctx.channel(&alice, "ch").await;
    let tag = unique("reindex");
    let public = ctx.short(&alice, &channel, &format!("公開 #{tag}")).await;
    let private = ctx
        .short_with(&alice, &channel, "非公開", |request| request.visibility = Visibility::VISIBILITY_PRIVATE.into())
        .await;
    let documents = |kind: &'static str, key: String| {
        let ctx = &ctx;
        async move {
            ctx.app
                .events("search.changed")
                .await
                .into_iter()
                .filter(|event| {
                    event.data["kind"] == kind && (event.data["id"] == key || event.data["doc"]["title"] == key)
                })
                .count()
        }
    };
    let before = documents("short", public.id.clone()).await;
    // An administrator asked for a rebuild: the event queues the job.
    let requested = serde_json::json!({ "kind": "short", "requested_by": alice.id });
    ctx.deliver_event("search.reindex.requested", Some(alice.id), requested).await;
    ctx.deliver_event("search.reindex.requested", Some(alice.id), serde_json::json!({ "kind": "note" })).await;
    ctx.shorts.reindex(ReindexJob { kind: "short".into(), after: None }).await.unwrap();
    assert_eq!(documents("short", public.id.clone()).await, before + 1);
    assert_eq!(documents("short", private.id.clone()).await, 0, "only public shorts are documents");
    let doc = ctx
        .app
        .events("search.changed")
        .await
        .into_iter()
        .filter(|event| event.data["kind"] == "short" && event.data["id"] == public.id)
        .next_back()
        .unwrap();
    assert_eq!(doc.data["doc"]["path"], format!("/shorts/{}", public.short_id));
    assert_eq!(doc.data["doc"]["tags"][0], tag);
    assert_eq!(doc.data["doc"]["owner_id"], alice.id.to_string());
    assert_eq!(doc.data["doc"]["extra"]["channel_handle"], channel.summary.handle);

    // Hashtags are marked and rewritten by the next ranking run.
    ctx.shorts.run_ranking().await.unwrap();
    let title = format!("#{tag}");
    let before = documents("hashtag", title.clone()).await;
    ctx.shorts.reindex(ReindexJob { kind: "hashtag".into(), after: None }).await.unwrap();
    ctx.shorts.run_ranking().await.unwrap();
    assert_eq!(documents("hashtag", title).await, before + 1);
    ctx.shorts.reindex(ReindexJob { kind: "unknown".into(), after: None }).await.unwrap();
}

#[tokio::test]
async fn the_export_names_what_is_the_users_and_what_they_may_still_see() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let a = ctx.channel(&alice, "a").await;
    let b = ctx.channel(&bob, "b").await;
    let hers = ctx.short(&alice, &a, "ありすの #export").await;
    let also_hers = ctx.short(&alice, &a, "ありすの二本目").await;
    let his = ctx.short(&bob, &b, "ボブの").await;
    for short in [&hers, &also_hers] {
        save(&ctx, &bob, &short.id).await;
        ctx.like(&bob, &short.id).await;
    }
    ctx.mark_not_interested(&bob, &hers.id).await;
    ctx.subscribe(&bob, &a, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    let exporter = ctx
        .app
        .core
        .exporters
        .all()
        .into_iter()
        .find(|exporter| exporter.name() == "shorts")
        .expect("the shorts exporter");
    let ids = |section: &serde_json::Value| -> Vec<String> {
        section.as_array().unwrap().iter().map(|entry| entry["short_id"].as_str().unwrap().to_owned()).collect()
    };

    let export = exporter.export(bob.id).await.unwrap();
    assert_eq!(export["shorts"].as_array().unwrap().len(), 1);
    assert_eq!(
        (export["shorts"][0]["id"].as_str(), export["shorts"][0]["caption"].as_str()),
        (Some(his.id.as_str()), Some("ボブの"))
    );
    assert_eq!(ids(&export["saved"]), [hers.id.clone(), also_hers.id.clone()]);
    assert_eq!(ids(&export["liked"]), [hers.id.clone(), also_hers.id.clone()]);
    assert_eq!(ids(&export["not_interested"]), [hers.id.clone()]);
    assert_eq!(export["followed_channels"][0]["channel_id"], a.summary.id);

    // A short bob can no longer open is not named: the export does not say
    // whether it was deleted or hidden from him.
    ctx.set_visibility(&alice, &hers.id, Visibility::VISIBILITY_PRIVATE).await;
    ctx.delete(&alice, &also_hers.id).await;
    let export = exporter.export(bob.id).await.unwrap();
    assert!(ids(&export["saved"]).is_empty() && ids(&export["liked"]).is_empty());
    // Someone with nothing gets empty sections, not an error.
    let nobody = ctx.app.user("nobody").await;
    let export = exporter.export(nobody.id).await.unwrap();
    assert!(export["shorts"].as_array().unwrap().is_empty() && export["saved"].as_array().unwrap().is_empty());
}
