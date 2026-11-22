//! Requests that arrive together.
//!
//! Wherever the domain relies on a lock, a unique key or an order, the
//! requests here really run at the same time (each on its own database
//! connection), and the outcome is checked against the rows, not only
//! against the answers. The first two tests are about the pool itself: the
//! test database has five connections, so a handler that asked for a second
//! one while holding a transaction would stall here — and with it everyone
//! else's requests.
mod common;

use std::{collections::HashSet, time::Duration};

use common::*;
use futures::future::{FutureExt, LocalBoxFuture, join_all};
use serde::Serialize;
use tokuly_community::CommunityConfig;
use tokuly_proto::{common::SubjectType, community::*};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

const SVC: &str = "CommunityService";
const BOARD: &str = "CommunityBoardService";
const MODERATION: &str = "CommunityModerationService";
const EVENTS: &str = "CommunityEventService";

/// Longer than any burst here takes, shorter than the pool's acquire
/// timeout (10 s): a burst that waits for connections runs into it.
const PROMPT: Duration = Duration::from_secs(8);

fn id_of(value: &str) -> Uuid {
    value.parse().expect("a UUID")
}

/// `count` new users who joined the (public) community.
async fn joined(ctx: &Ctx, community: &Community, count: usize) -> Vec<TestUser> {
    let mut users = Vec::with_capacity(count);
    for i in 0..count {
        let user = ctx.app.user(&format!("User{i}")).await;
        ctx.join(&user, community).await;
        users.push(user);
    }
    users
}

/// One request, to be run together with others.
fn fire<'a, Req: Serialize + 'a>(
    ctx: &'a Ctx,
    service: &'static str,
    method: &'static str,
    user: &'a TestUser,
    request: Req,
) -> LocalBoxFuture<'a, Result<(), RpcError>> {
    async move { ctx.call::<_, serde_json::Value>(service, method, Some(user), &request).await.map(|_| ()) }
        .boxed_local()
}

/// Panics unless every request succeeded.
fn all_succeeded(results: &[Result<(), RpcError>]) {
    for result in results {
        if let Err(error) = result {
            panic!("unexpected failure: {error}");
        }
    }
}

/// How many of the results failed with `reason`; panics on any other failure.
fn refused(results: &[Result<(), RpcError>], reason: &str) -> usize {
    results
        .iter()
        .filter(|result| match result {
            Ok(()) => false,
            Err(error) if error.reason == reason => true,
            Err(error) => panic!("unexpected failure: {error}"),
        })
        .count()
}

#[tokio::test]
async fn a_burst_from_one_account_does_not_tie_up_the_pool() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.app.user("Author").await;
    ctx.join(&author, &community).await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&author, &general, "連打されるスレ", "x").await;
    let reply = ctx.reply(&author, &thread, "連打される返信").await;
    let restricted = ctx.restricted(&owner).await;

    // Eight times the pool's size, all from one account and most of them
    // wanting the same two rows: edits of one thread and one reply, new
    // threads, new replies, and a request to join.
    let mut burst = Vec::new();
    for i in 0..10 {
        let edit = UpdateThreadRequest {
            thread_id: thread.id.clone(),
            body: Some(format!("書き直し{i}")),
            ..Default::default()
        };
        burst.push(fire(&ctx, BOARD, "UpdateThread", &author, edit));
        let edit =
            UpdateReplyRequest {
                reply_id: reply.id.clone(), body: format!("返信の書き直し{i}"), ..Default::default()
            };
        burst.push(fire(&ctx, BOARD, "UpdateReply", &author, edit));
        let new = CreateThreadRequest {
            channel_id: general.id.clone(),
            title: format!("新スレ{i}"),
            body: "x".into(),
            ..Default::default()
        };
        burst.push(fire(&ctx, BOARD, "CreateThread", &author, new));
        let new =
            CreateReplyRequest {
                thread_id: thread.id.clone(), body: format!("新しい返信{i}"), ..Default::default()
            };
        burst.push(fire(&ctx, BOARD, "CreateReply", &author, new));
    }
    let ask = JoinCommunityRequest { community_id: restricted.id.clone(), ..Default::default() };
    burst.push(fire(&ctx, SVC, "JoinCommunity", &author, ask));
    // Somebody else, meanwhile, just reads.
    let reader = async {
        let mut reads = 0;
        for _ in 0..10 {
            ctx.get(None, &community.id).await.expect("reading works during the burst");
            reads += 1;
        }
        reads
    };

    let (results, reads) = tokio::time::timeout(PROMPT, async { tokio::join!(join_all(burst), reader) })
        .await
        .expect("the burst and the reads finish well inside the pool's acquire timeout");
    all_succeeded(&results);
    assert_eq!(reads, 10);
    // Everything was really written.
    let seen = ctx.get_thread(Some(&author), &thread.id).await.unwrap();
    assert_eq!(seen.reply_count, 11);
    assert!(seen.body.starts_with("書き直し"));
    assert_eq!(ctx.get(None, &community.id).await.unwrap().thread_count, 11);
}

