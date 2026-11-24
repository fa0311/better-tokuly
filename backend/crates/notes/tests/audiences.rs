//! Audiences from the social graph: followers, close friends, circles,
//! direct notes, locked accounts, renote rules and the following timeline.
mod common;

use common::*;
use tokuly_proto::{
    common::Visibility,
    identity::{GetProfileRequest, GetProfileResponse, get_profile_request::By},
    notes::*,
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// Everything a viewer can find out about `author`'s notes without knowing
/// any note ID: profile tabs, the note count, the global timeline, a
/// hashtag, completion and search. Note IDs are replaced by their texts.
async fn what_one_learns(ctx: &Ctx, viewer: Option<&TestUser>, author: &TestUser, tag: &str) -> serde_json::Value {
    let mut tabs = Vec::new();
    for filter in [
        UserTimelineFilter::USER_TIMELINE_FILTER_NOTES,
        UserTimelineFilter::USER_TIMELINE_FILTER_WITH_REPLIES,
        UserTimelineFilter::USER_TIMELINE_FILTER_MEDIA,
        UserTimelineFilter::USER_TIMELINE_FILTER_LIKES,
    ] {
        let request = GetUserTimelineRequest {
            user_id: author.id.to_string(),
            filter: filter.into(),
            page: page(100, ""),
            ..Default::default()
        };
        let response: GetUserTimelineResponse = ctx.note("GetUserTimeline", viewer, &request).await.unwrap();
        tabs.push(texts(&response.entries).join("|"));
        tabs.push(response.pinned_note.as_option().map(|note| note.text.clone()).unwrap_or_default());
    }
    let profile: GetProfileResponse = ctx
        .app
        .call(
            "tokuly.identity.v1.ProfileService/GetProfile",
            viewer,
            &GetProfileRequest { by: Some(By::UserId(author.id.to_string())), ..Default::default() },
        )
        .await
        .unwrap();
    let global: GetGlobalTimelineResponse = ctx
        .note("GetGlobalTimeline", viewer, &GetGlobalTimelineRequest { page: page(100, ""), ..Default::default() })
        .await
        .unwrap();
    let in_global: Vec<String> = global
        .entries
        .iter()
        .filter(|entry| entry.note.author.id == author.id.to_string())
        .map(|entry| entry.note.text.clone())
        .collect();
    let hashtag: GetHashtagTimelineResponse = ctx
        .note(
            "GetHashtagTimeline",
            viewer,
            &GetHashtagTimelineRequest { tag: tag.to_owned(), page: page(100, ""), ..Default::default() },
        )
        .await
        .unwrap();
    let completed: SearchHashtagsResponse = ctx
        .note("SearchHashtags", viewer, &SearchHashtagsRequest { prefix: tag.to_owned(), ..Default::default() })
        .await
        .unwrap();
    serde_json::json!({
        "tabs": tabs,
        "note_count": profile.profile.stats.note_count,
        "global": in_global,
        "hashtag": [hashtag.hashtag.note_count.to_string(), texts(&hashtag.entries).join("|")],
        "completion": completed.hashtags.iter().map(|tag| tag.note_count).collect::<Vec<_>>(),
    })
}

async fn hashtag_count(ctx: &Ctx, tag: &str) -> i64 {
    sqlx::query_scalar("SELECT note_count FROM hashtags WHERE tag = $1")
        .bind(tag)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap()
}

/// The search document of every note of `author` that is in the index.
async fn indexed_notes(ctx: &Ctx, author: &TestUser) -> Vec<Uuid> {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM notes WHERE author_id = $1 ORDER BY id")
        .bind(author.id)
        .fetch_all(&ctx.app.core.db)
        .await
        .unwrap();
    let mut indexed = Vec::new();
    for note_id in ids {
        if ctx.is_indexed("note", note_id).await {
            indexed.push(note_id);
        }
    }
    indexed
}

#[tokio::test]
async fn a_stranger_learns_nothing_from_a_locked_accounts_notes() {
    let ctx = start().await;
    let stranger = ctx.member("stranger").await;
    let tag = unique("locked");
    // Two worlds that differ only in what a locked account posted publicly.
    let (posted, silent) = (ctx.member("posted").await, ctx.member("silent").await);
    let follower = ctx.member("follower").await;
    for world in [&posted, &silent] {
        ctx.follow(&follower, world).await;
        ctx.set_locked(world, true).await;
    }
    let note = ctx.post(&posted, &format!("鍵アカの日記 #{tag}")).await;
    ctx.reply(&posted, &note, &format!("つづき #{tag}")).await;
    let picture = ctx.note_image(&posted).await;
    ctx.post_with(&posted, "写真", |request| request.asset_ids = vec![picture]).await;
    assert_eq!(note.visibility, buffa::EnumValue::from(Visibility::VISIBILITY_PUBLIC), "still shared publicly");
    let _: PinNoteResponse = ctx
        .note("PinNote", Some(&posted), &PinNoteRequest { note_id: note.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let _: LikeNoteResponse = ctx
        .note("LikeNote", Some(&posted), &LikeNoteRequest { note_id: note.id.clone(), ..Default::default() })
        .await
        .unwrap();

    for viewer in [Some(&stranger), None] {
        let seen = what_one_learns(&ctx, viewer, &posted, &tag).await;
        let other = what_one_learns(&ctx, viewer, &silent, &tag).await;
        assert_eq!(seen, other, "the two worlds look the same from outside");
        assert_eq!(seen["note_count"], 0);
    }
    for viewer in [Some(&stranger), None] {
        assert_eq!(ctx.get_note(viewer, &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    }
    assert!(indexed_notes(&ctx, &posted).await.is_empty(), "nothing in search");
    let reason = ctx
        .note_reason("LikeNote", Some(&stranger), &LikeNoteRequest { note_id: note.id.clone(), ..Default::default() })
        .await;
    assert_eq!(reason, "NOTE_NOT_FOUND");

    // The follower sees everything, but cannot pass it on to the public.
    let seen = what_one_learns(&ctx, Some(&follower), &posted, &tag).await;
    assert_eq!(seen["note_count"], 2);
    assert_eq!(seen["global"], serde_json::json!([]), "the global timeline is for notes everyone may see");
    let shown = ctx.get_note(Some(&follower), &note.id).await.unwrap();
    assert!(!shown.viewer.can_renote && !shown.viewer.can_quote);
    let renote = RenoteNoteRequest { note_id: note.id.clone(), ..Default::default() };
    assert_eq!(ctx.note_reason("RenoteNote", Some(&follower), &renote).await, "NOTE_NOT_RENOTABLE");
    assert_eq!(ctx.post_reason(&follower, "quote", |r| r.quote_of_id = note.id.clone()).await, "NOTE_NOT_PUBLIC");
    // The author may renote their own note to their own audience.
    let own = ctx.get_note(Some(&posted), &note.id).await.unwrap();
    assert!(own.viewer.can_renote);
    ctx.renote(&posted, &note).await;
    assert_eq!(ctx.get_note(Some(&stranger), &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
}

#[tokio::test]
async fn locking_applies_at_once_and_unlocking_lists_the_notes_again() {
    let ctx = start().await;
    let (author, follower, stranger) =
        (ctx.member("author").await, ctx.member("follower").await, ctx.member("stranger").await);
    ctx.follow(&follower, &author).await;
    let tag = unique("lock");
    let note = ctx.post(&author, &format!("public for now #{tag}")).await;
    let note_id = id(&note.id);
    assert!(ctx.get_note(Some(&stranger), &note.id).await.is_ok());
    assert!(ctx.is_indexed("note", note_id).await);
    assert_eq!(hashtag_count(&ctx, &tag).await, 1);

    // Locked: hidden from strangers before this domain even heard of it.
    ctx.set_locked(&author, true).await;
    assert_eq!(ctx.get_note(Some(&stranger), &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    assert_eq!(ctx.get_note(None, &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    let global = |viewer: Option<TestUser>| {
        let ctx = &ctx;
        async move {
            let response: GetGlobalTimelineResponse = ctx
                .note(
                    "GetGlobalTimeline",
                    viewer.as_ref(),
                    &GetGlobalTimelineRequest { page: page(100, ""), ..Default::default() },
                )
                .await
                .unwrap();
            texts(&response.entries).iter().map(|text| text.to_string()).collect::<Vec<_>>()
        }
    };
    assert!(!global(Some(stranger.clone())).await.contains(&note.text));
    assert!(ctx.get_note(Some(&follower), &note.id).await.is_ok(), "followers still see it");

    // The event arrives (twice): out of search and the hashtag counts.
    ctx.deliver_lock_change(&author).await;
    ctx.deliver_lock_change(&author).await;
    assert!(!ctx.is_indexed("note", note_id).await);
    assert_eq!(hashtag_count(&ctx, &tag).await, 0);
    let changes = ctx.app.events_by("notes.note.visibility_changed", author.id).await;
    assert_eq!(changes.len(), 1, "one change per note, however often the event arrives");
    assert_eq!(changes[0].data["visibility"], 6, "in effect: followers");
    assert_eq!(changes[0].data["visible_to"], serde_json::json!([]), "nobody was told about it");

    // Unlocked: back for everyone.
    ctx.set_locked(&author, false).await;
    ctx.deliver_lock_change(&author).await;
    assert!(ctx.get_note(Some(&stranger), &note.id).await.is_ok());
    assert!(ctx.is_indexed("note", note_id).await);
    assert_eq!(hashtag_count(&ctx, &tag).await, 1);
    assert!(global(None).await.contains(&note.text));
    let changes = ctx.app.events_by("notes.note.visibility_changed", author.id).await;
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[1].data["visibility"], 1);
    assert!(changes[1].data["visible_to"].is_null());
}

#[tokio::test]
async fn a_note_posted_while_the_copy_is_behind_is_never_listed() {
    let ctx = start().await;
    let (author, stranger) = (ctx.member("author").await, ctx.member("stranger").await);
    ctx.post(&author, "first, while open").await;
    // Locked, but the event never reaches this domain.
    ctx.set_locked(&author, true).await;
    let tag = unique("behind");
    let note = ctx.post(&author, &format!("after locking #{tag}")).await;
    assert!(!ctx.is_indexed("note", id(&note.id)).await);
    assert_eq!(ctx.get_note(Some(&stranger), &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    // Posting brought the copy up to date, so the earlier note went too.
    let listed: i64 = sqlx::query_scalar("SELECT count(*) FROM notes WHERE author_id = $1 AND listed")
        .bind(author.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(listed, 0);
}

#[tokio::test]
async fn followers_notes_reach_followers_and_friends_only() {
    let ctx = start().await;
    let (author, follower, friend, stranger) = (
        ctx.member("author").await,
        ctx.member("follower").await,
        ctx.member("friend").await,
        ctx.member("stranger").await,
    );
    ctx.follow(&follower, &author).await;
    ctx.befriend(&author, &friend).await;
    let tag = unique("fol");
    let note = ctx.post_visible_to(&author, &format!("for followers #{tag}"), Visibility::VISIBILITY_FOLLOWERS).await;
    assert_eq!(note.visibility, buffa::EnumValue::from(Visibility::VISIBILITY_FOLLOWERS));
    assert!(ctx.get_note(Some(&follower), &note.id).await.is_ok());
    assert!(ctx.get_note(Some(&friend), &note.id).await.is_ok(), "friends are in the followers audience");
    for viewer in [Some(&stranger), None] {
        assert_eq!(ctx.get_note(viewer, &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    }
    assert!(!ctx.is_indexed("note", id(&note.id)).await);
    // A reply to it stays among followers, and the author answered sees it.
    assert_eq!(ctx.post_reason(&stranger, "?", |r| r.reply_to_id = note.id.clone()).await, "NOTE_NOT_FOUND");
    let answer = ctx.reply(&follower, &note, "nice").await;
    assert_eq!(
        answer.visibility,
        buffa::EnumValue::from(Visibility::VISIBILITY_FOLLOWERS),
        "a public reply is narrowed"
    );
    assert!(ctx.get_note(Some(&author), &answer.id).await.is_ok());
    assert_eq!(ctx.get_note(Some(&stranger), &answer.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    // Feed: the friends see it there; strangers do not.
    let activity = ctx.activities_of(author.id, "note.posted").await;
    assert_eq!(activity.last().unwrap().data["audience"], "friends");

    // Narrowed to followers after the fact: everyone who may still see it is named.
    let open = ctx.post(&author, "open").await;
    let changed: SetNoteVisibilityResponse = ctx
        .note(
            "SetNoteVisibility",
            Some(&author),
            &SetNoteVisibilityRequest {
                note_id: open.id.clone(),
                visibility: Visibility::VISIBILITY_FOLLOWERS.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.note.visibility, buffa::EnumValue::from(Visibility::VISIBILITY_FOLLOWERS));
    let event = ctx.app.events_by("notes.note.visibility_changed", author.id).await.pop().unwrap();
    let mut expected = vec![follower.id, friend.id];
    expected.sort();
    assert_eq!(event.data["visible_to"], serde_json::json!(expected));
    assert_eq!(event.data["visibility"], 6);
    assert!(!ctx.is_indexed("note", id(&open.id)).await);
}

#[tokio::test]
async fn close_friends_and_circles_reach_their_members_only_and_circles_stay_unnamed() {
    let ctx = start().await;
    let (author, close, member, outsider) = (
        ctx.member("author").await,
        ctx.member("close").await,
        ctx.member("member").await,
        ctx.member("outsider").await,
    );
    for person in [&close, &member, &outsider] {
        ctx.follow(person, &author).await;
    }
    let close_friends = ctx.close_friends_circle(&author).await;
    ctx.add_to_circle(&author, &close_friends, &[&close]).await;
    let circle = ctx.circle(&author, "部活", &[&member]).await;

    let cf = ctx.post_visible_to(&author, "close friends", Visibility::VISIBILITY_CLOSE_FRIENDS).await;
    assert!(ctx.get_note(Some(&close), &cf.id).await.is_ok());
    for viewer in [&member, &outsider] {
        assert_eq!(ctx.get_note(Some(viewer), &cf.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    }
    let shown = ctx.get_note(Some(&close), &cf.id).await.unwrap();
    assert_eq!(
        shown.visibility,
        buffa::EnumValue::from(Visibility::VISIBILITY_CLOSE_FRIENDS),
        "close friends see the mark"
    );

    let in_circle = ctx
        .post_with(&author, "the club", |request| {
            request.visibility = Visibility::VISIBILITY_CIRCLE.into();
            request.circle_id = circle.clone();
        })
        .await;
    assert_eq!(in_circle.circle_id, circle, "the author sees which circle");
    let shown = ctx.get_note(Some(&member), &in_circle.id).await.unwrap();
    assert_eq!(
        shown.visibility,
        buffa::EnumValue::from(Visibility::VISIBILITY_FOLLOWERS),
        "members are never told about the circle"
    );
    assert!(shown.circle_id.is_empty());
    for viewer in [&close, &outsider] {
        assert_eq!(ctx.get_note(Some(viewer), &in_circle.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    }
    // Nobody but the author renotes these, and a reply to them goes back directly.
    assert!(!shown.viewer.can_renote);
    let answer = ctx.reply(&member, &in_circle, "わかる").await;
    assert_eq!(answer.visibility, buffa::EnumValue::from(Visibility::VISIBILITY_PRIVATE));
    assert_eq!(
        answer.direct_recipients.iter().map(|user| user.id.clone()).collect::<Vec<_>>(),
        vec![author.id.to_string()]
    );
    assert!(ctx.get_note(Some(&author), &answer.id).await.is_ok());
    assert_eq!(ctx.get_note(Some(&outsider), &answer.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");

    // A member who stops following is out at once.
    let unfollow = tokuly_proto::social::UnfollowRequest { user_id: author.id.to_string(), ..Default::default() };
    let _: tokuly_proto::social::UnfollowResponse =
        ctx.app.call("tokuly.social.v1.FollowService/Unfollow", Some(&member), &unfollow).await.unwrap();
    assert_eq!(ctx.get_note(Some(&member), &in_circle.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");

    // Someone else's circle cannot be named.
    let reason = ctx
        .post_reason(&outsider, "theirs", |request| {
            request.visibility = Visibility::VISIBILITY_CIRCLE.into();
            request.circle_id = circle.clone();
        })
        .await;
    assert_eq!(reason, "CIRCLE_NOT_FOUND");
}

#[tokio::test]
async fn direct_notes_reach_and_notify_the_people_they_name() {
    let ctx = start().await;
    let (author, bob, carol, eve) =
        (ctx.member("author").await, ctx.member("bob").await, ctx.member("carol").await, ctx.member("eve").await);
    let direct = |people: &[&TestUser]| {
        let ids: Vec<String> = people.iter().map(|person| person.id.to_string()).collect();
        move |request: &mut CreateNoteRequest| {
            request.visibility = Visibility::VISIBILITY_PRIVATE.into();
            request.direct_user_ids = ids;
        }
    };
    let note = ctx.post_with(&author, "ひみつの相談", direct(&[&bob, &carol])).await;
    assert_eq!(note.direct_recipients.len(), 2);
    for person in [&bob, &carol] {
        assert!(ctx.get_note(Some(person), &note.id).await.is_ok());
        assert_eq!(ctx.notifications_for(person.id, "notes.direct").await.len(), 1);
        let mentions: GetMentionsTimelineResponse = ctx
            .note(
                "GetMentionsTimeline",
                Some(person),
                &GetMentionsTimelineRequest { page: page(10, ""), ..Default::default() },
            )
            .await
            .unwrap();
        assert_eq!(texts(&mentions.entries), vec!["ひみつの相談"]);
    }
    assert_eq!(ctx.get_note(Some(&eve), &note.id).await.unwrap_err().reason, "NOTE_NOT_FOUND");
    let reply = ctx.reply(&bob, &note, "了解").await;
    let mut recipients: Vec<String> = reply.direct_recipients.iter().map(|user| user.id.clone()).collect();
    recipients.sort();
    let mut expected = vec![author.id.to_string(), carol.id.to_string()];
    expected.sort();
    assert_eq!(recipients, expected, "a reply goes to the same people");

    // People who cannot be addressed are refused as one would be who does not exist.
    ctx.block(&eve, &author).await;
    assert_eq!(ctx.post_reason(&author, "x", direct(&[&eve])).await, "RECIPIENT_NOT_FOUND");
    let nobody = TestUser { id: Uuid::now_v7(), ..bob.clone() };
    assert_eq!(ctx.post_reason(&author, "x", direct(&[&nobody])).await, "RECIPIENT_NOT_FOUND");
    let reason = ctx
        .post_reason(&author, "x", |request| {
            request.visibility = Visibility::VISIBILITY_FRIENDS.into();
            request.direct_user_ids = vec![bob.id.to_string()];
        })
        .await;
    assert_eq!(reason, "INVALID_VISIBILITY");
}

#[tokio::test]
async fn the_following_timeline_shows_followed_accounts_and_leaves_out_muted_ones() {
    let ctx = start().await;
    let (me, followed, muted, friend) =
        (ctx.member("me").await, ctx.member("followed").await, ctx.member("muted").await, ctx.member("friend").await);
    ctx.follow(&me, &followed).await;
    ctx.follow(&me, &muted).await;
    ctx.befriend(&me, &friend).await;
    let mute = tokuly_proto::social::UpdateFollowPrefsRequest {
        user_id: muted.id.to_string(),
        muted: Some(true),
        ..Default::default()
    };
    let _: tokuly_proto::social::UpdateFollowPrefsResponse =
        ctx.app.call("tokuly.social.v1.FollowService/UpdateFollowPrefs", Some(&me), &mute).await.unwrap();
    ctx.post(&followed, "from someone I follow").await;
    ctx.post_visible_to(&followed, "for their followers", Visibility::VISIBILITY_FOLLOWERS).await;
    ctx.post(&muted, "muted").await;
    ctx.post(&friend, "from a friend").await;
    ctx.post(&me, "mine").await;
    let renoted = ctx.post(&friend, "renoted by someone I follow").await;
    ctx.renote(&followed, &renoted).await;

    let following: GetFollowingTimelineResponse = ctx
        .note(
            "GetFollowingTimeline",
            Some(&me),
            &GetFollowingTimelineRequest { page: page(20, ""), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(
        texts(&following.entries),
        vec!["renoted by someone I follow", "mine", "for their followers", "from someone I follow"]
    );
    let home: GetHomeTimelineResponse = ctx
        .note("GetHomeTimeline", Some(&me), &GetHomeTimelineRequest { page: page(20, ""), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(texts(&home.entries), vec!["renoted by someone I follow", "mine", "from a friend"]);
    assert_eq!(
        ctx.note_reason("GetFollowingTimeline", None, &GetFollowingTimelineRequest::default()).await,
        "SIGN_IN_REQUIRED"
    );
}
