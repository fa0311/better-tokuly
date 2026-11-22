//! Requests that arrive at the same time: votes, hint purchases, round
//! rewards, and enough round requests from one account to occupy every
//! database connection.
mod common;

use std::time::Duration;

use common::*;
use futures::future::{join, join_all};
use tokuly_ask::AskConfig;
use tokuly_proto::ask::*;
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

/// More requests than the test app's pool has connections (five).
const AT_ONCE: usize = 16;
/// How long a burst may take: far more than it needs, and less than the
/// thirty seconds a request waits for a connection that never comes free.
const BURST: Duration = Duration::from_secs(20);

fn reasons<T>(results: &[Result<T, RpcError>]) -> Vec<&str> {
    results.iter().filter_map(|result| result.as_ref().err()).map(|e| e.reason.as_str()).collect()
}

fn successes<T>(results: &[Result<T, RpcError>]) -> usize {
    results.iter().filter(|result| result.is_ok()).count()
}

fn poll_of(round: &Round) -> Poll {
    round.poll.as_option().cloned().expect("a poll")
}

async fn flame_for(ctx: &Ctx, voter: &TestUser, receiver: &TestUser) -> Flame {
    let round = ctx.round(voter).await;
    ctx.vote(voter, &poll_of(&round).id, &receiver.id.to_string()).await;
    ctx.flames(receiver).await.remove(0)
}