#[tokio::test]
async fn the_workers_hold_one_connection_at_a_time() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let reporters = joined(&ctx, &community, 8).await;
    let general = ctx.general(&owner, &community).await;
    let author = &reporters[0];
    let thread = ctx.thread(author, &general, "通報が重なるスレ", "x").await;
    let news = ctx.thread(&owner, &general, "お知らせ", "みんな読んでね").await;
    let announce = SetAnnouncementRequest { thread_id: news.id.clone(), announcement: true, ..Default::default() };
    let _: SetAnnouncementResponse = ctx.call(BOARD, "SetAnnouncement", Some(&owner), &announce).await.unwrap();
    let event = ctx.event(&owner, &community).await;
    ctx.rsvp(&reporters[1], &event, RsvpAnswer::RSVP_ANSWER_GOING).await;
    ctx.move_event(&event.id, chrono::Duration::minutes(40), chrono::Duration::hours(2)).await;

    // The report consumer, the announcement job and the reminder timer,
    // each many times at once — more than the pool has connections.
    let kind = SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD;
    let reports = join_all(reporters[1..].iter().map(|reporter| ctx.report(reporter, kind, &thread.id)));
    let announcements = join_all((0..8).map(|_| ctx.community.run_announcement(id_of(&news.id))));
    let reminders = join_all((0..8).map(|_| ctx.community.send_due_reminders()));
    let ((), announced, reminded) = tokio::time::timeout(PROMPT, async {
        let (_, announced, reminded) = tokio::join!(reports, announcements, reminders);
        ((), announced, reminded)
    })
    .await
    .expect("the workers finish well inside the pool's acquire timeout");
    for result in announced {
        result.expect("an announcement run");
    }
    for result in reminded {
        result.expect("a reminder run");
    }

    // And each did its work exactly once.
    assert_eq!(ctx.get_thread(Some(&reporters[2]), &thread.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    for member in &reporters {
        assert_eq!(ctx.notifications(member.id, "community.announcement").await.len(), 1, "one announcement each");
    }
    assert_eq!(ctx.notifications(reporters[1].id, "community.event_reminder").await.len(), 1);
}

#[tokio::test]
async fn the_last_seats_go_to_as_many_as_fit_and_the_waitlist_moves_up_in_order() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    // Three places; the organiser holds one.
    let event = ctx.try_event(&owner, &CreateEventRequest { capacity: 3, ..ctx.new_event(&community) }).await.unwrap();
    let users = joined(&ctx, &community, 8).await;
    let seats = |status: i32| {
        let (ctx, event_id) = (&ctx, id_of(&event.id));
        async move {
            let ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM community_event_attendees WHERE event_id = $1 AND status = $2 ORDER BY queued_at, user_id",
            )
            .bind(event_id)
            .bind(status)
            .fetch_all(&ctx.app.core.db)
            .await
            .unwrap();
            ids
        }
    };

    // Eight want the two places that are left, at the same moment.
    let answers = join_all(users.iter().map(|user| ctx.try_rsvp(user, &event.id, RsvpAnswer::RSVP_ANSWER_GOING))).await;
    let statuses: Vec<_> = answers.into_iter().map(|answer| answer.expect("an answer").viewer.status).collect();
    assert_eq!(statuses.iter().filter(|status| **status == AttendanceStatus::ATTENDANCE_STATUS_GOING).count(), 2);
    assert_eq!(statuses.iter().filter(|status| **status == AttendanceStatus::ATTENDANCE_STATUS_WAITLISTED).count(), 6);
    // The counters say what the rows say.
    let seen = ctx.get_event(Some(&owner), &event.id).await.unwrap();
    assert_eq!((seen.going_count, seen.waitlist_count), (3, 6));
    let queue = seats(2).await;
    assert_eq!((seats(1).await.len(), queue.len()), (3, 6));

    // The two who got in give their places up together: the two who have
    // waited longest move up, nobody else, nobody twice.
    let holders: Vec<&TestUser> = users
        .iter()
        .zip(&statuses)
        .filter(|(_, status)| **status == AttendanceStatus::ATTENDANCE_STATUS_GOING)
        .map(|(user, _)| user)
        .collect();
    let given_up =
        join_all(holders.iter().map(|user| ctx.try_rsvp(user, &event.id, RsvpAnswer::RSVP_ANSWER_NOT_GOING))).await;
    assert!(given_up.iter().all(Result::is_ok));
    let seen = ctx.get_event(Some(&owner), &event.id).await.unwrap();
    assert_eq!((seen.going_count, seen.waitlist_count), (3, 4));
    let going: HashSet<Uuid> = seats(1).await.into_iter().collect();
    assert_eq!(going, HashSet::from([owner.id, queue[0], queue[1]]));
    assert_eq!(seats(2).await, queue[2..].to_vec(), "the rest of the queue kept its order");
    for promoted in &queue[..2] {
        let told = ctx.notifications(*promoted, "community.event_update").await;
        assert_eq!(told.len(), 1, "told once that a place is theirs");
    }
    assert!(ctx.notifications(queue[2], "community.event_update").await.is_empty());
}

