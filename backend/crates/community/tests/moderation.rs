//! `CommunityModerationService`: sanctions, safety settings, the word
//! filter, auto-hide on reports, the review queue and the log.
mod common;

use common::*;
use tokuly_proto::{
    common::SubjectType,
    community::{community_event::Event as Pushed, *},
};
use tokuly_testkit::TestUser;

const SVC: &str = "CommunityModerationService";
const BOARD: &str = "CommunityBoardService";

fn target(community: &Community, user: &TestUser) -> (String, String) {
    (community.id.clone(), user.id.to_string())
}

fn ban(community: &Community, user: &TestUser, reason: &str) -> BanMemberRequest {
    let (community_id, user_id) = target(community, user);
    BanMemberRequest { community_id, user_id, reason: reason.into(), ..Default::default() }
}

fn timeout(community: &Community, user: &TestUser, seconds: i32) -> TimeoutMemberRequest {
    let (community_id, user_id) = target(community, user);
    TimeoutMemberRequest {
        community_id,
        user_id,
        duration_seconds: seconds,
        reason: "頭を冷やそう".into(),
        ..Default::default()
    }
}

async fn review(
    ctx: &Ctx,
    staff: &TestUser,
    post: PostRef,
    decision: ReviewDecision,
    reason: &str,
) -> Result<ReviewPostResponse, tokuly_testkit::RpcError> {
    let request =
        ReviewPostRequest { post: post.into(), decision: decision.into(), reason: reason.into(), ..Default::default() };
    ctx.call(SVC, "ReviewPost", Some(staff), &request).await
}

fn thread_post(thread: &Thread) -> PostRef {
    PostRef { post: Some(post_ref::Post::ThreadId(thread.id.clone())), ..Default::default() }
}

fn reply_post(reply: &Reply) -> PostRef {
    PostRef { post: Some(post_ref::Post::ReplyId(reply.id.clone())), ..Default::default() }
}

#[tokio::test]
async fn members_are_removed_by_those_above_them() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let other_moderator = ctx.staff(&owner, &community, "OtherMod", MemberRole::MEMBER_ROLE_MODERATOR).await;
    let admin = ctx.staff(&owner, &community, "Admin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let outsider = ctx.app.user("Outsider").await;
    let remove = |user: &TestUser| {
        let (community_id, user_id) = target(&community, user);
        RemoveMemberRequest { community_id, user_id, reason: "荒らし行為".into(), ..Default::default() }
    };

    assert_eq!(ctx.reason(SVC, "RemoveMember", Some(&member), &remove(&moderator)).await, "PERMISSION_DENIED");
    // A moderator removes members, not their peers or betters.
    for user in [&other_moderator, &admin, &owner] {
        assert_eq!(ctx.reason(SVC, "RemoveMember", Some(&moderator), &remove(user)).await, "PERMISSION_DENIED");
    }
    assert_eq!(ctx.reason(SVC, "RemoveMember", Some(&moderator), &remove(&moderator)).await, "CANNOT_TARGET_SELF");
    assert_eq!(ctx.reason(SVC, "RemoveMember", Some(&moderator), &remove(&outsider)).await, "MEMBER_NOT_FOUND");
    assert_eq!(ctx.reason(SVC, "RemoveMember", None, &remove(&member)).await, "SIGN_IN_REQUIRED");

    let mut frames = ctx.app.listen_user(member.id).await;
    let before = ctx.get(None, &community.id).await.unwrap().member_count;
    let _: RemoveMemberResponse = ctx.call(SVC, "RemoveMember", Some(&moderator), &remove(&member)).await.unwrap();
    assert_eq!(ctx.get(None, &community.id).await.unwrap().member_count, before - 1);
    assert_eq!(ctx.viewer(&member, &community.id).await.status, MembershipStatus::MEMBERSHIP_STATUS_NONE);
    assert!(pushes(&mut frames).await.iter().any(
        |push| matches!(push, Pushed::MembershipChanged(change) if change.status == MembershipStatus::MEMBERSHIP_STATUS_NONE)
    ));
    let told = ctx.notifications(member.id, "community.membership").await;
    assert_eq!(
        (told[0]["data"]["change"].as_str(), told[0]["body"]["ja"].as_str()),
        (Some("removed"), Some("荒らし行為"))
    );
    assert_eq!(ctx.app.events_by("community.member.left", member.id).await[0].data["cause"], "removed");
    let log = ctx.moderation_log(&owner, &community).await;
    let entry = log.iter().find(|entry| entry.action == ModerationAction::MODERATION_ACTION_MEMBER_REMOVED).unwrap();
    assert_eq!(
        (entry.actor.id.as_str(), entry.target_user.id.as_str(), entry.reason.as_str()),
        (moderator.id.to_string().as_str(), member.id.to_string().as_str(), "荒らし行為")
    );

    // Removed is not banned: they may come back. An admin removes a moderator.
    ctx.join(&member, &community).await;
    let _: RemoveMemberResponse = ctx.call(SVC, "RemoveMember", Some(&admin), &remove(&moderator)).await.unwrap();
}

