//! Removed and edited content: deleting a post, rewording it or closing
//! the community must not be a way to make it disappear before anyone can
//! act on it. What was removed stays reportable, readable by the staff
//! (with what it said before each edit) and attributable to its author
//! for the retention period — counted from the removal — and is purged
//! after it.
mod common;

use common::*;
use tokuly_proto::{
    common::SubjectType,
    community::{content_ref::Content, get_content_history_response::Current, *},
    media::AssetPurpose,
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

const BOARD: &str = "CommunityBoardService";
const MODERATION: &str = "CommunityModerationService";
const EVENTS: &str = "CommunityEventService";
const REMOVED_POST: &str = "削除された投稿 (deleted post)";
const REMOVED_EVENT: &str = "削除されたイベント (deleted event)";
const CLOSED_COMMUNITY: &str = "閉鎖されたコミュニティ (closed community)";

fn id_of(value: &str) -> Uuid {
    value.parse().expect("a UUID")
}

fn thread_of(thread: &Thread) -> Content {
    Content::ThreadId(thread.id.clone())
}

fn reply_of(reply: &Reply) -> Content {
    Content::ReplyId(reply.id.clone())
}

fn event_of(event: &CalendarEvent) -> Content {
    Content::EventId(event.id.clone())
}

/// `GetContentHistory` as `caller`.
async fn history(ctx: &Ctx, caller: &TestUser, content: Content) -> Result<GetContentHistoryResponse, RpcError> {
    let request = GetContentHistoryRequest {
        content: ContentRef { content: Some(content), ..Default::default() }.into(),
        ..Default::default()
    };
    ctx.call(MODERATION, "GetContentHistory", Some(caller), &request).await
}

async fn delete_thread(ctx: &Ctx, caller: &TestUser, thread: &Thread) {
    let request = DeleteThreadRequest { thread_id: thread.id.clone(), ..Default::default() };
    let _: DeleteThreadResponse = ctx.call(BOARD, "DeleteThread", Some(caller), &request).await.unwrap();
}

async fn delete_reply(ctx: &Ctx, caller: &TestUser, reply: &Reply) {
    let request = DeleteReplyRequest { reply_id: reply.id.clone(), ..Default::default() };
    let _: DeleteReplyResponse = ctx.call(BOARD, "DeleteReply", Some(caller), &request).await.unwrap();
}

async fn edit_thread(
    ctx: &Ctx,
    author: &TestUser,
    thread: &Thread,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<Thread, RpcError> {
    let request = UpdateThreadRequest {
        thread_id: thread.id.clone(),
        title: title.map(str::to_owned),
        body: body.map(str::to_owned),
        ..Default::default()
    };
    let response: UpdateThreadResponse = ctx.call(BOARD, "UpdateThread", Some(author), &request).await?;
    Ok(response.thread.as_option().expect("a thread").clone())
}

/// Pretends something was removed `days` ago.
async fn removed_days_ago(ctx: &Ctx, table: &str, id: &str, days: i32) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE {table} SET deleted_at = now() - make_interval(days => $2) WHERE id = $1"
    )))
    .bind(id_of(id))
    .bind(days)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
}

fn texts(response: &GetContentHistoryResponse) -> Vec<(&str, &str)> {
    response.revisions.iter().map(|revision| (revision.title.as_str(), revision.body.as_str())).collect()
}