#[tokio::test]
async fn votes_cast_together_are_each_counted_once() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let general = ctx.general(&owner, &community).await;
    let request = CreateThreadRequest {
        channel_id: general.id.clone(),
        title: "どれにする?".into(),
        poll: PollInput { options: vec!["a".into(), "b".into(), "c".into()], ..Default::default() }.into(),
        ..Default::default()
    };
    let thread = ctx.try_thread(&owner, &request).await.unwrap();
    let voters = joined(&ctx, &community, 8).await;

    // Everyone votes at once — and everyone twice, for two different
    // options, as a double tap or two devices would.
    let mut votes = Vec::new();
    for (i, voter) in voters.iter().enumerate() {
        for choice in [i % 3, (i + 1) % 3] {
            let vote = VotePollRequest {
                thread_id: thread.id.clone(),
                option_ids: vec![thread.poll.options[choice].id.clone()],
                ..Default::default()
            };
            votes.push(fire(&ctx, BOARD, "VotePoll", voter, vote));
        }
    }
    let results = join_all(votes).await;
    all_succeeded(&results);

    // One vote per voter, and the counters add up to exactly that.
    let poll = ctx.get_thread(Some(&owner), &thread.id).await.unwrap().poll.as_option().unwrap().clone();
    assert_eq!(poll.voter_count, 8);
    assert_eq!(poll.options.iter().map(|option| option.vote_count).sum::<i32>(), 8);
    let rows: Vec<(Uuid, i64)> =
        sqlx::query_as("SELECT option_id, count(*) FROM community_poll_votes WHERE thread_id = $1 GROUP BY option_id")
            .bind(id_of(&thread.id))
            .fetch_all(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!(rows.iter().map(|(_, count)| count).sum::<i64>(), 8);
    for option in &poll.options {
        let counted = rows.iter().find(|(id, _)| *id == id_of(&option.id)).map_or(0, |(_, count)| *count);
        assert_eq!(i64::from(option.vote_count), counted, "option {}", option.text);
    }
    let per_voter: i64 = sqlx::query_scalar(
        "SELECT coalesce(max(n), 0) FROM (SELECT count(*) AS n FROM community_poll_votes WHERE thread_id = $1 GROUP BY user_id) v",
    )
    .bind(id_of(&thread.id))
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(per_voter, 1);
}