#[tokio::test]
async fn a_ban_removes_and_keeps_out() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let admin = ctx.staff(&owner, &community, "Admin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let stranger = ctx.app.user("Stranger").await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&member, &general, "書き込み", "x").await;

    assert_eq!(
        ctx.reason(SVC, "BanMember", Some(&member), &ban(&community, &stranger, "x")).await,
        "PERMISSION_DENIED"
    );
    assert_eq!(ctx.reason(SVC, "BanMember", Some(&moderator), &ban(&community, &member, " ")).await, "REASON_REQUIRED");
    assert_eq!(
        ctx.reason(SVC, "BanMember", Some(&moderator), &ban(&community, &moderator, "x")).await,
        "CANNOT_TARGET_SELF"
    );
    assert_eq!(
        ctx.reason(SVC, "BanMember", Some(&moderator), &ban(&community, &admin, "x")).await,
        "PERMISSION_DENIED"
    );
    let past =
        BanMemberRequest { expires_at: ts(now() - chrono::Duration::minutes(1)), ..ban(&community, &member, "x") };
    assert_eq!(ctx.reason(SVC, "BanMember", Some(&moderator), &past).await, "EXPIRY_INVALID");
    let nobody = BanMemberRequest { user_id: uuid::Uuid::now_v7().to_string(), ..ban(&community, &member, "x") };
    assert_eq!(ctx.reason(SVC, "BanMember", Some(&moderator), &nobody).await, "USER_NOT_FOUND");

    let mut frames = ctx.app.listen_user(member.id).await;
    let request =
        BanMemberRequest { expires_at: ts(now() + chrono::Duration::days(7)), ..ban(&community, &member, "暴言") };
    let banned: BanMemberResponse = ctx.call(SVC, "BanMember", Some(&moderator), &request).await.unwrap();
    assert_eq!((banned.ban.user.id.as_str(), banned.ban.reason.as_str()), (member.id.to_string().as_str(), "暴言"));
    assert_eq!(banned.ban.banned_by.id, moderator.id.to_string());
    assert!(banned.ban.expires_at.as_option().is_some());
    assert!(pushes(&mut frames).await.iter().any(
        |push| matches!(push, Pushed::MembershipChanged(change) if change.status == MembershipStatus::MEMBERSHIP_STATUS_BANNED)
    ));

    // Out, told why, and kept out — by the door and by invite.
    let viewer = ctx.viewer(&member, &community.id).await;
    assert_eq!(viewer.status, MembershipStatus::MEMBERSHIP_STATUS_BANNED);
    assert_eq!(viewer.ban_reason, "暴言");
    assert!(viewer.ban_expires_at.as_option().is_some() && viewer.permissions.is_empty());
    assert_eq!(ctx.try_join(&member, &community.id, "").await.unwrap_err().reason, "BANNED");
    let invite = ctx.invite(&owner, &community).await;
    assert_eq!(ctx.try_join(&member, "", &invite.code).await.unwrap_err().reason, "BANNED");
    assert_eq!(ctx.notifications(member.id, "community.membership").await[0]["data"]["change"], "banned");
    // A public community stays readable, like for any passer-by; what they wrote stays.
    assert_eq!(ctx.get_thread(Some(&member), &thread.id).await.unwrap().author.id, member.id.to_string());
    let post =
        CreateReplyRequest { thread_id: thread.id.clone(), body: "まだ書ける?".into(), ..Default::default() };
    assert_eq!(ctx.reason(BOARD, "CreateReply", Some(&member), &post).await, "NOT_A_MEMBER");

    // Someone who never joined can be banned ahead of time.
    let _: BanMemberResponse =
        ctx.call(SVC, "BanMember", Some(&moderator), &ban(&community, &stranger, "別アカウント")).await.unwrap();
    assert_eq!(ctx.try_join(&stranger, &community.id, "").await.unwrap_err().reason, "BANNED");

    let list = ListBansRequest { community_id: community.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "ListBans", Some(&member), &list).await, "PERMISSION_DENIED");
    let bans: ListBansResponse = ctx.call(SVC, "ListBans", Some(&admin), &list).await.unwrap();
    assert_eq!(
        bans.bans.iter().map(|ban| ban.user.id.clone()).collect::<Vec<_>>(),
        vec![stranger.id.to_string(), member.id.to_string()]
    );

    // Lifting the ban opens the door again.
    let unban = |user: &TestUser| {
        let (community_id, user_id) = target(&community, user);
        UnbanMemberRequest { community_id, user_id, ..Default::default() }
    };
    assert_eq!(ctx.reason(SVC, "UnbanMember", Some(&member), &unban(&member)).await, "PERMISSION_DENIED");
    let _: UnbanMemberResponse = ctx.call(SVC, "UnbanMember", Some(&moderator), &unban(&member)).await.unwrap();
    assert_eq!(ctx.reason(SVC, "UnbanMember", Some(&moderator), &unban(&member)).await, "BAN_NOT_FOUND");
    ctx.join(&member, &community).await;
    // A ban that ran out is no ban.
    sqlx::query("UPDATE community_bans SET expires_at = now() - interval '1 minute' WHERE user_id = $1")
        .bind(stranger.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(ctx.viewer(&stranger, &community.id).await.status, MembershipStatus::MEMBERSHIP_STATUS_NONE);
    ctx.join(&stranger, &community).await;

    let log = ctx.moderation_log(&owner, &community).await;
    let actions: Vec<_> = log.iter().filter_map(|entry| entry.action.as_known()).collect();
    assert_eq!(
        actions.iter().filter(|action| **action == ModerationAction::MODERATION_ACTION_MEMBER_BANNED).count(),
        2
    );
    assert!(actions.contains(&ModerationAction::MODERATION_ACTION_MEMBER_UNBANNED));
}

