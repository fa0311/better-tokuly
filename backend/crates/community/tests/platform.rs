//! How the community domain meets the rest of the system: subject
//! resolvers and counters, the topic guard, account erasure, site-level
//! takedowns, uploads deleted by force, the data export and the workers.
mod common;

use common::*;
use tokuly_community::SweepJob;
use tokuly_proto::{
    common::SubjectType,
    community::{community_event::Event as Pushed, *},
    media::AssetPurpose,
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

const BOARD: &str = "CommunityBoardService";

fn id_of(value: &str) -> Uuid {
    value.parse().expect("a UUID")
}

/// Erases an account the way identity does, and tells the community domain.
async fn erase(ctx: &Ctx, user: &TestUser) {
    sqlx::query(
        "UPDATE users SET deleted_at = now(), handle = ('deleted-' || replace(id::text, '-', ''))::citext, display_name = ''
         WHERE id = $1",
    )
    .bind(user.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    ctx.deliver("identity.user.deleted", Some(user.id), serde_json::json!({ "user_id": user.id })).await;
}

async fn take_down(ctx: &Ctx, moderator: &TestUser, kind: SubjectType, subject_id: &str) {
    ctx.deliver(
        "admin.content.removed",
        Some(moderator.id),
        serde_json::json!({ "subject_type": kind as i32, "subject_id": subject_id, "reason": "規約違反", "moderator_id": moderator.id }),
    )
    .await;
}

#[tokio::test]
async fn subjects_resolve_for_those_who_may_see_them() {
    let ctx = start().await;
    let subjects = &ctx.app.core.subjects;
    let (owner, outsider) = (ctx.app.user("Owner").await, ctx.app.user("Outsider").await);
    let site_moderator = ctx.app.user_with_roles("SiteMod", &["moderator"]).await;
    let public = ctx.public(&owner).await;
    let hidden = ctx.hidden(&owner).await;
    let restricted = ctx.restricted(&owner).await;
    let author = ctx.member(&owner, &restricted, "Author").await;
    use SubjectType::*;

    // Communities: the front page rule.
    let found = subjects.resolve(SUBJECT_TYPE_COMMUNITY, &public.handle, None).await.unwrap();
    assert_eq!((found.id, found.owner_id), (id_of(&public.id), owner.id));
    assert_eq!(
        (found.title.as_str(), found.path.as_str()),
        (public.name.as_str(), format!("/c/{}", public.handle).as_str())
    );
    assert!(found.interactions_enabled);
    assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY, &restricted.id, Some(outsider.id)).await.is_ok());
    assert_eq!(
        subjects.resolve(SUBJECT_TYPE_COMMUNITY, &hidden.id, Some(outsider.id)).await.unwrap_err().reason,
        "SUBJECT_NOT_FOUND"
    );
    assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY, &hidden.id, Some(owner.id)).await.is_ok());

    // Threads and replies: the reading rule, by UUID or short ID.
    let general = ctx.general(&owner, &restricted).await;
    let thread = ctx.thread(&author, &general, "部内のお知らせ", "本文").await;
    let reply = ctx.reply(&owner, &thread, "了解です、ありがとう").await;
    let as_member = subjects.resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &thread.short_id, Some(owner.id)).await.unwrap();
    assert_eq!((as_member.id, as_member.owner_id), (id_of(&thread.id), author.id));
    assert_eq!((as_member.title.as_str(), as_member.path.as_str()), ("部内のお知らせ", thread.path.as_str()));
    assert!(as_member.interactions_enabled);
    for viewer in [Some(outsider.id), None] {
        assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, viewer).await.is_err());
        assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id, viewer).await.is_err());
    }
    let as_reply = subjects.resolve(SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id, Some(author.id)).await.unwrap();
    assert_eq!((as_reply.owner_id, as_reply.title.as_str()), (owner.id, "了解です、ありがとう"));
    assert!(as_reply.path.ends_with(&format!("?reply={}", reply.id)));
    // Site moderation reads everything: that is how a report is looked at.
    assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, Some(site_moderator.id)).await.is_ok());
    assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY_THREAD, "no-such-thread", Some(owner.id)).await.is_err());

    // A locked thread takes no comments or likes; neither do its replies.
    let lock = LockThreadRequest { thread_id: thread.id.clone(), locked: true, ..Default::default() };
    let _: LockThreadResponse = ctx.call(BOARD, "LockThread", Some(&owner), &lock).await.unwrap();
    assert!(
        !subjects
            .resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, Some(owner.id))
            .await
            .unwrap()
            .interactions_enabled
    );
    assert!(
        !subjects.resolve(SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id, Some(owner.id)).await.unwrap().interactions_enabled
    );

    // Someone banned from a public community still reads it, and that is all.
    let open_thread = ctx.thread(&owner, &ctx.general(&owner, &public).await, "だれでも", "x").await;
    let ban = BanMemberRequest {
        community_id: public.id.clone(),
        user_id: outsider.id.to_string(),
        reason: "荒らし".into(),
        ..Default::default()
    };
    let _: BanMemberResponse = ctx.call("CommunityModerationService", "BanMember", Some(&owner), &ban).await.unwrap();
    assert!(
        !subjects
            .resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &open_thread.id, Some(outsider.id))
            .await
            .unwrap()
            .interactions_enabled
    );
    assert!(
        subjects
            .resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &open_thread.id, Some(owner.id))
            .await
            .unwrap()
            .interactions_enabled
    );

    // Events: the organiser owns them; a cancelled one takes no more interactions.
    let event = ctx.event(&owner, &public).await;
    let as_event = subjects.resolve(SUBJECT_TYPE_COMMUNITY_EVENT, &event.id, Some(owner.id)).await.unwrap();
    assert_eq!(
        (as_event.owner_id, as_event.title.as_str(), as_event.path.as_str()),
        (owner.id, "オフ会", event.path.as_str())
    );
    assert!(as_event.interactions_enabled);
    let cancel = CancelEventRequest { event_id: event.id.clone(), ..Default::default() };
    let _: CancelEventResponse = ctx.call("CommunityEventService", "CancelEvent", Some(&owner), &cancel).await.unwrap();
    assert!(
        !subjects.resolve(SUBJECT_TYPE_COMMUNITY_EVENT, &event.id, Some(owner.id)).await.unwrap().interactions_enabled
    );
    let private_event = ctx.event(&owner, &restricted).await;
    assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY_EVENT, &private_event.id, Some(outsider.id)).await.is_err());
}

