//! The home feed: typed cards of friends' activity, de-duplicated and spread.
mod common;

use std::collections::HashSet;

use common::*;
use tokuly_core::outbound::ActivityAudience;
use tokuly_proto::{common::SubjectType, feed::*};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// An entry of `kind` about a piece of content.
fn about(
    user: &TestUser,
    kind: &str,
    subject_type: SubjectType,
    subject_id: Uuid,
) -> tokuly_core::outbound::ActivityEntry {
    entry(user, kind, ActivityAudience::Friends)
        .subject(subject_type, subject_id)
        .path(format!("/x/{subject_id}"))
        .image("https://cdn.example/thumb.webp")
}

fn types_of(cards: &[ActivityCard]) -> Vec<ActivityCardType> {
    cards.iter().map(|card| card.r#type.as_known().expect("a known card type")).collect()
}

#[tokio::test]
async fn the_home_feed_is_made_of_typed_cards() {
    use ActivityCardType as T;
    let ctx = start().await;
    let (me, anna, ben, stranger) = (
        ctx.app.user("me").await,
        ctx.app.user("anna").await,
        ctx.app.user("ben").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&me, &anna).await;
    ctx.befriend(&me, &ben).await;

    let video = Uuid::now_v7();
    let cards = [
        ctx.record(about(&anna, "story.posted", SubjectType::SUBJECT_TYPE_STORY, Uuid::now_v7()), 90).await,
        ctx.record(about(&ben, "video.published", SubjectType::SUBJECT_TYPE_VIDEO, video), 80).await,
        ctx.record(about(&anna, "live.started", SubjectType::SUBJECT_TYPE_LIVE_STREAM, Uuid::now_v7()), 70).await,
        ctx.record(about(&ben, "short.posted", SubjectType::SUBJECT_TYPE_SHORT, Uuid::now_v7()), 60).await,
        ctx.record(bump(&anna, &stranger, 35.0, 139.0), 50).await,
        ctx.record(about(&ben, "geo.new_place", SubjectType::SUBJECT_TYPE_PLACE, Uuid::now_v7()), 40).await,
        ctx.record(
            entry(&anna, "geo.new_region", ActivityAudience::Friends)
                .data(serde_json::json!({ "region_name": "北海道" })),
            30,
        )
        .await,
        ctx.record(about(&ben, "stamp.pack.published", SubjectType::SUBJECT_TYPE_STAMP_PACK, Uuid::now_v7()), 20).await,
        ctx.record(note(&anna), 10).await,
    ];
    // Not for the home screen: entries without a card type, private ones,
    // other people's, and my own.
    ctx.record(entry(&anna, "geo.visit", ActivityAudience::Friends).at(35.0, 139.0), 5).await;
    ctx.record(entry(&ben, "wallet.bonus", ActivityAudience::Friends), 4).await;
    let mut private_video = about(&ben, "video.published", SubjectType::SUBJECT_TYPE_VIDEO, Uuid::now_v7());
    private_video.audience = ActivityAudience::Private;
    ctx.record(private_video, 3).await;
    ctx.record(about(&stranger, "video.published", SubjectType::SUBJECT_TYPE_VIDEO, Uuid::now_v7()), 2).await;
    ctx.record(note(&me), 1).await;

    let feed = ctx.home_feed(&me).await;
    let expected: Vec<String> = cards.iter().rev().map(Uuid::to_string).collect();
    assert_eq!(feed.iter().map(head_of).collect::<Vec<_>>(), expected, "friends' typed cards, newest first");
    assert_eq!(
        types_of(&feed),
        vec![
            T::ACTIVITY_CARD_TYPE_NOTE,
            T::ACTIVITY_CARD_TYPE_STAMP_PACK,
            T::ACTIVITY_CARD_TYPE_REGION,
            T::ACTIVITY_CARD_TYPE_PLACE,
            T::ACTIVITY_CARD_TYPE_BUMP,
            T::ACTIVITY_CARD_TYPE_SHORT,
            T::ACTIVITY_CARD_TYPE_LIVE,
            T::ACTIVITY_CARD_TYPE_VIDEO,
            T::ACTIVITY_CARD_TYPE_STORY,
        ]
    );

    // A card carries what a client needs to render and link it, without this
    // domain ever loading the video itself.
    let video_card = feed[7].activity.as_option().unwrap();
    assert_eq!(video_card.kind, "video.published");
    let subject = video_card.subject.as_option().unwrap();
    assert_eq!(
        (subject.r#type.as_known(), subject.id.as_str()),
        (Some(SubjectType::SUBJECT_TYPE_VIDEO), video.to_string().as_str())
    );
    assert_eq!(video_card.actor.as_option().unwrap().id, ben.id.to_string());
    assert_eq!(video_card.summary, "video.published しました");
    assert_eq!(video_card.image_url, "https://cdn.example/thumb.webp");
    assert_eq!(video_card.path, format!("/x/{video}"));
    assert!(video_card.occurred_at.as_option().is_some());

    // Signed-in users only; cursors are checked.
    let anonymous =
        ctx.call::<_, ListHomeFeedResponse>(HOME, "ListHomeFeed", None, &ListHomeFeedRequest::default()).await;
    assert_eq!(anonymous.unwrap_err().code, "unauthenticated");
    let bad = ctx
        .call::<_, ListHomeFeedResponse>(
            HOME,
            "ListHomeFeed",
            Some(&me),
            &ListHomeFeedRequest { page: page(5, "garbage").into(), ..Default::default() },
        )
        .await;
    assert_eq!(bad.unwrap_err().reason, "INVALID_CURSOR");
    assert!(ctx.home_feed(&stranger).await.is_empty(), "no friends, no home feed");
}

#[tokio::test]
async fn duplicates_of_the_same_event_or_content_become_one_card() {
    let ctx = start().await;
    let (me, anna, ben, carol) =
        (ctx.app.user("me").await, ctx.app.user("anna").await, ctx.app.user("ben").await, ctx.app.user("carol").await);
    for friend in [&anna, &ben] {
        ctx.befriend(&me, friend).await;
    }

    // Anna and Ben bumped: the social domain records one entry for each of them.
    let anna_side = ctx.record(bump(&anna, &ben, 35.0, 139.0), 30).await;
    let ben_side = ctx.record(bump(&ben, &anna, 35.0, 139.0), 30).await;
    // Anna also bumped Carol, who is not my friend: only Anna's side exists for me.
    let with_carol = ctx.record(bump(&anna, &carol, 35.0, 139.0), 200).await;
    // Ben posted a note; later Anna renoted the same note.
    let note_id = Uuid::now_v7();
    let posted = ctx.record(about(&ben, "note.posted", SubjectType::SUBJECT_TYPE_NOTE, note_id), 120).await;
    let renoted = ctx.record(about(&anna, "note.renoted", SubjectType::SUBJECT_TYPE_NOTE, note_id), 100).await;

    let feed = ctx.home_feed(&me).await;
    let heads: Vec<&str> = feed.iter().map(head_of).collect();
    let kept_bump = if anna.id < ben.id { anna_side } else { ben_side };
    assert_eq!(
        heads,
        [kept_bump, renoted, with_carol].map(|id| id.to_string()),
        "one card per bump, the newest card per note"
    );
    let _ = posted;

    // The friends' feed is the complete log: it keeps all five.
    assert_eq!(ctx.friends_feed(&me).await.len(), 5);

    // Someone who is friends with only one of the two sees that one's side.
    let dana = ctx.app.user("dana").await;
    ctx.befriend(&dana, &ben).await;
    let danas = ctx.home_feed(&dana).await;
    assert_eq!(danas.iter().map(head_of).collect::<Vec<_>>(), [ben_side, posted].map(|id| id.to_string()));
}

#[tokio::test]
async fn bursts_collapse_and_one_persons_cards_are_spread_out() {
    let ctx = start().await;
    let (me, anna, ben, carol) =
        (ctx.app.user("me").await, ctx.app.user("anna").await, ctx.app.user("ben").await, ctx.app.user("carol").await);
    for friend in [&anna, &ben, &carol] {
        ctx.befriend(&me, friend).await;
    }
    // Anna was busy: five different things in a row, most recently.
    let mut anna_cards = Vec::new();
    for (minutes_ago, kind, subject) in [
        (50, "note.posted", SubjectType::SUBJECT_TYPE_NOTE),
        (40, "story.posted", SubjectType::SUBJECT_TYPE_STORY),
        (30, "video.published", SubjectType::SUBJECT_TYPE_VIDEO),
        (20, "short.posted", SubjectType::SUBJECT_TYPE_SHORT),
        (10, "live.started", SubjectType::SUBJECT_TYPE_LIVE_STREAM),
    ] {
        anna_cards.push(ctx.record(about(&anna, kind, subject, Uuid::now_v7()), minutes_ago).await);
    }
    // Before that, Ben and Carol each did one thing; and Ben posted three notes in a burst.
    let ben_video =
        ctx.record(about(&ben, "video.published", SubjectType::SUBJECT_TYPE_VIDEO, Uuid::now_v7()), 60).await;
    let carol_story =
        ctx.record(about(&carol, "story.posted", SubjectType::SUBJECT_TYPE_STORY, Uuid::now_v7()), 70).await;
    let mut ben_notes = Vec::new();
    for minutes_ago in [130, 110, 90] {
        ben_notes.push(ctx.record(note(&ben), minutes_ago).await);
    }

    let feed = ctx.home_feed(&me).await;
    assert_eq!(feed.len(), 8, "five of Anna's, Ben's video, Carol's story, Ben's burst of notes");
    let actors: Vec<Uuid> = feed.iter().map(actor_of).collect();
    assert_eq!(
        actors,
        vec![anna.id, ben.id, anna.id, carol.id, anna.id, ben.id, anna.id, anna.id],
        "other people's cards are pulled between Anna's"
    );
    // Anna's own cards keep their order, newest first.
    let annas: Vec<&str> = feed.iter().filter(|card| actor_of(card) == anna.id).map(head_of).collect();
    assert_eq!(annas, anna_cards.iter().rev().map(Uuid::to_string).collect::<Vec<_>>());
    assert_eq!(head_of(&feed[1]), ben_video.to_string());
    assert_eq!(head_of(&feed[3]), carol_story.to_string());
    // Ben's three notes are one card.
    let burst = &feed[5];
    assert_eq!((head_of(burst), burst.count), (ben_notes[2].to_string().as_str(), 3));
    assert_eq!(ids(&burst.more), [ben_notes[1], ben_notes[0]].map(|id| id.to_string()));
}

#[tokio::test]
async fn paging_through_the_home_feed_shows_every_card_exactly_once() {
    let ctx = start().await;
    let me = ctx.app.user("me").await;
    let friends = [ctx.app.user("f1").await, ctx.app.user("f2").await, ctx.app.user("f3").await];
    for friend in &friends {
        ctx.befriend(&me, friend).await;
    }
    // 23 cards, two hours apart so nothing collapses; one friend is much busier than the others.
    let mut all = HashSet::new();
    for n in 0..23_i64 {
        let author = &friends[if n % 5 == 0 {
            1
        } else if n % 7 == 0 {
            2
        } else {
            0
        }];
        let id = ctx
            .record(about(author, "video.published", SubjectType::SUBJECT_TYPE_VIDEO, Uuid::now_v7()), n * 120 + 1)
            .await;
        all.insert(id.to_string());
    }

    let mut seen: Vec<String> = Vec::new();
    let mut cursor = String::new();
    let mut pages = 0;
    loop {
        let response: ListHomeFeedResponse = ctx
            .call(
                HOME,
                "ListHomeFeed",
                Some(&me),
                &ListHomeFeedRequest { page: page(5, &cursor).into(), ..Default::default() },
            )
            .await
            .unwrap();
        pages += 1;
        assert!(response.cards.len() <= 5);
        seen.extend(response.cards.iter().map(|card| head_of(card).to_owned()));
        let info = response.page.as_option().unwrap();
        if !info.has_more {
            assert!(info.next_cursor.is_empty());
            break;
        }
        cursor = info.next_cursor.clone();
        assert!(pages < 20, "paging must end");
    }
    assert_eq!(pages, 5);
    assert_eq!(seen.len(), 23, "no card twice");
    assert_eq!(seen.iter().cloned().collect::<HashSet<_>>(), all, "no card missing");
}