#[tokio::test]
async fn a_timeout_silences_without_removing() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let bystander = ctx.member(&owner, &community, "Bystander").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let admin = ctx.staff(&owner, &community, "Admin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let outsider = ctx.app.user("Outsider").await;
    let general = ctx.general(&owner, &community).await;
    let request = CreateThreadRequest {
        channel_id: general.id.clone(),
        title: "投票".into(),
        poll: PollInput { options: vec!["a".into(), "b".into()], ..Default::default() }.into(),
        ..Default::default()
    };
    let thread = ctx.try_thread(&owner, &request).await.unwrap();
    let event = ctx.event(&owner, &community).await;

    assert_eq!(
        ctx.reason(SVC, "TimeoutMember", Some(&member), &timeout(&community, &bystander, 600)).await,
        "PERMISSION_DENIED"
    );
    assert_eq!(
        ctx.reason(SVC, "TimeoutMember", Some(&moderator), &timeout(&community, &member, 30)).await,
        "DURATION_INVALID"
    );
    assert_eq!(
        ctx.reason(SVC, "TimeoutMember", Some(&moderator), &timeout(&community, &member, 29 * 24 * 3600)).await,
        "DURATION_INVALID"
    );
    assert_eq!(
        ctx.reason(SVC, "TimeoutMember", Some(&moderator), &timeout(&community, &moderator, 600)).await,
        "CANNOT_TARGET_SELF"
    );
    assert_eq!(
        ctx.reason(SVC, "TimeoutMember", Some(&moderator), &timeout(&community, &admin, 600)).await,
        "PERMISSION_DENIED"
    );
    assert_eq!(
        ctx.reason(SVC, "TimeoutMember", Some(&moderator), &timeout(&community, &outsider, 600)).await,
        "MEMBER_NOT_FOUND"
    );

    let timed: TimeoutMemberResponse =
        ctx.call(SVC, "TimeoutMember", Some(&moderator), &timeout(&community, &member, 3600)).await.unwrap();
    assert!(timed.member.timeout_until.as_option().is_some());
    assert_eq!(timed.member.timeout_reason, "頭を冷やそう");
    let viewer = ctx.viewer(&member, &community.id).await;
    assert_eq!(viewer.status, MembershipStatus::MEMBERSHIP_STATUS_MEMBER);
    assert!(viewer.timeout_until.as_option().is_some());
    assert_eq!(ctx.notifications(member.id, "community.membership").await[0]["data"]["change"], "timed_out");

    // Still a member, still reading — but no posting, voting or answering events.
    assert_eq!(ctx.get_thread(Some(&member), &thread.id).await.unwrap().title, "投票");
    assert!(!ctx.channels(Some(&member), &community).await[0].can_post);
    let post = CreateThreadRequest {
        channel_id: general.id.clone(),
        title: "t".into(),
        body: "b".into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(BOARD, "CreateThread", Some(&member), &post).await, "TIMED_OUT");
    let reply = CreateReplyRequest { thread_id: thread.id.clone(), body: "b".into(), ..Default::default() };
    assert_eq!(ctx.reason(BOARD, "CreateReply", Some(&member), &reply).await, "TIMED_OUT");
    let vote = VotePollRequest {
        thread_id: thread.id.clone(),
        option_ids: vec![thread.poll.options[0].id.clone()],
        ..Default::default()
    };
    assert_eq!(ctx.reason(BOARD, "VotePoll", Some(&member), &vote).await, "TIMED_OUT");
    assert_eq!(ctx.try_rsvp(&member, &event.id, RsvpAnswer::RSVP_ANSWER_GOING).await.unwrap_err().reason, "TIMED_OUT");

    // The directory shows a timeout to the staff and to the member, not to everyone.
    let one = |viewer: &TestUser| {
        let (community_id, user_id) = target(&community, &member);
        let request = GetMemberRequest { community_id, user_id, ..Default::default() };
        let ctx = &ctx;
        let viewer = viewer.clone();
        async move {
            let response: GetMemberResponse =
                ctx.call("CommunityService", "GetMember", Some(&viewer), &request).await.unwrap();
            response.member.timeout_until.as_option().is_some()
        }
    };
    assert!(one(&moderator).await && one(&member).await);
    assert!(!one(&bystander).await);

    let clear = {
        let (community_id, user_id) = target(&community, &member);
        ClearTimeoutRequest { community_id, user_id, ..Default::default() }
    };
    assert_eq!(ctx.reason(SVC, "ClearTimeout", Some(&bystander), &clear).await, "PERMISSION_DENIED");
    let cleared: ClearTimeoutResponse = ctx.call(SVC, "ClearTimeout", Some(&moderator), &clear).await.unwrap();
    assert!(cleared.member.timeout_until.as_option().is_none());
    ctx.try_thread(&member, &post).await.unwrap();
    let actions: Vec<_> =
        ctx.moderation_log(&owner, &community).await.iter().filter_map(|entry| entry.action.as_known()).collect();
    assert!(actions.contains(&ModerationAction::MODERATION_ACTION_MEMBER_TIMED_OUT));
    assert!(actions.contains(&ModerationAction::MODERATION_ACTION_TIMEOUT_CLEARED));
}

