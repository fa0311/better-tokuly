//! `PollService`: date polls from the first candidate to the plan.
mod common;

use chrono::{DateTime, Duration, Utc};
use common::*;
use tokuly_proto::plans::{plans_event::Event as Push, *};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// The day `days` from now on a Tokyo calendar, as `YYYY-MM-DD`.
fn day(days: i64) -> String {
    (Utc::now() + Duration::days(days)).with_timezone(&chrono_tz::Asia::Tokyo).format("%Y-%m-%d").to_string()
}

fn on(days: i64) -> PollSlotInput {
    PollSlotInput { start: Some(poll_slot_input::Start::Date(day(days))), ..Default::default() }
}

fn at(start: DateTime<Utc>, end: Option<DateTime<Utc>>) -> PollSlotInput {
    PollSlotInput {
        start: Some(poll_slot_input::Start::StartAt(Box::new(start.into()))),
        end_at: end.map(timestamp).into(),
        ..Default::default()
    }
}

fn poll_on(title: &str, days: &[i64], invitees: &[&TestUser]) -> CreatePollRequest {
    CreatePollRequest {
        title: title.to_owned(),
        slots: days.iter().map(|days| on(*days)).collect(),
        invitee_ids: ids(invitees),
        ..Default::default()
    }
}

fn poll_ref(poll: &Poll) -> GetPollRequest {
    GetPollRequest { poll_id: poll.id.clone(), ..Default::default() }
}

fn tally(poll: &Poll, slot: usize) -> (i32, i32, i32) {
    (poll.slots[slot].yes_count, poll.slots[slot].maybe_count, poll.slots[slot].no_count)
}

fn member<'a>(poll: &'a Poll, user: &TestUser) -> &'a PollParticipant {
    poll.participants
        .iter()
        .find(|p| p.user.as_option().is_some_and(|u| u.id == user.id.to_string()))
        .expect("the user is in the poll")
}

async fn update(ctx: &Ctx, user: &TestUser, request: UpdatePollRequest) -> Poll {
    let response: UpdatePollResponse = ctx.call(POLLS, "UpdatePoll", user, &request).await.expect("update poll");
    response.poll.as_option().cloned().expect("a poll")
}

const YES: PollAnswer = PollAnswer::POLL_ANSWER_YES;
const MAYBE: PollAnswer = PollAnswer::POLL_ANSWER_MAYBE;
const NO: PollAnswer = PollAnswer::POLL_ANSWER_NO;