#[tokio::test]
async fn a_join_racing_a_ban_never_leaves_a_banned_member() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let mut users = Vec::new();
    for i in 0..8 {
        users.push(ctx.app.user(&format!("Racer{i}")).await);
    }

    // For each of them the join and the ban are on their way at once.
    let races = users.iter().map(|user| {
        let ban = BanMemberRequest {
            community_id: community.id.clone(),
            user_id: user.id.to_string(),
            reason: "荒らし".into(),
            ..Default::default()
        };
        let (ctx, owner, community) = (&ctx, &owner, &community);
        async move {
            tokio::join!(
                ctx.try_join(user, &community.id, ""),
                ctx.call::<_, BanMemberResponse>(MODERATION, "BanMember", Some(owner), &ban)
            )
        }
    });
    for (joined, banned) in join_all(races).await {
        banned.expect("the ban goes through whichever came first");
        // The join either came first (and the ban then removed the member) or was refused.
        if let Err(error) = joined {
            assert_eq!(error.reason, "BANNED");
        }
    }

    // Whatever the order was: banned, and not a member.
    for user in &users {
        assert!(!ctx.community.is_member(id_of(&community.id), user.id).await.unwrap());
        assert_eq!(ctx.viewer(user, &community.id).await.status, MembershipStatus::MEMBERSHIP_STATUS_BANNED);
        assert_eq!(ctx.try_join(user, &community.id, "").await.unwrap_err().reason, "BANNED");
    }
    let members: i64 = sqlx::query_scalar("SELECT count(*) FROM community_members WHERE community_id = $1")
        .bind(id_of(&community.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(members, 1);
    assert_eq!(ctx.get(None, &community.id).await.unwrap().member_count, 1, "the counter agrees with the rows");
}

#[tokio::test]
async fn an_invite_is_used_as_often_as_it_allows_and_no_more() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.hidden(&owner).await;
    let request = CreateInviteRequest { community_id: community.id.clone(), max_uses: 3, ..Default::default() };
    let created: CreateInviteResponse = ctx.call(SVC, "CreateInvite", Some(&owner), &request).await.unwrap();
    let code = created.invite.code.clone();
    let mut users = Vec::new();
    for i in 0..8 {
        users.push(ctx.app.user(&format!("Guest{i}")).await);
    }

    let results: Vec<_> = join_all(users.iter().map(|user| ctx.try_join(user, "", &code)))
        .await
        .into_iter()
        .map(|result| result.map(|_| ()))
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 3);
    assert_eq!(refused(&results, "INVITE_INVALID"), 5);

    let listed: ListInvitesResponse = ctx
        .call(
            SVC,
            "ListInvites",
            Some(&owner),
            &ListInvitesRequest { community_id: community.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(listed.invites.iter().all(|invite| invite.code != code), "a used-up invite is no longer listed");
    let used: i32 = sqlx::query_scalar("SELECT use_count FROM community_invites WHERE code = $1")
        .bind(&code)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(used, 3);
    assert_eq!(ctx.get(Some(&owner), &community.id).await.unwrap().member_count, 4);
}

#[tokio::test]
async fn replies_written_together_get_numbers_of_their_own() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&owner, &general, "一斉に返信", "x").await;
    let users = joined(&ctx, &community, 6).await;

    let mut writers = Vec::new();
    for (i, user) in users.iter().enumerate() {
        // Each of them twice.
        for n in 0..2 {
            let request =
                CreateReplyRequest {
                    thread_id: thread.id.clone(), body: format!("返信{i}-{n}"), ..Default::default()
                };
            writers.push(fire(&ctx, BOARD, "CreateReply", user, request));
        }
    }
    let results = join_all(writers).await;
    all_succeeded(&results);

    let mut numbers: Vec<i32> = ctx.replies(None, &thread.id).await.iter().map(|reply| reply.number).collect();
    numbers.sort();
    assert_eq!(numbers, (1..=12).collect::<Vec<_>>());
    assert_eq!(ctx.get_thread(None, &thread.id).await.unwrap().reply_count, 12);
}

