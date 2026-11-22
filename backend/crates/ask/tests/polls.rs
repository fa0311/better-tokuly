//! `ComplimentService`: rounds, polls, flames, hints and the weekly ranking;
//! and the staff side of the question bank.
mod common;

use std::collections::HashSet;

use common::*;
use tokuly_ask::AskConfig;
use tokuly_core::events::Event;
use tokuly_proto::ask::{ask_event::Event as Push, *};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

/// A name no response can contain by accident.
const VOTER: &str = "Zebedee Quillfeather";

fn ids(users: &[TestUser]) -> HashSet<String> {
    users.iter().map(|user| user.id.to_string()).collect()
}

fn choices(round: &Round) -> Vec<String> {
    round.poll.as_option().expect("a poll").choices.iter().map(|choice| choice.id.clone()).collect()
}

fn poll_id(round: &Round) -> String {
    round.poll.as_option().expect("a poll").id.clone()
}

async fn skip(ctx: &Ctx, voter: &TestUser, poll_id: &str) -> Result<Round, RpcError> {
    let response: SkipPollResponse = ctx
        .call(POLLS, "SkipPoll", voter, &SkipPollRequest { poll_id: poll_id.to_owned(), ..Default::default() })
        .await?;
    Ok(response.round.as_option().cloned().expect("round"))
}

async fn shuffle(ctx: &Ctx, voter: &TestUser, poll_id: &str) -> Result<Round, RpcError> {
    let response: ShufflePollResponse = ctx
        .call(POLLS, "ShufflePoll", voter, &ShufflePollRequest { poll_id: poll_id.to_owned(), ..Default::default() })
        .await?;
    Ok(response.round.as_option().cloned().expect("round"))
}

async fn buy_hint(ctx: &Ctx, receiver: &TestUser, flame_id: &str, key: &str) -> Result<BuyHintResponse, RpcError> {
    ctx.call(
        POLLS,
        "BuyHint",
        receiver,
        &BuyHintRequest { flame_id: flame_id.to_owned(), idempotency_key: key.to_owned(), ..Default::default() },
    )
    .await
}

async fn top_flames(ctx: &Ctx, viewer: &TestUser) -> GetTopFlamesResponse {
    ctx.call(POLLS, "GetTopFlames", viewer, &GetTopFlamesRequest::default()).await.expect("top flames")
}

fn shape(events: &[Event]) -> Vec<(String, Option<Uuid>, Vec<String>)> {
    events
        .iter()
        .map(|e| {
            (e.topic.clone(), e.actor_id, e.data.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default())
        })
        .collect()
}

#[tokio::test]
async fn a_round_needs_four_friends_and_shows_four_of_them() {
    let ctx = start().await;
    let voter = ctx.user("Voter").await;
    let mut friends = ctx.friends_of(&voter, 3).await;
    let too_few =
        ctx.call::<_, GetRoundResponse>(POLLS, "GetRound", &voter, &GetRoundRequest::default()).await.unwrap_err();
    assert_eq!((too_few.code.as_str(), too_few.reason.as_str()), ("failed_precondition", "NOT_ENOUGH_FRIENDS"));

    friends.extend(ctx.friends_of(&voter, 1).await);
    let round = ctx.round(&voter).await;
    assert_eq!(round.state, RoundState::ROUND_STATE_ACTIVE);
    assert_eq!((round.poll_count, round.answered_count, round.reward_coins), (12, 0, 0));
    let poll = round.poll.as_option().unwrap();
    assert_eq!((poll.position, poll.shuffles_left), (1, 3));
    assert_eq!(choices(&round).into_iter().collect::<HashSet<_>>(), ids(&friends), "four friends, each once");
    let question = poll.question.as_option().unwrap().text.as_option().unwrap();
    assert!(question.ja.ends_with('?') && question.en.ends_with('?'));

    // The poll stays the same until it is answered.
    let again = ctx.round(&voter).await;
    assert_eq!(
        (again.id.as_str(), poll_id(&again), choices(&again)),
        (round.id.as_str(), poll_id(&round), choices(&round))
    );
    let guest = ctx
        .app
        .call::<_, GetRoundResponse>(&format!("{POLLS}/GetRound"), None, &GetRoundRequest::default())
        .await
        .unwrap_err();
    assert_eq!(guest.code, "unauthenticated");
}

#[tokio::test]
async fn muted_blocked_and_opted_out_friends_are_never_shown() {
    let ctx = start().await;
    let voter = ctx.user("Voter").await;
    let friends = ctx.friends_of(&voter, 7).await;
    ctx.mute(&voter, &friends[0]).await;
    ctx.update_settings(&friends[1], UpdateSettingsRequest { in_polls: Some(false), ..Default::default() }).await;
    ctx.social_block(&friends[2], &voter).await;
    let shown = ids(&friends[3..]);

    let mut round = ctx.round(&voter).await;
    for _ in 0..6 {
        assert_eq!(choices(&round).into_iter().collect::<HashSet<_>>(), shown);
        round = skip(&ctx, &voter, &poll_id(&round)).await.unwrap();
    }

    // Someone who leaves while a poll is waiting is replaced, at no cost.
    ctx.update_settings(&friends[3], UpdateSettingsRequest { in_polls: Some(false), ..Default::default() }).await;
    let short =
        ctx.call::<_, GetRoundResponse>(POLLS, "GetRound", &voter, &GetRoundRequest::default()).await.unwrap_err();
    assert_eq!(short.reason, "NOT_ENOUGH_FRIENDS", "three friends are left");
    ctx.update_settings(&friends[1], UpdateSettingsRequest { in_polls: Some(true), ..Default::default() }).await;
    let refreshed = ctx.round(&voter).await;
    let expected: HashSet<String> =
        [&friends[1], &friends[4], &friends[5], &friends[6]].iter().map(|f| f.id.to_string()).collect();
    assert_eq!(choices(&refreshed).into_iter().collect::<HashSet<_>>(), expected);
    assert_eq!(refreshed.poll.as_option().unwrap().shuffles_left, 3, "a replacement is not a shuffle");
}

