//! Gatherings: friends in the same spot, as the pipeline detects them and as
//! each viewer gets to see them.
mod common;

use chrono::{Duration, Utc};
use common::*;
use tokuly_proto::{
    common::Visibility,
    geo::{geo_event::Event as Push, *},
    social::{GetFriendshipRequest, GetFriendshipResponse},
};
use tokuly_testkit::TestUser;

async fn gatherings(ctx: &Ctx, viewer: &TestUser) -> Vec<Gathering> {
    let response: ListGatheringsResponse =
        ctx.location("ListGatherings", viewer, &ListGatheringsRequest::default()).await.unwrap();
    response.gatherings
}

fn member_ids(gathering: &Gathering) -> Vec<String> {
    let mut ids: Vec<String> =
        gathering.members.iter().map(|member| member.user.as_option().unwrap().id.clone()).collect();
    ids.sort();
    ids
}

fn sorted(users: &[&TestUser]) -> Vec<String> {
    let mut ids: Vec<String> = users.iter().map(|user| user.id.to_string()).collect();
    ids.sort();
    ids
}

fn gathering_events(pushes: Vec<Push>) -> (Vec<Gathering>, Vec<String>) {
    let (mut updated, mut ended) = (Vec::new(), Vec::new());
    for push in pushes {
        match push {
            Push::GatheringUpdated(gathering) => updated.push(*gathering),
            Push::GatheringEndedId(id) => ended.push(id),
            _ => {}
        }
    }
    (updated, ended)
}