#[tokio::test]
async fn safety_settings_are_for_the_staff() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let get = GetSafetySettingsRequest { community_id: community.id.clone(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "GetSafetySettings", Some(&member), &get).await, "PERMISSION_DENIED");
    let defaults: GetSafetySettingsResponse = ctx.call(SVC, "GetSafetySettings", Some(&moderator), &get).await.unwrap();
    assert!(defaults.settings.muted_words.is_empty());
    assert_eq!((defaults.settings.auto_hide_reports, defaults.settings.effective_auto_hide_reports), (0, 3));

    let words = |values: Vec<String>| UpdateSafetySettingsRequest {
        community_id: community.id.clone(),
        muted_words: StringList { values, ..Default::default() }.into(),
        ..Default::default()
    };
    let threshold = |value: i32| UpdateSafetySettingsRequest {
        community_id: community.id.clone(),
        auto_hide_reports: Some(value),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "UpdateSafetySettings", Some(&member), &threshold(5)).await, "PERMISSION_DENIED");
    assert_eq!(ctx.reason(SVC, "UpdateSafetySettings", Some(&moderator), &threshold(1)).await, "THRESHOLD_INVALID");
    assert_eq!(ctx.reason(SVC, "UpdateSafetySettings", Some(&moderator), &threshold(51)).await, "THRESHOLD_INVALID");
    assert_eq!(
        ctx.reason(SVC, "UpdateSafetySettings", Some(&moderator), &words(vec![" ".into()])).await,
        "MUTED_WORD_INVALID"
    );
    assert_eq!(
        ctx.reason(SVC, "UpdateSafetySettings", Some(&moderator), &words((0..201).map(|i| format!("w{i}")).collect()))
            .await,
        "TOO_MANY_MUTED_WORDS"
    );

    // Words are kept the way they are compared: width, case and kana folded.
    let updated: UpdateSafetySettingsResponse = ctx
        .call(SVC, "UpdateSafetySettings", Some(&moderator), &words(vec!["バカ".into(), "ﾊﾞｶ".into(), "Spam".into()]))
        .await
        .unwrap();
    assert_eq!(updated.settings.muted_words, vec!["ばか", "spam"]);
    let updated: UpdateSafetySettingsResponse =
        ctx.call(SVC, "UpdateSafetySettings", Some(&moderator), &threshold(5)).await.unwrap();
    assert_eq!((updated.settings.auto_hide_reports, updated.settings.effective_auto_hide_reports), (5, 5));
    // One setting changes without touching the other.
    assert_eq!(updated.settings.muted_words.len(), 2);
    let log = ctx.moderation_log(&owner, &community).await;
    assert_eq!(
        log.iter().filter(|entry| entry.action == ModerationAction::MODERATION_ACTION_SAFETY_SETTINGS_CHANGED).count(),
        2
    );
}