#[tokio::test]
async fn a_poll_finds_the_day_and_becomes_a_plan() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);

    // Candidates are kept in chronological order, however they were given.
    let poll = ctx
        .create_poll(
            mika,
            CreatePollRequest {
                description: "夏祭り行こう".into(),
                ..poll_on("夏祭りいつ行く?", &[7, 5, 6], &[yui, sota])
            },
        )
        .await;
    assert_eq!(
        poll.slots.iter().map(|slot| slot.start.clone()).collect::<Vec<_>>(),
        [5, 6, 7].map(|days| Some(poll_slot::Start::Date(day(days))))
    );
    assert_eq!(
        (poll.status, poll.mine, poll.time_zone.as_str()),
        (PollStatus::POLL_STATUS_OPEN.into(), true, "Asia/Tokyo")
    );
    assert_eq!(poll.short_id.len(), 11);
    // The creator takes part like everyone else; nobody has answered, so nothing is suggested yet.
    assert_eq!((poll.participant_count, poll.answered_count, poll.best_slot_id.as_str()), (3, 0, ""));
    assert_eq!(poll.creator.as_option().unwrap().id, mika.id.to_string());
    assert!(member(&poll, yui).invited && !member(&poll, mika).invited);

    for invited in [yui, sota] {
        let told = ctx.notifications_for(invited.id, "plans.poll_invited").await;
        assert_eq!(told.len(), 1);
        assert_eq!(told[0].data["title"]["ja"], serde_json::json!("Mikaさんから日程調整のお願い!"));
        assert_eq!(told[0].data["path"], serde_json::json!(format!("/plans/polls/{}", poll.short_id)));
    }
    let created = ctx.app.events_by("plans.poll.created", mika.id).await;
    assert_eq!((created.len(), created[0].data["slot_count"].as_i64()), (1, Some(3)));

    // ○ △ × from everyone; the tallies are live.
    let mut mikas_phone = ctx.app.listen_user(mika.id).await;
    let request = AnswerPollRequest {
        poll_id: poll.id.clone(),
        answers: poll
            .slots
            .iter()
            .zip([YES, MAYBE, NO])
            .map(|(slot, answer)| SlotAnswer { slot_id: slot.id.clone(), answer: answer.into(), ..Default::default() })
            .collect(),
        comment: "5日が一番いい!".into(),
        ..Default::default()
    };
    let answered: AnswerPollResponse = ctx.call(POLLS, "AnswerPoll", yui, &request).await.unwrap();
    let seen = answered.poll.as_option().unwrap();
    assert_eq!(
        seen.slots.iter().map(|slot| slot.my_answer.to_i32()).collect::<Vec<_>>(),
        [YES as i32, MAYBE as i32, NO as i32]
    );
    assert!(seen.answered_by_me && !seen.mine);
    assert_eq!((member(seen, yui).comment.as_str(), member(seen, yui).answers.len()), ("5日が一番いい!", 3));
    match next_push(&mut mikas_phone).await {
        Some(Push::PollChanged(change)) => {
            assert_eq!(
                (change.poll_id.clone(), change.answered_count, change.status),
                (poll.id.clone(), 1, PollStatus::POLL_STATUS_OPEN.into())
            );
            assert_eq!(
                change
                    .tallies
                    .iter()
                    .map(|tally| (tally.yes_count, tally.maybe_count, tally.no_count))
                    .collect::<Vec<_>>(),
                [(1, 0, 0), (0, 1, 0), (0, 0, 1)]
            );
            assert_eq!(change.best_slot_id, poll.slots[0].id);
        }
        other => panic!("expected the tallies to be pushed, got {other:?}"),
    }
    ctx.answer_poll(sota, &poll, &[NO, YES, YES]).await;
    let tallied = ctx.answer_poll(mika, &poll, &[YES, YES, NO]).await;
    assert_eq!((tally(&tallied, 0), tally(&tallied, 1), tally(&tallied, 2)), ((2, 0, 1), (2, 1, 0), (1, 0, 2)));
    assert_eq!(tallied.answered_count, 3);
    // Two days have the most ○; the one nobody refused is suggested.
    assert_eq!(tallied.best_slot_id, poll.slots[1].id);

    // The creator hears about first answers (not their own), once per person.
    let heard = ctx.notifications_for(mika.id, "plans.poll_answer").await;
    assert_eq!(heard.len(), 2);
    assert!(
        heard
            .iter()
            .all(|notice| notice.data["collapse_key"] == serde_json::json!(format!("plans.poll_answer:{}", poll.id)))
    );
    // Answers can be changed until the poll closes, quietly.
    let changed = ctx.answer_poll(yui, &poll, &[YES, YES, NO]).await;
    assert_eq!(tally(&changed, 1), (3, 0, 0));
    assert_eq!(ctx.notifications_for(mika.id, "plans.poll_answer").await.len(), 2);

    // Finalizing is the creator's; without a slot the suggestion is taken.
    let finalize = FinalizePollRequest {
        poll_id: poll.id.clone(),
        emoji: "🏮".into(),
        place: PlanPlace { name: "神社".into(), ..Default::default() }.into(),
        ..Default::default()
    };
    assert_eq!(ctx.refusal(POLLS, "FinalizePoll", yui, &finalize).await, "NOT_THE_CREATOR");
    let done: FinalizePollResponse = ctx.call(POLLS, "FinalizePoll", mika, &finalize).await.unwrap();
    let (closed, plan) = (done.poll.as_option().unwrap(), done.plan.as_option().unwrap());
    assert_eq!(
        (closed.status, closed.final_slot_id.clone(), closed.plan_id.clone()),
        (PollStatus::POLL_STATUS_FINALIZED.into(), poll.slots[1].id.clone(), plan.id.clone())
    );

    // The plan: the poll's title and description, the chosen day, the creator as host...
    assert_eq!(
        (plan.title.as_str(), plan.emoji.as_str(), plan.description.as_str()),
        ("夏祭りいつ行く?", "🏮", "夏祭り行こう")
    );
    assert_eq!(plan.time.as_option().unwrap().start, Some(plan_time::Start::StartDate(day(6))));
    assert_eq!(
        (plan.host_user_id.clone(), plan.poll_id.clone(), plan.place.as_option().unwrap().name.as_str()),
        (mika.id.to_string(), poll.id.clone(), "神社")
    );
    // ...and everyone who could make that day invited.
    assert_eq!(plan.participants.len(), 3);
    assert_eq!(
        (participant(plan, yui).rsvp, participant(plan, sota).rsvp),
        (Rsvp::RSVP_INVITED.into(), Rsvp::RSVP_INVITED.into())
    );
    for invited in [yui, sota] {
        assert_eq!(ctx.notifications_for(invited.id, "plans.invited").await.len(), 1);
        assert_eq!(ctx.get_plan(invited, &plan.id).await.poll_id, poll.id);
    }
    let finalized = ctx.app.events_by("plans.poll.finalized", mika.id).await;
    assert_eq!((finalized.len(), finalized[0].data["answered_count"].as_i64()), (1, Some(3)));
    assert_eq!(ctx.app.events_by("plans.plan.created", mika.id).await.len(), 1);

    // A finalized poll is over.
    assert_eq!(ctx.refusal(POLLS, "AnswerPoll", yui, &request).await, "POLL_NOT_OPEN");
    assert_eq!(ctx.refusal(POLLS, "FinalizePoll", mika, &finalize).await, "POLL_FINALIZED");
    assert_eq!(
        ctx.refusal(POLLS, "ReopenPoll", mika, &ReopenPollRequest { poll_id: poll.id.clone(), ..Default::default() })
            .await,
        "POLL_NOT_CLOSED"
    );
}