#[tokio::test]
async fn everyone_comes_up_before_anyone_comes_up_twice() {
    let ctx = start().await;
    let voter = ctx.user("Voter").await;
    let friends = ctx.friends_of(&voter, 9).await;
    let mut seen: Vec<String> = Vec::new();
    let mut round = ctx.round(&voter).await;
    for _ in 0..2 {
        seen.extend(choices(&round));
        round = skip(&ctx, &voter, &poll_id(&round)).await.unwrap();
    }
    assert_eq!(seen.iter().collect::<HashSet<_>>().len(), 8, "two polls show eight different friends");
    let third = choices(&round);
    let last: Vec<&String> = third.iter().filter(|id| !seen.contains(id)).collect();
    assert_eq!(last.len(), 1, "the ninth friend comes next");
    assert!(ids(&friends).contains(last[0]));
}

#[tokio::test]
async fn a_vote_sends_a_flame_that_names_nobody() {
    let ctx = start().await;
    let voter = ctx.user(VOTER).await;
    let friends = ctx.friends_of(&voter, 4).await;
    let picked = &friends[0];
    let mut pushes = ctx.app.listen_user(picked.id).await;
    let round = ctx.round(&voter).await;
    let question = round.poll.as_option().unwrap().question.as_option().unwrap().clone();

    // Votes that are refused.
    let vote = |poll: String, user: String| VoteRequest { poll_id: poll, user_id: user, ..Default::default() };
    for (request, reason) in [
        (vote(Uuid::now_v7().to_string(), picked.id.to_string()), "POLL_NOT_CURRENT"),
        (vote(poll_id(&round), voter.id.to_string()), "CHOICE_INVALID"),
        (vote(poll_id(&round), Uuid::now_v7().to_string()), "CHOICE_INVALID"),
        (vote(poll_id(&round), "not-an-id".into()), "INVALID_ID"),
    ] {
        assert_eq!(ctx.call::<_, VoteResponse>(POLLS, "Vote", &voter, &request).await.unwrap_err().reason, reason);
    }

    ctx.new_events().await;
    let voted: VoteResponse =
        ctx.call(POLLS, "Vote", &voter, &vote(poll_id(&round), picked.id.to_string())).await.unwrap();
    let after = voted.round.as_option().unwrap();
    assert_eq!((after.answered_count, after.poll.as_option().unwrap().position), (1, 2));
    let events = ctx.new_events().await;
    assert_eq!(
        shape(&events),
        vec![("ask.flame.stored".to_owned(), None, vec!["id".to_owned()])],
        "the vote writes one event, without an actor"
    );
    ctx.handle(&events).await;

    // What reaches the friend who was picked.
    match next_push(&mut pushes).await.expect("the flame is pushed") {
        Push::FlameReceived(received) => {
            assert_no_trace_of(&voter, VOTER, &*received, "the realtime push");
            assert_eq!(received.unseen, 1);
            assert_eq!(received.flame.as_option().unwrap().question.as_option().unwrap().id, question.id);
        }
        other => panic!("unexpected push {other:?}"),
    }
    let notices = ctx.notifications_for(picked.id, "ask.flame").await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0].actor_id.is_none() && notices[0].data["actor_id"].is_null());
    assert_no_trace_of(&voter, VOTER, &notices[0].data, "the notification");
    for event in ctx.deliver().await {
        assert_no_trace_of(&voter, VOTER, &event.data, &format!("the {} event", event.topic));
    }
    for (rpc, response) in [
        ("ListFlames", ctx.json(POLLS, "ListFlames", picked, &ListFlamesRequest::default()).await),
        ("GetFlameSummary", ctx.json(POLLS, "GetFlameSummary", picked, &GetFlameSummaryRequest::default()).await),
        ("MarkFlamesSeen", ctx.json(POLLS, "MarkFlamesSeen", picked, &MarkFlamesSeenRequest::default()).await),
    ] {
        assert_no_trace_of(&voter, VOTER, &response, rpc);
    }
    let flames = ctx.flames(picked).await;
    assert_eq!(flames.len(), 1);
    assert_eq!(flames[0].question.as_option().unwrap().id, question.id);
    assert!(!flames[0].seen && flames[0].hints.is_empty());
    assert_eq!(flames[0].next_hint_price, 30);
    assert!(ctx.flames(&voter).await.is_empty(), "the voter received nothing");

    // The numbers.
    let summary: GetFlameSummaryResponse =
        ctx.call(POLLS, "GetFlameSummary", picked, &GetFlameSummaryRequest::default()).await.unwrap();
    let summary = summary.summary.as_option().unwrap();
    assert_eq!((summary.total, summary.unseen, summary.this_week), (1, 1, 1));
    assert_eq!(summary.top_questions[0].question.as_option().unwrap().id, question.id);
    assert_eq!(summary.top_questions[0].count, 1);
    let seen: MarkFlamesSeenResponse = ctx
        .call(
            POLLS,
            "MarkFlamesSeen",
            picked,
            &MarkFlamesSeenRequest { flame_ids: vec![flames[0].id.clone()], ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(seen.unseen, 0);
    assert!(ctx.flames(picked).await[0].seen);
    let too_many = MarkFlamesSeenRequest {
        flame_ids: (0..101).map(|_| Uuid::now_v7().to_string()).collect(),
        ..Default::default()
    };
    assert_eq!(
        ctx.call::<_, MarkFlamesSeenResponse>(POLLS, "MarkFlamesSeen", picked, &too_many).await.unwrap_err().reason,
        "TOO_MANY_IDS"
    );
}

#[tokio::test]
async fn polls_can_be_skipped_and_shuffled() {
    let ctx = start().await;
    let voter = ctx.user("Voter").await;
    ctx.friends_of(&voter, 9).await;
    let round = ctx.round(&voter).await;
    let before = choices(&round);

    let shuffled = shuffle(&ctx, &voter, &poll_id(&round)).await.unwrap();
    let after = choices(&shuffled);
    assert_eq!(poll_id(&shuffled), poll_id(&round), "the same poll");
    assert_eq!(after.len(), 4);
    assert!(after.iter().all(|id| !before.contains(id)), "four others");
    assert_eq!(shuffled.poll.as_option().unwrap().shuffles_left, 2);
    for left in [1, 0] {
        assert_eq!(
            shuffle(&ctx, &voter, &poll_id(&round)).await.unwrap().poll.as_option().unwrap().shuffles_left,
            left
        );
    }
    let none_left = shuffle(&ctx, &voter, &poll_id(&round)).await.unwrap_err();
    assert_eq!((none_left.code.as_str(), none_left.reason.as_str()), ("failed_precondition", "NO_SHUFFLES_LEFT"));

    let skipped = skip(&ctx, &voter, &poll_id(&round)).await.unwrap();
    assert_eq!((skipped.answered_count, skipped.poll.as_option().unwrap().position), (1, 2));
    assert_eq!(skipped.poll.as_option().unwrap().shuffles_left, 3, "each poll has its own shuffles");
    assert_eq!(skip(&ctx, &voter, &poll_id(&round)).await.unwrap_err().reason, "POLL_NOT_CURRENT");
    assert_eq!(shuffle(&ctx, &voter, &poll_id(&round)).await.unwrap_err().reason, "POLL_NOT_CURRENT");
    assert!(ctx.app.events_by("ask.flame.", voter.id).await.is_empty());

    // With exactly four friends there is nobody to shuffle to; with five,
    // the fifth comes in next to three of the four.
    let few = ctx.user("Few").await;
    let mut circle = ctx.friends_of(&few, 4).await;
    let round = ctx.round(&few).await;
    assert_eq!(shuffle(&ctx, &few, &poll_id(&round)).await.unwrap_err().reason, "NOT_ENOUGH_FRIENDS");
    circle.extend(ctx.friends_of(&few, 1).await);
    let shuffled = choices(&shuffle(&ctx, &few, &poll_id(&round)).await.unwrap());
    assert_eq!(shuffled.iter().collect::<HashSet<_>>().len(), 4);
    assert!(shuffled.contains(&circle[4].id.to_string()));
}

#[tokio::test]
async fn finishing_a_round_pays_for_the_votes_and_starts_the_cooldown() {
    let ctx = start_with(AskConfig { round_polls: 3, round_reward_coins: 12, ..Default::default() }).await;
    let voter = ctx.user("Voter").await;
    let friends = ctx.friends_of(&voter, 4).await;
    let pick = friends[0].id.to_string();
    ctx.new_events().await;

    let round = ctx.round(&voter).await;
    assert_eq!(round.poll_count, 3);
    let second = ctx.vote(&voter, &poll_id(&round), &pick).await;
    let third = skip(&ctx, &voter, &poll_id(&second)).await.unwrap();
    assert_eq!(ctx.coins(&voter).await, 0, "nothing is paid before the round ends");
    let done = ctx.vote(&voter, &poll_id(&third), &pick).await;

    assert_eq!(done.state, RoundState::ROUND_STATE_COOLDOWN);
    assert_eq!(
        (done.answered_count, done.poll_count, done.reward_coins),
        (3, 3, 8),
        "two votes out of three polls: two thirds of twelve coins"
    );
    assert!(done.poll.is_unset());
    let wait =
        done.next_round_at.as_option().expect("when the next round opens").seconds - chrono::Utc::now().timestamp();
    assert!((3500..=3600).contains(&wait), "the next round opens in an hour, not in {wait} s");
    assert_eq!(ctx.coins(&voter).await, 8);
    let completed = ctx.app.events_by("ask.round.completed", voter.id).await;
    assert_eq!(completed.len(), 1);
    assert_eq!(
        completed[0].data,
        serde_json::json!({ "user_id": voter.id, "round_id": done.id, "votes": 2, "skipped": 1, "coins": 8 })
    );

    // During the cooldown there is nothing to answer.
    let waiting = ctx.round(&voter).await;
    assert_eq!(
        (waiting.id.as_str(), waiting.state.as_known(), waiting.reward_coins),
        (done.id.as_str(), Some(RoundState::ROUND_STATE_COOLDOWN), 8)
    );
    assert_eq!(ctx.coins(&voter).await, 8, "asking again does not pay again");
    let request = VoteRequest { poll_id: poll_id(&third), user_id: pick.clone(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, VoteResponse>(POLLS, "Vote", &voter, &request).await.unwrap_err().reason,
        "ROUND_NOT_ACTIVE"
    );
    assert_eq!(skip(&ctx, &voter, &poll_id(&third)).await.unwrap_err().reason, "ROUND_NOT_ACTIVE");
    assert_eq!(shuffle(&ctx, &voter, &poll_id(&third)).await.unwrap_err().reason, "ROUND_NOT_ACTIVE");

    // A round that was only skipped pays nothing.
    let skipper = ctx.user("Skipper").await;
    ctx.friends_of(&skipper, 4).await;
    let mut round = ctx.round(&skipper).await;
    for _ in 0..2 {
        round = skip(&ctx, &skipper, &poll_id(&round)).await.unwrap();
    }
    let done = skip(&ctx, &skipper, &poll_id(&round)).await.unwrap();
    assert_eq!((done.state.as_known(), done.reward_coins), (Some(RoundState::ROUND_STATE_COOLDOWN), 0));
    assert_eq!(ctx.coins(&skipper).await, 0);
}

#[tokio::test]
async fn a_new_round_opens_when_the_cooldown_is_over() {
    let ctx = start_with(AskConfig { round_polls: 2, round_cooldown_seconds: 0, ..Default::default() }).await;
    let voter = ctx.user("Voter").await;
    let friends = ctx.friends_of(&voter, 4).await;
    let pick = friends[1].id.to_string();
    let first = ctx.round(&voter).await;
    let first_question = first.poll.as_option().unwrap().question.as_option().unwrap().id.clone();
    let second_poll = ctx.vote(&voter, &poll_id(&first), &pick).await;
    let second_question = second_poll.poll.as_option().unwrap().question.as_option().unwrap().id.clone();
    assert_ne!(first_question, second_question, "a round does not ask the same question twice");
    ctx.vote(&voter, &poll_id(&second_poll), &pick).await;

    let next = ctx.round(&voter).await;
    assert_eq!(next.state, RoundState::ROUND_STATE_ACTIVE);
    assert_ne!(next.id, first.id);
    assert_eq!(next.answered_count, 0);
    let next_question = next.poll.as_option().unwrap().question.as_option().unwrap().id.clone();
    assert!(
        next_question != first_question && next_question != second_question,
        "a new round prefers questions the last rounds did not have"
    );
}

#[tokio::test]
async fn a_voter_the_receiver_blocked_cannot_tell() {
    let ctx = start().await;
    let voter = ctx.user(VOTER).await;
    let friends = ctx.friends_of(&voter, 4).await;
    let (blocker, other) = (&friends[0], &friends[1]);
    let moderator = ctx.app.user_with_roles("Moderator", &["moderator"]).await;

    // The first flame arrives; its receiver blocks whoever sent it.
    let round = ctx.round(&voter).await;
    ctx.vote(&voter, &poll_id(&round), &blocker.id.to_string()).await;
    let flame = ctx.flames(blocker).await.remove(0);
    let request = BlockSenderRequest {
        target: Some(block_sender_request::Target::FlameId(flame.id.clone())),
        ..Default::default()
    };
    let block: BlockSenderResponse = ctx.call(BOX, "BlockSender", blocker, &request).await.unwrap();
    let block = block.block.as_option().unwrap();
    assert_eq!(block.source, BlockSource::BLOCK_SOURCE_FLAME);
    assert_eq!(
        block.preview,
        flame.question.as_option().unwrap().text.as_option().unwrap().ja,
        "the block remembers the question, not the person"
    );
    assert_no_trace_of(&voter, VOTER, block, "the block");
    assert_eq!(ctx.flames(blocker).await.len(), 1, "the flame that is there stays");
    assert_eq!(
        ctx.call::<_, BlockSenderResponse>(BOX, "BlockSender", other, &request).await.unwrap_err().reason,
        "BLOCK_TARGET_NOT_FOUND"
    );

    // Two more votes: one for the friend who blocked the voter, one for another.
    let mut pushes = ctx.app.listen_user(blocker.id).await;
    ctx.new_events().await;
    let round = ctx.round(&voter).await;
    assert!(
        choices(&round).contains(&blocker.id.to_string()),
        "the friend who blocked the voter is still among the choices"
    );
    let vote = |round: &Round, user: &TestUser| VoteRequest {
        poll_id: poll_id(round),
        user_id: user.id.to_string(),
        ..Default::default()
    };
    let dropped = ctx.json(POLLS, "Vote", &voter, &vote(&round, blocker)).await;
    let dropped_events = ctx.new_events().await;
    let round = ctx.round(&voter).await;
    let arrived = ctx.json(POLLS, "Vote", &voter, &vote(&round, other)).await;
    let arrived_events = ctx.new_events().await;
    let keys =
        |response: &serde_json::Value| response["round"].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&dropped), keys(&arrived), "the response has the same fields either way");
    assert_eq!(
        arrived["round"]["answeredCount"].as_i64().unwrap(),
        dropped["round"]["answeredCount"].as_i64().unwrap() + 1,
        "both votes moved the round on"
    );
    assert_eq!(shape(&dropped_events), shape(&arrived_events), "the vote writes the same events either way");
    ctx.handle(&dropped_events).await;
    ctx.handle(&arrived_events).await;

    // One of the two arrived.
    assert_eq!(ctx.flames(blocker).await.len(), 1);
    assert_eq!(ctx.notifications_for(blocker.id, "ask.flame").await.len(), 1);
    assert!(stays_quiet(&mut pushes).await, "nothing is pushed for the dropped flame");
    assert_eq!(ctx.flames(other).await.len(), 1);

    // The ranking the voter sees counts their vote: a count that stood
    // still would give the block away. The receiver's own count matches
    // their inbox.
    let for_voter = top_flames(&ctx, &voter).await;
    let count_of = |ranking: &GetTopFlamesResponse, user: &TestUser| {
        ranking
            .entries
            .iter()
            .find(|entry| entry.user.as_option().unwrap().id == user.id.to_string())
            .map(|entry| entry.count)
    };
    assert_eq!(count_of(&for_voter, blocker), Some(2));
    assert_eq!(count_of(&for_voter, other), Some(1));
    assert_eq!(top_flames(&ctx, blocker).await.my_count, 1);

    // The dropped vote earns its coin like any other: twelve votes, twelve coins.
    for _ in 0..9 {
        let round = ctx.round(&voter).await;
        ctx.vote(&voter, &poll_id(&round), &blocker.id.to_string()).await;
    }
    assert_eq!(ctx.round(&voter).await.state, RoundState::ROUND_STATE_COOLDOWN);
    assert_eq!(ctx.coins(&voter).await, 12);
    assert_eq!(ctx.flames(blocker).await.len(), 1, "none of the later votes arrived either");

    // Moderators see the dropped flame and who sent it.
    let dropped_id: Uuid =
        sqlx::query_scalar("SELECT id FROM ask_flames WHERE receiver_id = $1 AND dropped ORDER BY id LIMIT 1")
            .bind(blocker.id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    let request = InspectFlameRequest { flame_id: dropped_id.to_string(), ..Default::default() };
    let seen: InspectFlameResponse = ctx.call(MODERATION, "InspectFlame", &moderator, &request).await.unwrap();
    let record = seen.record.as_option().unwrap();
    assert_eq!(record.sender.as_option().unwrap().id, voter.id.to_string());
    assert!(record.dropped && !record.signed);
    assert_eq!(
        ctx.call::<_, InspectFlameResponse>(MODERATION, "InspectFlame", blocker, &request).await.unwrap_err().reason,
        "FORBIDDEN"
    );
    let missing = InspectFlameRequest { flame_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, InspectFlameResponse>(MODERATION, "InspectFlame", &moderator, &missing).await.unwrap_err().reason,
        "FLAME_NOT_FOUND"
    );
}

#[tokio::test]
async fn hints_cost_coins_and_never_name_the_sender() {
    let ctx = start().await;
    let voter = ctx.user(VOTER).await;
    let friends = ctx.friends_of(&voter, 4).await;
    let receiver = &friends[0];
    let round = ctx.round(&voter).await;
    ctx.vote(&voter, &poll_id(&round), &receiver.id.to_string()).await;
    let flame = ctx.flames(receiver).await.remove(0);

    for bad_key in ["", "short", &"k".repeat(129)] {
        assert_eq!(buy_hint(&ctx, receiver, &flame.id, bad_key).await.unwrap_err().reason, "IDEMPOTENCY_KEY_INVALID");
    }
    assert_eq!(
        buy_hint(&ctx, &friends[1], &flame.id, "purchase-0001").await.unwrap_err().reason,
        "FLAME_NOT_FOUND",
        "someone else's flame"
    );
    assert_eq!(
        buy_hint(&ctx, receiver, &Uuid::now_v7().to_string(), "purchase-0001").await.unwrap_err().reason,
        "FLAME_NOT_FOUND"
    );
    let broke = buy_hint(&ctx, receiver, &flame.id, "purchase-0001").await.unwrap_err();
    assert_eq!((broke.code.as_str(), broke.reason.as_str()), ("failed_precondition", "INSUFFICIENT_COINS"));
    assert!(ctx.flames(receiver).await[0].hints.is_empty(), "a purchase that could not be paid leaves no hint behind");

    // The first hint: the first letter.
    ctx.give_coins(receiver, 100).await;
    let first = buy_hint(&ctx, receiver, &flame.id, "purchase-0001").await.unwrap();
    let hint = first.hint.as_option().unwrap();
    assert_eq!((hint.kind.as_known(), hint.value.as_str()), (Some(HintKind::HINT_KIND_INITIAL), "Z"));
    assert!(hint.text.as_option().unwrap().ja.contains("「Z」"));
    assert_eq!(first.coins_spent, 30);
    assert_eq!(
        (first.flame.as_option().unwrap().hints.len(), first.flame.as_option().unwrap().next_hint_price),
        (1, 60)
    );
    assert_eq!(ctx.coins(receiver).await, 70);

    // Repeating the purchase charges nothing and buys nothing new.
    let repeated = buy_hint(&ctx, receiver, &flame.id, "purchase-0001").await.unwrap();
    assert_eq!((repeated.coins_spent, repeated.hint.as_option().unwrap().value.as_str()), (0, "Z"));
    assert_eq!(ctx.coins(receiver).await, 70);

    // The second: how long the two have been friends (they just met).
    let second = buy_hint(&ctx, receiver, &flame.id, "purchase-0002").await.unwrap();
    let hint = second.hint.as_option().unwrap();
    assert_eq!(
        (hint.kind.as_known(), hint.value.as_str(), second.coins_spent),
        (Some(HintKind::HINT_KIND_FRIEND_MONTHS), "0", 60)
    );
    assert_eq!(second.flame.as_option().unwrap().next_hint_price, 0);
    assert_eq!(ctx.coins(receiver).await, 10);
    let no_more = buy_hint(&ctx, receiver, &flame.id, "purchase-0003").await.unwrap_err();
    assert_eq!((no_more.code.as_str(), no_more.reason.as_str()), ("failed_precondition", "NO_MORE_HINTS"));
    assert_eq!(ctx.coins(receiver).await, 10);

    // Nothing the receiver got names the sender: not the responses, not the
    // flame with its hints, not what the wallet says about the purchase.
    for (what, value) in [
        ("the first purchase", serde_json::to_value(&first).unwrap()),
        ("the second purchase", serde_json::to_value(&second).unwrap()),
    ] {
        assert_no_trace_of(&voter, VOTER, &value, what);
    }
    assert_no_trace_of(
        &voter,
        VOTER,
        &ctx.json(POLLS, "ListFlames", receiver, &ListFlamesRequest::default()).await,
        "the flames with their hints",
    );
    let wallet = ctx.app.events_by("wallet.balance.changed", receiver.id).await;
    assert_eq!(wallet.iter().filter(|e| e.data["reason"] == serde_json::json!("ask.hint")).count(), 2);
    for event in wallet {
        assert_no_trace_of(&voter, VOTER, &event.data, "the wallet event");
    }
}

#[tokio::test]
async fn the_second_hint_is_the_school_or_the_months_of_friendship() {
    let ctx = start_with(AskConfig { round_cooldown_seconds: 0, ..Default::default() }).await;
    let (schooled, old_friend) = (ctx.user("Schooled").await, ctx.user("Oldfriend").await);
    let receiver = ctx.user("Receiver").await;
    ctx.lone_friends_of(&receiver, 1).await;
    for voter in [&schooled, &old_friend] {
        ctx.befriend(voter, &receiver).await;
        ctx.friends_of(voter, 3).await;
    }
    ctx.give_coins(&receiver, 500).await;
    sqlx::query("UPDATE users SET school_name = '桜丘高校' WHERE id = $1")
        .bind(schooled.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    sqlx::query("UPDATE ask_friend_links SET since = now() - interval '95 days' WHERE user_id = $1 AND friend_id = $2")
        .bind(receiver.id)
        .bind(old_friend.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();

    ctx.vote_for(&schooled, &receiver).await;
    let from_schooled = ctx.flames(&receiver).await.remove(0);
    buy_hint(&ctx, &receiver, &from_schooled.id, "school-hint-1").await.unwrap();
    let school = buy_hint(&ctx, &receiver, &from_schooled.id, "school-hint-2").await.unwrap();
    let hint = school.hint.as_option().unwrap();
    assert_eq!((hint.kind.as_known(), hint.value.as_str()), (Some(HintKind::HINT_KIND_SCHOOL), "桜丘高校"));

    ctx.vote_for(&old_friend, &receiver).await;
    let from_old_friend = ctx.flames(&receiver).await.remove(0);
    let initial = buy_hint(&ctx, &receiver, &from_old_friend.id, "months-hint-1").await.unwrap();
    assert_eq!(initial.hint.as_option().unwrap().value, "O");
    let months = buy_hint(&ctx, &receiver, &from_old_friend.id, "months-hint-2").await.unwrap();
    let hint = months.hint.as_option().unwrap();
    assert_eq!((hint.kind.as_known(), hint.value.as_str()), (Some(HintKind::HINT_KIND_FRIEND_MONTHS), "3"));
    assert!(hint.text.as_option().unwrap().en.contains("3 months"));
}

#[tokio::test]
async fn what_a_flame_sells_is_fixed_when_the_vote_is_cast() {
    let ctx = start_with(AskConfig { round_cooldown_seconds: 0, ..Default::default() }).await;
    let voter = ctx.user(VOTER).await;
    let friends = ctx.friends_of(&voter, 4).await;
    let receiver = &friends[0];
    let vote = async || {
        let round = ctx.round(&voter).await;
        ctx.vote(&voter, &poll_id(&round), &receiver.id.to_string()).await;
    };

    // One vote with hints on, one after the voter switched them off.
    vote().await;
    ctx.update_settings(&voter, UpdateSettingsRequest { allow_hints: Some(false), ..Default::default() }).await;
    vote().await;
    let flames = ctx.flames(receiver).await;
    let (silent, selling) = (flames[0].clone(), flames[1].clone());
    assert_eq!((selling.next_hint_price, silent.next_hint_price), (30, 0));

    // What the receiver can find out without paying, now and after every
    // change to the voter's account: always the same.
    let listed = async || ctx.json(POLLS, "ListFlames", receiver, &ListFlamesRequest::default()).await;
    let probe = async || {
        (
            buy_hint(&ctx, receiver, &selling.id, "probe-selling").await.unwrap_err().reason,
            buy_hint(&ctx, receiver, &silent.id, "probe-silent").await.unwrap_err().reason,
        )
    };
    let flames_before = listed().await;
    let probed_before = probe().await;
    assert_eq!(probed_before, ("INSUFFICIENT_COINS".to_owned(), "HINTS_UNAVAILABLE".to_owned()));

    // The voter switches hints back on: the flame cast without them stays
    // without them.
    ctx.update_settings(&voter, UpdateSettingsRequest { allow_hints: Some(true), ..Default::default() }).await;
    assert_eq!((listed().await, probe().await), (flames_before.clone(), probed_before.clone()));
    // ... changes their name and school: the hints do not follow.
    sqlx::query("UPDATE users SET display_name = 'Someone Else', school_name = '別の高校' WHERE id = $1")
        .bind(voter.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!((listed().await, probe().await), (flames_before.clone(), probed_before.clone()));
    // ... is suspended.
    ctx.suspend(&voter).await;
    assert_eq!(
        (listed().await, probe().await),
        (flames_before.clone(), probed_before.clone()),
        "a suspension shows on a flame's price list or on a purchase"
    );

    // The hints are sold as they were on the day of the vote.
    ctx.give_coins(receiver, 100).await;
    let first = buy_hint(&ctx, receiver, &selling.id, "purchase-0001").await.unwrap();
    assert_eq!((first.coins_spent, first.hint.as_option().unwrap().value.as_str()), (30, "Z"));
    // ... and still after the account is gone.
    ctx.announce("identity.user.deleted", serde_json::json!({ "user_id": voter.id })).await;
    let after_deletion = ctx.flames(receiver).await;
    assert_eq!((after_deletion[1].hints.len(), after_deletion[1].next_hint_price), (1, 60));
    assert_eq!(after_deletion[0].next_hint_price, 0);
    let second = buy_hint(&ctx, receiver, &selling.id, "purchase-0002").await.unwrap();
    assert_eq!(
        (second.coins_spent, second.hint.as_option().unwrap().kind.as_known()),
        (60, Some(HintKind::HINT_KIND_FRIEND_MONTHS))
    );
    let unavailable = buy_hint(&ctx, receiver, &silent.id, "purchase-0003").await.unwrap_err();
    assert_eq!((unavailable.code.as_str(), unavailable.reason.as_str()), ("failed_precondition", "HINTS_UNAVAILABLE"));
    assert_eq!(ctx.coins(receiver).await, 10, "nothing is charged for a hint that is not given");
}

#[tokio::test]
async fn a_purchase_interrupted_after_the_charge_is_completed_without_paying_again() {
    let ctx = start().await;
    let voter = ctx.user(VOTER).await;
    let friends = ctx.friends_of(&voter, 4).await;
    let receiver = &friends[0];
    ctx.give_coins(receiver, 100).await;
    let round = ctx.round(&voter).await;
    ctx.vote(&voter, &poll_id(&round), &receiver.id.to_string()).await;
    let flame = ctx.flames(receiver).await.remove(0);

    // The wallet was charged, and the request stopped before the hint was
    // stored.
    let flame_id = Uuid::parse_str(&flame.id).unwrap();
    tokuly_wallet::ledger::spend(
        &ctx.app.core,
        receiver.id,
        30,
        "ask.hint",
        &format!("ask.hint:{flame_id}:0"),
        serde_json::json!({ "flame_id": flame_id, "hint": 1 }),
    )
    .await
    .unwrap();
    assert_eq!(ctx.coins(receiver).await, 70);
    assert!(ctx.flames(receiver).await[0].hints.is_empty());

    // The next attempt gets the hint that was paid for.
    let completed = buy_hint(&ctx, receiver, &flame.id, "retry-after-crash").await.unwrap();
    assert_eq!((completed.coins_spent, completed.hint.as_option().unwrap().value.as_str()), (0, "Z"));
    assert_eq!(ctx.coins(receiver).await, 70, "the buyer paid once");
    assert_eq!(ctx.flames(receiver).await[0].hints.len(), 1);
    // And the one after that buys the second.
    let second = buy_hint(&ctx, receiver, &flame.id, "second-purchase").await.unwrap();
    assert_eq!(second.coins_spent, 60);
    assert_eq!(ctx.coins(receiver).await, 10);
}

#[tokio::test]
async fn only_friends_with_three_friends_of_their_own_are_picked() {
    let ctx = start().await;
    let voter = ctx.user("Voter").await;
    // Four friends who have nobody but the voter: a flame for one of them
    // could only have come from the voter.
    let lonely = ctx.lone_friends_of(&voter, 4).await;
    let too_few =
        ctx.call::<_, GetRoundResponse>(POLLS, "GetRound", &voter, &GetRoundRequest::default()).await.unwrap_err();
    assert_eq!((too_few.code.as_str(), too_few.reason.as_str()), ("failed_precondition", "NOT_ENOUGH_FRIENDS"));

    // With four friends who have a crowd of their own, a round starts, and
    // the lonely ones never appear in it.
    let friends = ctx.friends_of(&voter, 4).await;
    let mut round = ctx.round(&voter).await;
    for _ in 0..3 {
        assert_eq!(choices(&round).into_iter().collect::<HashSet<_>>(), ids(&friends));
        round = skip(&ctx, &voter, &poll_id(&round)).await.unwrap();
    }
    assert_eq!(shuffle(&ctx, &voter, &poll_id(&round)).await.unwrap_err().reason, "NOT_ENOUGH_FRIENDS");

    // A lonely friend who gets two more friends joins in.
    ctx.lone_friends_of(&lonely[0], 2).await;
    let shuffled = choices(&shuffle(&ctx, &voter, &poll_id(&round)).await.unwrap());
    assert!(shuffled.contains(&lonely[0].id.to_string()));

    // Someone who lost their crowd while the poll was waiting cannot be
    // voted for; the poll is repaired the next time it is looked at.
    let picked = &friends[0];
    let round = ctx.round(&voter).await;
    let shown = choices(&round);
    let target = if shown.contains(&picked.id.to_string()) { picked } else { &friends[1] };
    let bystanders: Vec<Uuid> =
        sqlx::query_scalar("SELECT friend_id FROM ask_friend_links WHERE user_id = $1 AND friend_id <> $2")
            .bind(target.id)
            .bind(voter.id)
            .fetch_all(&ctx.app.core.db)
            .await
            .unwrap();
    for bystander in bystanders {
        let _: tokuly_proto::social::RemoveFriendResponse = ctx
            .call(
                "tokuly.social.v1.FriendService",
                "RemoveFriend",
                target,
                &tokuly_proto::social::RemoveFriendRequest { user_id: bystander.to_string(), ..Default::default() },
            )
            .await
            .unwrap();
    }
    ctx.deliver().await;
    let request = VoteRequest { poll_id: poll_id(&round), user_id: target.id.to_string(), ..Default::default() };
    let refused = ctx.call::<_, VoteResponse>(POLLS, "Vote", &voter, &request).await.unwrap_err();
    assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("invalid_argument", "CHOICE_INVALID"));
    assert!(ctx.flames(target).await.is_empty());
    let repaired = ctx.round(&voter).await;
    assert_eq!(poll_id(&repaired), poll_id(&round), "the same poll");
    assert!(!choices(&repaired).contains(&target.id.to_string()), "with someone else in their seat");
    assert_eq!(repaired.poll.as_option().unwrap().shuffles_left, round.poll.as_option().unwrap().shuffles_left);
}

#[tokio::test]
async fn the_weekly_ranking_orders_friends_by_flames() {
    let ctx = start().await;
    let voter = ctx.user("Voter").await;
    let friends = ctx.friends_of(&voter, 4).await;
    let (most, some, hidden) = (&friends[0], &friends[1], &friends[2]);
    for picked in [most, most, some, hidden] {
        let round = ctx.round(&voter).await;
        ctx.vote(&voter, &poll_id(&round), &picked.id.to_string()).await;
    }
    ctx.update_settings(hidden, UpdateSettingsRequest { in_polls: Some(false), ..Default::default() }).await;

    let ranking = top_flames(&ctx, &voter).await;
    let lines: Vec<(Uuid, i32, i32)> =
        ranking.entries.iter().map(|entry| (id_of(entry.user.as_option().unwrap()), entry.count, entry.rank)).collect();
    assert_eq!(lines, vec![(most.id, 2, 1), (some.id, 1, 2)], "most first; someone who left the polls is not ranked");
    assert_eq!(ranking.my_count, 0);
    assert!(ranking.week_start.as_option().unwrap().seconds <= chrono::Utc::now().timestamp());

    // The receiver sees themselves, and only among their own friends.
    let own = top_flames(&ctx, most).await;
    assert_eq!(own.my_count, 2);
    assert_eq!(
        own.entries.iter().map(|entry| (id_of(entry.user.as_option().unwrap()), entry.count)).collect::<Vec<_>>(),
        vec![(most.id, 2)]
    );
}

#[tokio::test]
async fn staff_manage_the_question_bank() {
    let ctx = start().await;
    let user = ctx.user("User").await;
    let moderator = ctx.app.user_with_roles("Moderator", &["moderator"]).await;
    let admin = ctx.app.user_with_roles("Admin", &["admin"]).await;

    // The built-in bank.
    let mut built_in = 0;
    let mut cursor = String::new();
    loop {
        let request = ListPollQuestionsRequest {
            page: tokuly_proto::common::PageRequest { limit: 50, cursor, ..Default::default() }.into(),
            ..Default::default()
        };
        let page: ListPollQuestionsResponse =
            ctx.call(MODERATION, "ListPollQuestions", &moderator, &request).await.unwrap();
        built_in += page.questions.iter().filter(|entry| entry.built_in).count();
        for entry in page.questions.iter().filter(|entry| entry.built_in) {
            let text = entry.question.as_option().unwrap().text.as_option().unwrap();
            assert!(!text.ja.is_empty() && !text.en.is_empty());
        }
        let info = page.page.as_option().unwrap();
        if !info.has_more {
            break;
        }
        cursor = info.next_cursor.clone();
    }
    assert!(built_in >= 150, "the bank ships at least 150 questions, found {built_in}");
    let refused = ctx
        .call::<_, ListPollQuestionsResponse>(
            MODERATION,
            "ListPollQuestions",
            &user,
            &ListPollQuestionsRequest::default(),
        )
        .await
        .unwrap_err();
    assert_eq!((refused.code.as_str(), refused.reason.as_str()), ("permission_denied", "FORBIDDEN"));

    // Adding and switching off need an admin.
    let text = |ja: &str, en: &str| tokuly_proto::common::LocalizedText {
        ja: ja.to_owned(),
        en: en.to_owned(),
        ..Default::default()
    };
    let add = AddPollQuestionRequest {
        text: text("給食をおいしそうに食べる人は?", "Who enjoys school lunch the most?").into(),
        emoji: "🍛".into(),
        ..Default::default()
    };
    assert_eq!(
        ctx.call::<_, AddPollQuestionResponse>(MODERATION, "AddPollQuestion", &moderator, &add)
            .await
            .unwrap_err()
            .reason,
        "FORBIDDEN"
    );
    for invalid in [
        AddPollQuestionRequest { text: text("日本語だけ", "").into(), ..Default::default() },
        AddPollQuestionRequest::default(),
        AddPollQuestionRequest { text: text(&"あ".repeat(81), "Too long?").into(), ..Default::default() },
        AddPollQuestionRequest { emoji: "lunch".into(), ..add.clone() },
    ] {
        assert_eq!(
            ctx.call::<_, AddPollQuestionResponse>(MODERATION, "AddPollQuestion", &admin, &invalid)
                .await
                .unwrap_err()
                .reason,
            "POLL_QUESTION_INVALID"
        );
    }
    let added: AddPollQuestionResponse = ctx.call(MODERATION, "AddPollQuestion", &admin, &add).await.unwrap();
    let added = added.question.as_option().unwrap();
    assert!(added.enabled && !added.built_in);
    assert_eq!(added.use_count, 0);
    let question_id = added.question.as_option().unwrap().id.clone();

    let switch = |enabled: bool| SetPollQuestionEnabledRequest {
        question_id: question_id.clone(),
        enabled,
        ..Default::default()
    };
    assert_eq!(
        ctx.call::<_, SetPollQuestionEnabledResponse>(MODERATION, "SetPollQuestionEnabled", &moderator, &switch(false))
            .await
            .unwrap_err()
            .reason,
        "FORBIDDEN"
    );
    let off: SetPollQuestionEnabledResponse =
        ctx.call(MODERATION, "SetPollQuestionEnabled", &admin, &switch(false)).await.unwrap();
    assert!(!off.question.as_option().unwrap().enabled);
    let unknown =
        SetPollQuestionEnabledRequest { question_id: Uuid::now_v7().to_string(), enabled: false, ..Default::default() };
    assert_eq!(
        ctx.call::<_, SetPollQuestionEnabledResponse>(MODERATION, "SetPollQuestionEnabled", &admin, &unknown)
            .await
            .unwrap_err()
            .reason,
        "POLL_QUESTION_NOT_FOUND"
    );
}