#[tokio::test]
async fn muted_words_hold_a_post_for_review() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let reader = ctx.member(&owner, &community, "Reader").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let general = ctx.general(&owner, &community).await;
    ctx.mute_words(&moderator, &community, &["バカ"]).await;
    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let mut staff_frames = ctx.app.listen_user(moderator.id).await;

    let held = ctx.thread(&author, &general, "おまえﾊﾞｶだろ", "ひどい").await;
    assert_eq!(
        (held.state, held.hidden_reason),
        (PostState::POST_STATE_HIDDEN.into(), HiddenReason::HIDDEN_REASON_MUTED_WORD.into())
    );
    // It exists for its author and the staff, and for nobody else — nowhere.
    assert_eq!(ctx.get_thread(Some(&reader), &held.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(ctx.get_thread(None, &held.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert!(ctx.thread_titles(Some(&reader), &community).await.is_empty());
    assert_eq!(ctx.thread_titles(Some(&author), &community).await, vec!["おまえﾊﾞｶだろ"]);
    assert_eq!(ctx.get_thread(Some(&moderator), &held.id).await.unwrap().state, PostState::POST_STATE_HIDDEN);
    assert_eq!(ctx.get(None, &community.id).await.unwrap().thread_count, 0);
    assert!(ctx.community_events("community.thread.created", &community.id).await.is_empty());
    assert!(ctx.search_changes("thread", &held.id).await.is_empty());
    assert!(!pushes(&mut topic).await.iter().any(|push| matches!(push, Pushed::ThreadCreated(_))));
    // The author is told it waits; the staff is told there is something to look at.
    assert_eq!(ctx.notifications(author.id, "community.moderation").await[0]["data"]["change"], "held");
    for staff in [&owner, &moderator] {
        assert_eq!(ctx.notifications(staff.id, "community.review").await.len(), 1);
    }
    assert!(ctx.notifications(reader.id, "community.review").await.is_empty());
    assert!(
        pushes(&mut staff_frames)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::StaffQueueChanged(queue) if queue.review_queue_count == 1))
    );
    assert_eq!(ctx.viewer(&moderator, &community.id).await.review_queue_count, 1);
    assert_eq!(ctx.viewer(&reader, &community.id).await.review_queue_count, 0);

    // The staff's own posts are not held; replies are screened like threads.
    let open = ctx.thread(&moderator, &general, "バカって言わないで", "お願い").await;
    assert_eq!(open.state, PostState::POST_STATE_VISIBLE);
    let held_reply = ctx.reply(&author, &open, "ば・か").await;
    assert_eq!(held_reply.state, PostState::POST_STATE_HIDDEN);
    assert_eq!(ctx.get_thread(None, &open.id).await.unwrap().reply_count, 0);
    assert!(ctx.replies(Some(&reader), &open.id).await.is_empty());
    assert_eq!(ctx.replies(Some(&author), &open.id).await.len(), 1);
    assert_eq!(ctx.replies(Some(&moderator), &open.id).await[0].state, PostState::POST_STATE_HIDDEN);
    assert_eq!(ctx.get_reply(Some(&reader), &held_reply.id).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    // Nothing can be answered on a post that is waiting.
    let on_held = CreateReplyRequest { thread_id: held.id.clone(), body: "自己レス".into(), ..Default::default() };
    assert_eq!(ctx.reason(BOARD, "CreateReply", Some(&author), &on_held).await, "THREAD_HIDDEN");

    // The queue: oldest first, with why.
    assert_eq!(
        ctx.reason(
            SVC,
            "ListReviewQueue",
            Some(&reader),
            &ListReviewQueueRequest { community_id: community.id.clone(), ..Default::default() }
        )
        .await,
        "PERMISSION_DENIED"
    );
    let queue = ctx.review_queue(&moderator, &community).await;
    assert_eq!(queue.len(), 2);
    assert!(matches!(&queue[0].post, Some(review_item::Post::Thread(thread)) if thread.id == held.id));
    assert!(matches!(&queue[1].post, Some(review_item::Post::Reply(reply)) if reply.id == held_reply.id));
    assert_eq!(queue[0].reason, HiddenReason::HIDDEN_REASON_MUTED_WORD);
    assert_eq!(
        (queue[0].thread_title.as_str(), queue[1].thread_title.as_str()),
        ("おまえﾊﾞｶだろ", "バカって言わないで")
    );

    // Restoring: the thread appears, for the first time, with all that follows.
    // (A member cannot even see the held post; reviewing is not theirs on any post.)
    assert_eq!(
        review(&ctx, &reader, thread_post(&held), ReviewDecision::REVIEW_DECISION_RESTORE, "")
            .await
            .unwrap_err()
            .reason,
        "THREAD_NOT_FOUND"
    );
    assert_eq!(
        review(&ctx, &reader, thread_post(&open), ReviewDecision::REVIEW_DECISION_RESTORE, "")
            .await
            .unwrap_err()
            .reason,
        "PERMISSION_DENIED"
    );
    assert_eq!(
        review(&ctx, &moderator, thread_post(&held), ReviewDecision::REVIEW_DECISION_UNSPECIFIED, "")
            .await
            .unwrap_err()
            .reason,
        "DECISION_INVALID"
    );
    assert_eq!(
        review(&ctx, &moderator, PostRef::default(), ReviewDecision::REVIEW_DECISION_RESTORE, "")
            .await
            .unwrap_err()
            .reason,
        "POST_REQUIRED"
    );
    assert_eq!(
        review(&ctx, &owner, thread_post(&open), ReviewDecision::REVIEW_DECISION_RESTORE, "").await.unwrap_err().reason,
        "POST_NOT_HIDDEN"
    );
    review(&ctx, &moderator, thread_post(&held), ReviewDecision::REVIEW_DECISION_RESTORE, "").await.unwrap();
    assert_eq!(ctx.get_thread(Some(&reader), &held.id).await.unwrap().state, PostState::POST_STATE_VISIBLE);
    assert_eq!(ctx.get(None, &community.id).await.unwrap().thread_count, 2);
    assert_eq!(ctx.app.events_by("community.thread.created", author.id).await.len(), 1);
    assert_eq!(ctx.search_changes("thread", &held.id).await.last().unwrap()["doc"]["title"], "おまえﾊﾞｶだろ");
    assert!(
        pushes(&mut topic)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::ThreadCreated(created) if created.thread_id == held.id))
    );
    assert_eq!(
        ctx.notifications(author.id, "community.moderation").await.last().unwrap()["data"]["change"],
        "restored"
    );
    assert_eq!(
        review(&ctx, &moderator, thread_post(&held), ReviewDecision::REVIEW_DECISION_RESTORE, "")
            .await
            .unwrap_err()
            .reason,
        "POST_NOT_HIDDEN"
    );
    // Once the staff let it through, an edit does not send it back into the queue.
    let edit =
        UpdateThreadRequest {
            thread_id: held.id.clone(), body: Some("バカは言い過ぎた".into()), ..Default::default()
        };
    let edited: UpdateThreadResponse = ctx.call(BOARD, "UpdateThread", Some(&author), &edit).await.unwrap();
    assert_eq!(edited.thread.state, PostState::POST_STATE_VISIBLE);

    // Removing: the reply is gone and its author hears why.
    review(&ctx, &moderator, reply_post(&held_reply), ReviewDecision::REVIEW_DECISION_REMOVE, "暴言はだめ")
        .await
        .unwrap();
    assert_eq!(ctx.get_reply(Some(&author), &held_reply.id).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    assert_eq!(ctx.notifications(author.id, "community.moderation").await.last().unwrap()["body"]["ja"], "暴言はだめ");
    assert!(ctx.review_queue(&moderator, &community).await.is_empty());
    let actions: Vec<_> =
        ctx.moderation_log(&owner, &community).await.iter().filter_map(|entry| entry.action.as_known()).collect();
    assert_eq!(actions.iter().filter(|action| **action == ModerationAction::MODERATION_ACTION_POST_HIDDEN).count(), 2);
    assert!(actions.contains(&ModerationAction::MODERATION_ACTION_POST_RESTORED));
    assert!(actions.contains(&ModerationAction::MODERATION_ACTION_POST_REMOVED));
}

#[tokio::test]
async fn an_edit_passes_the_word_filter_again() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let general = ctx.general(&owner, &community).await;
    ctx.mute_words(&owner, &community, &["spam"]).await;
    let thread = ctx.thread(&author, &general, "ふつうの題", "class of spammers is fine: whole words only").await;
    assert_eq!(thread.state, PostState::POST_STATE_VISIBLE);
    let reply = ctx.reply(&author, &thread, "ふつうの返信").await;

    let edit = UpdateReplyRequest { reply_id: reply.id.clone(), body: "buy SPAM now".into(), ..Default::default() };
    let edited: UpdateReplyResponse = ctx.call(BOARD, "UpdateReply", Some(&author), &edit).await.unwrap();
    assert_eq!(edited.reply.state, PostState::POST_STATE_HIDDEN);
    assert_eq!(ctx.get_thread(None, &thread.id).await.unwrap().reply_count, 0);
    let edit = UpdateThreadRequest { thread_id: thread.id.clone(), title: Some("spam".into()), ..Default::default() };
    let edited: UpdateThreadResponse = ctx.call(BOARD, "UpdateThread", Some(&author), &edit).await.unwrap();
    assert_eq!(edited.thread.state, PostState::POST_STATE_HIDDEN);
    assert_eq!(ctx.get(None, &community.id).await.unwrap().thread_count, 0);
    assert_eq!(ctx.review_queue(&owner, &community).await.len(), 2);
    assert!(ctx.search_changes("thread", &thread.id).await.last().unwrap()["doc"].is_null());
}