#[tokio::test]
async fn a_burst_of_round_requests_from_one_account_does_not_stall_the_service() {
    let ctx = start().await;
    let voter = ctx.user("Voter").await;
    let friends = ctx.friends_of(&voter, 4).await;
    let bystander = ctx.user("Bystander").await;
    let (look, settings) = (GetRoundRequest::default(), GetSettingsRequest::default());

    // Every request waits for the same lock. If the one holding it needed a
    // second connection, it would wait for the others, who wait for it.
    let looks = join_all((0..AT_ONCE).map(|_| ctx.call::<_, GetRoundResponse>(POLLS, "GetRound", &voter, &look)));
    let elsewhere = ctx.call::<_, GetSettingsResponse>(BOX, "GetSettings", &bystander, &settings);
    let (rounds, other) =
        tokio::time::timeout(BURST, join(looks, elsewhere)).await.expect("the burst of GetRound did not finish");
    other.expect("someone else's request went through meanwhile");
    assert_eq!(reasons(&rounds), Vec::<&str>::new());
    let rounds: Vec<Round> = rounds.into_iter().map(|r| r.unwrap().round.as_option().cloned().unwrap()).collect();
    let first = &rounds[0];
    let shown = |round: &Round| {
        let poll = poll_of(round);
        (round.id.clone(), poll.id, poll.choices.iter().map(|choice| choice.id.clone()).collect::<Vec<_>>())
    };
    for round in &rounds {
        assert_eq!(shown(round), shown(first), "one round, one poll, the same four friends");
    }
    let started: i64 = sqlx::query_scalar("SELECT count(*) FROM ask_rounds WHERE user_id = $1")
        .bind(voter.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(started, 1);

    // The same vote sixteen times: one flame.
    let picked = &friends[0];
    let vote = VoteRequest { poll_id: poll_of(first).id, user_id: picked.id.to_string(), ..Default::default() };
    let votes = join_all((0..AT_ONCE).map(|_| ctx.call::<_, VoteResponse>(POLLS, "Vote", &voter, &vote)));
    let votes = tokio::time::timeout(BURST, votes).await.expect("the burst of Vote did not finish");
    assert_eq!(successes(&votes), 1, "{:?}", reasons(&votes));
    assert!(reasons(&votes).iter().all(|reason| *reason == "POLL_NOT_CURRENT"), "{:?}", reasons(&votes));
    ctx.deliver().await;
    assert_eq!(ctx.flames(picked).await.len(), 1);
    assert_eq!(ctx.notifications_for(picked.id, "ask.flame").await.len(), 1);

    // Everything one can do to a poll, all at once: the poll is closed by
    // exactly one of the votes and skips, and the round moves on by one.
    let second = poll_of(&ctx.round(&voter).await);
    let vote = VoteRequest { poll_id: second.id.clone(), user_id: second.choices[0].id.clone(), ..Default::default() };
    let skip = SkipPollRequest { poll_id: second.id.clone(), ..Default::default() };
    let shuffle = ShufflePollRequest { poll_id: second.id.clone(), ..Default::default() };
    let (votes, skips, shuffles, looks) = tokio::time::timeout(BURST, async {
        tokio::join!(
            join_all((0..4).map(|_| ctx.call::<_, VoteResponse>(POLLS, "Vote", &voter, &vote))),
            join_all((0..4).map(|_| ctx.call::<_, SkipPollResponse>(POLLS, "SkipPoll", &voter, &skip))),
            join_all((0..4).map(|_| ctx.call::<_, ShufflePollResponse>(POLLS, "ShufflePoll", &voter, &shuffle))),
            join_all((0..4).map(|_| ctx.call::<_, GetRoundResponse>(POLLS, "GetRound", &voter, &look))),
        )
    })
    .await
    .expect("the mixed burst did not finish");
    assert_eq!(successes(&votes) + successes(&skips), 1, "votes {:?}, skips {:?}", reasons(&votes), reasons(&skips));
    assert!(reasons(&votes).iter().chain(&reasons(&skips)).all(|reason| *reason == "POLL_NOT_CURRENT"));
    assert!(
        reasons(&shuffles).iter().all(|reason| ["POLL_NOT_CURRENT", "NOT_ENOUGH_FRIENDS"].contains(reason)),
        "{:?}",
        reasons(&shuffles)
    );
    assert_eq!(reasons(&looks), Vec::<&str>::new());
    let now = ctx.round(&voter).await;
    assert_eq!((now.answered_count, poll_of(&now).position), (2, 3));
    ctx.deliver().await;
    let sent: i64 = sqlx::query_scalar("SELECT count(*) FROM ask_flames WHERE sender_id = $1")
        .bind(voter.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(sent as usize, 1 + successes(&votes));
}

#[tokio::test]
async fn one_purchase_sent_many_times_at_once_is_charged_once() {
    let ctx = start().await;
    let voter = ctx.user("Voter").await;
    let friends = ctx.friends_of(&voter, 4).await;
    let receiver = &friends[0];
    ctx.give_coins(receiver, 100).await;
    let flame = flame_for(&ctx, &voter, receiver).await;

    let request = BuyHintRequest {
        flame_id: flame.id.clone(),
        idempotency_key: "one-purchase-0001".into(),
        ..Default::default()
    };
    let purchases = join_all((0..8).map(|_| ctx.call::<_, BuyHintResponse>(POLLS, "BuyHint", receiver, &request)));
    let purchases = tokio::time::timeout(BURST, purchases).await.expect("the purchases did not finish");
    assert_eq!(reasons(&purchases), Vec::<&str>::new());
    let purchases: Vec<BuyHintResponse> = purchases.into_iter().map(Result::unwrap).collect();
    assert_eq!(purchases.iter().map(|p| p.coins_spent).sum::<i64>(), 30, "one of them paid, once");
    assert!(purchases.iter().all(|p| p.hint.as_option().unwrap().kind == HintKind::HINT_KIND_INITIAL));
    assert_eq!(ctx.coins(receiver).await, 70);
    let after = ctx.flames(receiver).await.remove(0);
    assert_eq!((after.hints.len(), after.next_hint_price), (1, 60));
    let charges = ctx.app.events_by("wallet.balance.changed", receiver.id).await;
    assert_eq!(charges.iter().filter(|e| e.data["reason"] == serde_json::json!("ask.hint")).count(), 1);
}

#[tokio::test]
async fn two_purchases_at_once_never_pay_twice_for_the_same_hint() {
    let ctx = start_with(AskConfig { round_cooldown_seconds: 0, ..Default::default() }).await;
    let voter = ctx.user("Voter").await;
    let friends = ctx.friends_of(&voter, 4).await;
    let receiver = &friends[0];
    ctx.give_coins(receiver, 500).await;

    for attempt in 0..4 {
        let flame = flame_for(&ctx, &voter, receiver).await;
        let buy =
            |key: String| BuyHintRequest { flame_id: flame.id.clone(), idempotency_key: key, ..Default::default() };
        let (one, other) = (buy(format!("two-at-once-{attempt}-a")), buy(format!("two-at-once-{attempt}-b")));
        let before = ctx.coins(receiver).await;
        let (first, second) = tokio::join!(
            ctx.call::<_, BuyHintResponse>(POLLS, "BuyHint", receiver, &one),
            ctx.call::<_, BuyHintResponse>(POLLS, "BuyHint", receiver, &other),
        );
        let (first, second) = (first.unwrap(), second.unwrap());
        let charged = first.coins_spent + second.coins_spent;
        let hints = ctx.flames(receiver).await.remove(0).hints.len();
        // Either both asked for the first hint (one paid, both have it), or
        // one of them came second and bought the second hint.
        assert!(
            (charged, hints) == (30, 1) || (charged, hints) == (90, 2),
            "charged {charged} coins for {hints} hints"
        );
        assert_eq!(ctx.coins(receiver).await, before - charged);
    }
}

#[tokio::test]
async fn a_reward_that_could_not_be_paid_is_paid_later_and_only_once() {
    let ctx = start().await;
    let (late, recent) = (ctx.user("Late").await, ctx.user("Recent").await);
    for voter in [&late, &recent] {
        ctx.friends_of(voter, 4).await;
    }
    // A finished round whose reward never reached the wallet (the wallet
    // was down): one that ended three hours ago, one ten minutes ago.
    let unpaid = |user: &TestUser, ago: &'static str| {
        let (db, user) = (ctx.app.core.db.clone(), user.id);
        async move {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO ask_rounds (user_id, question_ids, completed_at, reward_coins, created_at)
                 VALUES ($1, '{}', now() - $2::interval, 12, now() - $2::interval - interval '5 minutes') RETURNING id",
            )
            .bind(user)
            .bind(ago)
            .fetch_one(&db)
            .await
            .unwrap();
            id
        }
    };
    let (old_round, recent_round) = (unpaid(&late, "3 hours").await, unpaid(&recent, "10 minutes").await);
    let look = GetRoundRequest::default();

    // After the cooldown: looking starts the next round, and the old one
    // is paid all the same. Eight looks at once pay it once.
    let looks = join_all((0..8).map(|_| ctx.call::<_, GetRoundResponse>(POLLS, "GetRound", &late, &look)));
    let looks = tokio::time::timeout(BURST, looks).await.expect("settling did not finish");
    assert_eq!(reasons(&looks), Vec::<&str>::new());
    for response in &looks {
        let round = response.as_ref().unwrap().round.as_option().unwrap();
        assert_eq!(round.state, RoundState::ROUND_STATE_ACTIVE);
        assert_ne!(round.id, old_round.to_string());
    }
    assert_eq!(ctx.coins(&late).await, 12, "the reward of the old round arrived, once");

    // Within the cooldown: the same, and the cooldown shows the coins.
    let looks = join_all((0..8).map(|_| ctx.call::<_, GetRoundResponse>(POLLS, "GetRound", &recent, &look)));
    let looks = tokio::time::timeout(BURST, looks).await.expect("settling did not finish");
    assert_eq!(reasons(&looks), Vec::<&str>::new());
    let shown = ctx.round(&recent).await;
    assert_eq!(
        (shown.id, shown.state.as_known(), shown.reward_coins),
        (recent_round.to_string(), Some(RoundState::ROUND_STATE_COOLDOWN), 12)
    );
    assert_eq!(ctx.coins(&recent).await, 12);

    for (user, round) in [(&late, old_round), (&recent, recent_round)] {
        let paid: bool = sqlx::query_scalar("SELECT rewarded_at IS NOT NULL FROM ask_rounds WHERE id = $1")
            .bind(round)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
        assert!(paid);
        let grants = ctx.app.events_by("wallet.balance.changed", user.id).await;
        assert_eq!(grants.iter().filter(|e| e.data["reason"] == serde_json::json!("ask.round")).count(), 1);
    }
}

