//! Gifts, clips, OBS overlays and analytics.
mod common;

use common::*;
use futures::future::join_all;
use tokuly_core::prelude::*;
use tokuly_live::{clips, gifts, tiers};
use tokuly_proto::{
    common::{SubjectType, Visibility},
    live::{list_clips_request::Filter, overlay_settings::Widget, *},
    media::AssetPurpose,
    video::NotificationLevel,
};
use tokuly_testkit::TestUser;
use tokuly_video::videos::{self, NewVideo};

// ───────────────────────────── gifts ─────────────────────────────

#[tokio::test]
async fn the_gift_ladder_is_served_by_the_api() {
    let ctx = start().await;
    let response: GetGiftTiersResponse =
        ctx.call(GIFTS, "GetGiftTiers", None, &GetGiftTiersRequest::default()).await.unwrap();
    assert_eq!(response.tiers.len(), 12);
    assert_eq!((response.min_coins, response.max_coins), (tiers::MIN_GIFT_COINS, tiers::MAX_GIFT_COINS));
    assert_eq!(response.platform_fee_percent, ctx.wallet.platform_fee_percent() as i32);
    assert_eq!(response.tiers[0].min_coins, response.min_coins);
    assert!(response.tiers.windows(2).all(|pair| pair[1].min_coins > pair[0].min_coins && pair[1].highlight_seconds >= pair[0].highlight_seconds));
}

