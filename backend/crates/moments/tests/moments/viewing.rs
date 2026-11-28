//! Who sees which moment, locked or not, and with which location.
use tokuly_core::geoindex::{self, Sight, SightRule};
use tokuly_proto::{
    common::SubjectType,
    moments::{
        DeleteMomentRequest, DeleteMomentResponse, GetCalendarRequest, GetCalendarResponse, ListMapMomentsRequest,
        ListMapMomentsResponse, MomentAudience, PostMomentResponse,
    },
};

use crate::common::*;

#[tokio::test]
async fn friends_moments_stay_blurred_until_you_post() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに"]).await;
    let (a, b) = (&users[0], &users[1]);
    let region = ctx.region(&[a, b]).await;
    ctx.prompt(&region).await;
    geoindex::set(&ctx.app.core.cache, b.id, 35.66, 139.70, chrono::Utc::now()).await.unwrap();
    let theirs = ctx.post(b).await;

    let locked = ctx.friend_moments(a).await;
    assert!(!locked.unlocked);
    let shown = &locked.moments[0];
    assert_eq!(shown.id, theirs.id);
    assert!(shown.locked);
    assert!(shown.back.url.is_empty() && shown.back.variants.is_empty() && shown.back.asset_id.is_empty());
    assert!(!shown.back.thumbhash.is_empty(), "a blurred placeholder");
    assert!(shown.caption.is_empty() && !shown.location.is_set() && shown.reactions.is_empty());
    assert_eq!(shown.author.as_option().unwrap().id, b.id.to_string());
    let map: ListMapMomentsResponse = ctx.call("ListMapMoments", a, &ListMapMomentsRequest::default()).await.unwrap();
    assert!(map.moments.is_empty() && !map.unlocked);

    let mine = ctx.post(a).await;
    let open = ctx.friend_moments(a).await;
    assert!(open.unlocked);
    let shown = &open.moments[0];
    assert!(!shown.locked && !shown.back.url.is_empty() && shown.caption == "放課後🍦");
    assert!(shown.location.is_set(), "b shares their location with a");
    // The region would tell roughly where b is, ghost mode or not: b's own only.
    let day = shown.day.as_option().unwrap();
    assert!(day.region.is_empty() && !day.date.is_empty() && day.prompt_at.is_set());
    assert!(locked.moments[0].day.as_option().unwrap().region.is_empty());
    assert_eq!(ctx.get(b, &theirs.id).await.unwrap().day.as_option().unwrap().region, region);
    assert_eq!(mine.day.as_option().unwrap().region, region);
    let map: ListMapMomentsResponse = ctx.call("ListMapMoments", a, &ListMapMomentsRequest::default()).await.unwrap();
    assert_eq!(map.moments.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), [theirs.id.as_str()]);

    // Deleting your own locks again.
    let _: DeleteMomentResponse =
        ctx.call("DeleteMoment", a, &DeleteMomentRequest { moment_id: mine.id, ..Default::default() }).await.unwrap();
    assert!(ctx.get(a, &theirs.id).await.unwrap().locked);
}