#[tokio::test]
async fn a_reworded_and_deleted_thread_stays_reportable_and_readable_by_the_staff() {
    let ctx = start().await;
    let subjects = &ctx.app.core.subjects;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.restricted(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let reader = ctx.member(&owner, &community, "Reader").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let outsider = ctx.app.user("Outsider").await;
    let site_moderator = ctx.app.user_with_roles("SiteMod", &["moderator"]).await;
    let general = ctx.general(&owner, &community).await;
    let image = ctx.image(&author, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let request = CreateThreadRequest {
        channel_id: general.id.clone(),
        title: "ひどいスレ".into(),
        body: "最初の悪口".into(),
        asset_ids: vec![image.clone()],
        ..Default::default()
    };
    let thread = ctx.try_thread(&author, &request).await.unwrap();
    let reply = ctx.reply(&author, &thread, "返信でも悪口").await;

    // The usual way out: reword it, reword it again, then delete it.
    edit_thread(&ctx, &author, &thread, None, Some("少しましな文")).await.unwrap();
    let edited = edit_thread(&ctx, &author, &thread, Some("ふつうのスレ"), Some("何もなかった")).await.unwrap();
    assert!(edited.edited_at.as_option().is_some());
    // Saving the same words again is not an edit.
    edit_thread(&ctx, &author, &thread, Some("ふつうのスレ"), Some("何もなかった")).await.unwrap();
    delete_thread(&ctx, &author, &thread).await;

    // For the members it is gone, with its replies.
    assert_eq!(ctx.get_thread(Some(&reader), &thread.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(ctx.get_reply(Some(&reader), &reply.id).await.unwrap_err().reason, "REPLY_NOT_FOUND");

    // It can still be reported by whoever could read it: the subject
    // resolves, to its author, showing nothing of what it said.
    use SubjectType::*;
    let subject = subjects.resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, Some(reader.id)).await.unwrap();
    assert_eq!((subject.id, subject.owner_id), (id_of(&thread.id), author.id));
    assert_eq!(subject.title, REMOVED_POST);
    assert!(subject.image_url.is_none() && !subject.interactions_enabled);
    let as_reply = subjects.resolve(SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id, Some(reader.id)).await.unwrap();
    assert_eq!((as_reply.owner_id, as_reply.title.as_str()), (author.id, REMOVED_POST));
    // Not by those who never could.
    for viewer in [Some(outsider.id), None] {
        assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, viewer).await.is_err());
        assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id, viewer).await.is_err());
    }

    // The staff reads what it says and what it said.
    let seen = history(&ctx, &moderator, thread_of(&thread)).await.unwrap();
    match &seen.current {
        Some(Current::Thread(current)) => {
            assert_eq!((current.title.as_str(), current.body.as_str()), ("ふつうのスレ", "何もなかった"));
            assert_eq!(current.author.id, author.id.to_string());
            assert_eq!(current.images.len(), 1, "the picture is still on file");
        }
        other => panic!("expected the thread, got {other:?}"),
    }
    assert!(seen.removed_at.as_option().is_some());
    assert_eq!(texts(&seen), vec![("ひどいスレ", "最初の悪口"), ("ひどいスレ", "少しましな文")]);
    let seen = history(&ctx, &moderator, reply_of(&reply)).await.unwrap();
    assert!(matches!(&seen.current, Some(Current::Reply(current)) if current.body == "返信でも悪口"));
    assert!(seen.removed_at.as_option().is_some() && seen.revisions.is_empty());
    // So does site moderation, which is where reports end up.
    assert_eq!(history(&ctx, &site_moderator, thread_of(&thread)).await.unwrap().revisions.len(), 2);
    assert!(ctx.is_held(&image, &format!("community:thread:{}", thread.id)).await);

    // Nobody else does: to a member a removed post does not exist, and the
    // history of a post that is there is not theirs to read.
    for member in [&reader, &author] {
        assert_eq!(history(&ctx, member, thread_of(&thread)).await.unwrap_err().reason, "THREAD_NOT_FOUND");
        assert_eq!(history(&ctx, member, reply_of(&reply)).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    }
    assert_eq!(history(&ctx, &outsider, thread_of(&thread)).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    let live = ctx.thread(&author, &general, "まだあるスレ", "x").await;
    assert_eq!(history(&ctx, &reader, thread_of(&live)).await.unwrap_err().reason, "PERMISSION_DENIED");
    let seen = history(&ctx, &moderator, thread_of(&live)).await.unwrap();
    assert!(seen.removed_at.as_option().is_none() && seen.revisions.is_empty());
    let nothing = GetContentHistoryRequest::default();
    assert_eq!(ctx.reason(MODERATION, "GetContentHistory", Some(&moderator), &nothing).await, "CONTENT_REQUIRED");
    assert_eq!(ctx.reason(MODERATION, "GetContentHistory", None, &nothing).await, "SIGN_IN_REQUIRED");

    // And the author is still there to be dealt with.
    let ban = BanMemberRequest {
        community_id: community.id.clone(),
        user_id: author.id.to_string(),
        reason: "悪口を書いて消した".into(),
        ..Default::default()
    };
    let _: BanMemberResponse = ctx.call(MODERATION, "BanMember", Some(&moderator), &ban).await.unwrap();
    assert_eq!(ctx.viewer(&author, &community.id).await.status, MembershipStatus::MEMBERSHIP_STATUS_BANNED);
}

#[tokio::test]
async fn deleting_an_anonymous_post_does_not_put_its_author_out_of_reach() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Zebediah Secretkeeper").await;
    let reader = ctx.member(&owner, &community, "Reader").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let channel = ctx.channel(&owner, &community, "ないしょ話", true).await;
    let thread = ctx.thread(&author, &channel, "匿名のいじめ", "ひどい内容").await;
    let others = ctx.thread(&reader, &channel, "別の人のスレ", "x").await;
    let reply = ctx.reply(&author, &others, "ここにもひどい返信").await;
    // Written, seen, deleted.
    delete_reply(&ctx, &author, &reply).await;
    delete_thread(&ctx, &author, &thread).await;
    let reveal = |post, reason: &str| RevealAuthorRequest { post, reason: reason.into(), ..Default::default() };

    // The staff still finds out who it was, and the lookup is still logged.
    let revealed: RevealAuthorResponse = ctx
        .call(MODERATION, "RevealAuthor", Some(&moderator), &reveal(thread_ref(&thread.id), "消された書き込みの通報"))
        .await
        .unwrap();
    assert_eq!(revealed.author.id, author.id.to_string());
    let of_reply: RevealAuthorResponse = ctx
        .call(MODERATION, "RevealAuthor", Some(&owner), &reveal(reply_ref(&reply.id), "同じ人か確認"))
        .await
        .unwrap();
    assert_eq!(of_reply.author.id, author.id.to_string());
    let log = ctx.moderation_log(&owner, &community).await;
    assert_eq!(
        log.iter().filter(|entry| entry.action == ModerationAction::MODERATION_ACTION_AUTHOR_REVEALED).count(),
        2
    );
    // To members the removed posts do not exist, whoever wrote them.
    for member in [&reader, &author] {
        assert_eq!(
            ctx.reason(MODERATION, "RevealAuthor", Some(member), &reveal(thread_ref(&thread.id), "x")).await,
            "THREAD_NOT_FOUND"
        );
        assert_eq!(
            ctx.reason(MODERATION, "RevealAuthor", Some(member), &reveal(reply_ref(&reply.id), "x")).await,
            "REPLY_NOT_FOUND"
        );
    }

    // What the staff reads of it is still signed with the pseudonym only.
    let seen = history(&ctx, &moderator, thread_of(&thread)).await.unwrap();
    assert!(matches!(&seen.current, Some(Current::Thread(current))
        if current.author.as_option().is_none() && current.pseudonym.as_option() == thread.pseudonym.as_option()));
    for needle in [author.id.to_string(), author.handle.clone(), "Zebediah".to_owned()] {
        assert!(!mentions(&seen, &needle), "the history names the author ({needle})");
        assert!(!mentions(&log, &needle), "the log names the author ({needle})");
    }
    // Reports name nobody; site moderation asks the domain.
    let subject = ctx
        .app
        .core
        .subjects
        .resolve(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, Some(reader.id))
        .await
        .unwrap();
    assert!(subject.owner_id.is_nil());
    let asked = ctx
        .community
        .author_for_moderation(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, id_of(&thread.id))
        .await
        .unwrap();
    assert_eq!(asked, Some(author.id));

    // So the author can be banned like anyone else.
    let ban = BanMemberRequest {
        community_id: community.id.clone(),
        user_id: author.id.to_string(),
        reason: "いじめ".into(),
        ..Default::default()
    };
    let _: BanMemberResponse = ctx.call(MODERATION, "BanMember", Some(&moderator), &ban).await.unwrap();
    assert!(!ctx.community.is_member(id_of(&community.id), author.id).await.unwrap());
}