#[tokio::test]
async fn only_those_who_could_make_it_are_invited() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let (mika, yui, sota, hana) = (&users[0], &users[1], &users[2], &users[3]);
    let start = Utc::now() + Duration::days(4);
    let end = start + Duration::hours(2);
    let request = CreatePollRequest {
        slots: vec![at(start, Some(end)), at(start + Duration::days(1), None)],
        ..poll_on("映画", &[], &[yui, sota, hana])
    };
    let poll = ctx.create_poll(mika, request).await;
    assert!(matches!(&poll.slots[0].start, Some(poll_slot::Start::StartAt(at)) if at.seconds == start.timestamp()));
    assert_eq!(time_of(&poll.slots[0].end_at).timestamp(), end.timestamp());

    ctx.answer_poll(yui, &poll, &[MAYBE, NO]).await;
    ctx.answer_poll(sota, &poll, &[NO, YES]).await;
    // Hana does not answer at all.

    // Nobody has to follow the suggestion: the creator picks the first slot.
    let finalize = FinalizePollRequest {
        poll_id: poll.id.clone(),
        slot_id: poll.slots[0].id.clone(),
        description: "ポップコーンは割り勘で".into(),
        ..Default::default()
    };
    let done: FinalizePollResponse = ctx.call(POLLS, "FinalizePoll", mika, &finalize).await.unwrap();
    let plan = done.plan.as_option().unwrap();
    // A slot with a time becomes a plan with that time and end.
    let time = plan.time.as_option().unwrap();
    assert!(matches!(&time.start, Some(plan_time::Start::StartAt(at)) if at.seconds == start.timestamp()));
    assert!(matches!(&time.end, Some(plan_time::End::EndAt(at)) if at.seconds == end.timestamp()));
    assert_eq!(plan.description, "ポップコーンは割り勘で");
    // △ counts as "could make it"; × and no answer do not.
    let invited: Vec<String> = plan
        .participants
        .iter()
        .filter(|p| p.rsvp == Rsvp::RSVP_INVITED)
        .map(|p| p.user.as_option().unwrap().id.clone())
        .collect();
    assert_eq!(invited, ids(&[yui]));
    assert!(ctx.notifications_for(sota.id, "plans.invited").await.is_empty());
    assert!(ctx.notifications_for(hana.id, "plans.invited").await.is_empty());
}