#[tokio::test]
async fn the_location_follows_the_ghost_rules_of_the_map() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに", "ちか"]).await;
    let (a, b, c) = (&users[0], &users[1], &users[2]);
    ctx.befriend(b, c).await;
    let region = ctx.region(&[a, b, c]).await;
    ctx.prompt(&region).await;
    ctx.post(b).await;
    ctx.post(c).await;
    let moment = ctx.post(a).await;
    let (ctx, cache) = (&ctx, &ctx.app.core.cache);
    let seen_by = |viewer| {
        let moment_id = moment.id.clone();
        async move { ctx.get(viewer, &moment_id).await.unwrap().location.is_set() }
    };

    // Not sharing a location at all: nobody but the owner sees one.
    assert!(!seen_by(b).await && !seen_by(c).await);
    assert!(ctx.get(a, &moment.id).await.unwrap().location.is_set());

    geoindex::set(cache, a.id, 35.66, 139.70, chrono::Utc::now()).await.unwrap();
    assert!(seen_by(b).await && seen_by(c).await);

    // Blurred (or frozen) for b: b sees no location, c still does.
    let rule = SightRule { precise: false, until_ms: None };
    let sight = Sight { overrides: [(b.id, rule)].into_iter().collect(), ..Default::default() };
    geoindex::set_sight(cache, a.id, &sight).await.unwrap();
    assert!(!seen_by(b).await && seen_by(c).await);

    // Ghost for everyone: nobody.
    let sight = Sight { everyone: Some(rule), ..Default::default() };
    geoindex::set_sight(cache, a.id, &sight).await.unwrap();
    assert!(!seen_by(b).await && !seen_by(c).await);
    let listed = ctx.friend_moments(b).await;
    assert!(listed.moments.iter().all(|m| !m.location.is_set()));
    let map: ListMapMomentsResponse = ctx.call("ListMapMoments", b, &ListMapMomentsRequest::default()).await.unwrap();
    assert!(map.moments.iter().all(|m| m.id != moment.id), "not on the map either");

    // One list, each owner judged on their own: c shares, a hides.
    geoindex::set(cache, c.id, 35.67, 139.71, chrono::Utc::now()).await.unwrap();
    let map: ListMapMomentsResponse = ctx.call("ListMapMoments", b, &ListMapMomentsRequest::default()).await.unwrap();
    let authors: Vec<String> = map.moments.iter().map(|m| m.author.as_option().unwrap().id.clone()).collect();
    assert_eq!(authors, [c.id.to_string()]);
    geoindex::remove(cache, a.id).await.unwrap();
    geoindex::remove(cache, c.id).await.unwrap();
}

#[tokio::test]
async fn only_the_audience_sees_and_never_across_a_block() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに", "ちか", "でん"]).await;
    let (owner, friend, close, blocked) = (&users[0], &users[1], &users[2], &users[3]);
    let stranger = ctx.app.user("しらない").await;
    let region = ctx.region(&[owner, friend, close, blocked, &stranger]).await;
    ctx.prompt(&region).await;
    for viewer in [friend, close, blocked, &stranger] {
        ctx.post(viewer).await;
    }
    ctx.add_close_friends(owner, &[close]).await;
    let mut request = ctx.pair(owner).await;
    request.audience = MomentAudience::MOMENT_AUDIENCE_CLOSE_FRIENDS.into();
    let posted: PostMomentResponse = ctx.call("PostMoment", owner, &request).await.unwrap();
    let moment = posted.moment.as_option().unwrap().clone();
    assert_eq!(moment.audience, buffa::EnumValue::from(MomentAudience::MOMENT_AUDIENCE_CLOSE_FRIENDS));

    let ctx = &ctx;
    let sees = |viewer| {
        let moment_id = moment.id.clone();
        async move {
            let listed = ctx.friend_moments(viewer).await.moments.iter().any(|m| m.id == moment_id);
            let got = ctx.get(viewer, &moment_id).await;
            assert_eq!(listed, got.is_ok(), "list and lookup agree");
            if let Err(e) = got {
                assert_eq!(e.reason, "MOMENT_NOT_FOUND");
            }
            listed
        }
    };
    assert!(sees(close).await);
    assert!(!sees(friend).await, "a friend outside the close friends");
    assert!(!sees(&stranger).await);

    ctx.add_close_friends(owner, &[blocked]).await;
    assert!(sees(blocked).await);
    ctx.block(blocked, owner).await;
    assert!(!sees(blocked).await, "a block either way hides it");
    assert!(
        ctx.friend_moments(owner)
            .await
            .moments
            .iter()
            .all(|m| m.author.as_option().unwrap().id != blocked.id.to_string())
    );

    // A suspended owner's moment is gone for everyone else.
    sqlx::query("UPDATE users SET banned_at = now() WHERE id = $1")
        .bind(owner.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(!sees(close).await);
}