#[tokio::test]
async fn enough_reports_hide_a_post_until_the_staff_looked() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let mut reporters = Vec::new();
    for i in 0..4 {
        reporters.push(ctx.member(&owner, &community, &format!("Reporter{i}")).await);
    }
    let moderator = ctx.moderator(&owner, &community).await;
    let outsider = ctx.app.user("Outsider").await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&author, &general, "あやしい書き込み", "x").await;
    let kind = SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD;
    let visible = |viewer: &TestUser| {
        let (ctx, viewer, id) = (&ctx, viewer.clone(), thread.id.clone());
        async move { ctx.get_thread(Some(&viewer), &id).await.is_ok() }
    };

    // Passers-by, the author and one member reporting twice do not add up.
    ctx.report(&outsider, kind, &thread.id).await;
    ctx.report(&author, kind, &thread.id).await;
    ctx.report(&reporters[0], kind, &thread.id).await;
    ctx.report(&reporters[0], kind, &thread.id).await;
    ctx.report(&reporters[1], kind, &thread.id).await;
    assert!(visible(&reporters[3]).await);

    // The third member does it.
    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    ctx.report(&reporters[2], kind, &thread.id).await;
    assert!(!visible(&reporters[3]).await);
    assert!(visible(&author).await && visible(&moderator).await);
    assert!(
        pushes(&mut topic)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::ThreadRemoved(removed) if removed.thread_id == thread.id))
    );
    assert_eq!(ctx.get(None, &community.id).await.unwrap().thread_count, 0);
    assert_eq!(ctx.notifications(author.id, "community.moderation").await[0]["data"]["change"], "held");
    assert_eq!(ctx.notifications(moderator.id, "community.review").await.len(), 1);
    let queue = ctx.review_queue(&moderator, &community).await;
    assert_eq!((queue[0].report_count, queue[0].reason), (3, HiddenReason::HIDDEN_REASON_REPORTS.into()));
    let log = ctx.moderation_log(&owner, &community).await;
    let entry = log.iter().find(|entry| entry.action == ModerationAction::MODERATION_ACTION_POST_HIDDEN).unwrap();
    assert!(entry.actor.as_option().is_none(), "nobody did it: it is automatic");
    assert_eq!((entry.details["cause"].as_str(), entry.details["reports"].as_str()), ("reports", "3"));

    // Restored by the staff, the same crowd cannot hide it again.
    review(&ctx, &moderator, thread_post(&thread), ReviewDecision::REVIEW_DECISION_RESTORE, "").await.unwrap();
    assert!(visible(&reporters[3]).await);
    assert_eq!(
        ctx.app.events_by("community.thread.created", author.id).await.len(),
        1,
        "it was announced once, when it was written"
    );
    for reporter in &reporters {
        ctx.report(reporter, kind, &thread.id).await;
    }
    assert!(visible(&reporters[3]).await);

    // What the staff writes is not hidden by reports.
    let official = ctx.thread(&moderator, &general, "運営より", "x").await;
    for reporter in &reporters {
        ctx.report(reporter, kind, &official.id).await;
    }
    assert!(ctx.get_thread(Some(&reporters[0]), &official.id).await.is_ok());
    // Reports about things this domain does not know are ignored.
    ctx.report(&reporters[0], kind, &uuid::Uuid::now_v7().to_string()).await;
    ctx.report(&reporters[0], SubjectType::SUBJECT_TYPE_NOTE, &thread.id).await;
}