#[tokio::test]
async fn friends_with_the_link_can_join_an_open_poll() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let stranger = ctx.app.user("Stranger").await;
    let by_link = |poll: &Poll| GetPollRequest { poll_id: poll.short_id.clone(), ..Default::default() };

    let open = ctx
        .create_poll(mika, CreatePollRequest { open_to_friends: true, ..poll_on("だれでも歓迎", &[3, 4], &[]) })
        .await;
    // The link opens the poll for a friend of the creator, without joining it yet.
    let seen = ctx.get_poll(yui, &open.short_id).await;
    assert_eq!((seen.id.clone(), seen.participant_count, seen.answered_by_me), (open.id.clone(), 1, false));
    // Not for anyone else, even with the link.
    assert_eq!(ctx.refusal(POLLS, "GetPoll", &stranger, &by_link(&open)).await, "POLL_NOT_FOUND");

    // Answering is what joins.
    let request = AnswerPollRequest {
        poll_id: open.short_id.clone(),
        answers: vec![SlotAnswer { slot_id: open.slots[0].id.clone(), answer: YES.into(), ..Default::default() }],
        ..Default::default()
    };
    let joined: AnswerPollResponse = ctx.call(POLLS, "AnswerPoll", yui, &request).await.unwrap();
    let joined = joined.poll.as_option().unwrap();
    assert_eq!((joined.participant_count, joined.answered_count), (2, 1));
    assert!(!member(joined, yui).invited);
    // Slots she left out count as not answered.
    assert_eq!((tally(joined, 0), tally(joined, 1)), ((1, 0, 0), (0, 0, 0)));
    let request = AnswerPollRequest { answers: Vec::new(), ..request };
    assert_eq!(ctx.refusal(POLLS, "AnswerPoll", &stranger, &request).await, "POLL_NOT_FOUND");

    // A poll that is not open to friends is for the invited only.
    let private = ctx.create_poll(mika, poll_on("内緒", &[3], &[yui])).await;
    assert_eq!(ctx.refusal(POLLS, "GetPoll", sota, &by_link(&private)).await, "POLL_NOT_FOUND");
    assert_eq!(ctx.refusal(POLLS, "GetPoll", sota, &poll_ref(&private)).await, "POLL_NOT_FOUND");
    assert_eq!(ctx.get_poll(yui, &private.id).await.id, private.id);

    // Closing the link keeps the people who already joined.
    update(
        &ctx,
        mika,
        UpdatePollRequest { poll_id: open.id.clone(), open_to_friends: Some(false), ..Default::default() },
    )
    .await;
    assert_eq!(ctx.refusal(POLLS, "GetPoll", sota, &by_link(&open)).await, "POLL_NOT_FOUND");
    assert_eq!(ctx.get_poll(yui, &open.short_id).await.id, open.id);

    // Taking every answer back is "not answered" again.
    let withdrawn: AnswerPollResponse = ctx.call(POLLS, "AnswerPoll", yui, &request).await.unwrap();
    let withdrawn = withdrawn.poll.as_option().unwrap();
    assert_eq!((withdrawn.answered_count, withdrawn.answered_by_me, withdrawn.participant_count), (0, false, 2));
}