#[tokio::test]
async fn past_days_are_the_owners_alone() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに"]).await;
    let (a, b) = (&users[0], &users[1]);
    let region = ctx.region(&[a, b]).await;
    let first = ctx.prompt(&region).await;
    ctx.post(a).await;
    let yesterday = ctx.post(b).await;
    ctx.age_day(first, 3600).await;

    ctx.prompt(&region).await;
    ctx.post(a).await;
    assert!(ctx.friend_moments(a).await.moments.is_empty(), "b has not posted today");
    assert_eq!(ctx.get(a, &yesterday.id).await.unwrap_err().reason, "MOMENT_NOT_FOUND");
    assert!(!ctx.get(b, &yesterday.id).await.unwrap().locked, "the owner keeps it");

    let date: chrono::NaiveDate = yesterday.day.as_option().unwrap().date.parse().unwrap();
    let request = GetCalendarRequest {
        year: chrono::Datelike::year(&date),
        month: chrono::Datelike::month(&date) as i32,
        ..Default::default()
    };
    let calendar: GetCalendarResponse = ctx.call("GetCalendar", b, &request).await.unwrap();
    assert_eq!(calendar.days.iter().map(|d| d.moment_id.as_str()).collect::<Vec<_>>(), [yesterday.id.as_str()]);
    assert_eq!((calendar.posted_count, calendar.on_time_count), (1, 1));
    let bad = GetCalendarRequest { year: 2026, month: 13, ..Default::default() };
    assert_eq!(ctx.call::<_, GetCalendarResponse>("GetCalendar", b, &bad).await.unwrap_err().reason, "INVALID_MONTH");
}

#[tokio::test]
async fn friends_in_another_region_are_seen_on_their_own_day() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに"]).await;
    let (a, b) = (&users[0], &users[1]);
    let here = ctx.region(&[a]).await;
    let there = ctx.region(&[b]).await;
    ctx.prompt(&there).await;
    let theirs = ctx.post(b).await;
    // a's region has had no prompt yet: b's moment shows, locked.
    let listed = ctx.friend_moments(a).await;
    assert!(listed.moments.iter().any(|m| m.id == theirs.id && m.locked));
    ctx.prompt(&here).await;
    ctx.post(a).await;
    assert!(ctx.friend_moments(a).await.moments.iter().any(|m| m.id == theirs.id && !m.locked));
}

#[tokio::test]
async fn a_locked_moment_lends_its_caption_to_no_bookmark_or_report() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに"]).await;
    let (a, b) = (&users[0], &users[1]);
    let region = ctx.region(&[a, b]).await;
    ctx.prompt(&region).await;
    let theirs = ctx.post(b).await;
    let subjects = &ctx.app.core.subjects;
    let titles = |viewer: uuid::Uuid| {
        let moment_id = theirs.id.clone();
        async move {
            let kind = SubjectType::SUBJECT_TYPE_MOMENT;
            let shown = subjects.resolve(kind, &moment_id, Some(viewer)).await.unwrap().title;
            let reported = subjects.resolve_for_report(kind, &moment_id, Some(viewer)).await.unwrap().title;
            [shown, reported]
        }
    };

    // A bookmark's preview and a report's title come from the resolver:
    // while a has not posted, they must not carry what the blur hides.
    for title in titles(a.id).await {
        assert!(!title.contains("放課後"), "{title}");
    }
    assert!(titles(b.id).await.iter().all(|title| title == "放課後🍦"), "the owner sees their own caption");

    ctx.post(a).await;
    assert!(titles(a.id).await.iter().all(|title| title == "放課後🍦"), "unlocked, the caption is theirs to see");
}

#[tokio::test]
async fn the_location_goes_once_the_live_position_is_old() {
    let ctx = start().await;
    let users = ctx.circle_of(&["あや", "べに"]).await;
    let (a, b) = (&users[0], &users[1]);
    let region = ctx.region(&[a, b]).await;
    ctx.prompt(&region).await;
    ctx.post(b).await;
    let moment = ctx.post(a).await;
    let cache = &ctx.app.core.cache;
    let minutes_ago = |minutes| chrono::Utc::now() - chrono::Duration::minutes(minutes);

    geoindex::set(cache, a.id, 35.66, 139.70, minutes_ago(4)).await.unwrap();
    assert!(ctx.get(b, &moment.id).await.unwrap().location.is_set(), "four minutes old: still live");
    // The phone stopped reporting: the pin is history now, and so is the place.
    geoindex::set(cache, a.id, 35.66, 139.70, minutes_ago(6)).await.unwrap();
    assert!(!ctx.get(b, &moment.id).await.unwrap().location.is_set());
    let map: ListMapMomentsResponse = ctx.call("ListMapMoments", b, &ListMapMomentsRequest::default()).await.unwrap();
    assert!(map.moments.iter().all(|m| m.id != moment.id));
    assert!(ctx.get(a, &moment.id).await.unwrap().location.is_set(), "the owner keeps theirs");
    geoindex::remove(cache, a.id).await.unwrap();
}
