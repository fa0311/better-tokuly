//! Finding rooms and people: public rooms by listeners, live among
//! friends, popular tags, friends in rooms, the profile card — and what
//! the domain plugs into the generic ones (subjects, the realtime topic
//! guard, the data export).
mod common;

use common::*;
use tokuly_proto::{
    common::{PageRequest, SubjectType},
    identity::GetProfileResponse,
    spaces::*,
};
use tokuly_testkit::TestUser;

fn live(scope: LiveScope, tag: &str) -> ListLiveSpacesRequest {
    ListLiveSpacesRequest { scope: scope.into(), tag: tag.to_owned(), ..Default::default() }
}

fn titles(spaces: &[Space]) -> Vec<&str> {
    spaces.iter().map(|space| space.title.as_str()).collect()
}

/// A tag no other test uses, so that lists of public rooms hold only this
/// test's rooms.
fn unique_tag(prefix: &str) -> String {
    format!("{prefix}{}", &uuid::Uuid::now_v7().simple().to_string()[20..])
}

async fn tagged_room(ctx: &Ctx, host: &TestUser, title: &str, tags: &[&str], language: &str) -> Space {
    let request = CreateSpaceRequest {
        title: title.to_owned(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
        language: language.to_owned(),
        audience: SpaceAudience::SPACE_AUDIENCE_PUBLIC.into(),
        ..Default::default()
    };
    let response: CreateSpaceResponse = ctx.call("Space", "CreateSpace", host, &request).await.expect("create room");
    response.space.as_option().unwrap().clone()
}

async fn cards(ctx: &Ctx, viewer: &TestUser, owner: &TestUser) -> Vec<tokuly_proto::identity::ProfileCard> {
    let request = serde_json::json!({ "userId": owner.id.to_string() });
    let response: GetProfileResponse =
        ctx.app.call("tokuly.identity.v1.ProfileService/GetProfile", Some(viewer), &request).await.expect("profile");
    response.profile.cards.iter().filter(|card| card.kind == "spaces.in_room").cloned().collect()
}

#[tokio::test]
async fn public_rooms_are_ranked_by_listeners() {
    let ctx = start().await;
    let tag = unique_tag("rank");
    let (small, big, middle) = (ctx.app.user("small").await, ctx.app.user("big").await, ctx.app.user("middle").await);
    let _small_room = tagged_room(&ctx, &small, "ちいさい", &[&tag], "en").await;
    let big_room = tagged_room(&ctx, &big, "おおきい", &[&tag, "talk"], "tlh").await;
    let middle_room = tagged_room(&ctx, &middle, "ふつう", &[&tag, "talk"], "tlh").await;
    // A friends room with the same tag is nobody's business.
    let private = ctx.app.user("private").await;
    let request = CreateSpaceRequest { title: "ないしょ".into(), tags: vec![tag.clone()], ..Default::default() };
    let _: CreateSpaceResponse = ctx.call("Space", "CreateSpace", &private, &request).await.unwrap();
    for (room, listeners) in [(&big_room, 2), (&middle_room, 1)] {
        for n in 0..listeners {
            let listener = ctx.app.user(&format!("listener{n}")).await;
            ctx.join(&listener, &room.id).await;
        }
    }

    // Anyone can look, signed in or not.
    let listed: ListLiveSpacesResponse =
        ctx.anonymous("Space", "ListLiveSpaces", &live(LiveScope::LIVE_SCOPE_PUBLIC, &tag)).await.unwrap();
    assert_eq!(titles(&listed.spaces), ["おおきい", "ふつう", "ちいさい"]);
    assert_eq!(listed.spaces.iter().map(|space| space.listener_count).collect::<Vec<_>>(), [3, 2, 1]);
    assert_eq!(listed.spaces[0].stage[0].user.id, big.id.to_string(), "the stage is previewed");
    assert!(listed.spaces[0].viewer.is_unset());
    assert!(listed.spaces[0].tags.contains(&tag));
    // The tag is matched the way it is stored.
    let shouted: ListLiveSpacesResponse = ctx
        .anonymous("Space", "ListLiveSpaces", &live(LiveScope::LIVE_SCOPE_PUBLIC, &format!("#{}", tag.to_uppercase())))
        .await
        .unwrap();
    assert_eq!(shouted.spaces.len(), 3);

    // By language.
    let mut english = live(LiveScope::LIVE_SCOPE_PUBLIC, &tag);
    english.language = "en-GB".into();
    let listed: ListLiveSpacesResponse = ctx.call("Space", "ListLiveSpaces", &small, &english).await.unwrap();
    assert_eq!(titles(&listed.spaces), ["ちいさい"]);
    assert!(listed.spaces[0].viewer.inside);

    // Pages follow the ranking.
    let mut paged = live(LiveScope::LIVE_SCOPE_PUBLIC, &tag);
    paged.page = PageRequest { limit: 2, ..Default::default() }.into();
    let first: ListLiveSpacesResponse = ctx.anonymous("Space", "ListLiveSpaces", &paged).await.unwrap();
    assert_eq!(titles(&first.spaces), ["おおきい", "ふつう"]);
    assert!(first.page.has_more);
    paged.page = PageRequest { limit: 2, cursor: first.page.next_cursor.clone(), ..Default::default() }.into();
    let second: ListLiveSpacesResponse = ctx.anonymous("Space", "ListLiveSpaces", &paged).await.unwrap();
    assert_eq!(titles(&second.spaces), ["ちいさい"]);
    assert!(!second.page.has_more);

    // What people are talking about right now.
    let popular: ListPopularTagsResponse = ctx
        .anonymous("Space", "ListPopularTags", &ListPopularTagsRequest { language: "tlh".into(), ..Default::default() })
        .await
        .unwrap();
    let found: Vec<(&str, i32, i32)> = popular
        .tags
        .iter()
        .map(|popular| (popular.tag.as_str(), popular.space_count, popular.listener_count))
        .collect();
    assert_eq!(found, [(tag.as_str(), 2, 5), ("talk", 2, 5)], "by listeners, then by name");
    let one: ListPopularTagsResponse = ctx
        .anonymous(
            "Space",
            "ListPopularTags",
            &ListPopularTagsRequest { language: "tlh".into(), limit: 1, ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(one.tags.len(), 1);

    // An ended room leaves the ranking.
    let _: EndSpaceResponse = ctx
        .call("Space", "EndSpace", &big, &EndSpaceRequest { space_id: big_room.id.clone(), ..Default::default() })
        .await
        .unwrap();
    let listed: ListLiveSpacesResponse =
        ctx.anonymous("Space", "ListLiveSpaces", &live(LiveScope::LIVE_SCOPE_PUBLIC, &tag)).await.unwrap();
    assert_eq!(titles(&listed.spaces), ["ふつう", "ちいさい"]);

    assert_eq!(ctx.reason("Space", "ListLiveSpaces", &small, &ListLiveSpacesRequest::default()).await, "INVALID_SCOPE");
    assert_eq!(
        ctx.anonymous_reason("Space", "ListLiveSpaces", &live(LiveScope::LIVE_SCOPE_FRIENDS, "")).await,
        "SIGN_IN_REQUIRED"
    );
    let mut nonsense = live(LiveScope::LIVE_SCOPE_PUBLIC, &tag);
    nonsense.language = "nihongo".into();
    assert_eq!(ctx.reason("Space", "ListLiveSpaces", &small, &nonsense).await, "INVALID_LANGUAGE");
}

#[tokio::test]
async fn rooms_with_friends_inside_are_found() {
    let ctx = start().await;
    let viewer = ctx.app.user("viewer").await;
    let (hosting, listening, hiding) =
        (ctx.app.user("hosting").await, ctx.app.user("listening").await, ctx.app.user("hiding").await);
    for friend in [&hosting, &listening, &hiding] {
        ctx.befriend(&viewer, friend).await;
    }
    let stranger = ctx.app.user("stranger").await;
    ctx.befriend(&stranger, &listening).await;

    // A friend hosts a friends room; another listens to a stranger's public room.
    let _ = ctx.open_room(&hosting, "なかよし", SpaceAudience::SPACE_AUDIENCE_FRIENDS).await;
    let public_room = ctx.public_room(&stranger, "ひろば").await;
    ctx.join(&listening, &public_room.id).await;
    // A friend who hides their online status listens somewhere else.
    ctx.set_privacy(&hiding, |privacy| privacy.share_online_status = false).await;
    let other_host = ctx.app.user("other").await;
    let quiet_room = ctx.public_room(&other_host, "しずかな部屋").await;
    ctx.join(&hiding, &quiet_room.id).await;

    let among_friends = || async {
        let listed: ListLiveSpacesResponse =
            ctx.call("Space", "ListLiveSpaces", &viewer, &live(LiveScope::LIVE_SCOPE_FRIENDS, "")).await.unwrap();
        listed.spaces
    };
    let spaces = among_friends().await;
    assert_eq!(titles(&spaces), ["ひろば", "なかよし"], "most listeners first");
    assert_eq!(
        spaces[0].friends_inside.iter().map(|friend| friend.display_name.as_str()).collect::<Vec<_>>(),
        ["listening"]
    );
    assert_eq!(spaces[1].friends_inside[0].id, hosting.id.to_string());
    assert!(!spaces[0].viewer.inside);

    let friends_in_rooms = || async {
        let listed: ListFriendsInRoomsResponse =
            ctx.call("Space", "ListFriendsInRooms", &viewer, &ListFriendsInRoomsRequest::default()).await.unwrap();
        listed.friends
    };
    let friends = friends_in_rooms().await;
    let seen: Vec<(&str, &str, Option<SpaceRole>)> = friends
        .iter()
        .map(|friend| (friend.user.display_name.as_str(), friend.title.as_str(), friend.role.as_known()))
        .collect();
    assert_eq!(
        seen,
        [
            ("listening", "ひろば", Some(SpaceRole::SPACE_ROLE_LISTENER)),
            ("hosting", "なかよし", Some(SpaceRole::SPACE_ROLE_HOST)),
        ]
    );
    assert_eq!(friends[0].space_id, public_room.id);
    assert!(friends[0].hangout_id.is_empty() && friends[0].since.is_set());

    // On stage, everyone in the room sees them anyway: no secret left to keep.
    ctx.bring_on_stage(&other_host, &hiding, &quiet_room.id).await;
    assert_eq!(
        titles(&among_friends().await),
        ["しずかな部屋", "ひろば", "なかよし"],
        "equally full: the newer room leads"
    );
    assert_eq!(friends_in_rooms().await.len(), 3);

    // A room the viewer cannot enter is not advertised: invited-only, or banned from.
    let _: EndSpaceResponse = ctx
        .call(
            "Space",
            "EndSpace",
            &stranger,
            &EndSpaceRequest { space_id: public_room.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let request = CreateSpaceRequest {
        title: "招待制".into(),
        audience: SpaceAudience::SPACE_AUDIENCE_INVITED.into(),
        invitee_ids: vec![listening.id.to_string()],
        ..Default::default()
    };
    let created: CreateSpaceResponse = ctx.call("Space", "CreateSpace", &stranger, &request).await.unwrap();
    ctx.join(&listening, &created.space.id).await;
    assert_eq!(titles(&among_friends().await), ["しずかな部屋", "なかよし"]);
    let ban = RemoveParticipantRequest {
        space_id: quiet_room.id.clone(),
        user_id: viewer.id.to_string(),
        ban: true,
        ..Default::default()
    };
    let _: RemoveParticipantResponse = ctx.call("Space", "RemoveParticipant", &other_host, &ban).await.unwrap();
    assert_eq!(titles(&among_friends().await), ["なかよし"]);
    assert_eq!(friends_in_rooms().await.len(), 1);
}

#[tokio::test]
async fn profiles_show_friends_where_someone_is() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let stranger = ctx.app.user("jiro").await;
    assert!(cards(&ctx, &friend, &host).await.is_empty(), "not in a room");

    let (room, _) = ctx.open_room(&host, "放課後トーク", SpaceAudience::SPACE_AUDIENCE_FRIENDS).await;
    let shown = cards(&ctx, &friend, &host).await;
    assert_eq!(shown.len(), 1);
    let card = &shown[0];
    assert_eq!((card.title.ja.as_str(), card.title.en.as_str()), ("ルームにいるよ!", "In a room right now"));
    assert_eq!(card.text.ja, "「放課後トーク」");
    assert_eq!(card.path, format!("/spaces/{}", room.id));
    assert_eq!(card.emoji, "🎙️");
    assert_eq!(card.data.get("space_id"), Some(&room.id));
    assert_eq!(card.data.get("role").map(String::as_str), Some("host"));
    // Only friends who could enter see it; the card is not for strangers or for oneself.
    assert!(cards(&ctx, &stranger, &host).await.is_empty());
    assert!(cards(&ctx, &host, &host).await.is_empty());

    // A listener who hides their online status keeps their whereabouts to themselves.
    ctx.join(&friend, &room.id).await;
    assert_eq!(cards(&ctx, &host, &friend).await.len(), 1);
    ctx.set_privacy(&friend, |privacy| privacy.share_online_status = false).await;
    assert!(cards(&ctx, &host, &friend).await.is_empty());

    // After the room: nothing.
    let _: EndSpaceResponse = ctx
        .call("Space", "EndSpace", &host, &EndSpaceRequest { space_id: room.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert!(cards(&ctx, &friend, &host).await.is_empty());

    // In a hangout, for the friends who belong to it.
    let outsider_friend = ctx.app.user("saburo").await;
    ctx.befriend(&host, &outsider_friend).await;
    let hangout = ctx.hangout(&host, "いつメン", &[&friend]).await;
    ctx.join_hangout(&host, &hangout.id).await;
    let shown = cards(&ctx, &friend, &host).await;
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].title.ja, "たまり場にいるよ!");
    assert_eq!(shown[0].path, format!("/hangouts/{}", hangout.id));
    assert_eq!(shown[0].data.get("hangout_id"), Some(&hangout.id));
    assert!(cards(&ctx, &outsider_friend, &host).await.is_empty(), "a friend, but not of this hangout");
}

#[tokio::test]
async fn rooms_are_subjects_and_their_topic_is_guarded() {
    let ctx = start().await;
    let (host, friend) = ctx.friends("taro", "hanako").await;
    let stranger = ctx.app.user("jiro").await;
    let core = &ctx.app.core;
    let public = ctx.schedule(&host, "公開", SpaceAudience::SPACE_AUDIENCE_PUBLIC).await;
    let private = ctx.schedule(&host, "友達だけ", SpaceAudience::SPACE_AUDIENCE_FRIENDS).await;
    let space = SubjectType::SUBJECT_TYPE_SPACE;

    // Reports, bookmarks and notifications reach rooms through the subject registry.
    let subject = core.subjects.resolve(space, &public.id, None).await.unwrap();
    assert_eq!((subject.owner_id, subject.title.as_str()), (host.id, "公開"));
    assert_eq!(subject.path, format!("/spaces/{}", public.id));
    assert!(!subject.interactions_enabled, "no comments on audio rooms");
    assert!(core.subjects.resolve(space, &private.id, Some(friend.id)).await.is_ok());
    for outsider in [Some(stranger.id), None] {
        let error = core.subjects.resolve(space, &private.id, outsider).await.unwrap_err();
        assert_eq!(error.reason, "SUBJECT_NOT_FOUND");
    }
    assert!(core.subjects.resolve(space, "not-an-id", Some(host.id)).await.is_err());
    // Hangouts can be reported by the people in them.
    let hangout = ctx.hangout(&host, "いつメン", &[&friend]).await;
    let subject = core.subjects.resolve(space, &hangout.id, Some(friend.id)).await.unwrap();
    assert_eq!((subject.owner_id, subject.title.as_str()), (host.id, "いつメン"));
    assert_eq!(subject.path, format!("/hangouts/{}", hangout.id));
    assert!(core.subjects.resolve(space, &hangout.id, Some(stranger.id)).await.is_err());

    // The realtime topic of a room follows who may see it.
    let topic = |space_id: &str| format!("space:{space_id}");
    let guards = &core.topic_guards;
    assert!(
        guards.may_subscribe(None, &topic(&public.id)).await.unwrap(),
        "public rooms are open, even without signing in"
    );
    assert!(guards.may_subscribe(Some(stranger.id), &topic(&public.id)).await.unwrap());
    assert!(guards.may_subscribe(Some(host.id), &topic(&private.id)).await.unwrap());
    assert!(guards.may_subscribe(Some(friend.id), &topic(&private.id)).await.unwrap());
    assert!(!guards.may_subscribe(Some(stranger.id), &topic(&private.id)).await.unwrap());
    assert!(!guards.may_subscribe(None, &topic(&private.id)).await.unwrap());
    for malformed in
        ["space:", "space:not-an-id", &format!("space:{}:extra", public.id), &topic(&uuid::Uuid::now_v7().to_string())]
    {
        assert!(!guards.may_subscribe(Some(host.id), malformed).await.unwrap(), "{malformed}");
    }

    // The personal data export has a section of this domain.
    let exporter = core.exporters.all().into_iter().find(|exporter| exporter.name() == "spaces").expect("registered");
    let export = exporter.export(host.id).await.unwrap();
    let hosted: Vec<&str> =
        export["hosted_rooms"].as_array().unwrap().iter().filter_map(|room| room["title"].as_str()).collect();
    assert_eq!(hosted, ["公開", "友達だけ"]);
    assert_eq!(export["hangouts"][0]["name"], "いつメン");
    assert_eq!(export["hangouts"][0]["owner"], true);
    let theirs = exporter.export(friend.id).await.unwrap();
    assert!(theirs["hosted_rooms"].as_array().unwrap().is_empty());
    assert_eq!(theirs["hangouts"][0]["owner"], false);
}