#[tokio::test]
async fn friends_in_the_same_spot_form_a_gathering_their_friends_can_see() {
    let ctx = start().await;
    let (aoi, ren, mio, stranger) = (
        ctx.app.user("aoi").await,
        ctx.app.user("ren").await,
        ctx.app.user("mio").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&mio, &aoi).await;
    ctx.befriend(&mio, &ren).await;
    let spot = somewhere();
    let t = Utc::now();
    let mut mios = ctx.app.listen_user(mio.id).await;
    let mut aois = ctx.app.listen_user(aoi.id).await;

    // Passing each other is not a gathering: a minute in, nothing yet.
    seen_at(&ctx, &aoi, spot, t, &[300, 270, 240]).await;
    seen_at(&ctx, &ren, offset(spot, 90.0, 8.0), t, &[300, 270, 240]).await;
    seen_at(&ctx, &aoi, spot, t, &[200]).await;
    assert!(gatherings(&ctx, &mio).await.is_empty());
    seen_at(&ctx, &ren, offset(spot, 90.0, 8.0), t, &[110]).await;
    seen_at(&ctx, &aoi, spot, t, &[60]).await;

    // Their mutual friend sees it on the map...
    let listed = gatherings(&ctx, &mio).await;
    assert_eq!(listed.len(), 1);
    let gathering = &listed[0];
    assert_eq!(member_ids(gathering), sorted(&[&aoi, &ren]));
    assert!(distance(point(gathering.center.as_option()), spot) < 10.0);
    assert!(!gathering.at_private_place && gathering.place_name.is_empty());
    assert!(
        gathering.started_at.as_option().is_some()
            && gathering.members.iter().all(|member| member.arrived_at.as_option().is_some())
    );
    // ...was pushed it...
    let (updated, ended) = gathering_events(pushes(&mut mios, QUIET).await);
    assert!(ended.is_empty());
    assert_eq!(
        updated.last().map(|g| (g.id.clone(), member_ids(g))),
        Some((gathering.id.clone(), sorted(&[&aoi, &ren])))
    );
    // ...and notified, once.
    let notified = ctx.notifications_for(mio.id, "geo.gathering").await;
    assert_eq!(notified.len(), 1);
    let title = notified[0].data["title"]["ja"].as_str().unwrap();
    assert!(title.contains("aoi") && title.contains("ren") && title.ends_with("集合中!"), "{title}");
    assert_eq!(notified[0].data["data"]["gathering_id"], gathering.id);
    // The two themselves see it too, without being notified about themselves.
    assert_eq!(
        gatherings(&ctx, &aoi).await.iter().map(|g| g.id.clone()).collect::<Vec<_>>(),
        vec![gathering.id.clone()]
    );
    assert_eq!(gatherings(&ctx, &ren).await.len(), 1);
    assert!(!gathering_events(pushes(&mut aois, QUIET).await).0.is_empty());
    assert!(ctx.notifications_for(aoi.id, "geo.gathering").await.is_empty());
    // Strangers see nothing.
    assert!(gatherings(&ctx, &stranger).await.is_empty());
    assert!(ctx.notifications_for(stranger.id, "geo.gathering").await.is_empty());

    // Ren walks off: with one person left it is over, and time together is credited.
    ctx.at(&ren, offset(spot, 0.0, 600.0)).await;
    assert!(gatherings(&ctx, &mio).await.is_empty() && gatherings(&ctx, &aoi).await.is_empty());
    let (_, ended) = gathering_events(pushes(&mut mios, QUIET).await);
    assert_eq!(ended, vec![gathering.id.clone()]);
    let friendship: GetFriendshipResponse = ctx
        .app
        .call(
            "tokuly.social.v1.FriendService/GetFriendship",
            Some(&aoi),
            &GetFriendshipRequest { user_id: ren.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let together = friendship.friendship.as_option().unwrap().time_together_seconds;
    assert!((60..=300).contains(&together), "about two minutes together: {together} s");
}

#[tokio::test]
async fn a_third_friend_joins_the_same_gathering_and_viewers_see_only_their_own_friends() {
    let ctx = start().await;
    let (aoi, ren, kai, mio) =
        (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("kai").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &kai).await; // Kai knows Aoi only
    ctx.befriend(&mio, &aoi).await;
    ctx.befriend(&mio, &ren).await; // Mio knows Aoi and Ren, not Kai
    let spot = somewhere();
    let t = Utc::now();
    meet(&ctx, &aoi, &ren, spot, t).await;
    let before = gatherings(&ctx, &aoi).await;
    assert_eq!(before.len(), 1);

    // Kai turns up next to Aoi and stays.
    let kais = offset(spot, 270.0, 10.0);
    seen_at(&ctx, &kai, kais, t, &[250, 220]).await;
    seen_at(&ctx, &kai, kais, t, &[40]).await;
    let after = gatherings(&ctx, &aoi).await;
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, before[0].id, "the gathering keeps its ID as people join");
    assert_eq!(member_ids(&after[0]), sorted(&[&aoi, &ren, &kai]));
    // Each viewer sees the members who are their friends.
    assert_eq!(member_ids(&gatherings(&ctx, &mio).await[0]), sorted(&[&aoi, &ren]));
    assert_eq!(member_ids(&gatherings(&ctx, &kai).await[0]), sorted(&[&aoi, &kai]));
    assert_eq!(member_ids(&gatherings(&ctx, &ren).await[0]), sorted(&[&aoi, &ren]));

    // Aoi leaves (a good minute and a half after she was last seen there).
    // Ren and Kai are still standing next to each other: to Aoi, who knows
    // both, two friends together; to Mio, who knows only Ren, nothing.
    let later = Utc::now() + Duration::seconds(45);
    ctx.report(&aoi, vec![sample(offset(spot, 0.0, 800.0), later)]).await;
    let left = gatherings(&ctx, &aoi).await;
    assert_eq!(left.len(), 1);
    assert_eq!((left[0].id.clone(), member_ids(&left[0])), (before[0].id.clone(), sorted(&[&ren, &kai])));
    assert!(gatherings(&ctx, &mio).await.is_empty(), "Mio could only see Ren now: not a gathering");
    // Ren and Kai are not friends: as soon as one of them is heard from again, it is over.
    ctx.report(&ren, vec![sample(offset(spot, 90.0, 8.0), later)]).await;
    assert!(gatherings(&ctx, &aoi).await.is_empty());
}

#[tokio::test]
async fn two_gatherings_whose_members_meet_become_the_older_one() {
    let ctx = start().await;
    let (aoi, ren, kai, mio) =
        (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("kai").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&kai, &mio).await;
    ctx.befriend(&ren, &kai).await; // the only link between the two pairs
    let spot = somewhere();
    let far = offset(spot, 0.0, 150.0);
    // Everything below happens within the five minutes that count as the
    // present (a backlog upload forms no gatherings), ending just ahead of the clock.
    let t = Utc::now() + Duration::seconds(45);
    let (rens, mios_first) = (offset(spot, 90.0, 8.0), offset(far, 90.0, 8.0));
    // Aoi and Ren stand together; so do Kai and Mio, a little later and out of reach.
    seen_at(&ctx, &aoi, spot, t, &[295]).await;
    seen_at(&ctx, &ren, rens, t, &[290]).await;
    seen_at(&ctx, &aoi, spot, t, &[280]).await;
    seen_at(&ctx, &kai, far, t, &[275]).await;
    seen_at(&ctx, &mio, mios_first, t, &[270]).await;
    seen_at(&ctx, &kai, far, t, &[255]).await;
    seen_at(&ctx, &ren, rens, t, &[170]).await; // two minutes for Ren: the first gathering
    seen_at(&ctx, &aoi, spot, t, &[160]).await;
    seen_at(&ctx, &mio, mios_first, t, &[150]).await; // two minutes for Mio: the second
    let older = gatherings(&ctx, &aoi).await.pop().expect("the first pair's gathering");
    let younger = gatherings(&ctx, &kai).await.pop().expect("the second pair's gathering");
    assert_ne!(older.id, younger.id);
    assert_eq!(member_ids(&younger), sorted(&[&kai, &mio]));
    assert_eq!(
        member_ids(&gatherings(&ctx, &ren).await[0]),
        sorted(&[&aoi, &ren]),
        "Kai is with someone Ren cannot see"
    );
    let mut mios = ctx.app.listen_user(mio.id).await;

    // Kai and Mio walk over and stand with the others. Two minutes next to Ren make it one gathering.
    let kais = offset(spot, 270.0, 10.0);
    seen_at(&ctx, &mio, offset(spot, 180.0, 10.0), t, &[130]).await;
    seen_at(&ctx, &kai, kais, t, &[125]).await;
    assert_eq!(gatherings(&ctx, &kai).await[0].id, younger.id, "not yet");
    seen_at(&ctx, &kai, kais, t, &[0]).await;

    // The older gathering keeps its ID and takes everyone in; each sees their own friends in it.
    for (viewer, visible) in [
        (&aoi, sorted(&[&aoi, &ren])),
        (&ren, sorted(&[&aoi, &ren, &kai])),
        (&kai, sorted(&[&ren, &kai, &mio])),
        (&mio, sorted(&[&kai, &mio])),
    ] {
        let seen = gatherings(&ctx, viewer).await;
        assert_eq!(seen.len(), 1, "{}", viewer.handle);
        assert_eq!((seen[0].id.as_str(), member_ids(&seen[0])), (older.id.as_str(), visible), "{}", viewer.handle);
    }
    // The members of the younger one are told that it is over and what they are part of now.
    let (updated, ended) = gathering_events(pushes(&mut mios, QUIET).await);
    assert_eq!(ended, vec![younger.id.clone()]);
    let latest = updated.last().expect("the merged gathering");
    assert_eq!((latest.id.as_str(), member_ids(latest)), (older.id.as_str(), sorted(&[&kai, &mio])));
}

#[tokio::test]
async fn ghost_mode_hides_people_from_gatherings() {
    let ctx = start().await;
    let (aoi, ren, mio) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&mio, &aoi).await;
    ctx.befriend(&mio, &ren).await;
    let spot = somewhere();
    // Aoi is blurred for Mio: a gathering would give the true position away.
    ctx.ghost(&aoi, &[&mio], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    let mut mios = ctx.app.listen_user(mio.id).await;
    meet(&ctx, &aoi, &ren, spot, Utc::now()).await;

    assert_eq!(gatherings(&ctx, &ren).await.len(), 1, "the people in it see it");
    assert!(gatherings(&ctx, &mio).await.is_empty());
    assert!(gathering_events(pushes(&mut mios, QUIET).await).0.is_empty());
    assert!(ctx.notifications_for(mio.id, "geo.gathering").await.is_empty());
    // Precise again: it appears.
    ctx.ghost(&aoi, &[&mio], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    assert_eq!(gatherings(&ctx, &mio).await.len(), 1);
}

#[tokio::test]
async fn a_gathering_at_someones_home_is_not_announced() {
    let ctx = start().await;
    let (aoi, ren, mio) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&mio, &aoi).await;
    ctx.befriend(&mio, &ren).await;
    let home = somewhere();
    let request = CreatePlaceRequest {
        name: "うち".into(),
        label: PlaceLabel::PLACE_LABEL_HOME.into(),
        center: lat_lng(home).into(),
        visibility: Visibility::VISIBILITY_PRIVATE.into(),
        ..Default::default()
    };
    let _: CreatePlaceResponse = ctx.places("CreatePlace", &aoi, &request).await.unwrap();
    meet(&ctx, &aoi, &ren, home, Utc::now()).await;

    let listed = gatherings(&ctx, &mio).await;
    assert_eq!(listed.len(), 1, "it is on the map");
    assert!(listed[0].at_private_place, "but marked as a private place");
    assert!(listed[0].place_name.is_empty(), "and home is never named");
    assert!(ctx.notifications_for(mio.id, "geo.gathering").await.is_empty(), "and nobody is told");
}

#[tokio::test]
async fn a_gathering_at_a_public_place_carries_its_name() {
    let ctx = start().await;
    let (aoi, ren, mio, owner) =
        (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("mio").await, ctx.app.user("owner").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&mio, &aoi).await;
    ctx.befriend(&mio, &ren).await;
    let spot = somewhere();
    let request = CreatePlaceRequest {
        name: "カラオケ館".into(),
        center: lat_lng(spot).into(),
        visibility: Visibility::VISIBILITY_PUBLIC.into(),
        ..Default::default()
    };
    let _: CreatePlaceResponse = ctx.places("CreatePlace", &owner, &request).await.unwrap();
    meet(&ctx, &aoi, &ren, spot, Utc::now()).await;
    let listed = gatherings(&ctx, &mio).await;
    assert_eq!(listed[0].place_name, "カラオケ館");
    let notified = ctx.notifications_for(mio.id, "geo.gathering").await;
    assert_eq!(notified[0].data["body"]["ja"], "📍カラオケ館");
}

#[tokio::test]
async fn silent_members_are_taken_out_by_the_sweeper() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    meet(&ctx, &aoi, &ren, somewhere(), Utc::now()).await;
    assert_eq!(gatherings(&ctx, &aoi).await.len(), 1);
    ctx.geo.expire_gatherings().await.unwrap();
    assert_eq!(gatherings(&ctx, &aoi).await.len(), 1, "members seen recently stay");
    // Both phones go quiet.
    sqlx::query("UPDATE geo_gathering_members SET last_seen_at = now() - interval '2 hours' WHERE user_id = ANY($1)")
        .bind(vec![aoi.id, ren.id])
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(ctx.geo.expire_gatherings().await.unwrap() >= 1);
    assert!(gatherings(&ctx, &aoi).await.is_empty());

    // Stopping sharing leaves a gathering at once.
    let (mio, kai) = (ctx.app.user("mio").await, ctx.app.user("kai").await);
    ctx.befriend(&mio, &kai).await;
    meet(&ctx, &mio, &kai, somewhere(), Utc::now()).await;
    assert_eq!(gatherings(&ctx, &kai).await.len(), 1);
    let _: StopSharingResponse = ctx.location("StopSharing", &mio, &StopSharingRequest::default()).await.unwrap();
    assert!(gatherings(&ctx, &kai).await.is_empty());
}