#[tokio::test]
async fn posts_and_events_are_reworded_only_so_often() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let general = ctx.general(&owner, &community).await;
    let thread = ctx.thread(&owner, &general, "何度も書き直すスレ", "x").await;
    let reply = ctx.reply(&owner, &thread, "何度も書き直す返信").await;
    let event = ctx.event(&owner, &community).await;
    // Thirty rewordings have happened.
    for (table, id) in
        [("community_threads", &thread.id), ("community_replies", &reply.id), ("community_events", &event.id)]
    {
        sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE {table} SET edit_count = 30 WHERE id = $1")))
            .bind(id_of(id))
            .execute(&ctx.app.core.db)
            .await
            .unwrap();
    }

    assert_eq!(edit_thread(&ctx, &owner, &thread, None, Some("31回目")).await.unwrap_err().reason, "TOO_MANY_EDITS");
    assert_eq!(
        edit_thread(&ctx, &owner, &thread, Some("31回目の題"), None).await.unwrap_err().reason,
        "TOO_MANY_EDITS"
    );
    let reword = UpdateReplyRequest { reply_id: reply.id.clone(), body: "31回目".into(), ..Default::default() };
    assert_eq!(ctx.reason(BOARD, "UpdateReply", Some(&owner), &reword).await, "TOO_MANY_EDITS");
    let retitle =
        UpdateEventRequest { event_id: event.id.clone(), title: Some("31回目".into()), ..Default::default() };
    assert_eq!(ctx.reason(EVENTS, "UpdateEvent", Some(&owner), &retitle).await, "TOO_MANY_EDITS");
    let move_place = UpdateEventRequest {
        event_id: event.id.clone(),
        place: EventPlace { name: "別の場所".into(), ..Default::default() }.into(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(EVENTS, "UpdateEvent", Some(&owner), &move_place).await, "TOO_MANY_EDITS");

    // What is not a rewording still works: tags, the same words again, an event's size.
    let retag = UpdateThreadRequest {
        thread_id: thread.id.clone(),
        tags: StringList { values: vec!["meta".into()], ..Default::default() }.into(),
        ..Default::default()
    };
    let retagged: UpdateThreadResponse = ctx.call(BOARD, "UpdateThread", Some(&owner), &retag).await.unwrap();
    assert_eq!(retagged.thread.tags, vec!["meta"]);
    edit_thread(&ctx, &owner, &thread, Some("何度も書き直すスレ"), Some("x")).await.unwrap();
    let resize = UpdateEventRequest { event_id: event.id.clone(), capacity: Some(10), ..Default::default() };
    let resized: UpdateEventResponse = ctx.call(EVENTS, "UpdateEvent", Some(&owner), &resize).await.unwrap();
    assert_eq!(resized.event.capacity, 10);
    assert!(
        history(&ctx, &owner, thread_of(&thread)).await.unwrap().revisions.is_empty(),
        "none of that left a revision"
    );
}