#[tokio::test]
async fn a_community_sets_its_own_report_threshold() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let (one, two) = (ctx.member(&owner, &community, "One").await, ctx.member(&owner, &community, "Two").await);
    let general = ctx.general(&owner, &community).await;
    let request = UpdateSafetySettingsRequest {
        community_id: community.id.clone(),
        auto_hide_reports: Some(2),
        ..Default::default()
    };
    let _: UpdateSafetySettingsResponse = ctx.call(SVC, "UpdateSafetySettings", Some(&owner), &request).await.unwrap();
    let thread = ctx.thread(&owner, &general, "スレ", "x").await;
    let reply = ctx.reply(&author, &thread, "あやしい返信").await;
    let kind = SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY;

    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    ctx.report(&one, kind, &reply.id).await;
    assert_eq!(ctx.replies(None, &thread.id).await.len(), 1);
    ctx.report(&two, kind, &reply.id).await;
    assert!(ctx.replies(None, &thread.id).await.is_empty());
    assert_eq!(ctx.get_thread(None, &thread.id).await.unwrap().reply_count, 0);
    assert!(
        pushes(&mut topic)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::ReplyRemoved(removed) if removed.reply_id == reply.id))
    );
    // Its author still sees it, marked as waiting.
    let own = ctx.replies(Some(&author), &thread.id).await;
    assert_eq!(
        (own[0].state, own[0].hidden_reason),
        (PostState::POST_STATE_HIDDEN.into(), HiddenReason::HIDDEN_REASON_REPORTS.into())
    );

    review(&ctx, &owner, reply_post(&reply), ReviewDecision::REVIEW_DECISION_RESTORE, "").await.unwrap();
    assert_eq!(ctx.get_thread(None, &thread.id).await.unwrap().reply_count, 1);
    assert!(
        pushes(&mut topic)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::ReplyCreated(created) if created.reply_id == reply.id))
    );
}

#[tokio::test]
async fn the_log_is_for_the_staff() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.restricted(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let outsider = ctx.app.user("Outsider").await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&member, &general, "スレ", "x").await;
    for (locked, pinned) in [(true, true), (false, false)] {
        let _: LockThreadResponse = ctx
            .call(
                BOARD,
                "LockThread",
                Some(&moderator),
                &LockThreadRequest { thread_id: thread.id.clone(), locked, ..Default::default() },
            )
            .await
            .unwrap();
        let _: PinThreadResponse = ctx
            .call(
                BOARD,
                "PinThread",
                Some(&moderator),
                &PinThreadRequest { thread_id: thread.id.clone(), pinned, ..Default::default() },
            )
            .await
            .unwrap();
    }
    let all = ListModerationLogRequest { community_id: community.id.clone(), ..Default::default() };
    for user in [&member, &outsider] {
        assert_eq!(ctx.reason(SVC, "ListModerationLog", Some(user), &all).await, "PERMISSION_DENIED");
    }
    assert_eq!(ctx.reason(SVC, "ListModerationLog", None, &all).await, "SIGN_IN_REQUIRED");

    // Newest first, in pages, optionally one kind of action.
    let log = ctx.moderation_log(&moderator, &community).await;
    assert_eq!(log[0].action, ModerationAction::MODERATION_ACTION_THREAD_UNPINNED);
    assert_eq!(log[0].thread_id, thread.id);
    assert_eq!(log[0].details["title"], "スレ");
    let first: ListModerationLogResponse = ctx
        .call(SVC, "ListModerationLog", Some(&owner), &ListModerationLogRequest { page: page(2, ""), ..all.clone() })
        .await
        .unwrap();
    assert!(first.page.has_more);
    let second: ListModerationLogResponse = ctx
        .call(
            SVC,
            "ListModerationLog",
            Some(&owner),
            &ListModerationLogRequest { page: page(2, &first.page.next_cursor), ..all.clone() },
        )
        .await
        .unwrap();
    assert_eq!(second.entries[0].id, log[2].id);
    let locks =
        ListModerationLogRequest { action: ModerationAction::MODERATION_ACTION_THREAD_LOCKED.into(), ..all.clone() };
    let locks: ListModerationLogResponse = ctx.call(SVC, "ListModerationLog", Some(&owner), &locks).await.unwrap();
    assert_eq!(locks.entries.len(), 1);
    assert_eq!(locks.entries[0].actor.id, moderator.id.to_string());
}