#[tokio::test]
async fn a_gift_moves_coins_once_and_shows_up_highlighted() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ギフトch").await;
    let stream = ctx.on_air(&alice, &channel, "ギフトの配信").await;
    ctx.coins(&bob, 1_000).await;
    let fee = i64::from(ctx.wallet.platform_fee_percent());
    let mut topic = ctx.app.listen_topic(&stream.realtime_topic).await;

    let sent = ctx.gift(&bob, &stream, 500, " がんばれ! ", "gift-key-0001").await.unwrap();
    let gift = sent.gift.as_option().unwrap();
    assert_eq!((gift.coins, gift.tier, gift.message.as_str()), (500, 5, "がんばれ!"));
    assert_eq!(gift.sender.as_option().unwrap().id, bob.id.to_string());
    assert_eq!(gift.net_coins, 0, "what the streamer nets is the streamer's business");
    assert_eq!(sent.balance, 500);
    let line = sent.message.as_option().unwrap();
    assert_eq!((line.kind, line.text.as_str()), (ChatMessageKind::CHAT_MESSAGE_KIND_GIFT.into(), "がんばれ!"));
    let carried = line.gift.as_option().unwrap();
    assert_eq!((carried.coins, carried.tier, carried.gift_id.as_str()), (500, 5, gift.id.as_str()));
    assert!(carried.highlighted_until.is_set());
    let pushed = wait_for(&mut topic, |push| match push {
        Push::ChatMessage(message) => Some(message),
        _ => None,
    })
    .await;
    assert_eq!(pushed.id, line.id);
    // The coins moved: payer down by the gift, streamer's earnings up by it minus the fee.
    let net = 500 - 500 * fee / 100;
    assert_eq!(ctx.balance(&bob).await.0, 500);
    assert_eq!(ctx.balance(&alice).await.1, net);
    let row = ctx.row(&stream).await;
    assert_eq!((row.gift_coins, row.gift_net_coins, row.gift_count), (500, net, 1));
    // The sender's wallet shows what was paid, not the account behind the channel:
    // a gift does not tell who runs a channel.
    let history = |user: &TestUser| {
        let user = user.clone();
        let ctx = &ctx;
        async move {
            let listed: tokuly_proto::wallet::ListLedgerEntriesResponse = ctx
                .call(
                    "tokuly.wallet.v1.WalletService",
                    "ListLedgerEntries",
                    Some(&user),
                    &tokuly_proto::wallet::ListLedgerEntriesRequest::default(),
                )
                .await
                .unwrap();
            listed
        }
    };
    let paid = history(&bob).await;
    let entry =
        paid.entries.iter().find(|entry| entry.reason == "live.gift").expect("the gift in the sender's history");
    assert!(entry.counterpart_hidden && !entry.counterpart.is_set());
    assert!(entry.memo_json.contains(&channel.summary.id), "the entry names the channel");
    let everything = serde_json::to_string(&paid).unwrap();
    assert!(!everything.contains(&alice.id.to_string()) && !everything.contains(&alice.handle));
    // The streamer sees who gave (the chat showed it to everyone anyway).
    let earned = history(&alice).await;
    let entry =
        earned.entries.iter().find(|entry| entry.reason == "live.gift").expect("the gift in the streamer's history");
    assert!(!entry.counterpart_hidden);
    assert_eq!(entry.counterpart.as_option().unwrap().id, bob.id.to_string());
    // The page shows the gift while it is highlighted.
    assert_eq!(ctx.get_stream(None, &card(&stream).id).await.unwrap().highlighted_gifts.len(), 1);
    // The streamer was not looking at the dashboard: they are told.
    let notices = ctx.notifications(&alice, "live.gift").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].data["data"]["coins"], 500);
    assert_eq!(ctx.app.events_by("live.gift.sent", bob.id).await.len(), 1);

    // The same key again: the same gift, nothing moves.
    let again = ctx.gift(&bob, &stream, 500, "がんばれ!", "gift-key-0001").await.unwrap();
    assert_eq!(again.gift.as_option().unwrap().id, gift.id);
    assert_eq!(again.message.as_option().unwrap().id, line.id);
    assert_eq!(ctx.balance(&bob).await.0, 500);
    assert_eq!(ctx.row(&stream).await.gift_count, 1);

    // With the dashboard open no notification is needed.
    let _: GetRealtimeStatsResponse = ctx
        .call(
            ANALYTICS,
            "GetRealtimeStats",
            Some(&alice),
            &GetRealtimeStatsRequest { stream_id: card(&stream).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    ctx.gift(&bob, &stream, 10, "", "gift-key-0002").await.unwrap();
    assert_eq!(ctx.notifications(&alice, "live.gift").await.len(), 1);

    // What is refused, and that a refusal moves nothing.
    for (coins, message, key, reason) in [
        (5, "", "gift-key-0003", "GIFT_AMOUNT_INVALID"),
        (tiers::MAX_GIFT_COINS + 1, "", "gift-key-0003", "GIFT_AMOUNT_INVALID"),
        (10, "ひとこと", "gift-key-0003", "GIFT_MESSAGE_TOO_LONG"),
        (50, "ok", "short", "IDEMPOTENCY_KEY_INVALID"),
        (5_000, "", "gift-key-0003", "INSUFFICIENT_COINS"),
    ] {
        assert_eq!(ctx.gift(&bob, &stream, coins, message, key).await.unwrap_err().reason, reason);
    }
    assert_eq!(ctx.balance(&bob).await.0, 490);
    // A chat that takes no links takes none in gift messages either.
    let no_links = UpdateChatSettingsRequest {
        stream_id: card(&stream).id.clone(),
        chat: Some(ChatSettings {
            enabled: true,
            stamps_enabled: true,
            link_policy: LinkPolicy::LINK_POLICY_BLOCK.into(),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    };
    let _: UpdateChatSettingsResponse = ctx.call(CHAT, "UpdateChatSettings", Some(&alice), &no_links).await.unwrap();
    assert_eq!(
        ctx.gift(&bob, &stream, 100, "見てね https://example.com/x", "gift-key-link").await.unwrap_err().reason,
        "LINKS_NOT_ALLOWED"
    );
    assert_eq!(ctx.balance(&bob).await.0, 490);
    assert_eq!(ctx.gift(&alice, &stream, 50, "", "gift-key-self").await.unwrap_err().reason, "GIFT_OWN_STREAM");
    let request = SendGiftRequest {
        stream_id: card(&stream).id.clone(),
        coins: 50,
        idempotency_key: "gift-key-anon".into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(GIFTS, "SendGift", None, &request).await, "SIGN_IN_REQUIRED");
    // A refused payment leaves no half-made gift behind.
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM live_gifts WHERE sender_id = $1 AND status = 1")
        .bind(bob.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(pending, 0);

    // Banned from the chat means no gift messages either; gifts can be switched off; they need a stream on air.
    ctx.restrict(&alice, &channel, &bob, 0, false).await.unwrap();
    assert_eq!(ctx.gift(&bob, &stream, 50, "", "gift-key-0004").await.unwrap_err().reason, "CHAT_BANNED");
    let carol = ctx.app.user("carol").await;
    ctx.coins(&carol, 100).await;
    let off =
        UpdateStreamRequest { stream_id: card(&stream).id.clone(), gifts_enabled: Some(false), ..Default::default() };
    let _: UpdateStreamResponse = ctx.call(STREAMS, "UpdateStream", Some(&alice), &off).await.unwrap();
    assert_eq!(ctx.gift(&carol, &stream, 50, "", "gift-key-0005").await.unwrap_err().reason, "GIFTS_DISABLED");
    ctx.end(&alice, &stream).await;
    assert_eq!(ctx.gift(&carol, &stream, 50, "", "gift-key-0006").await.unwrap_err().reason, "STREAM_NOT_LIVE");
    assert_eq!(ctx.balance(&carol).await.0, 100);

    // History: for the sender, and for the streamer with what they netted.
    let sent: ListSentGiftsResponse =
        ctx.call(GIFTS, "ListSentGifts", Some(&bob), &ListSentGiftsRequest::default()).await.unwrap();
    assert_eq!(sent.gifts.iter().map(|g| g.coins).collect::<Vec<_>>(), vec![10, 500]);
    assert_eq!(sent.gifts[1].stream_title, "ギフトの配信");
    let received = ListReceivedGiftsRequest { channel_id: channel.summary.id.clone(), ..Default::default() };
    let got: ListReceivedGiftsResponse = ctx.call(GIFTS, "ListReceivedGifts", Some(&alice), &received).await.unwrap();
    assert_eq!((got.gifts.len(), got.total_coins), (2, 510));
    assert_eq!(got.total_net_coins, got.gifts.iter().map(|g| g.net_coins).sum::<i64>());
    assert_eq!(ctx.reason(GIFTS, "ListReceivedGifts", Some(&bob), &received).await, "CHANNEL_NOT_FOUND");
    let by_stream = ListReceivedGiftsRequest { stream_id: card(&stream).id.clone(), ..received.clone() };
    let got: ListReceivedGiftsResponse = ctx.call(GIFTS, "ListReceivedGifts", Some(&alice), &by_stream).await.unwrap();
    assert_eq!(got.gifts.len(), 2);
    let top: ListTopSupportersResponse = ctx
        .call(
            GIFTS,
            "ListTopSupporters",
            None,
            &ListTopSupportersRequest { stream_id: card(&stream).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(top.supporters.len(), 1);
    assert_eq!((top.supporters[0].coins, top.supporters[0].gift_count), (510, 2));
}

/// The same gift submitted many times at once, and a gift whose payment
/// was cut off: coins move exactly once and nothing is lost (checklist 5, 6).
#[tokio::test]
async fn gifts_settle_exactly_once_even_when_requests_race_or_die() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "一度だけch").await;
    let stream = ctx.on_air(&alice, &channel, "一度だけの配信").await;
    ctx.coins(&bob, 1_000).await;
    let fee = i64::from(ctx.wallet.platform_fee_percent());

    // Six copies of one request at the same moment.
    let results = join_all((0..6).map(|_| ctx.gift(&bob, &stream, 200, "同時", "gift-race-0001"))).await;
    let ids: Vec<String> =
        results.into_iter().map(|result| result.unwrap().gift.as_option().unwrap().id.clone()).collect();
    assert!(ids.iter().all(|id| *id == ids[0]), "every copy got the one gift");
    assert_eq!(ctx.balance(&bob).await.0, 800);
    assert_eq!(ctx.balance(&alice).await.1, 200 - 200 * fee / 100);
    let row = ctx.row(&stream).await;
    assert_eq!((row.gift_count, row.gift_coins), (1, 200));
    let lines = ctx.chat(None, &stream).await;
    assert_eq!(lines.iter().filter(|line| line.kind == ChatMessageKind::CHAT_MESSAGE_KIND_GIFT).count(), 1);

    // A request that died after writing the gift down, before (or after) the wallet moved the coins.
    let db = &ctx.app.core.db;
    let stuck: Uuid = sqlx::query_scalar(
        "INSERT INTO live_gifts (stream_id, channel_id, recipient_id, sender_id, coins, tier, message, highlight_seconds, client_key, created_at)
         VALUES ($1, $2, $3, $4, 300, 4, '途中で切れた', 300, 'gift-stuck-0001', now() - interval '1 minute') RETURNING id",
    )
    .bind(sid(&stream))
    .bind(id(&channel.summary.id))
    .bind(alice.id)
    .bind(bob.id)
    .fetch_one(db)
    .await
    .unwrap();
    // Unfinished gifts are invisible until they are paid for...
    let sent_before: i64 = sqlx::query_scalar("SELECT count(*) FROM live_gifts WHERE sender_id = $1 AND status = 2")
        .bind(bob.id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(sent_before, 1);
    // ...and the sweep (or the sender coming back) finishes them, once.
    assert_eq!(ctx.live.settle_pending_gifts(None).await, 1);
    assert_eq!(ctx.live.settle_pending_gifts(None).await, 0);
    let settled = gifts::find_by_id(&ctx.app.core, stuck).await.unwrap().unwrap();
    assert!(settled.is_settled());
    assert_eq!(
        (settled.tier, settled.highlight_seconds),
        (4, 300),
        "what the gift contains was decided when it was made"
    );
    assert_eq!(ctx.balance(&bob).await.0, 500);
    assert_eq!(ctx.row(&stream).await.gift_count, 2);
    // Settling again by hand changes nothing.
    ctx.live.settle_gift(stuck).await.unwrap();
    assert_eq!(ctx.balance(&bob).await.0, 500);
    assert_eq!(ctx.row(&stream).await.gift_count, 2);
    // Asking for the history is an entry point too: the client retrying with its key gets the settled gift.
    let retried = ctx.gift(&bob, &stream, 300, "途中で切れた", "gift-stuck-0001").await.unwrap();
    assert_eq!(retried.gift.as_option().unwrap().id, stuck.to_string());
    assert_eq!(ctx.balance(&bob).await.0, 500);

    // An unfinished gift its sender can no longer pay for is dropped, not kept forever.
    let broke = ctx.app.user("broke").await;
    sqlx::query(
        "INSERT INTO live_gifts (stream_id, channel_id, recipient_id, sender_id, coins, tier, message, highlight_seconds, client_key, created_at)
         VALUES ($1, $2, $3, $4, 300, 4, '', 300, 'gift-broke-0001', now() - interval '1 minute')",
    )
    .bind(sid(&stream))
    .bind(id(&channel.summary.id))
    .bind(alice.id)
    .bind(broke.id)
    .execute(db)
    .await
    .unwrap();
    let listed: ListSentGiftsResponse =
        ctx.call(GIFTS, "ListSentGifts", Some(&broke), &ListSentGiftsRequest::default()).await.unwrap();
    assert!(listed.gifts.is_empty());
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM live_gifts WHERE sender_id = $1")
        .bind(broke.id)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(left, 0);

    // A gift that was on its way when the stream was deleted: the coins come back, nothing is
    // kept for a stream that is gone, and the streamer earns nothing from it.
    let short_lived = ctx.on_air(&alice, &channel, "すぐ消える配信").await;
    let lost: Uuid = sqlx::query_scalar(
        "INSERT INTO live_gifts (stream_id, channel_id, recipient_id, sender_id, coins, tier, message, highlight_seconds, client_key, created_at)
         VALUES ($1, $2, $3, $4, 100, 3, 'まにあわなかった', 120, 'gift-lost-00001', now() - interval '1 minute') RETURNING id",
    )
    .bind(sid(&short_lived))
    .bind(id(&channel.summary.id))
    .bind(alice.id)
    .bind(bob.id)
    .fetch_one(db)
    .await
    .unwrap();
    ctx.end(&alice, &short_lived).await;
    let _: DeleteStreamResponse = ctx
        .call(
            STREAMS,
            "DeleteStream",
            Some(&alice),
            &DeleteStreamRequest { stream_id: card(&short_lived).id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let (coins_before, earnings_before) = (ctx.balance(&bob).await.0, ctx.balance(&alice).await.1);
    assert_eq!(ctx.live.settle_gift(lost).await.unwrap_err().reason, "GIFT_RETURNED");
    assert!(gifts::find_by_id(&ctx.app.core, lost).await.unwrap().is_none());
    assert_eq!((ctx.balance(&bob).await.0, ctx.balance(&alice).await.1), (coins_before, earnings_before));
    // The sweep finds nothing left to do, and the payment and its return are both in the sender's history.
    assert_eq!(ctx.live.settle_pending_gifts(None).await, 0);
    let returned: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM wallet_entries WHERE reason = 'live.gift.returned' AND to_user_id = $1",
    )
    .bind(bob.id)
    .fetch_one(db)
    .await
    .unwrap();
    assert_eq!(returned, 1);
}

/// Several settlers and the deletion of the stream at the same moment: the
/// gift ends up either settled (and paid) or returned (and refunded), never
/// both and never neither.
#[tokio::test]
async fn a_gift_is_settled_or_returned_never_both() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "どっちかch").await;
    ctx.coins(&bob, 10_000).await;
    let fee = i64::from(ctx.wallet.platform_fee_percent());
    let db = &ctx.app.core.db;
    let (mut settled_rounds, mut returned_rounds) = (0, 0);
    for round in 0..10 {
        let stream = ctx.on_air(&alice, &channel, "消えるかも").await;
        let gift_id: Uuid = sqlx::query_scalar(
            // (Young enough that the sweep another test runs leaves it to this one.)
            "INSERT INTO live_gifts (stream_id, channel_id, recipient_id, sender_id, coins, tier, message, highlight_seconds, client_key)
             VALUES ($1, $2, $3, $4, 100, 3, 'きわどい', 120, $5) RETURNING id",
        )
        .bind(sid(&stream))
        .bind(id(&channel.summary.id))
        .bind(alice.id)
        .bind(bob.id)
        .bind(format!("gift-either-{round:04}"))
        .fetch_one(db)
        .await
        .unwrap();
        let (coins_before, earnings_before) = (ctx.balance(&bob).await.0, ctx.balance(&alice).await.1);
        let delete = DeleteStreamRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
        let deletion = async {
            // Sometimes before the settlers, sometimes in the middle of them.
            tokio::time::sleep(std::time::Duration::from_millis(round * 3)).await;
            let _: DeleteStreamResponse = ctx.call(STREAMS, "DeleteStream", Some(&alice), &delete).await.unwrap();
        };
        let (outcomes, ()) = tokio::join!(join_all((0..4).map(|_| ctx.live.settle_gift(gift_id))), deletion);
        // Whatever was left undecided is finished the way the sweep would.
        let _ = ctx.live.settle_gift(gift_id).await;

        let row = gifts::find_by_id(&ctx.app.core, gift_id).await.unwrap();
        let (refunded, refunds): (bool, i64) = sqlx::query_as(
            "SELECT e.refunded_at IS NOT NULL, (SELECT count(*) FROM wallet_entries r WHERE r.refund_of = e.id)
             FROM wallet_entries e WHERE e.reason = 'live.gift' AND e.memo ->> 'gift_id' = $1",
        )
        .bind(gift_id.to_string())
        .fetch_one(db)
        .await
        .unwrap();
        let (coins, earnings) = (ctx.balance(&bob).await.0, ctx.balance(&alice).await.1);
        match row {
            Some(gift) => {
                assert!(gift.is_settled(), "round {round}: the gift is left in state {}", gift.status);
                assert!(!refunded && refunds == 0, "round {round}: a settled gift was refunded");
                assert_eq!((coins, earnings), (coins_before - 100, earnings_before + 100 - 100 * fee / 100));
                assert!(outcomes.iter().all(|outcome| outcome.as_ref().is_ok_and(|gift| gift.is_settled())));
                settled_rounds += 1;
            }
            None => {
                assert!(refunded && refunds == 1, "round {round}: a dropped gift was not refunded exactly once");
                assert_eq!((coins, earnings), (coins_before, earnings_before));
                // (A settler that came after the gift was dropped finds nothing.)
                assert!(outcomes.iter().all(|outcome| {
                    outcome.as_ref().is_err_and(|e| e.reason == "GIFT_RETURNED" || e.reason == "GIFT_NOT_FOUND")
                }));
                returned_rounds += 1;
            }
        }
    }
    eprintln!("settled in {settled_rounds} rounds, returned in {returned_rounds}");
}

// ───────────────────────────── clips ─────────────────────────────

/// A published public video on `channel` (a fixture upload, 12 s long).
async fn video(
    ctx: &Ctx,
    owner: &TestUser,
    channel: &tokuly_proto::video::Channel,
    visibility: Visibility,
) -> videos::VideoRow {
    let asset =
        tokuly_media::testing::ready_video(&ctx.app.core, owner.id, AssetPurpose::ASSET_PURPOSE_VIDEO).await.unwrap();
    let video_id = videos::create_from_asset(
        &ctx.app.core,
        NewVideo { visibility, publish: true, ..NewVideo::new(id(&channel.summary.id), asset.id, "元動画") },
    )
    .await
    .unwrap();
    videos::find_by_id(&ctx.app.core, video_id).await.unwrap().unwrap()
}

async fn make_ready(ctx: &Ctx, clip_id: &str) {
    sqlx::query("UPDATE live_clips SET status = 2 WHERE id = $1")
        .bind(id(clip_id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn clips_follow_their_source_and_belong_to_their_creator() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let carol = ctx.app.user("carol").await;
    let channel = ctx.channel(&alice, "クリップch").await;
    let public = video(&ctx, &alice, &channel, Visibility::VISIBILITY_PUBLIC).await;
    let hidden = video(&ctx, &alice, &channel, Visibility::VISIBILITY_PRIVATE).await;
    let cut = |video_id: String, title: &str, start: f64, duration: f64| CreateClipRequest {
        video_id,
        title: title.to_owned(),
        start_seconds: start,
        duration_seconds: duration,
        ..Default::default()
    };
    for (request, reason) in [
        (cut(public.short_id.clone(), "", 0.0, 5.0), "CLIP_TITLE_INVALID"),
        (cut(public.short_id.clone(), &"あ".repeat(65), 0.0, 5.0), "CLIP_TITLE_INVALID"),
        (cut(public.short_id.clone(), "短すぎ", 0.0, 0.05), "CLIP_RANGE_INVALID"),
        (cut(public.short_id.clone(), "長すぎ", 0.0, 61.0), "CLIP_RANGE_INVALID"),
        (cut(public.short_id.clone(), "はみ出し", 10.0, 5.0), "CLIP_RANGE_INVALID"),
        (cut(hidden.short_id.clone(), "見えない", 0.0, 5.0), "VIDEO_NOT_FOUND"),
        (cut(String::new(), "元なし", 0.0, 5.0), "CLIP_SOURCE_REQUIRED"),
    ] {
        assert_eq!(ctx.reason(CLIPS, "CreateClip", Some(&bob), &request).await, reason);
    }
    assert_eq!(
        ctx.reason(CLIPS, "CreateClip", None, &cut(public.short_id.clone(), "ゲスト", 0.0, 5.0)).await,
        "SIGN_IN_REQUIRED"
    );
    // What is shown to a chosen few cannot be cut out and passed on: not by a friend who may
    // watch it, and not by the owner either (a clip is made to be shared).
    ctx.befriend(&alice, &carol).await;
    let for_friends = video(&ctx, &alice, &channel, Visibility::VISIBILITY_FRIENDS).await;
    for user in [&carol, &alice] {
        assert_eq!(
            ctx.reason(CLIPS, "CreateClip", Some(user), &cut(for_friends.short_id.clone(), "友達だけの", 0.0, 5.0))
                .await,
            "CLIP_SOURCE_NOT_PUBLIC"
        );
    }
    let friends_stream = ctx
        .on_air_with(&alice, &channel, "友達だけの配信", |r| {
            r.visibility = Visibility::VISIBILITY_FRIENDS.into()
        })
        .await;
    let from_stream = CreateClipRequest {
        stream_id: card(&friends_stream).id.clone(),
        title: "友達だけの".into(),
        start_seconds: 0.0,
        duration_seconds: 5.0,
        ..Default::default()
    };
    assert_eq!(ctx.reason(CLIPS, "CreateClip", Some(&carol), &from_stream).await, "CLIP_SOURCE_NOT_PUBLIC");
    ctx.end(&alice, &friends_stream).await;

    let created: CreateClipResponse =
        ctx.call(CLIPS, "CreateClip", Some(&bob), &cut(public.id.to_string(), "  神回 ", 2.5, 6.0)).await.unwrap();
    let clip = created.clip.as_option().unwrap().clone();
    assert_eq!((clip.title.as_str(), clip.status), ("神回", ClipStatus::CLIP_STATUS_PROCESSING.into()));
    assert_eq!(
        (clip.start_seconds, clip.duration_seconds, clip.video_id.as_str()),
        (2.5, 6.0, public.short_id.as_str())
    );
    assert_eq!(clip.creator.as_option().unwrap().id, bob.id.to_string());
    assert_eq!(clip.channel.as_option().unwrap().id, channel.summary.id);
    assert!(clip.can_delete && !clip.playback.is_set());
    // While it is being cut only its creator sees it.
    let get = GetClipRequest { clip_id: clip.short_id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(CLIPS, "GetClip", Some(&carol), &get).await, "CLIP_NOT_FOUND");
    let _: GetClipResponse = ctx.call(CLIPS, "GetClip", Some(&bob), &get).await.unwrap();
    make_ready(&ctx, &clip.id).await;
    let seen: GetClipResponse = ctx.call(CLIPS, "GetClip", None, &get).await.unwrap();
    let seen = seen.clip.as_option().unwrap();
    assert_eq!((seen.visibility, seen.can_delete), (Visibility::VISIBILITY_PUBLIC.into(), false));
    assert_eq!(seen.source_title, "元動画");
    // Its media, which posting it as a short takes, is told to its creator only.
    let media = Uuid::now_v7();
    sqlx::query("UPDATE live_clips SET asset_id = COALESCE(asset_id, $2) WHERE id = $1")
        .bind(id(&clip.id))
        .bind(media)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let media: Uuid = sqlx::query_scalar("SELECT asset_id FROM live_clips WHERE id = $1")
        .bind(id(&clip.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    for (viewer, expected) in [(Some(&bob), media.to_string()), (Some(&carol), String::new()), (None, String::new())] {
        let got: GetClipResponse = ctx.call(CLIPS, "GetClip", viewer, &get).await.unwrap();
        assert_eq!(got.clip.as_option().unwrap().asset_id, expected);
    }

    // Listed by video, channel and creator.
    let list = |filter: Filter| ListClipsRequest { filter: Some(filter), ..Default::default() };
    for filter in [
        Filter::VideoId(public.short_id.clone()),
        Filter::ChannelId(channel.summary.id.clone()),
        Filter::CreatorId(bob.id.to_string()),
    ] {
        let listed: ListClipsResponse = ctx.call(CLIPS, "ListClips", Some(&carol), &list(filter)).await.unwrap();
        assert_eq!(listed.clips.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec![clip.id.as_str()]);
    }
    assert_eq!(ctx.reason(CLIPS, "ListClips", None, &ListClipsRequest::default()).await, "CLIP_FILTER_REQUIRED");
    assert_eq!(
        ctx.reason(CLIPS, "ListClips", None, &list(Filter::VideoId(hidden.short_id.clone()))).await,
        "VIDEO_NOT_FOUND"
    );
    assert_eq!(
        ctx.reason(CLIPS, "ListClips", None, &list(Filter::ChannelId("nobody-here".into()))).await,
        "CHANNEL_NOT_FOUND"
    );

    // Renaming: the creator only.
    let rename = UpdateClipRequest { clip_id: clip.id.clone(), title: "改題".into(), ..Default::default() };
    assert_eq!(ctx.reason(CLIPS, "UpdateClip", Some(&alice), &rename).await, "CLIP_NOT_FOUND");
    let renamed: UpdateClipResponse = ctx.call(CLIPS, "UpdateClip", Some(&bob), &rename).await.unwrap();
    assert_eq!(renamed.clip.as_option().unwrap().title, "改題");
    let doc = ctx.search_docs(&clip.id).await.pop().unwrap().expect("a ready clip of a public video is searchable");
    assert_eq!((doc["kind"].as_str(), doc["title"].as_str()), (Some("clip"), Some("改題")));

    // The clip is a subject: liked, reported.
    let subject =
        ctx.app.core.subjects.resolve(SubjectType::SUBJECT_TYPE_CLIP, &clip.short_id, Some(carol.id)).await.unwrap();
    assert_eq!((subject.owner_id, subject.interactions_enabled), (bob.id, true));
    ctx.app.core.subjects.counter_changed(SubjectType::SUBJECT_TYPE_CLIP, id(&clip.id), "like_count", 1).await.unwrap();
    let liked: GetClipResponse = ctx.call(CLIPS, "GetClip", None, &get).await.unwrap();
    assert_eq!(liked.clip.as_option().unwrap().like_count, 1);

    // Visibility follows the source: the video goes private, the clip with it (except for its creator and the channel's owner).
    sqlx::query("UPDATE videos SET visibility = 5 WHERE id = $1")
        .bind(public.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.reason(CLIPS, "GetClip", Some(&carol), &get).await, "CLIP_NOT_FOUND");
    assert!(ctx.app.core.subjects.resolve(SubjectType::SUBJECT_TYPE_CLIP, &clip.id, Some(carol.id)).await.is_err());
    let _: GetClipResponse = ctx.call(CLIPS, "GetClip", Some(&bob), &get).await.unwrap();
    let own: GetClipResponse = ctx.call(CLIPS, "GetClip", Some(&alice), &get).await.unwrap();
    assert!(own.clip.as_option().unwrap().can_delete);
    ctx.deliver_event("video.video.unpublished", None, serde_json::json!({ "video_id": public.id, "short_id": public.short_id, "channel_id": public.channel_id, "owner_id": alice.id, "visibility": 5, "origin": 1 })).await;
    assert_eq!(ctx.search_docs(&clip.id).await.pop().unwrap(), None, "no longer searchable");

    // Deleting: the creator or the owner of the channel it was cut from.
    let delete = DeleteClipRequest { clip_id: clip.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(CLIPS, "DeleteClip", Some(&carol), &delete).await, "CLIP_NOT_FOUND");
    let _: DeleteClipResponse = ctx.call(CLIPS, "DeleteClip", Some(&alice), &delete).await.unwrap();
    assert_eq!(ctx.reason(CLIPS, "GetClip", Some(&bob), &get).await, "CLIP_NOT_FOUND");
    assert!(clips::find_ref(&ctx.app.core, &clip.short_id).await.unwrap().is_none());

    // A video that is deleted takes its clips with it.
    let other = video(&ctx, &alice, &channel, Visibility::VISIBILITY_PUBLIC).await;
    let created: CreateClipResponse =
        ctx.call(CLIPS, "CreateClip", Some(&bob), &cut(other.short_id.clone(), "消える", 0.0, 3.0)).await.unwrap();
    let doomed = created.clip.as_option().unwrap().id.clone();
    ctx.deliver_event("video.video.deleted", None, serde_json::json!({ "video_id": other.id, "short_id": other.short_id, "channel_id": other.channel_id, "owner_id": alice.id, "visibility": 1, "origin": 1 })).await;
    assert!(clips::find_ref(&ctx.app.core, &doomed).await.unwrap().is_none());
}

#[tokio::test]
async fn a_stream_is_clipped_while_on_air_within_its_dvr_window() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "ライブクリップch").await;
    let stream = ctx.on_air(&alice, &channel, "クリップされる配信").await;
    sqlx::query("UPDATE live_streams SET started_at = now() - interval '100 seconds' WHERE id = $1")
        .bind(sid(&stream))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let cut = |stream_id: String, start: f64, duration: f64| CreateClipRequest {
        stream_id,
        title: "いまの!".into(),
        start_seconds: start,
        duration_seconds: duration,
        ..Default::default()
    };
    // Not beyond what has been broadcast.
    assert_eq!(
        ctx.reason(CLIPS, "CreateClip", Some(&bob), &cut(card(&stream).id.clone(), 90.0, 30.0)).await,
        "CLIP_RANGE_INVALID"
    );
    let created: CreateClipResponse =
        ctx.call(CLIPS, "CreateClip", Some(&bob), &cut(card(&stream).short_id.clone(), 60.0, 30.0)).await.unwrap();
    let clip = created.clip.as_option().unwrap();
    assert_eq!((clip.stream_id.as_str(), clip.video_id.as_str()), (card(&stream).short_id.as_str(), ""));
    make_ready(&ctx, &clip.id).await;
    let by_stream: ListClipsResponse = ctx
        .call(
            CLIPS,
            "ListClips",
            None,
            &ListClipsRequest { filter: Some(Filter::StreamId(card(&stream).id.clone())), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(by_stream.clips.len(), 1);
    // No DVR, no clips; and a stream that is not on air has nothing to cut from.
    let no_dvr = ctx.on_air_with(&alice, &channel, "巻き戻しなし", |r| r.dvr_enabled = Some(false)).await;
    assert_eq!(
        ctx.reason(CLIPS, "CreateClip", Some(&bob), &cut(card(&no_dvr).id.clone(), 0.0, 1.0)).await,
        "DVR_DISABLED"
    );
    let waiting = ctx.stream(&alice, &channel, "まだ").await;
    assert_eq!(
        ctx.reason(CLIPS, "CreateClip", Some(&bob), &cut(card(&waiting).id.clone(), 0.0, 1.0)).await,
        "CLIP_SOURCE_NOT_READY"
    );
    // Deleting the stream deletes the clips that only it carries.
    let delete = DeleteStreamRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    let _: DeleteStreamResponse = ctx.call(STREAMS, "DeleteStream", Some(&alice), &delete).await.unwrap();
    assert!(clips::find_ref(&ctx.app.core, &clip.id).await.unwrap().is_none());
}

// ───────────────────────────── overlays ─────────────────────────────

#[tokio::test]
async fn overlays_are_configured_by_the_owner_and_opened_with_their_token() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "オーバーレイch").await;
    let presets: ListOverlayPresetsResponse =
        ctx.call(OVERLAYS, "ListOverlayPresets", None, &ListOverlayPresetsRequest::default()).await.unwrap();
    assert!(
        presets
            .presets
            .iter()
            .any(|p| p.id == "bubble" && p.kind == OverlayKind::OVERLAY_KIND_CHAT_BOX && !p.css.is_empty())
    );
    for kind in [
        OverlayKind::OVERLAY_KIND_CHAT_BOX,
        OverlayKind::OVERLAY_KIND_ALERT_BOX,
        OverlayKind::OVERLAY_KIND_GOAL,
        OverlayKind::OVERLAY_KIND_POLL,
    ] {
        assert!(presets.presets.iter().any(|p| p.kind == kind));
    }
    let create = |kind: OverlayKind, name: &str| CreateOverlayRequest {
        channel_id: channel.summary.id.clone(),
        kind: kind.into(),
        name: name.to_owned(),
        ..Default::default()
    };
    for (request, reason) in [
        (create(OverlayKind::OVERLAY_KIND_UNSPECIFIED, "x"), "OVERLAY_KIND_INVALID"),
        (create(OverlayKind::OVERLAY_KIND_CHAT_BOX, ""), "OVERLAY_NAME_INVALID"),
        (
            CreateOverlayRequest { preset: "nope".into(), ..create(OverlayKind::OVERLAY_KIND_CHAT_BOX, "x") },
            "OVERLAY_PRESET_UNKNOWN",
        ),
        (
            CreateOverlayRequest { custom_css: "a".repeat(50_001), ..create(OverlayKind::OVERLAY_KIND_CHAT_BOX, "x") },
            "OVERLAY_CSS_TOO_LONG",
        ),
        // Custom CSS styles the widget; it does not make the streamer's OBS fetch anything.
        (
            CreateOverlayRequest {
                custom_css: "@import 'https://example.com/a.css';".into(),
                ..create(OverlayKind::OVERLAY_KIND_CHAT_BOX, "x")
            },
            "OVERLAY_CSS_UNSAFE",
        ),
        (
            CreateOverlayRequest {
                custom_css: ".chat { background: URL ( https://example.com/pixel.gif ) }".into(),
                ..create(OverlayKind::OVERLAY_KIND_CHAT_BOX, "x")
            },
            "OVERLAY_CSS_UNSAFE",
        ),
        (
            CreateOverlayRequest {
                settings: Some(OverlaySettings { widget: Some(Widget::Poll(Box::default())), ..Default::default() })
                    .into(),
                ..create(OverlayKind::OVERLAY_KIND_CHAT_BOX, "x")
            },
            "OVERLAY_SETTINGS_INVALID",
        ),
    ] {
        assert_eq!(ctx.reason(OVERLAYS, "CreateOverlay", Some(&alice), &request).await, reason);
    }
    assert_eq!(
        ctx.reason(OVERLAYS, "CreateOverlay", Some(&bob), &create(OverlayKind::OVERLAY_KIND_CHAT_BOX, "x")).await,
        "CHANNEL_NOT_FOUND"
    );

    let made: CreateOverlayResponse = ctx
        .call(OVERLAYS, "CreateOverlay", Some(&alice), &create(OverlayKind::OVERLAY_KIND_CHAT_BOX, "チャット"))
        .await
        .unwrap();
    let chat_box = made.overlay.as_option().unwrap().clone();
    assert_eq!(chat_box.preset, "bubble");
    assert!(chat_box.token.len() >= 32 && chat_box.url.ends_with(&format!("/overlay/{}", chat_box.token)));
    let alerts: CreateOverlayResponse = ctx
        .call(OVERLAYS, "CreateOverlay", Some(&alice), &create(OverlayKind::OVERLAY_KIND_ALERT_BOX, "アラート"))
        .await
        .unwrap();
    let alerts = alerts.overlay.as_option().unwrap().clone();
    let goal: CreateOverlayResponse = ctx
        .call(
            OVERLAYS,
            "CreateOverlay",
            Some(&alice),
            &CreateOverlayRequest {
                settings: Some(OverlaySettings {
                    widget: Some(Widget::Goal(Box::new(GoalSettings {
                        kind: GoalKind::GOAL_KIND_COINS.into(),
                        title: "新しいマイク".into(),
                        target: 1_000,
                        ..Default::default()
                    }))),
                    ..Default::default()
                })
                .into(),
                ..create(OverlayKind::OVERLAY_KIND_GOAL, "目標")
            },
        )
        .await
        .unwrap();
    let goal = goal.overlay.as_option().unwrap().clone();
    let polls: CreateOverlayResponse = ctx
        .call(OVERLAYS, "CreateOverlay", Some(&alice), &create(OverlayKind::OVERLAY_KIND_POLL, "投票"))
        .await
        .unwrap();
    let polls = polls.overlay.as_option().unwrap().clone();
    let listed: ListOverlaysResponse = ctx
        .call(
            OVERLAYS,
            "ListOverlays",
            Some(&alice),
            &ListOverlaysRequest { channel_id: channel.summary.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(listed.overlays.len(), 4);
    assert_eq!(
        ctx.reason(
            OVERLAYS,
            "ListOverlays",
            Some(&bob),
            &ListOverlaysRequest { channel_id: channel.summary.id.clone(), ..Default::default() }
        )
        .await,
        "CHANNEL_NOT_FOUND"
    );

    // The token opens the widget without a sign-in, and its topic; nothing else does.
    let guards = &ctx.app.core.topic_guards;
    let open = |token: &str| GetOverlayRequest { token: token.to_owned(), ..Default::default() };
    assert_eq!(ctx.reason(OVERLAYS, "GetOverlay", None, &open("not-a-token")).await, "OVERLAY_NOT_FOUND");
    assert!(!guards.may_subscribe(None, "overlay:not-a-token").await.unwrap());
    let stream = ctx.on_air(&alice, &channel, "オーバーレイの配信").await;
    ctx.say(&bob, &stream, "うつってる?").await.unwrap();
    let widget: GetOverlayResponse = ctx.call(OVERLAYS, "GetOverlay", None, &open(&chat_box.token)).await.unwrap();
    let shown = widget.overlay.as_option().unwrap();
    assert!(shown.token.is_empty() && shown.url.is_empty(), "the widget does not need its own secret back");
    assert!(widget.css.contains(".chat-message"));
    assert_eq!(widget.channel.as_option().unwrap().id, channel.summary.id);
    assert_eq!(widget.stream.as_option().unwrap().id, card(&stream).id);
    // The topic is derived from the token, never the token itself: topic names are logged
    // and travel in frames, the token is what opens the widget.
    assert!(widget.realtime_topic.starts_with("overlay:") && !widget.realtime_topic.contains(&chat_box.token));
    assert!(!guards.may_subscribe(None, &format!("overlay:{}", chat_box.token)).await.unwrap());
    assert_eq!(widget.recent_messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["うつってる?"]);
    assert!(guards.may_subscribe(None, &widget.realtime_topic).await.unwrap());

    // Each widget hears what concerns its kind.
    let topic_of = |token: &str| {
        let request = open(token);
        let ctx = &ctx;
        async move {
            let widget: GetOverlayResponse = ctx.call(OVERLAYS, "GetOverlay", None, &request).await.unwrap();
            widget.realtime_topic
        }
    };
    let old_chat_topic = widget.realtime_topic.clone();
    let mut chat_topic = ctx.app.listen_topic(&old_chat_topic).await;
    let mut alert_topic = ctx.app.listen_topic(&topic_of(&alerts.token).await).await;
    let mut goal_topic = ctx.app.listen_topic(&topic_of(&goal.token).await).await;
    let mut poll_topic = ctx.app.listen_topic(&topic_of(&polls.token).await).await;
    ctx.coins(&bob, 500).await;
    ctx.gift(&bob, &stream, 300, "目標の足しに", "gift-overlay-01").await.unwrap();
    let line = wait_for(&mut chat_topic, |push| match push {
        Push::ChatMessage(message) => Some(message),
        _ => None,
    })
    .await;
    assert_eq!(line.kind, ChatMessageKind::CHAT_MESSAGE_KIND_GIFT);
    let alert = wait_for(&mut alert_topic, |push| match push {
        Push::Alert(alert) => Some(alert),
        _ => None,
    })
    .await;
    assert_eq!((alert.kind, alert.coins, alert.test), (AlertKind::ALERT_KIND_GIFT.into(), 300, false));
    let progress = wait_for(&mut goal_topic, |push| match push {
        Push::GoalProgress(progress) => Some(progress),
        _ => None,
    })
    .await;
    assert_eq!((progress.current, progress.target, progress.title.as_str()), (300, 1_000, "新しいマイク"));
    let _: CreatePollResponse = ctx
        .call(
            POLLS,
            "CreatePoll",
            Some(&alice),
            &CreatePollRequest {
                stream_id: card(&stream).id.clone(),
                question: "どっち?".into(),
                options: vec!["a".into(), "b".into()],
                duration_seconds: 60,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let poll = wait_for(&mut poll_topic, |push| match push {
        Push::PollUpdated(poll) => Some(poll),
        _ => None,
    })
    .await;
    assert_eq!(poll.question, "どっち?");
    let poll_widget: GetOverlayResponse = ctx.call(OVERLAYS, "GetOverlay", None, &open(&polls.token)).await.unwrap();
    assert_eq!(poll_widget.poll.as_option().unwrap().question, "どっち?");
    let goal_widget: GetOverlayResponse = ctx.call(OVERLAYS, "GetOverlay", None, &open(&goal.token)).await.unwrap();
    assert_eq!(goal_widget.goal.as_option().unwrap().current, 300);
    // A gift that is not paid for yet is no progress.
    sqlx::query(
        "INSERT INTO live_gifts (stream_id, channel_id, recipient_id, sender_id, coins, tier, message, highlight_seconds, client_key)
         VALUES ($1, $2, $3, $4, 700, 5, '', 600, 'gift-unpaid-0001')",
    )
    .bind(sid(&stream))
    .bind(id(&channel.summary.id))
    .bind(alice.id)
    .bind(bob.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let goal_widget: GetOverlayResponse = ctx.call(OVERLAYS, "GetOverlay", None, &open(&goal.token)).await.unwrap();
    assert_eq!(goal_widget.goal.as_option().unwrap().current, 300);
    sqlx::query("DELETE FROM live_gifts WHERE client_key = 'gift-unpaid-0001'")
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    // The alert box did not get the chat.
    assert!(pushes(alert_topic.drain(QUIET).await).iter().all(|push| !matches!(push, Push::ChatMessage(_))));

    // A new subscriber is announced once, however often they come and go or the event arrives (checklist 10).
    ctx.subscribe(&bob, &channel, NotificationLevel::NOTIFICATION_LEVEL_ALL).await;
    assert_eq!(ctx.deliver("video.channel.subscribed", bob.id).await, 1);
    ctx.deliver("video.channel.subscribed", bob.id).await;
    let announced = pushes(alert_topic.drain(QUIET).await)
        .into_iter()
        .filter(|push| matches!(push, Push::Alert(alert) if alert.kind == AlertKind::ALERT_KIND_SUBSCRIBER))
        .count();
    assert_eq!(announced, 1);
    assert_eq!(ctx.row(&stream).await.new_subscribers, 1);

    // A sample alert, to position the box.
    let test = SendTestAlertRequest {
        channel_id: channel.summary.id.clone(),
        kind: AlertKind::ALERT_KIND_RAID.into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(OVERLAYS, "SendTestAlert", Some(&bob), &test).await, "CHANNEL_NOT_FOUND");
    let sent: SendTestAlertResponse = ctx.call(OVERLAYS, "SendTestAlert", Some(&alice), &test).await.unwrap();
    assert_eq!(sent.overlay_count, 1);
    let sample = wait_for(&mut alert_topic, |push| match push {
        Push::Alert(alert) => Some(alert),
        _ => None,
    })
    .await;
    assert!(sample.test && sample.kind == AlertKind::ALERT_KIND_RAID);

    // Editing reloads the open widget; a goal that is reset counts from zero.
    let update = UpdateOverlayRequest {
        overlay_id: goal.id.clone(),
        name: Some("目標2".into()),
        custom_css: Some(".goal { color: red; }".into()),
        reset_goal: true,
        ..Default::default()
    };
    assert_eq!(ctx.reason(OVERLAYS, "UpdateOverlay", Some(&bob), &update).await, "OVERLAY_NOT_FOUND");
    let updated: UpdateOverlayResponse = ctx.call(OVERLAYS, "UpdateOverlay", Some(&alice), &update).await.unwrap();
    assert_eq!(updated.overlay.as_option().unwrap().name, "目標2");
    let changed = wait_for(&mut goal_topic, |push| match push {
        Push::OverlayChanged(change) => Some(change),
        _ => None,
    })
    .await;
    assert!(!changed.revoked);
    let goal_widget: GetOverlayResponse = ctx.call(OVERLAYS, "GetOverlay", None, &open(&goal.token)).await.unwrap();
    assert_eq!(goal_widget.goal.as_option().unwrap().current, 0);
    assert!(goal_widget.css.ends_with(".goal { color: red; }"));

    // A rotated token: the old page is told to stop and no longer opens anything.
    let rotate = RotateOverlayTokenRequest { overlay_id: chat_box.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(OVERLAYS, "RotateOverlayToken", Some(&bob), &rotate).await, "OVERLAY_NOT_FOUND");
    let rotated: RotateOverlayTokenResponse =
        ctx.call(OVERLAYS, "RotateOverlayToken", Some(&alice), &rotate).await.unwrap();
    let new_token = rotated.overlay.as_option().unwrap().token.clone();
    assert_ne!(new_token, chat_box.token);
    let revoked = wait_for(&mut chat_topic, |push| match push {
        Push::OverlayChanged(change) => Some(change),
        _ => None,
    })
    .await;
    assert!(revoked.revoked);
    assert_eq!(ctx.reason(OVERLAYS, "GetOverlay", None, &open(&chat_box.token)).await, "OVERLAY_NOT_FOUND");
    assert!(!guards.may_subscribe(None, &old_chat_topic).await.unwrap(), "the old topic went with the old token");
    let _: GetOverlayResponse = ctx.call(OVERLAYS, "GetOverlay", None, &open(&new_token)).await.unwrap();
    ctx.say(&bob, &stream, "新しいほうへ").await.unwrap();
    assert!(pushes(chat_topic.drain(QUIET).await).is_empty(), "nothing is sent to the old token any more");

    let delete = DeleteOverlayRequest { overlay_id: alerts.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(OVERLAYS, "DeleteOverlay", Some(&bob), &delete).await, "OVERLAY_NOT_FOUND");
    let _: DeleteOverlayResponse = ctx.call(OVERLAYS, "DeleteOverlay", Some(&alice), &delete).await.unwrap();
    assert_eq!(ctx.reason(OVERLAYS, "GetOverlay", None, &open(&alerts.token)).await, "OVERLAY_NOT_FOUND");
}

// ───────────────────────────── analytics ─────────────────────────────

#[tokio::test]
async fn the_streamer_gets_a_dashboard_and_a_report() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    let bob = ctx.app.user("bob").await;
    let channel = ctx.channel(&alice, "数字ch").await;
    let stream = ctx.on_air(&alice, &channel, "数字の配信").await;
    ctx.coins(&bob, 500).await;
    ctx.say(&bob, &stream, "一").await.unwrap();
    ctx.say(&bob, &stream, "二").await.unwrap();
    ctx.gift(&bob, &stream, 100, "", "gift-stats-0001").await.unwrap();
    ctx.audience(&stream, 5).await;
    ctx.app
        .core
        .subjects
        .counter_changed(SubjectType::SUBJECT_TYPE_LIVE_STREAM, sid(&stream), "like_count", 3)
        .await
        .unwrap();
    let watch = ReportWatchingRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    let _: ReportWatchingResponse = ctx.call(STREAMS, "ReportWatching", Some(&bob), &watch).await.unwrap();

    let of_stream = GetRealtimeStatsRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(ANALYTICS, "GetRealtimeStats", Some(&bob), &of_stream).await, "STREAM_NOT_FOUND");
    assert_eq!(ctx.reason(ANALYTICS, "GetRealtimeStats", None, &of_stream).await, "SIGN_IN_REQUIRED");
    let live: GetRealtimeStatsResponse =
        ctx.call(ANALYTICS, "GetRealtimeStats", Some(&alice), &of_stream).await.unwrap();
    let stats = live.stats.as_option().unwrap();
    assert_eq!(stats.status, StreamStatus::STREAM_STATUS_LIVE);
    assert!(stats.viewers >= 5 && stats.peak_viewers >= 5);
    assert_eq!((stats.chat_message_count, stats.chat_messages_per_minute), (3, 3));
    assert_eq!((stats.gift_coins, stats.gift_count, stats.like_count), (100, 1, 3));
    assert!(!stats.viewer_samples.is_empty());

    ctx.end(&alice, &stream).await;
    ctx.app.flush_analytics().await;
    let report = GetStreamAnalyticsRequest { stream_id: card(&stream).id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(ANALYTICS, "GetStreamAnalytics", Some(&bob), &report).await, "STREAM_NOT_FOUND");
    let analytics: GetStreamAnalyticsResponse =
        ctx.call(ANALYTICS, "GetStreamAnalytics", Some(&alice), &report).await.unwrap();
    let analytics = analytics.analytics.as_option().unwrap();
    let headline = analytics.report.as_option().unwrap();
    assert!(headline.peak_viewers >= 5 && headline.average_viewers > 0.0);
    assert_eq!(
        (headline.chat_message_count, headline.gift_coins, headline.gift_count, headline.like_count),
        (3, 100, 1, 3)
    );
    assert_eq!(headline.gift_net_coins, 100 - 100 * i64::from(ctx.wallet.platform_fee_percent()) / 100);
    assert_eq!((analytics.unique_viewers, analytics.unique_chatters), (1, 1));
    assert_eq!(analytics.top_supporters.len(), 1);
    assert!(!analytics.viewer_samples.is_empty());

    // The channel's streams that went on air, with their numbers; a cancelled one is not a broadcast.
    let cancelled = ctx.stream(&alice, &channel, "やめた").await;
    ctx.end(&alice, &cancelled).await;
    let list = ListStreamReportsRequest { channel_id: channel.summary.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(ANALYTICS, "ListStreamReports", Some(&bob), &list).await, "CHANNEL_NOT_FOUND");
    let reports: ListStreamReportsResponse =
        ctx.call(ANALYTICS, "ListStreamReports", Some(&alice), &list).await.unwrap();
    assert_eq!(reports.reports.len(), 1);
    assert_eq!(reports.reports[0].stream.as_option().unwrap().id, card(&stream).id);
}