#[tokio::test]
async fn a_removed_event_keeps_what_it_said() {
    let ctx = start().await;
    let subjects = &ctx.app.core.subjects;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let cover = ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_BANNER).await;
    let event = ctx
        .try_event(&owner, &CreateEventRequest { cover_asset_id: cover.clone(), ..ctx.new_event(&community) })
        .await
        .unwrap();
    let update = |request: UpdateEventRequest| {
        let (ctx, owner) = (&ctx, &owner);
        async move {
            let _: UpdateEventResponse = ctx.call(EVENTS, "UpdateEvent", Some(owner), &request).await.unwrap();
        }
    };
    update(UpdateEventRequest {
        event_id: event.id.clone(),
        title: Some("悪口イベント".into()),
        description: Some("ひどい説明".into()),
        ..Default::default()
    })
    .await;
    update(UpdateEventRequest {
        event_id: event.id.clone(),
        place: EventPlace { name: "ひどい場所".into(), ..Default::default() }.into(),
        ..Default::default()
    })
    .await;
    update(UpdateEventRequest {
        event_id: event.id.clone(),
        title: Some("オフ会".into()),
        description: Some("みんなで集まろう".into()),
        ..Default::default()
    })
    .await;
    // More places is not a rewording.
    update(UpdateEventRequest { event_id: event.id.clone(), capacity: Some(20), ..Default::default() }).await;
    let delete = DeleteEventRequest { event_id: event.id.clone(), ..Default::default() };
    let _: DeleteEventResponse = ctx.call(EVENTS, "DeleteEvent", Some(&owner), &delete).await.unwrap();

    assert_eq!(ctx.get_event(Some(&member), &event.id).await.unwrap_err().reason, "EVENT_NOT_FOUND");
    let subject =
        subjects.resolve(SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT, &event.id, Some(member.id)).await.unwrap();
    assert_eq!(
        (subject.owner_id, subject.title.as_str(), subject.interactions_enabled),
        (owner.id, REMOVED_EVENT, false)
    );

    let seen = history(&ctx, &moderator, event_of(&event)).await.unwrap();
    assert!(
        matches!(&seen.current, Some(Current::Event(current)) if current.title == "オフ会" && current.cover.asset_id == cover)
    );
    assert!(seen.removed_at.as_option().is_some());
    // Title, description, place: each version, oldest first.
    assert_eq!(seen.revisions.len(), 3);
    assert_eq!(texts(&seen)[0], ("オフ会", "みんなで集まろう\n駅前のカフェ"));
    assert_eq!(texts(&seen)[1], ("悪口イベント", "ひどい説明\n駅前のカフェ"));
    assert_eq!(texts(&seen)[2], ("悪口イベント", "ひどい説明\nひどい場所"));
    assert!(ctx.is_held(&cover, &format!("community:event:{}", event.id)).await);
    assert_eq!(history(&ctx, &member, event_of(&event)).await.unwrap_err().reason, "EVENT_NOT_FOUND");
    let live = ctx.event(&owner, &community).await;
    assert_eq!(history(&ctx, &member, event_of(&live)).await.unwrap_err().reason, "PERMISSION_DENIED");
}