#[tokio::test]
async fn likes_and_comments_are_counted_on_the_posts() {
    let ctx = start().await;
    let subjects = &ctx.app.core.subjects;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&owner, &general, "いいね!", "x").await;
    let reply = ctx.reply(&owner, &thread, "返信").await;
    let event = ctx.event(&owner, &community).await;
    use SubjectType::*;

    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    subjects.counter_changed(SUBJECT_TYPE_COMMUNITY_THREAD, id_of(&thread.id), "like_count", 3).await.unwrap();
    subjects.counter_changed(SUBJECT_TYPE_COMMUNITY_THREAD, id_of(&thread.id), "comment_count", 2).await.unwrap();
    subjects.counter_changed(SUBJECT_TYPE_COMMUNITY_THREAD, id_of(&thread.id), "like_count", -1).await.unwrap();
    // Counters this domain does not keep are simply not kept.
    subjects.counter_changed(SUBJECT_TYPE_COMMUNITY_THREAD, id_of(&thread.id), "bookmark_count", 5).await.unwrap();
    let seen = ctx.get_thread(None, &thread.id).await.unwrap();
    assert_eq!((seen.like_count, seen.comment_count, seen.reply_count), (2, 2, 1));
    // The topic hears about it — at once, then at most about once a second.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let counts: Vec<_> = pushes(&mut topic)
        .await
        .into_iter()
        .filter_map(|push| match push {
            Pushed::ThreadCountsChanged(counts) => Some((counts.like_count, counts.comment_count, counts.reply_count)),
            _ => None,
        })
        .collect();
    assert_eq!(counts.first(), Some(&(3, 0, 1)));
    assert_eq!(counts.last(), Some(&(2, 2, 1)));
    assert!(counts.len() <= 3);

    subjects.counter_changed(SUBJECT_TYPE_COMMUNITY_REPLY, id_of(&reply.id), "like_count", 4).await.unwrap();
    subjects.counter_changed(SUBJECT_TYPE_COMMUNITY_REPLY, id_of(&reply.id), "comment_count", 1).await.unwrap();
    let seen = ctx.get_reply(None, &reply.id).await.unwrap();
    assert_eq!((seen.like_count, seen.comment_count), (4, 1));
    subjects.counter_changed(SUBJECT_TYPE_COMMUNITY_EVENT, id_of(&event.id), "like_count", 1).await.unwrap();
    assert_eq!(ctx.get_event(None, &event.id).await.unwrap().like_count, 1);
    // Never below zero, whatever arrives.
    subjects.counter_changed(SUBJECT_TYPE_COMMUNITY_EVENT, id_of(&event.id), "like_count", -5).await.unwrap();
    assert_eq!(ctx.get_event(None, &event.id).await.unwrap().like_count, 0);
    subjects.counter_changed(SUBJECT_TYPE_COMMUNITY, id_of(&community.id), "like_count", 1).await.unwrap();
}