#[tokio::test]
async fn nobody_reviews_their_own_post() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let general = ctx.general(&owner, &community).await;
    ctx.mute_words(&owner, &community, &["バカ"]).await;
    // Held as a member's post; then the member joins the staff.
    let held = ctx.thread(&author, &general, "バカって書いた", "x").await;
    let open = ctx.thread(&owner, &general, "ふつうのスレ", "x").await;
    let held_reply = ctx.reply(&author, &open, "バカ").await;
    assert_eq!(
        (held.state, held_reply.state),
        (PostState::POST_STATE_HIDDEN.into(), PostState::POST_STATE_HIDDEN.into())
    );
    ctx.set_role(&owner, &community, &author, MemberRole::MEMBER_ROLE_MODERATOR).await;

    // Their own posts are in the queue they now see — and not theirs to decide, either way.
    assert_eq!(ctx.review_queue(&author, &community).await.len(), 2);
    for decision in [ReviewDecision::REVIEW_DECISION_RESTORE, ReviewDecision::REVIEW_DECISION_REMOVE] {
        assert_eq!(
            review(&ctx, &author, thread_post(&held), decision, "").await.unwrap_err().reason,
            "CONFLICT_OF_INTEREST"
        );
        assert_eq!(
            review(&ctx, &author, reply_post(&held_reply), decision, "").await.unwrap_err().reason,
            "CONFLICT_OF_INTEREST"
        );
    }
    assert_eq!(ctx.get_thread(None, &held.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    // Another member of the staff decides.
    let other = ctx.moderator(&owner, &community).await;
    review(&ctx, &other, thread_post(&held), ReviewDecision::REVIEW_DECISION_RESTORE, "").await.unwrap();
    assert_eq!(ctx.get_thread(None, &held.id).await.unwrap().state, PostState::POST_STATE_VISIBLE);
    // Taking their own post back is still theirs to do, like any author.
    let delete = DeleteReplyRequest { reply_id: held_reply.id.clone(), ..Default::default() };
    let _: DeleteReplyResponse = ctx.call(BOARD, "DeleteReply", Some(&author), &delete).await.unwrap();
    assert!(ctx.review_queue(&other, &community).await.is_empty());
    // Sanctions stay as they were: never on oneself, never on one's own level.
    assert_eq!(
        ctx.reason(SVC, "TimeoutMember", Some(&author), &timeout(&community, &author, 600)).await,
        "CANNOT_TARGET_SELF"
    );
    assert_eq!(
        ctx.reason(SVC, "BanMember", Some(&author), &ban(&community, &other, "同格")).await,
        "PERMISSION_DENIED"
    );
}

#[tokio::test]
async fn accounts_that_joined_after_a_post_cannot_hide_it() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let mut regulars = Vec::new();
    for i in 0..3 {
        regulars.push(ctx.member(&owner, &community, &format!("Regular{i}")).await);
    }
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&author, &general, "気に入らない人がいるスレ", "x").await;
    let reply = ctx.reply(&author, &thread, "気に入らない人がいる返信").await;

    // Four fresh accounts join the public community and report at once.
    for i in 0..4 {
        let fresh = ctx.app.user(&format!("Fresh{i}")).await;
        ctx.join(&fresh, &community).await;
        ctx.report(&fresh, SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id).await;
        ctx.report(&fresh, SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id).await;
    }
    assert!(ctx.get_thread(None, &thread.id).await.is_ok());
    assert!(ctx.get_reply(None, &reply.id).await.is_ok());
    assert!(ctx.review_queue(&owner, &community).await.is_empty());

    // Members who were there when it was written still can.
    for regular in &regulars {
        ctx.report(regular, SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id).await;
    }
    assert_eq!(ctx.get_thread(None, &thread.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(ctx.review_queue(&owner, &community).await[0].report_count, 3);
}

#[tokio::test]
async fn the_log_says_which_words_were_muted() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let moderator = ctx.moderator(&owner, &community).await;
    ctx.mute_words(&moderator, &community, &["バカ", "あほ"]).await;
    ctx.mute_words(&moderator, &community, &["バカ", "たなか"]).await;
    let log = ctx.moderation_log(&owner, &community).await;
    let changes: Vec<_> = log
        .iter()
        .filter(|entry| entry.action == ModerationAction::MODERATION_ACTION_SAFETY_SETTINGS_CHANGED)
        .collect();
    // Newest first.
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].actor.id, moderator.id.to_string());
    assert_eq!(
        (changes[0].details["muted_words_added"].as_str(), changes[0].details["muted_words_removed"].as_str()),
        ("たなか", "あほ")
    );
    // (Words are stored the way they are matched: folded to hiragana.)
    assert_eq!(changes[1].details["muted_words_added"], "ばか, あほ");
    assert_eq!(changes[1].details["muted_words_removed"], "");
}