#[tokio::test]
async fn removed_content_is_purged_once_its_retention_is_over() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let author = ctx.member(&owner, &community, "Author").await;
    let general = ctx.general(&owner, &community).await;
    let image = |user: &TestUser, purpose: AssetPurpose| {
        let (ctx, user) = (&ctx, user.clone());
        async move { ctx.image(&user, purpose).await }
    };
    let (thread_image, reply_image, lone_image) = (
        image(&author, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await,
        image(&author, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await,
        image(&author, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await,
    );
    let cover = image(&owner, AssetPurpose::ASSET_PURPOSE_BANNER).await;
    let with_image = |title: &str, asset: &str| CreateThreadRequest {
        channel_id: general.id.clone(),
        title: title.into(),
        body: "本文".into(),
        asset_ids: vec![asset.to_owned()],
        ..Default::default()
    };
    let thread = ctx.try_thread(&author, &with_image("消えるスレ", &thread_image)).await.unwrap();
    edit_thread(&ctx, &author, &thread, None, Some("書き直した本文")).await.unwrap();
    let inside = ctx
        .try_reply(
            &author,
            &CreateReplyRequest {
                thread_id: thread.id.clone(),
                body: "スレと一緒に消える".into(),
                asset_ids: vec![reply_image.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let kept = ctx.thread(&owner, &general, "残るスレ", "x").await;
    let lone = ctx
        .try_reply(
            &author,
            &CreateReplyRequest {
                thread_id: kept.id.clone(),
                body: "これだけ消える".into(),
                asset_ids: vec![lone_image.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let staying = ctx.reply(&author, &kept, "これは残る").await;
    let event = ctx
        .try_event(&owner, &CreateEventRequest { cover_asset_id: cover.clone(), ..ctx.new_event(&community) })
        .await
        .unwrap();
    delete_thread(&ctx, &author, &thread).await;
    delete_reply(&ctx, &author, &lone).await;
    let _: DeleteEventResponse = ctx
        .call(
            EVENTS,
            "DeleteEvent",
            Some(&owner),
            &DeleteEventRequest { event_id: event.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let holds = || {
        let ctx = &ctx;
        let holders = [
            (thread_image.clone(), format!("community:thread:{}", thread.id)),
            (reply_image.clone(), format!("community:reply:{}", inside.id)),
            (lone_image.clone(), format!("community:reply:{}", lone.id)),
            (cover.clone(), format!("community:event:{}", event.id)),
        ];
        async move {
            let mut held = Vec::new();
            for (asset, holder) in &holders {
                held.push(ctx.is_held(asset, holder).await);
            }
            held
        }
    };
    let targets = [("community_threads", &thread.id), ("community_replies", &lone.id), ("community_events", &event.id)];

    // The clock starts at the removal, not at the writing: a year-old post
    // deleted today is kept as long as one written today.
    sqlx::query("UPDATE community_threads SET created_at = now() - interval '400 days' WHERE id = $1")
        .bind(id_of(&thread.id))
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.community.purge_removed().await.unwrap();
    assert_eq!(holds().await, vec![true; 4]);
    for (table, id) in targets {
        removed_days_ago(&ctx, table, id, 29).await;
    }
    ctx.community.purge_removed().await.unwrap();
    assert_eq!(holds().await, vec![true; 4]);
    assert_eq!(history(&ctx, &owner, thread_of(&thread)).await.unwrap().revisions.len(), 1);

    // Past the retention (30 days), maintenance purges it.
    for (table, id) in targets {
        removed_days_ago(&ctx, table, id, 31).await;
    }
    assert!(!ctx.app.subject_deleted(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id).await);
    ctx.community.run_maintenance().await.unwrap();
    assert_eq!(holds().await, vec![false; 4], "the uploads are released");
    // Only now are they gone for the domains that attached things to them.
    for (kind, id) in [
        (SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id),
        (SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY, &inside.id),
        (SubjectType::SUBJECT_TYPE_COMMUNITY_REPLY, &lone.id),
        (SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT, &event.id),
    ] {
        assert!(ctx.app.subject_deleted(kind, id).await, "{kind:?}");
    }
    assert_eq!(history(&ctx, &owner, thread_of(&thread)).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(history(&ctx, &owner, reply_of(&inside)).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    assert_eq!(history(&ctx, &owner, reply_of(&lone)).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    assert_eq!(history(&ctx, &owner, event_of(&event)).await.unwrap_err().reason, "EVENT_NOT_FOUND");
    let subjects = &ctx.app.core.subjects;
    assert!(subjects.resolve(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, Some(owner.id)).await.is_err());
    assert!(subjects.resolve(SubjectType::SUBJECT_TYPE_COMMUNITY_EVENT, &event.id, Some(owner.id)).await.is_err());
    let asked = ctx
        .community
        .author_for_moderation(SubjectType::SUBJECT_TYPE_COMMUNITY_THREAD, id_of(&thread.id))
        .await
        .unwrap();
    assert_eq!(asked, None);
    // Nothing of what it said is left in the database.
    let (title, body): (String, String) = sqlx::query_as("SELECT title, body FROM community_threads WHERE id = $1")
        .bind(id_of(&thread.id))
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!((title.as_str(), body.as_str()), ("", ""));
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM community_replies WHERE id = ANY($1) AND body <> '')
              + (SELECT count(*) FROM community_revisions WHERE content_id = ANY($2))",
    )
    .bind(vec![id_of(&inside.id), id_of(&lone.id)])
    .bind(vec![id_of(&thread.id), id_of(&event.id)])
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(left, 0);
    // A second pass finds nothing more to do, and what was not removed is untouched.
    ctx.community.purge_removed().await.unwrap();
    let seen = ctx.get_thread(None, &kept.id).await.unwrap();
    assert_eq!((seen.title.as_str(), seen.reply_count), ("残るスレ", 1));
    assert_eq!(ctx.get_reply(None, &staying.id).await.unwrap().body, "これは残る");
}

#[tokio::test]
async fn closing_a_community_does_not_destroy_what_was_in_it() {
    let ctx = start().await;
    let subjects = &ctx.app.core.subjects;
    let owner = ctx.app.user("Owner").await;
    let site_moderator = ctx.app.user_with_roles("SiteMod", &["moderator"]).await;
    let icon = ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_AVATAR).await;
    let request = CreateCommunityRequest {
        icon_asset_id: icon.clone(),
        ..ctx.new_community(CommunityVisibility::COMMUNITY_VISIBILITY_PUBLIC)
    };
    let community = ctx.try_create(&owner, &request).await.unwrap();
    let member = ctx.member(&owner, &community, "Member").await;
    let general = ctx.general(&owner, &community).await;
    let image = ctx.image(&owner, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await;
    let thread = ctx
        .try_thread(
            &owner,
            &CreateThreadRequest {
                channel_id: general.id.clone(),
                title: "通報されたスレ".into(),
                asset_ids: vec![image.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let reply = ctx.reply(&member, &thread, "被害を受けた人の返信").await;

    // The owner closes the community the moment the reports come in.
    let close = DeleteCommunityRequest { community_id: community.id.clone(), ..Default::default() };
    let _: DeleteCommunityResponse =
        ctx.call("CommunityService", "DeleteCommunity", Some(&owner), &close).await.unwrap();
    assert_eq!(ctx.get(Some(&member), &community.id).await.unwrap_err().reason, "COMMUNITY_NOT_FOUND");
    assert_eq!(ctx.get_thread(Some(&member), &thread.id).await.unwrap_err().reason, "THREAD_NOT_FOUND");

    // It can still be reported, as a whole and post by post…
    use SubjectType::*;
    let as_community = subjects.resolve(SUBJECT_TYPE_COMMUNITY, &community.id, Some(member.id)).await.unwrap();
    assert_eq!((as_community.owner_id, as_community.title.as_str()), (owner.id, CLOSED_COMMUNITY));
    let as_thread = subjects.resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, Some(member.id)).await.unwrap();
    assert_eq!((as_thread.owner_id, as_thread.title.as_str()), (owner.id, REMOVED_POST));
    assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY_REPLY, &reply.id, Some(member.id)).await.is_ok());
    // …and site moderation reads it. Its former staff does not: there is no staff of a closed community.
    let seen = history(&ctx, &site_moderator, thread_of(&thread)).await.unwrap();
    assert!(matches!(&seen.current, Some(Current::Thread(current)) if current.title == "通報されたスレ"));
    assert!(seen.removed_at.as_option().is_some());
    assert_eq!(history(&ctx, &owner, thread_of(&thread)).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert!(ctx.is_held(&icon, &format!("community:icon:{}", community.id)).await);
    assert!(ctx.is_held(&image, &format!("community:thread:{}", thread.id)).await);

    // Retention runs from the closing; after it everything goes.
    ctx.community.purge_removed().await.unwrap();
    assert!(history(&ctx, &site_moderator, thread_of(&thread)).await.is_ok());
    removed_days_ago(&ctx, "communities", &community.id, 31).await;
    ctx.community.purge_removed().await.unwrap();
    assert_eq!(history(&ctx, &site_moderator, thread_of(&thread)).await.unwrap_err().reason, "THREAD_NOT_FOUND");
    assert_eq!(history(&ctx, &site_moderator, reply_of(&reply)).await.unwrap_err().reason, "REPLY_NOT_FOUND");
    assert!(!ctx.is_held(&icon, &format!("community:icon:{}", community.id)).await);
    assert!(!ctx.is_held(&image, &format!("community:thread:{}", thread.id)).await);
    assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY, &community.id, Some(member.id)).await.is_err());
    assert!(subjects.resolve(SUBJECT_TYPE_COMMUNITY_THREAD, &thread.id, Some(member.id)).await.is_err());
    // Nothing is left that says who was part of it.
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM communities WHERE id = $1)
              + (SELECT count(*) FROM community_members WHERE community_id = $1)
              + (SELECT count(*) FROM community_moderation_log WHERE community_id = $1)
              + (SELECT count(*) FROM community_threads WHERE community_id = $1)",
    )
    .bind(id_of(&community.id))
    .fetch_one(&ctx.app.core.db)
    .await
    .unwrap();
    assert_eq!(left, 0);
}