#[tokio::test]
async fn the_topic_is_for_those_who_may_read() {
    let ctx = start().await;
    let guards = &ctx.app.core.topic_guards;
    let (owner, outsider) = (ctx.app.user("Owner").await, ctx.app.user("Outsider").await);
    let site_moderator = ctx.app.user_with_roles("SiteMod", &["moderator"]).await;
    let public = ctx.public(&owner).await;
    let restricted = ctx.restricted(&owner).await;
    let hidden = ctx.hidden(&owner).await;
    let topic = |community: &Community| format!("community:{}", community.id);

    assert!(guards.may_subscribe(None, &topic(&public)).await.unwrap());
    assert!(guards.may_subscribe(Some(outsider.id), &topic(&public)).await.unwrap());
    for community in [&restricted, &hidden] {
        assert!(guards.may_subscribe(Some(owner.id), &topic(community)).await.unwrap());
        assert!(!guards.may_subscribe(Some(outsider.id), &topic(community)).await.unwrap());
        assert!(!guards.may_subscribe(None, &topic(community)).await.unwrap());
        assert!(guards.may_subscribe(Some(site_moderator.id), &topic(community)).await.unwrap());
    }
    assert!(!guards.may_subscribe(Some(owner.id), "community:not-a-uuid").await.unwrap());
    assert!(!guards.may_subscribe(Some(owner.id), &format!("community:{}", Uuid::now_v7())).await.unwrap());
    // Leaving closes the topic again.
    let member = ctx.member(&owner, &restricted, "Member").await;
    assert!(guards.may_subscribe(Some(member.id), &topic(&restricted)).await.unwrap());
    let leave = LeaveCommunityRequest { community_id: restricted.id.clone(), ..Default::default() };
    let _: LeaveCommunityResponse =
        ctx.call("CommunityService", "LeaveCommunity", Some(&member), &leave).await.unwrap();
    assert!(!guards.may_subscribe(Some(member.id), &topic(&restricted)).await.unwrap());
}