#[tokio::test]
async fn the_ownership_limit_and_the_member_cap_hold_under_a_burst() {
    let ctx = start_with(CommunityConfig { max_owned: 2, max_members: 4, ..test_config() }).await;
    let owner = ctx.app.user("Owner").await;

    // Five creations at once, room for two.
    let creations = (0..5).map(|_| {
        fire(&ctx, SVC, "CreateCommunity", &owner, ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC))
    });
    let results = join_all(creations).await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 2);
    assert_eq!(refused(&results, "COMMUNITY_LIMIT_REACHED"), 3);
    let mine: ListMyCommunitiesResponse =
        ctx.call(SVC, "ListMyCommunities", Some(&owner), &ListMyCommunitiesRequest::default()).await.unwrap();
    assert_eq!(mine.communities.len(), 2);

    // Eight joins at once, room for three next to the owner.
    let community = mine.communities[0].clone();
    let mut users = Vec::new();
    for i in 0..8 {
        users.push(ctx.app.user(&format!("Joiner{i}")).await);
    }
    let results: Vec<_> = join_all(users.iter().map(|user| ctx.try_join(user, &community.id, "")))
        .await
        .into_iter()
        .map(|result| result.map(|_| ()))
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 3);
    assert_eq!(refused(&results, "COMMUNITY_FULL"), 5);
    assert_eq!(ctx.get(None, &community.id).await.unwrap().member_count, 4);
}

#[tokio::test]
async fn reports_arriving_together_hide_a_post_once() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let users = joined(&ctx, &community, 7).await;
    let general = ctx.general(&owner, &community).await;
    let (author, reporters) = (&users[0], &users[1..]);
    let thread = ctx.thread(author, &general, "通報が殺到するスレ", "x").await;
    let reply = ctx.reply(author, &ctx.thread(&owner, &general, "別のスレ", "x").await, "通報が殺到する返信").await;

    // Six members report both at the same moment; three are enough.
    let on_thread = join_all(
        reporters.iter().map(|reporter| ctx.report(reporter, SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id)),
    );
    let on_reply = join_all(
        reporters.iter().map(|reporter| ctx.report(reporter, SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id)),
    );
    tokio::join!(on_thread, on_reply);

    assert_eq!(ctx.get_thread(Some(&reporters[0]), &thread.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(ctx.get_reply(Some(&reporters[0]), &reply.id).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    // Hidden once each: one log entry, one notice to the author, and the
    // counters went down by one.
    let hidden = ctx
        .moderation_log(&owner, &community)
        .await
        .iter()
        .filter(|entry| entry.action == ModerationAction::MODERATION_ACTION_POST_HIDDEN)
        .count();
    assert_eq!(hidden, 2);
    assert_eq!(ctx.notifications(author.id, "community.moderation").await.len(), 2);
    assert_eq!(ctx.get(None, &community.id).await.unwrap().thread_count, 1);
    let queue = ctx.review_queue(&owner, &community).await;
    assert_eq!(queue.len(), 2);
    assert!(queue.iter().all(|item| item.report_count == 6), "every report was counted");
}

#[tokio::test]
async fn checking_in_twice_at_once_counts_once() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let guest = ctx.app.user("Guest").await;
    ctx.join(&guest, &community).await;
    let event = ctx.event(&owner, &community).await;
    ctx.rsvp(&guest, &event, RsvpAnswer::RSVP_ANSWER_GOING).await;
    ctx.move_event(&event.id, chrono::Duration::minutes(-10), chrono::Duration::hours(2)).await;
    let shown: GetCheckInCodeResponse = ctx
        .call(
            EVENTS,
            "GetCheckInCode",
            Some(&owner),
            &GetCheckInCodeRequest { event_id: event.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();

    // The guest's phone sends the code five times; the organiser ticks
    // them off by hand at the same moment.
    let mut check_ins = Vec::new();
    for _ in 0..5 {
        let request = CheckInRequest { event_id: event.id.clone(), code: shown.code.clone(), ..Default::default() };
        check_ins.push(fire(&ctx, EVENTS, "CheckIn", &guest, request));
    }
    let by_hand =
        CheckInAttendeeRequest { event_id: event.id.clone(), user_id: guest.id.to_string(), ..Default::default() };
    check_ins.push(fire(&ctx, EVENTS, "CheckInAttendee", &owner, by_hand));
    let results = join_all(check_ins).await;
    all_succeeded(&results);

    assert_eq!(ctx.get_event(Some(&owner), &event.id).await.unwrap().attended_count, 1);
    let attended: Vec<_> = ctx
        .community_events("community.event.attended", &community.id)
        .await
        .into_iter()
        .filter(|event| event.data["user_id"] == serde_json::json!(guest.id))
        .collect();
    assert_eq!(attended.len(), 1, "other domains hear of one attendance");
}