#[tokio::test]
async fn the_creator_runs_the_poll() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota", "Hana"]).await;
    let (mika, yui, sota, hana) = (&users[0], &users[1], &users[2], &users[3]);
    let stranger = ctx.app.user("Stranger").await;
    let poll = ctx.create_poll(mika, poll_on("クリスマス会", &[10, 11], &[yui, sota])).await;
    let change = || UpdatePollRequest { poll_id: poll.id.clone(), ..Default::default() };
    ctx.answer_poll(yui, &poll, &[YES, NO]).await;

    // Texts, slots and the deadline are the creator's to change.
    assert_eq!(
        ctx.refusal(POLLS, "UpdatePoll", yui, &UpdatePollRequest { title: Some("わたしの会".into()), ..change() })
            .await,
        "NOT_THE_CREATOR"
    );
    assert_eq!(ctx.refusal(POLLS, "UpdatePoll", &stranger, &change()).await, "POLL_NOT_FOUND");
    let deadline = Utc::now() + Duration::days(5);
    let updated = update(
        &ctx,
        mika,
        UpdatePollRequest {
            title: Some("クリスマス会🎄".into()),
            description: Some("プレゼント交換あり".into()),
            deadline: timestamp(deadline).into(),
            add_slots: vec![on(12)],
            remove_slot_ids: vec![poll.slots[0].id.clone()],
            ..change()
        },
    )
    .await;
    assert_eq!((updated.title.as_str(), updated.description.as_str()), ("クリスマス会🎄", "プレゼント交換あり"));
    assert_eq!(time_of(&updated.deadline).timestamp(), deadline.timestamp());
    assert_eq!(
        updated.slots.iter().map(|slot| slot.start.clone()).collect::<Vec<_>>(),
        [11, 12].map(|days| Some(poll_slot::Start::Date(day(days))))
    );
    // Yui's ○ went with the removed day; her × stays.
    assert_eq!((tally(&updated, 0), tally(&updated, 1)), ((0, 0, 1), (0, 0, 0)));
    let cleared = update(&ctx, mika, UpdatePollRequest { clear_deadline: true, ..change() }).await;
    assert!(cleared.deadline.is_unset());

    let cases = [
        (UpdatePollRequest { title: Some(" ".into()), ..change() }, "TITLE_REQUIRED"),
        (UpdatePollRequest { remove_slot_ids: vec![Uuid::now_v7().to_string()], ..change() }, "SLOT_NOT_FOUND"),
        (UpdatePollRequest { add_slots: vec![on(11)], ..change() }, "SLOT_DUPLICATE"),
        (UpdatePollRequest { add_slots: (20..50).map(on).collect(), ..change() }, "TOO_MANY_SLOTS"),
        (
            UpdatePollRequest {
                remove_slot_ids: updated.slots.iter().map(|slot| slot.id.clone()).collect(),
                ..change()
            },
            "SLOTS_REQUIRED",
        ),
        (
            UpdatePollRequest { deadline: timestamp(Utc::now() - Duration::hours(1)).into(), ..change() },
            "DEADLINE_INVALID",
        ),
    ];
    for (request, reason) in cases {
        assert_eq!(ctx.refusal(POLLS, "UpdatePoll", mika, &request).await, reason);
    }

    // More invitations.
    let invite = |invitees: &[&TestUser]| InviteToPollRequest {
        poll_id: poll.id.clone(),
        user_ids: ids(invitees),
        ..Default::default()
    };
    assert_eq!(ctx.refusal(POLLS, "InviteToPoll", yui, &invite(&[hana])).await, "NOT_THE_CREATOR");
    assert_eq!(ctx.refusal(POLLS, "InviteToPoll", mika, &invite(&[&stranger])).await, "NOT_FRIENDS");
    let invited: InviteToPollResponse = ctx.call(POLLS, "InviteToPoll", mika, &invite(&[hana, yui])).await.unwrap();
    assert_eq!((invited.invited_count, invited.poll.as_option().unwrap().participant_count), (1, 4));
    assert_eq!(ctx.notifications_for(hana.id, "plans.poll_invited").await.len(), 1);
    assert_eq!(ctx.notifications_for(yui.id, "plans.poll_invited").await.len(), 1);

    // A nudge for those who have not answered: Sota and Hana, not Yui.
    let remind = RemindPollParticipantsRequest { poll_id: poll.id.clone(), ..Default::default() };
    assert_eq!(ctx.refusal(POLLS, "RemindPollParticipants", yui, &remind).await, "NOT_THE_CREATOR");
    let reminded: RemindPollParticipantsResponse =
        ctx.call(POLLS, "RemindPollParticipants", mika, &remind).await.unwrap();
    assert_eq!(reminded.reminded_count, 2);
    assert_eq!(ctx.notifications_for(sota.id, "plans.poll_reminder").await.len(), 1);
    assert!(ctx.notifications_for(yui.id, "plans.poll_reminder").await.is_empty());
    // Not again right away.
    assert_eq!(ctx.refusal(POLLS, "RemindPollParticipants", mika, &remind).await, "RATE_LIMITED");

    // Closing stops the answers; reopening takes them again.
    let close = ClosePollRequest { poll_id: poll.id.clone(), ..Default::default() };
    let reopen = ReopenPollRequest { poll_id: poll.id.clone(), ..Default::default() };
    assert_eq!(ctx.refusal(POLLS, "ClosePoll", yui, &close).await, "NOT_THE_CREATOR");
    assert_eq!(ctx.refusal(POLLS, "ReopenPoll", mika, &reopen).await, "POLL_NOT_CLOSED");
    let closed: ClosePollResponse = ctx.call(POLLS, "ClosePoll", mika, &close).await.unwrap();
    assert_eq!(closed.poll.as_option().unwrap().status, PollStatus::POLL_STATUS_CLOSED);
    let answer = AnswerPollRequest { poll_id: poll.id.clone(), ..Default::default() };
    for request_refused in [
        ctx.refusal(POLLS, "AnswerPoll", sota, &answer).await,
        ctx.refusal(POLLS, "ClosePoll", mika, &close).await,
        ctx.refusal(POLLS, "UpdatePoll", mika, &UpdatePollRequest { title: Some("x".into()), ..change() }).await,
        ctx.refusal(POLLS, "InviteToPoll", mika, &invite(&[hana])).await,
        ctx.refusal(POLLS, "RemindPollParticipants", mika, &remind).await,
    ] {
        assert_eq!(request_refused, "POLL_NOT_OPEN");
    }
    assert_eq!(ctx.refusal(POLLS, "ReopenPoll", yui, &reopen).await, "NOT_THE_CREATOR");
    let reopened: ReopenPollResponse = ctx.call(POLLS, "ReopenPoll", mika, &reopen).await.unwrap();
    assert_eq!(reopened.poll.as_option().unwrap().status, PollStatus::POLL_STATUS_OPEN);
    ctx.answer_poll(sota, &updated, &[YES, YES]).await;

    // The list: open polls first, then newest first; without the participants.
    let second = ctx.create_poll(mika, poll_on("初詣", &[40], &[yui])).await;
    let _: ClosePollResponse = ctx
        .call(POLLS, "ClosePoll", mika, &ClosePollRequest { poll_id: second.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let third = ctx.create_poll(mika, poll_on("スキー", &[50], &[])).await;
    let mine: ListPollsResponse = ctx.call(POLLS, "ListPolls", mika, &ListPollsRequest::default()).await.unwrap();
    assert_eq!(
        mine.polls.iter().map(|poll| poll.id.clone()).collect::<Vec<_>>(),
        [third.id.clone(), poll.id.clone(), second.id.clone()]
    );
    assert!(mine.polls[1].participants.is_empty());
    assert_eq!((mine.polls[1].participant_count, mine.polls[1].answered_count), (4, 2));
    // In pages of two: the open ones, then the closed one.
    let page = |cursor: &str| ListPollsRequest {
        page: tokuly_proto::common::PageRequest { limit: 2, cursor: cursor.to_owned(), ..Default::default() }.into(),
        ..Default::default()
    };
    let first: ListPollsResponse = ctx.call(POLLS, "ListPolls", mika, &page("")).await.unwrap();
    let info = first.page.as_option().unwrap();
    assert!(info.has_more);
    assert_eq!(first.polls.iter().map(|poll| poll.id.clone()).collect::<Vec<_>>(), [third.id.clone(), poll.id.clone()]);
    let rest: ListPollsResponse = ctx.call(POLLS, "ListPolls", mika, &page(&info.next_cursor)).await.unwrap();
    assert!(!rest.page.as_option().unwrap().has_more);
    assert_eq!(rest.polls.iter().map(|poll| poll.id.clone()).collect::<Vec<_>>(), [second.id.clone()]);
    let yuis: ListPollsResponse = ctx.call(POLLS, "ListPolls", yui, &ListPollsRequest::default()).await.unwrap();
    assert_eq!(yuis.polls.iter().map(|poll| poll.id.clone()).collect::<Vec<_>>(), [poll.id.clone(), second.id.clone()]);
    assert!(yuis.polls[0].answered_by_me && !yuis.polls[0].mine);

    // Deleting is the creator's, and everyone in the poll sees it go.
    let delete = DeletePollRequest { poll_id: poll.id.clone(), ..Default::default() };
    assert_eq!(ctx.refusal(POLLS, "DeletePoll", yui, &delete).await, "NOT_THE_CREATOR");
    let mut yuis_phone = ctx.app.listen_user(yui.id).await;
    let _: DeletePollResponse = ctx.call(POLLS, "DeletePoll", mika, &delete).await.unwrap();
    assert!(
        matches!(next_push(&mut yuis_phone).await, Some(Push::PollChanged(change)) if change.deleted && change.poll_id == poll.id)
    );
    assert_eq!(ctx.refusal(POLLS, "GetPoll", yui, &poll_ref(&poll)).await, "POLL_NOT_FOUND");
    assert_eq!(ctx.refusal(POLLS, "DeletePoll", mika, &delete).await, "POLL_NOT_FOUND");
}

#[tokio::test]
async fn invalid_polls_and_answers_are_rejected() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let stranger = ctx.app.user("Stranger").await;
    let base = || poll_on("いつにする?", &[3, 4], &[yui]);
    let soon = Utc::now() + Duration::days(3);

    let cases = [
        (CreatePollRequest { title: String::new(), ..base() }, "TITLE_REQUIRED"),
        (CreatePollRequest { title: "あ".repeat(81), ..base() }, "TITLE_TOO_LONG"),
        (CreatePollRequest { description: "あ".repeat(1001), ..base() }, "DESCRIPTION_TOO_LONG"),
        (CreatePollRequest { slots: Vec::new(), ..base() }, "SLOTS_REQUIRED"),
        (CreatePollRequest { slots: (1..=31).map(on).collect(), ..base() }, "TOO_MANY_SLOTS"),
        (CreatePollRequest { slots: vec![PollSlotInput::default()], ..base() }, "SLOT_INVALID"),
        (CreatePollRequest { slots: vec![on(-30)], ..base() }, "SLOT_INVALID"),
        (CreatePollRequest { slots: vec![at(soon, Some(soon - Duration::hours(1)))], ..base() }, "SLOT_INVALID"),
        (CreatePollRequest { slots: vec![on(3), on(3)], ..base() }, "SLOT_DUPLICATE"),
        (
            CreatePollRequest { deadline: timestamp(Utc::now() - Duration::minutes(1)).into(), ..base() },
            "DEADLINE_INVALID",
        ),
        (CreatePollRequest { time_zone: "Mars/Olympus".into(), ..base() }, "TIME_ZONE_INVALID"),
        (CreatePollRequest { invitee_ids: ids(&[&stranger]), ..base() }, "NOT_FRIENDS"),
    ];
    for (request, reason) in cases {
        assert_eq!(ctx.refusal(POLLS, "CreatePoll", mika, &request).await, reason);
    }
    let none: ListPollsResponse = ctx.call(POLLS, "ListPolls", mika, &ListPollsRequest::default()).await.unwrap();
    assert!(none.polls.is_empty());

    let poll = ctx.create_poll(mika, base()).await;
    let answer = |slot_id: &str, answer: PollAnswer| SlotAnswer {
        slot_id: slot_id.to_owned(),
        answer: answer.into(),
        ..Default::default()
    };
    let with = |answers: Vec<SlotAnswer>| AnswerPollRequest { poll_id: poll.id.clone(), answers, ..Default::default() };
    let first = poll.slots[0].id.as_str();
    assert_eq!(
        ctx.refusal(POLLS, "AnswerPoll", yui, &with(vec![answer(&Uuid::now_v7().to_string(), YES)])).await,
        "SLOT_NOT_FOUND"
    );
    assert_eq!(
        ctx.refusal(POLLS, "AnswerPoll", yui, &with(vec![answer(first, PollAnswer::POLL_ANSWER_UNSPECIFIED)])).await,
        "ANSWER_INVALID"
    );
    assert_eq!(
        ctx.refusal(POLLS, "AnswerPoll", yui, &with(vec![answer(first, YES), answer(first, NO)])).await,
        "SLOT_DUPLICATE"
    );
    assert_eq!(
        ctx.refusal(POLLS, "AnswerPoll", yui, &AnswerPollRequest { comment: "あ".repeat(141), ..with(Vec::new()) })
            .await,
        "COMMENT_TOO_LONG"
    );
    assert_eq!(
        ctx.refusal(POLLS, "AnswerPoll", yui, &AnswerPollRequest { poll_id: "???".into(), ..Default::default() }).await,
        "INVALID_ID"
    );

    // Nothing can be picked while nobody has answered, unless the creator names a slot.
    let finalize = FinalizePollRequest { poll_id: poll.id.clone(), ..Default::default() };
    assert_eq!(ctx.refusal(POLLS, "FinalizePoll", mika, &finalize).await, "SLOT_REQUIRED");
    assert_eq!(
        ctx.refusal(
            POLLS,
            "FinalizePoll",
            mika,
            &FinalizePollRequest { slot_id: Uuid::now_v7().to_string(), ..finalize.clone() }
        )
        .await,
        "SLOT_NOT_FOUND"
    );
    assert_eq!(
        ctx.refusal(POLLS, "FinalizePoll", mika, &FinalizePollRequest { emoji: "matsuri".into(), ..finalize.clone() })
            .await,
        "EMOJI_INVALID"
    );

    // Every RPC of the service needs a signed-in caller.
    for method in [
        "CreatePoll",
        "GetPoll",
        "UpdatePoll",
        "ListPolls",
        "InviteToPoll",
        "AnswerPoll",
        "ClosePoll",
        "ReopenPoll",
        "FinalizePoll",
        "DeletePoll",
        "RemindPollParticipants",
    ] {
        let error = ctx
            .app
            .call::<_, serde_json::Value>(&format!("{POLLS}/{method}"), None, &serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!((error.code.as_str(), error.reason.as_str()), ("unauthenticated", "SIGN_IN_REQUIRED"), "{method}");
    }
}