#[tokio::test]
async fn the_last_vote_of_a_round_sent_twice_pays_once() {
    let ctx = start_with(AskConfig { round_polls: 1, round_reward_coins: 12, ..Default::default() }).await;
    let voter = ctx.user("Voter").await;
    let friends = ctx.friends_of(&voter, 4).await;
    let poll = poll_of(&ctx.round(&voter).await);
    let vote = VoteRequest { poll_id: poll.id, user_id: friends[0].id.to_string(), ..Default::default() };
    let votes = join_all((0..6).map(|_| ctx.call::<_, VoteResponse>(POLLS, "Vote", &voter, &vote)));
    let votes = tokio::time::timeout(BURST, votes).await.expect("the votes did not finish");
    assert_eq!(successes(&votes), 1);
    assert!(reasons(&votes).iter().all(|reason| *reason == "ROUND_NOT_ACTIVE"), "{:?}", reasons(&votes));
    assert_eq!(ctx.coins(&voter).await, 12);
    assert_eq!(ctx.app.events_by("ask.round.completed", voter.id).await.len(), 1);
}

#[tokio::test]
async fn answering_or_responding_twice_at_once_happens_once() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user("Sender").await);
    ctx.befriend(&owner, &sender).await;
    ctx.crowd(&owner).await;
    ctx.open_box(&owner, true).await;

    // One question, six answers at once.
    let question = ctx.ask(&sender, &owner, "好きな季節は?").await;
    let answer = AnswerQuestionRequest { question_id: question.id.clone(), text: "夏!".into(), ..Default::default() };
    let answers =
        join_all((0..6).map(|_| ctx.call::<_, AnswerQuestionResponse>(BOX, "AnswerQuestion", &owner, &answer)));
    let answers = tokio::time::timeout(BURST, answers).await.expect("the answers did not finish");
    assert_eq!(successes(&answers), 1);
    assert!(reasons(&answers).iter().all(|reason| *reason == "QUESTION_ALREADY_ANSWERED"), "{:?}", reasons(&answers));
    let events = ctx.deliver().await;
    assert_eq!(events.iter().filter(|e| e.topic == "ask.answer.published").count(), 1);
    assert_eq!(ctx.notifications_for(sender.id, "ask.answered").await.len(), 1);

    // One prompt, six responses at once from one friend.
    let prompt: CreatePromptResponse = ctx
        .call(
            PROMPTS,
            "CreatePrompt",
            &owner,
            &CreatePromptRequest { text: "私の第一印象は?".into(), ..Default::default() },
        )
        .await
        .unwrap();
    let prompt = prompt.prompt.as_option().unwrap().clone();
    ctx.deliver().await;
    let respond =
        RespondToPromptRequest { prompt_id: prompt.id.clone(), text: "やさしそう!".into(), ..Default::default() };
    let responses =
        join_all((0..6).map(|_| ctx.call::<_, RespondToPromptResponse>(PROMPTS, "RespondToPrompt", &sender, &respond)));
    let responses = tokio::time::timeout(BURST, responses).await.expect("the responses did not finish");
    assert_eq!(successes(&responses), 1);
    assert!(reasons(&responses).iter().all(|reason| *reason == "ALREADY_RESPONDED"), "{:?}", reasons(&responses));
    ctx.deliver().await;
    assert_eq!(ctx.notifications_for(owner.id, "ask.prompt_response").await.len(), 1);
}

#[tokio::test]
async fn an_event_handled_many_times_at_once_delivers_once() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user("Sender").await);
    ctx.open_box(&owner, true).await;
    ctx.new_events().await;
    ctx.try_ask(&sender, &owner, "一回だけ届く", false).await.unwrap();
    let stored: Vec<_> = ctx.new_events().await.into_iter().filter(|e| e.topic == "ask.question.stored").collect();
    assert_eq!(stored.len(), 1);

    // The relay redelivers, and several workers pick the same event up.
    let handled = join_all((0..8).map(|_| ctx.ask.handle_event(stored[0].clone())));
    let handled = tokio::time::timeout(BURST, handled).await.expect("the workers did not finish");
    assert!(handled.iter().all(Result::is_ok));
    assert_eq!(ctx.counts(&owner).await.inbox, 1);
    assert_eq!(ctx.notifications_for(owner.id, "ask.question").await.len(), 1);
}