#[tokio::test]
async fn an_erased_account_leaves_its_posts_and_nothing_else() {
    let ctx = start().await;
    let (owner, leaver, waiting) =
        (ctx.app.user("Owner").await, ctx.app.user("Leaver").await, ctx.app.user("Waiting").await);
    let community = ctx.public(&owner).await;
    ctx.join(&leaver, &community).await;
    ctx.join(&waiting, &community).await;
    let general = ctx.general(&owner, &community).await;
    let poll_thread = ctx
        .try_thread(
            &owner,
            &CreateThreadRequest {
                channel_id: general.id.clone(),
                title: "投票".into(),
                poll: PollInput { options: vec!["a".into(), "b".into()], ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let vote = VotePollRequest {
        thread_id: poll_thread.id.clone(),
        option_ids: vec![poll_thread.poll.options[0].id.clone()],
        ..Default::default()
    };
    let _: VotePollResponse = ctx.call(BOARD, "VotePoll", Some(&leaver), &vote).await.unwrap();
    let thread = ctx.thread(&leaver, &general, "残る書き込み", "消えない").await;
    let reply = ctx.reply(&leaver, &poll_thread, "残る返信").await;
    // A full event: the leaver holds the last place, someone waits behind them.
    let event = ctx.try_event(&owner, &CreateEventRequest { capacity: 2, ..ctx.new_event(&community) }).await.unwrap();
    ctx.rsvp(&leaver, &event, RsvpAnswer::RSVP_ANSWER_GOING).await;
    assert_eq!(
        ctx.rsvp(&waiting, &event, RsvpAnswer::RSVP_ANSWER_GOING).await.viewer.status,
        AttendanceStatus::ATTENDANCE_STATUS_WAITLISTED
    );
    let restricted = ctx.restricted(&owner).await;
    ctx.try_join(&leaver, &restricted.id, "").await.unwrap();
    let before = ctx.get(None, &community.id).await.unwrap().member_count;

    erase(&ctx, &leaver).await;
    // Delivered twice, done once.
    ctx.deliver("identity.user.deleted", Some(leaver.id), serde_json::json!({ "user_id": leaver.id })).await;

    // The membership, the place at the event and the request are gone.
    assert_eq!(ctx.get(None, &community.id).await.unwrap().member_count, before - 1);
    assert!(!ctx.community.is_member(id_of(&community.id), leaver.id).await.unwrap());
    // So is the vote — but what the poll counts does not move: a number
    // dropping at that moment would tell everyone how the account voted.
    let votes: i64 = sqlx::query_scalar("SELECT count(*) FROM community_poll_votes WHERE user_id = $1")
        .bind(leaver.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(votes, 0);
    let poll = ctx.get_thread(Some(&owner), &poll_thread.id).await.unwrap().poll.as_option().unwrap().clone();
    assert_eq!((poll.voter_count, poll.options[0].vote_count), (1, 1));
    let after = ctx.get_event(Some(&waiting), &event.id).await.unwrap();
    assert_eq!(
        (after.viewer.status, after.going_count, after.waitlist_count),
        (AttendanceStatus::ATTENDANCE_STATUS_GOING.into(), 2, 0)
    );
    let requests: ListJoinRequestsResponse = ctx
        .call(
            "CommunityService",
            "ListJoinRequests",
            Some(&owner),
            &ListJoinRequestsRequest { community_id: restricted.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert!(requests.requests.is_empty());
    assert_eq!(ctx.app.events_by("community.member.left", leaver.id).await[0].data["cause"], "account_deleted");

    // What they wrote stays, signed "deleted user".
    let kept = ctx.get_thread(None, &thread.id).await.unwrap();
    assert_eq!(kept.title, "残る書き込み");
    assert!(kept.author.deleted && kept.author.display_name.is_empty());
    assert!(ctx.get_reply(None, &reply.id).await.unwrap().author.deleted);
    assert_eq!(ctx.get_thread(None, &poll_thread.id).await.unwrap().reply_count, 1);
}

#[tokio::test]
async fn an_owned_community_passes_to_the_longest_serving_admin_or_closes() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let inherited = ctx.public(&owner).await;
    let early_member = ctx.member(&owner, &inherited, "EarlyMember").await;
    let moderator = ctx.moderator(&owner, &inherited).await;
    let first_admin = ctx.staff(&owner, &inherited, "FirstAdmin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let second_admin = ctx.staff(&owner, &inherited, "SecondAdmin", MemberRole::MEMBER_ROLE_ADMIN).await;
    let no_admins = ctx.public(&owner).await;
    let lone_member = ctx.member(&owner, &no_admins, "LoneMember").await;
    let abandoned = ctx.public(&owner).await;
    let thread = ctx.thread(&owner, &ctx.general(&owner, &abandoned).await, "だれもいない", "x").await;

    erase(&ctx, &owner).await;

    // Admins come before moderators and members, the one who joined first before the other.
    let passed = ctx.get(None, &inherited.id).await.unwrap();
    assert_eq!(passed.owner.id, first_admin.id.to_string());
    assert_eq!(ctx.viewer(&first_admin, &inherited.id).await.role, MemberRole::MEMBER_ROLE_OWNER);
    assert_eq!(ctx.viewer(&second_admin, &inherited.id).await.role, MemberRole::MEMBER_ROLE_ADMIN);
    assert_eq!(
        ctx.notifications(first_admin.id, "community.membership").await.last().unwrap()["data"]["change"],
        "ownership"
    );
    let log = ctx.moderation_log(&first_admin, &inherited).await;
    let entry =
        log.iter().find(|entry| entry.action == ModerationAction::MODERATION_ACTION_OWNERSHIP_TRANSFERRED).unwrap();
    assert!(entry.actor.as_option().is_none());
    assert_eq!(
        (entry.target_user.id.as_str(), entry.details["cause"].as_str()),
        (first_admin.id.to_string().as_str(), "account_deleted")
    );
    // Without staff, whoever has been there longest keeps the lights on.
    assert_eq!(ctx.get(None, &no_admins.id).await.unwrap().owner.id, lone_member.id.to_string());
    // Nobody left: the community closes.
    assert_eq!(ctx.get(None, &abandoned.id).await.unwrap_err().reason, "COMMUNITY_NOT_FOUND");
    assert_eq!(ctx.get_thread(None, &thread.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(ctx.community_events("community.deleted", &abandoned.id).await.len(), 1);
    let _ = (early_member, moderator);
}

#[tokio::test]
async fn site_moderation_takes_content_down() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let site_moderator = ctx.app.user_with_roles("SiteMod", &["moderator"]).await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&member, &general, "問題のスレ", "x").await;
    let keep = ctx.thread(&member, &general, "ふつうのスレ", "x").await;
    let reply = ctx.reply(&member, &keep, "問題の返信").await;
    let event = ctx.event(&owner, &community).await;
    use SubjectType::*;

    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    take_down(&ctx, &site_moderator, SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id).await;
    take_down(&ctx, &site_moderator, SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id).await;
    take_down(&ctx, &site_moderator, SUBJECT_TYPE_COMMUNITY_EVENT, &event.id).await;
    // The same event again, and one about content that never existed, change nothing.
    take_down(&ctx, &site_moderator, SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id).await;
    take_down(&ctx, &site_moderator, SUBJECT_TYPE_COMMUNITY_THREAD, &Uuid::now_v7().to_string()).await;

    assert_eq!(ctx.get_reply(None, &reply.id).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    assert_eq!(ctx.get_thread(None, &thread.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(ctx.get_event(None, &event.id).await.unwrap_err().reason, "EVENT_NOT_FOUND");
    assert_eq!(ctx.get_thread(None, &keep.id).await.unwrap().reply_count, 0);
    assert_eq!(ctx.get(None, &community.id).await.unwrap().thread_count, 1);
    let pushed = pushes(&mut topic).await;
    assert!(pushed.iter().any(|push| matches!(push, Pushed::ThreadRemoved(removed) if removed.thread_id == thread.id)));
    assert!(pushed.iter().any(|push| matches!(push, Pushed::ReplyRemoved(removed) if removed.reply_id == reply.id)));
    // The community's own log says site moderation did it; the admin domain tells the owner of the content.
    let log = ctx.moderation_log(&owner, &community).await;
    let entries: Vec<_> =
        log.iter().filter(|entry| entry.action == ModerationAction::MODERATION_ACTION_CONTENT_TAKEN_DOWN).collect();
    assert_eq!(entries.len(), 3);
    assert!(entries.iter().all(|entry| entry.site_moderation
        && entry.reason == "規約違反"
        && entry.actor.id == site_moderator.id.to_string()));
    assert!(ctx.notification_kinds(member.id).await.is_empty());

    // A whole community can be taken down.
    take_down(&ctx, &site_moderator, SUBJECT_TYPE_COMMUNITY, &community.id).await;
    assert_eq!(ctx.get(Some(&owner), &community.id).await.unwrap_err().reason, "COMMUNITY_NOT_FOUND");
}

#[tokio::test]
async fn an_upload_deleted_by_force_disappears_from_what_showed_it() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let icon = ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_AVATAR).await;
    let request = CreateCommunityRequest {
        icon_asset_id: icon.clone(),
        ..ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC)
    };
    let community = ctx.try_create(&owner, &request).await.unwrap();
    let general = ctx.general(&owner, &community).await;
    let (first, second) = (
        ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await,
        ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await,
    );
    let thread = ctx
        .try_thread(
            &owner,
            &CreateThreadRequest {
                channel_id: general.id.clone(),
                title: "写真".into(),
                asset_ids: vec![first.clone(), second.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let only_image = ctx
        .try_reply(
            &owner,
            &CreateReplyRequest { thread_id: thread.id.clone(), asset_ids: vec![first.clone()], ..Default::default() },
        )
        .await
        .unwrap();
    let with_text = ctx
        .try_reply(
            &owner,
            &CreateReplyRequest {
                thread_id: thread.id.clone(),
                body: "説明つき".into(),
                asset_ids: vec![first.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let gone =
        |asset_id: &str| serde_json::json!({ "asset_id": asset_id, "owner_id": owner.id, "purpose": 8, "kind": 1 });

    ctx.deliver("media.asset.deleted", None, gone(&first)).await;
    let seen = ctx.get_thread(None, &thread.id).await.unwrap();
    assert_eq!(seen.images.iter().map(|image| image.asset_id.clone()).collect::<Vec<_>>(), vec![second.clone()]);
    // A reply with words keeps them; one that was only the picture goes.
    assert!(ctx.get_reply(None, &with_text.id).await.unwrap().images.is_empty());
    assert_eq!(ctx.get_reply(None, &only_image.id).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    assert_eq!(seen.reply_count, 1);

    ctx.deliver("media.asset.deleted", None, gone(&icon)).await;
    assert!(ctx.get(None, &community.id).await.unwrap().icon.as_option().is_none());
    // An upload nothing here shows is none of this domain's business.
    ctx.deliver("media.asset.deleted", None, gone(&Uuid::now_v7().to_string())).await;
}

#[tokio::test]
async fn the_export_holds_what_the_domain_knows_about_a_user() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let user = ctx.member(&owner, &community, "User").await;
    let anonymous = ctx.channel(&owner, &community, "匿名", true).await;
    ctx.thread(&user, &anonymous, "匿名で書いた", "でも自分のデータ").await;
    let thread = ctx.thread(&owner, &ctx.general(&owner, &community).await, "スレ", "x").await;
    ctx.reply(&user, &thread, "返信した").await;
    let event = ctx.event(&owner, &community).await;
    ctx.rsvp(&user, &event, RsvpAnswer::RSVP_ANSWER_INTERESTED).await;

    let exporter =
        ctx.app.core.exporters.all().into_iter().find(|exporter| exporter.name() == "community").expect("the exporter");
    let data = exporter.export(user.id).await.unwrap();
    assert_eq!(data["memberships"][0]["handle"], community.handle);
    // Anonymous to everyone else, but the author's own.
    assert_eq!(data["threads"][0]["title"], "匿名で書いた");
    assert_eq!(data["threads"][0]["anonymous"], true);
    assert_eq!(data["replies"][0]["body"], "返信した");
    assert_eq!(data["event_answers"][0]["title"], "オフ会");
    assert_eq!(data["followed_threads"].as_array().unwrap().len(), 2);
    // Someone the domain knows nothing about gets empty lists, not errors.
    let nobody = exporter.export(Uuid::now_v7()).await.unwrap();
    assert!(nobody["memberships"].as_array().unwrap().is_empty() && nobody["threads"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn search_follows_the_communitys_visibility() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&owner, &general, "検索できる", "本文").await;
    let last_doc = |kind: &'static str, id: String| {
        let ctx = &ctx;
        async move { ctx.search_changes(kind, &id).await.last().cloned().unwrap() }
    };
    assert_eq!(last_doc("thread", thread.id.clone()).await["doc"]["extra"]["community_handle"], community.handle);
    let sweep = SweepJob { community_id: id_of(&community.id), channel_id: None };
    let set = |visibility: CommunityVisibility| UpdateCommunityRequest {
        community_id: community.id.clone(),
        visibility: Some(visibility.into()),
        ..Default::default()
    };

    // Restricted: the community stays findable, its threads leave the index.
    let _: UpdateCommunityResponse = ctx
        .call(
            "CommunityService",
            "UpdateCommunity",
            Some(&owner),
            &set(CommunityVisibility::COMMUNITY_VISIBILITY_RESTRICTED),
        )
        .await
        .unwrap();
    ctx.community.run_sweep(sweep.clone()).await.unwrap();
    assert!(!last_doc("community", community.id.clone()).await["doc"].is_null());
    assert!(last_doc("thread", thread.id.clone()).await["doc"].is_null());
    // Hidden: the community leaves too.
    let _: UpdateCommunityResponse = ctx
        .call(
            "CommunityService",
            "UpdateCommunity",
            Some(&owner),
            &set(CommunityVisibility::COMMUNITY_VISIBILITY_HIDDEN),
        )
        .await
        .unwrap();
    assert!(last_doc("community", community.id.clone()).await["doc"].is_null());
    // Public again: everything comes back.
    let _: UpdateCommunityResponse = ctx
        .call(
            "CommunityService",
            "UpdateCommunity",
            Some(&owner),
            &set(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC),
        )
        .await
        .unwrap();
    ctx.community.run_sweep(sweep).await.unwrap();
    assert_eq!(last_doc("community", community.id.clone()).await["doc"]["title"], community.name);
    assert_eq!(last_doc("thread", thread.id.clone()).await["doc"]["title"], "検索できる");

    // A deleted channel's threads leave the index; their uploads stay on
    // file for as long as the removed threads are retained.
    let side = ctx.channel(&owner, &community, "side", false).await;
    let image = ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let doomed = ctx
        .try_thread(
            &owner,
            &CreateThreadRequest {
                channel_id: side.id.clone(),
                title: "消えるスレ".into(),
                asset_ids: vec![image.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let _: DeleteChannelResponse = ctx
        .call(
            BOARD,
            "DeleteChannel",
            Some(&owner),
            &DeleteChannelRequest { channel_id: side.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    ctx.community
        .run_sweep(SweepJob { community_id: id_of(&community.id), channel_id: Some(id_of(&side.id)) })
        .await
        .unwrap();
    assert!(last_doc("thread", doomed.id.clone()).await["doc"].is_null());
    assert!(ctx.is_held(&image, &format!("community:thread:{}", doomed.id)).await);
    assert!(!last_doc("thread", thread.id.clone()).await["doc"].is_null());
}

#[tokio::test]
async fn maintenance_prunes_what_ran_out() {
    let ctx = start().await;
    let (owner, banned, declined) =
        (ctx.app.user("Owner").await, ctx.app.user("Banned").await, ctx.app.user("Declined").await);
    let community = ctx.restricted(&owner).await;
    let ban = BanMemberRequest {
        community_id: community.id.clone(),
        user_id: banned.id.to_string(),
        reason: "一時的".into(),
        expires_at: ts(now() + chrono::Duration::hours(1)),
        ..Default::default()
    };
    let _: BanMemberResponse = ctx.call("CommunityModerationService", "BanMember", Some(&owner), &ban).await.unwrap();
    ctx.try_join(&declined, &community.id, "").await.unwrap();
    let pending: ListJoinRequestsResponse = ctx
        .call(
            "CommunityService",
            "ListJoinRequests",
            Some(&owner),
            &ListJoinRequestsRequest { community_id: community.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let decline =
        RespondJoinRequestRequest { request_id: pending.requests[0].id.clone(), approve: false, ..Default::default() };
    let _: RespondJoinRequestResponse =
        ctx.call("CommunityService", "RespondJoinRequest", Some(&owner), &decline).await.unwrap();

    // Nothing has run out yet.
    ctx.community.run_maintenance().await.unwrap();
    assert_eq!(ctx.viewer(&banned, &community.id).await.status, MembershipStatus::MEMBERSHIP_STATUS_BANNED);
    assert_eq!(ctx.viewer(&declined, &community.id).await.status, MembershipStatus::MEMBERSHIP_STATUS_REQUESTED);

    sqlx::query("UPDATE community_bans SET expires_at = now() - interval '1 minute' WHERE user_id = $1")
        .bind(banned.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    sqlx::query("UPDATE community_join_requests SET declined_at = now() - interval '8 days' WHERE user_id = $1")
        .bind(declined.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.community.run_maintenance().await.unwrap();
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM community_bans WHERE user_id = $1) + (SELECT count(*) FROM community_join_requests WHERE user_id = $2)",
    )
    .bind(banned.id)
    .bind(declined.id)
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(left, 0);
    assert_eq!(ctx.viewer(&declined, &community.id).await.status, MembershipStatus::MEMBERSHIP_STATUS_NONE);
}

#[tokio::test]
async fn a_private_communitys_topic_carries_no_content() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let restricted = ctx.restricted(&owner).await;
    let removed = ctx.member(&owner, &restricted, "Removed").await;
    let general = ctx.general(&owner, &restricted).await;
    // The member follows the topic while they are one (the guard lets them)…
    assert!(
        ctx.app
            .core
            .topic_guards
            .may_subscribe(Some(removed.id), &format!("community:{}", restricted.id))
            .await
            .unwrap()
    );
    let mut topic = ctx.app.listen_topic(&format!("community:{}", restricted.id)).await;
    // …and is removed. The subscription is still open.
    let kick = RemoveMemberRequest {
        community_id: restricted.id.clone(),
        user_id: removed.id.to_string(),
        ..Default::default()
    };
    let _: RemoveMemberResponse =
        ctx.call("CommunityModerationService", "RemoveMember", Some(&owner), &kick).await.unwrap();

    let thread = ctx.thread(&owner, &general, "部外秘のスレ", "ここだけの本文").await;
    let reply = ctx.reply(&owner, &thread, "ここだけの返信").await;
    let edit =
        UpdateThreadRequest {
            thread_id: thread.id.clone(), body: Some("書き直した秘密".into()), ..Default::default()
        };
    let _: UpdateThreadResponse = ctx.call(BOARD, "UpdateThread", Some(&owner), &edit).await.unwrap();
    let edit =
        UpdateReplyRequest { reply_id: reply.id.clone(), body: "書き直した返信".into(), ..Default::default() };
    let _: UpdateReplyResponse = ctx.call(BOARD, "UpdateReply", Some(&owner), &edit).await.unwrap();
    let event = ctx.event(&owner, &restricted).await;

    // What still arrives says that something happened, and where to ask —
    // and asking is refused.
    let pushed = pushes(&mut topic).await;
    assert!(pushed.iter().any(|push| matches!(push, Pushed::ThreadCreated(created)
        if created.thread_id == thread.id && created.thread.as_option().is_none())));
    assert!(pushed.iter().any(|push| matches!(push, Pushed::ReplyCreated(created)
        if created.reply_id == reply.id && created.reply.as_option().is_none())));
    assert!(pushed.iter().any(|push| matches!(push, Pushed::CalendarEventChanged(changed)
        if changed.event_id == event.id && changed.event.as_option().is_none())));
    for push in &pushed {
        match push {
            Pushed::ThreadCreated(created) => assert!(created.thread.as_option().is_none()),
            Pushed::ThreadUpdated(updated) => assert!(updated.thread.as_option().is_none()),
            Pushed::ReplyCreated(created) => assert!(created.reply.as_option().is_none()),
            Pushed::ReplyUpdated(updated) => assert!(updated.reply.as_option().is_none()),
            Pushed::CalendarEventChanged(changed) => assert!(changed.event.as_option().is_none()),
            _ => {}
        }
    }
    for secret in ["部外秘", "ここだけ", "書き直した", "オフ会", "駅前"] {
        assert!(!mentions(&pushed, secret), "the topic of a private community carries content ({secret})");
    }
    assert_eq!(ctx.get_thread(Some(&removed), &thread.id).await.unwrap_err().reason, "NOT_A_MEMBER");
    assert!(ctx.get_reply(Some(&removed), &reply.id).await.is_err());
    assert!(ctx.get_event(Some(&removed), &event.id).await.is_err());

    // The same when a public community closes its doors: whoever followed
    // it before hears nothing more than IDs.
    let public = ctx.public(&owner).await;
    let lobby = ctx.general(&owner, &public).await;
    let mut passer_by = ctx.app.listen_topic(&format!("community:{}", public.id)).await;
    let open = ctx.thread(&owner, &lobby, "公開のスレ", "だれでも読める").await;
    assert!(pushes(&mut passer_by).await.iter().any(|push| matches!(push, Pushed::ThreadCreated(created)
        if created.thread_id == open.id && created.thread.as_option().is_some())));
    let close = UpdateCommunityRequest {
        community_id: public.id.clone(),
        visibility: Some(CommunityVisibility::COMMUNITY_VISIBILITY_HIDDEN.into()),
        ..Default::default()
    };
    let _: UpdateCommunityResponse =
        ctx.call("CommunityService", "UpdateCommunity", Some(&owner), &close).await.unwrap();
    let inside = ctx.thread(&owner, &lobby, "非公開になってからのスレ", "メンバーだけの本文").await;
    ctx.reply(&owner, &inside, "メンバーだけの返信").await;
    let pushed = pushes(&mut passer_by).await;
    assert!(pushed.iter().any(|push| matches!(push, Pushed::ThreadCreated(created) if created.thread_id == inside.id)));
    assert!(!mentions(&pushed, "非公開になってから") && !mentions(&pushed, "メンバーだけ"));
}

#[tokio::test]
async fn comments_and_likes_on_posts_are_for_those_who_could_write_there() {
    let ctx = start().await;
    let subjects = &ctx.app.core.subjects;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let silenced = ctx.member(&owner, &community, "Silenced").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let outsider = ctx.app.user("Outsider").await;
    let general = ctx.general(&owner, &community).await;
    let staff_only = CreateChannelRequest {
        community_id: community.id.clone(),
        name: "お知らせ".into(),
        posting: PostingPolicy::POSTING_POLICY_MODERATORS.into(),
        ..Default::default()
    };
    let notices: CreateChannelResponse = ctx.call(BOARD, "CreateChannel", Some(&owner), &staff_only).await.unwrap();
    let thread = ctx.thread(&member, &general, "みんなのスレ", "x").await;
    let reply = ctx.reply(&member, &thread, "返信").await;
    let notice = ctx.thread(&owner, notices.channel.as_option().unwrap(), "運営からのお知らせ", "x").await;
    let event = ctx.event(&owner, &community).await;
    let timeout = TimeoutMemberRequest {
        community_id: community.id.clone(),
        user_id: silenced.id.to_string(),
        duration_seconds: 600,
        reason: "頭を冷やそう".into(),
        ..Default::default()
    };
    let _: TimeoutMemberResponse =
        ctx.call("CommunityModerationService", "TimeoutMember", Some(&owner), &timeout).await.unwrap();
    use SubjectType::*;
    let open = |kind: SubjectType, id: &str, viewer: Option<&TestUser>| {
        let (id, viewer) = (id.to_owned(), viewer.map(|user| user.id));
        async move { subjects.resolve(kind, &id, viewer).await.unwrap().interactions_enabled }
    };

    // Everyone reads a public community; an engagement comment is not a
    // way to write in it without joining, or around a timeout.
    for (kind, id) in [
        (SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id),
        (SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id),
        (SUBJECT_TYPE_COMMUNITY_EVENT, &event.id),
    ] {
        assert!(open(kind, id, Some(&member)).await, "{kind:?} for a member");
        assert!(open(kind, id, Some(&moderator)).await, "{kind:?} for the staff");
        assert!(!open(kind, id, Some(&outsider)).await, "{kind:?} for someone who has not joined");
        assert!(!open(kind, id, None).await, "{kind:?} for a guest");
        assert!(!open(kind, id, Some(&silenced)).await, "{kind:?} for a timed-out member");
    }
    // Nor around "only the staff posts here".
    assert!(!open(SUBJECT_TYPE_COMMUNITY_THREAD, &notice.id, Some(&member)).await);
    assert!(open(SUBJECT_TYPE_COMMUNITY_THREAD, &notice.id, Some(&moderator)).await);
}
