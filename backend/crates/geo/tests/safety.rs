//! `SafetyService`: place alerts fired by the pipeline, and SOS.
mod common;

use chrono::Utc;
use common::*;
use tokuly_core::time::from_ts;
use tokuly_proto::{
    common::Visibility,
    geo::{create_place_alert_request::Where, geo_event::Event as Push, *},
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

fn alert_at(subject: &TestUser, center: Point, trigger: AlertTrigger, recurring: bool) -> CreatePlaceAlertRequest {
    CreatePlaceAlertRequest {
        subject_user_id: subject.id.to_string(),
        r#where: Some(Where::Center(Box::new(lat_lng(center)))),
        place_name: "駅".into(),
        trigger: trigger.into(),
        recurring,
        ..Default::default()
    }
}

async fn create_alert(ctx: &Ctx, owner: &TestUser, request: CreatePlaceAlertRequest) -> Result<PlaceAlert, RpcError> {
    let response: CreatePlaceAlertResponse = ctx.safety("CreatePlaceAlert", owner, &request).await?;
    Ok(response.alert.into_option().expect("alert"))
}

async fn alerts(ctx: &Ctx, user: &TestUser) -> ListPlaceAlertsResponse {
    ctx.safety("ListPlaceAlerts", user, &ListPlaceAlertsRequest::default()).await.unwrap()
}

async fn respond(ctx: &Ctx, user: &TestUser, alert_id: &str, approve: bool) -> Result<PlaceAlert, RpcError> {
    let response: RespondPlaceAlertResponse = ctx
        .safety(
            "RespondPlaceAlert",
            user,
            &RespondPlaceAlertRequest { alert_id: alert_id.to_owned(), approve, ..Default::default() },
        )
        .await?;
    Ok(response.alert.into_option().expect("alert"))
}

async fn delete_alert(ctx: &Ctx, user: &TestUser, alert_id: &str) -> Result<DeletePlaceAlertResponse, RpcError> {
    ctx.safety(
        "DeletePlaceAlert",
        user,
        &DeletePlaceAlertRequest { alert_id: alert_id.to_owned(), ..Default::default() },
    )
    .await
}

async fn make_home(ctx: &Ctx, owner: &TestUser, center: Point) {
    let request = CreatePlaceRequest {
        name: "うち".into(),
        label: PlaceLabel::PLACE_LABEL_HOME.into(),
        center: lat_lng(center).into(),
        radius_meters: 100.0,
        visibility: Visibility::VISIBILITY_PRIVATE.into(),
        ..Default::default()
    };
    let _: CreatePlaceResponse = ctx.places("CreatePlace", owner, &request).await.unwrap();
}

async fn active_sos(ctx: &Ctx, user: &TestUser) -> GetActiveSosResponse {
    ctx.safety("GetActiveSos", user, &GetActiveSosRequest::default()).await.unwrap()
}

#[tokio::test]
async fn a_one_time_alert_fires_once_when_the_friend_arrives() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let station = somewhere();
    let mut aois = ctx.app.listen_user(aoi.id).await;
    let mut rens = ctx.app.listen_user(ren.id).await;

    let alert =
        create_alert(&ctx, &aoi, alert_at(&ren, station, AlertTrigger::ALERT_TRIGGER_ARRIVE, false)).await.unwrap();
    assert_eq!(alert.status, AlertStatus::ALERT_STATUS_ACTIVE, "one-time alerts work at once");
    assert_eq!(
        (alert.owner.as_option().unwrap().id.clone(), alert.subject.as_option().unwrap().id.clone()),
        (aoi.id.to_string(), ren.id.to_string())
    );
    assert_eq!((alert.place_name.as_str(), alert.radius_meters, alert.recurring), ("駅", 150.0, false));
    assert_at!(point(alert.center.as_option()), station);
    let lapses_in = (from_ts(alert.expires_at.as_option().expect("a one-time alert lapses")) - Utc::now()).num_hours();
    assert!((23..=24).contains(&lapses_in));

    // The friend is told, not asked.
    let told = ctx.notifications_for(ren.id, "geo.place_alert_set").await;
    assert_eq!(told.len(), 1);
    assert!(told[0].data["body"]["ja"].as_str().unwrap().contains("「駅」"));
    assert!(matches!(next_push(&mut rens).await, Some(Push::PlaceAlertUpdated(pushed)) if pushed.id == alert.id));
    assert!(matches!(next_push(&mut aois).await, Some(Push::PlaceAlertUpdated(pushed)) if pushed.id == alert.id));
    assert_eq!(alerts(&ctx, &aoi).await.mine.iter().map(|a| a.id.clone()).collect::<Vec<_>>(), vec![alert.id.clone()]);
    assert_eq!(
        alerts(&ctx, &ren).await.about_me.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
        vec![alert.id.clone()]
    );
    assert!(alerts(&ctx, &ren).await.mine.is_empty() && alerts(&ctx, &aoi).await.about_me.is_empty());

    // Far away: nothing. Arriving: the owner is notified.
    ctx.at(&ren, offset(station, 0.0, 2000.0)).await;
    assert!(ctx.notifications_for(aoi.id, "geo.place_alert").await.is_empty());
    ctx.at(&ren, offset(station, 0.0, 40.0)).await;
    let fired = ctx.notifications_for(aoi.id, "geo.place_alert").await;
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].data["title"]["ja"], "renさんが「駅」に到着しました!");
    assert_eq!(fired[0].data["title"]["en"], "ren arrived at 駅");
    assert_eq!(fired[0].actor_id, Some(ren.id));
    let event = pushes(&mut aois, QUIET)
        .await
        .into_iter()
        .find_map(|push| if let Push::PlaceAlertFired(fired) = push { Some(*fired) } else { None })
        .expect("a fired event");
    assert_eq!(event.alert.as_option().unwrap().id, alert.id);
    assert!(event.fired_at.as_option().is_some());

    // Done: a one-time alert does not fire again.
    let mine = alerts(&ctx, &aoi).await.mine;
    assert_eq!(mine[0].status, AlertStatus::ALERT_STATUS_FIRED);
    assert!(mine[0].last_fired_at.as_option().is_some() && mine[0].expires_at.as_option().is_none());
    assert!(alerts(&ctx, &ren).await.about_me.is_empty());
    ctx.at(&ren, offset(station, 0.0, 2000.0)).await;
    ctx.at(&ren, offset(station, 0.0, 30.0)).await;
    assert_eq!(ctx.notifications_for(aoi.id, "geo.place_alert").await.len(), 1);
}

#[tokio::test]
async fn a_recurring_alert_on_the_friends_home_needs_their_approval() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let home = somewhere();
    let request = || CreatePlaceAlertRequest {
        subject_user_id: ren.id.to_string(),
        r#where: Some(Where::SubjectPlaceLabel(PlaceLabel::PLACE_LABEL_HOME.into())),
        trigger: AlertTrigger::ALERT_TRIGGER_LEAVE.into(),
        recurring: true,
        ..Default::default()
    };
    // No home to watch yet — and a friend who hides place labels has none as far as others are concerned.
    assert_eq!(create_alert(&ctx, &aoi, request()).await.unwrap_err().reason, "SUBJECT_PLACE_UNAVAILABLE");
    make_home(&ctx, &ren, home).await;
    ctx.set_privacy(&ren, |privacy| privacy.share_place_labels = false).await;
    assert_eq!(create_alert(&ctx, &aoi, request()).await.unwrap_err().reason, "SUBJECT_PLACE_UNAVAILABLE");
    ctx.set_privacy(&ren, |privacy| privacy.share_place_labels = true).await;

    let alert = create_alert(&ctx, &aoi, request()).await.unwrap();
    assert_eq!(alert.status, AlertStatus::ALERT_STATUS_PENDING_APPROVAL);
    assert_eq!(alert.subject_place_label, PlaceLabel::PLACE_LABEL_HOME);
    assert!(
        alert.center.as_option().is_none() && alert.radius_meters == 0.0,
        "the owner never learns where the friend's home is"
    );
    assert!(alert.expires_at.as_option().is_some());
    let asked = ctx.notifications_for(ren.id, "geo.place_alert_request").await;
    assert_eq!(asked.len(), 1);
    assert!(asked[0].data["body"]["ja"].as_str().unwrap().contains("自宅"));

    // Until approved it does nothing.
    ctx.at(&ren, home).await;
    ctx.at(&ren, offset(home, 0.0, 1000.0)).await;
    assert!(ctx.notifications_for(aoi.id, "geo.place_alert").await.is_empty());

    // Only the subject answers, once.
    assert_eq!(respond(&ctx, &aoi, &alert.id, true).await.unwrap_err().reason, "ALERT_NOT_FOUND");
    assert_eq!(respond(&ctx, &ren, "x", true).await.unwrap_err().reason, "INVALID_ID");
    let approved = respond(&ctx, &ren, &alert.id, true).await.unwrap();
    assert_eq!(approved.status, AlertStatus::ALERT_STATUS_ACTIVE);
    assert!(approved.expires_at.as_option().is_none(), "an approved recurring alert does not lapse");
    assert_eq!(respond(&ctx, &ren, &alert.id, true).await.unwrap_err().reason, "ALERT_NOT_PENDING");
    let answer = ctx.notifications_for(aoi.id, "geo.place_alert_response").await;
    assert_eq!((answer.len(), answer[0].data["data"]["approved"].as_bool()), (1, Some(true)));

    // At home, then leaving: fires, and stays active.
    ctx.at(&ren, home).await;
    ctx.at(&ren, offset(home, 0.0, 1000.0)).await;
    let fired = ctx.notifications_for(aoi.id, "geo.place_alert").await;
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].data["title"]["ja"], "renさんが自宅を出発しました");
    let mine = alerts(&ctx, &aoi).await.mine;
    assert_eq!(mine[0].status, AlertStatus::ALERT_STATUS_ACTIVE);
    assert!(mine[0].last_fired_at.as_option().is_some());

    // Popping back in and out right away does not fire again...
    ctx.at(&ren, home).await;
    ctx.at(&ren, offset(home, 0.0, 1000.0)).await;
    assert_eq!(ctx.notifications_for(aoi.id, "geo.place_alert").await.len(), 1);
    // ...but the next time, later, does.
    sqlx::query("UPDATE geo_place_alerts SET last_fired_at = now() - interval '10 minutes' WHERE id = $1")
        .bind(Uuid::parse_str(&alert.id).unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    ctx.deliver("identity.user.status_changed", ren.id, serde_json::json!({ "user_id": ren.id })).await; // drops the cached alert state
    ctx.at(&ren, home).await;
    ctx.at(&ren, offset(home, 0.0, 1000.0)).await;
    assert_eq!(ctx.notifications_for(aoi.id, "geo.place_alert").await.len(), 2);

    // Declining.
    let second = create_alert(
        &ctx,
        &aoi,
        CreatePlaceAlertRequest { trigger: AlertTrigger::ALERT_TRIGGER_ARRIVE.into(), ..request() },
    )
    .await
    .unwrap();
    let declined = respond(&ctx, &ren, &second.id, false).await.unwrap();
    assert_eq!(declined.status, AlertStatus::ALERT_STATUS_DECLINED);
    assert_eq!(alerts(&ctx, &ren).await.about_me.len(), 1, "only the approved one is still about them");
    assert_eq!(ctx.notifications_for(aoi.id, "geo.place_alert_response").await.len(), 2);
}

#[tokio::test]
async fn alerts_never_fire_from_a_position_the_owner_may_not_see() {
    let ctx = start().await;
    let (aoi, ren) = (ctx.app.user("aoi").await, ctx.app.user("ren").await);
    ctx.befriend(&aoi, &ren).await;
    let station = somewhere();
    let alert =
        create_alert(&ctx, &aoi, alert_at(&ren, station, AlertTrigger::ALERT_TRIGGER_ARRIVE, false)).await.unwrap();
    ctx.at(&ren, offset(station, 90.0, 3000.0)).await;

    // Ren goes ghost for Aoi and arrives: silence.
    ctx.ghost(&ren, &[&aoi], GhostMode::GHOST_MODE_BLURRED, 0).await.unwrap();
    ctx.at(&ren, station).await;
    assert!(ctx.notifications_for(aoi.id, "geo.place_alert").await.is_empty());
    assert_eq!(alerts(&ctx, &aoi).await.mine[0].status, AlertStatus::ALERT_STATUS_ACTIVE);

    // Visible again while already there: the arrival under ghost mode is not reported after the fact.
    ctx.ghost(&ren, &[&aoi], GhostMode::GHOST_MODE_PRECISE, 0).await.unwrap();
    ctx.at(&ren, offset(station, 0.0, 10.0)).await;
    assert!(ctx.notifications_for(aoi.id, "geo.place_alert").await.is_empty());
    // A real arrival from now on is.
    ctx.at(&ren, offset(station, 90.0, 3000.0)).await;
    ctx.at(&ren, station).await;
    assert_eq!(ctx.notifications_for(aoi.id, "geo.place_alert").await.len(), 1);
    assert_eq!(alerts(&ctx, &aoi).await.mine[0].id, alert.id);
}

#[tokio::test]
async fn alert_requests_are_validated_and_either_side_can_remove_an_alert() {
    let ctx = start().await;
    let (aoi, ren, stranger) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("stranger").await);
    ctx.befriend(&aoi, &ren).await;
    let spot = somewhere();
    let ok = || alert_at(&ren, spot, AlertTrigger::ALERT_TRIGGER_ARRIVE, false);
    let reason = async |request: CreatePlaceAlertRequest| create_alert(&ctx, &aoi, request).await.unwrap_err().reason;

    assert_eq!(reason(alert_at(&stranger, spot, AlertTrigger::ALERT_TRIGGER_ARRIVE, false)).await, "NOT_FRIENDS");
    assert_eq!(reason(alert_at(&aoi, spot, AlertTrigger::ALERT_TRIGGER_ARRIVE, false)).await, "NOT_FRIENDS");
    assert_eq!(reason(CreatePlaceAlertRequest { subject_user_id: "x".into(), ..ok() }).await, "INVALID_ID");
    assert_eq!(reason(CreatePlaceAlertRequest { r#where: None, ..ok() }).await, "MISSING_TARGET");
    assert_eq!(
        reason(CreatePlaceAlertRequest {
            r#where: Some(Where::SubjectPlaceLabel(PlaceLabel::PLACE_LABEL_CUSTOM.into())),
            ..ok()
        })
        .await,
        "MISSING_TARGET"
    );
    assert_eq!(
        reason(CreatePlaceAlertRequest { r#where: Some(Where::Center(Box::new(lat_lng((0.0, 300.0))))), ..ok() }).await,
        "INVALID_LOCATION"
    );
    assert_eq!(reason(CreatePlaceAlertRequest { radius_meters: 10.0, ..ok() }).await, "INVALID_RADIUS");
    assert_eq!(
        reason(CreatePlaceAlertRequest { trigger: AlertTrigger::ALERT_TRIGGER_UNSPECIFIED.into(), ..ok() }).await,
        "INVALID_TRIGGER"
    );
    assert_eq!(reason(CreatePlaceAlertRequest { place_name: "あ".repeat(61), ..ok() }).await, "INVALID_NAME");
    assert_eq!(
        reason(CreatePlaceAlertRequest { r#where: Some(Where::PlaceId(Uuid::now_v7().to_string())), ..ok() }).await,
        "PLACE_NOT_FOUND"
    );
    assert!(alerts(&ctx, &aoi).await.mine.is_empty());

    // At one of the owner's own places: its position, radius and name.
    let place: CreatePlaceResponse = ctx
        .places(
            "CreatePlace",
            &aoi,
            &CreatePlaceRequest {
                name: "うちのカフェ".into(),
                center: lat_lng(spot).into(),
                radius_meters: 200.0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let place = place.place.into_option().unwrap();
    let at_place =
        CreatePlaceAlertRequest { r#where: Some(Where::PlaceId(place.id.clone())), place_name: String::new(), ..ok() };
    let alert = create_alert(&ctx, &aoi, at_place.clone()).await.unwrap();
    assert_eq!((alert.place_name.as_str(), alert.radius_meters), ("うちのカフェ", 200.0));
    assert_at!(point(alert.center.as_option()), spot);
    // Someone else's place is not theirs to use.
    assert_eq!(
        create_alert(&ctx, &ren, CreatePlaceAlertRequest { subject_user_id: aoi.id.to_string(), ..at_place })
            .await
            .unwrap_err()
            .reason,
        "PLACE_NOT_FOUND"
    );

    // Removal: by the owner or by the person watched, nobody else.
    assert_eq!(delete_alert(&ctx, &stranger, &alert.id).await.unwrap_err().reason, "ALERT_NOT_FOUND");
    let mut aois = ctx.app.listen_user(aoi.id).await;
    delete_alert(&ctx, &ren, &alert.id).await.unwrap();
    assert!(alerts(&ctx, &aoi).await.mine.is_empty() && alerts(&ctx, &ren).await.about_me.is_empty());
    assert!(
        matches!(next_push(&mut aois).await, Some(Push::PlaceAlertUpdated(gone)) if gone.id == alert.id && gone.status == AlertStatus::ALERT_STATUS_EXPIRED)
    );
    assert_eq!(delete_alert(&ctx, &aoi, &alert.id).await.unwrap_err().reason, "ALERT_NOT_FOUND");
    ctx.at(&ren, spot).await;
    assert!(ctx.notifications_for(aoi.id, "geo.place_alert").await.is_empty());

    // A one-time alert that never fires lapses.
    let waiting = create_alert(&ctx, &aoi, ok()).await.unwrap();
    assert_eq!(ctx.geo.expire_place_alerts().await.unwrap(), 0);
    sqlx::query("UPDATE geo_place_alerts SET expires_at = now() - interval '1 minute' WHERE id = $1")
        .bind(Uuid::parse_str(&waiting.id).unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(ctx.geo.expire_place_alerts().await.unwrap() >= 1);
    assert_eq!(alerts(&ctx, &aoi).await.mine[0].status, AlertStatus::ALERT_STATUS_EXPIRED);
    assert!(alerts(&ctx, &ren).await.about_me.is_empty());

    // The end of a friendship ends the alerts between the two.
    create_alert(&ctx, &aoi, ok()).await.unwrap();
    ctx.unfriend(&aoi, &ren).await;
    assert!(alerts(&ctx, &aoi).await.mine.iter().all(|a| a.status != AlertStatus::ALERT_STATUS_ACTIVE));
    assert!(alerts(&ctx, &ren).await.about_me.is_empty());
}

#[tokio::test]
async fn an_sos_shows_the_people_alerted_the_precise_pin_until_it_is_resolved() {
    let ctx = start().await;
    let (aoi, ren, mio) = (ctx.app.user("aoi").await, ctx.app.user("ren").await, ctx.app.user("mio").await);
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &mio).await;
    let home = somewhere();
    ctx.at(&aoi, home).await;
    // Frozen for everyone, then out and about.
    ctx.ghost(&aoi, &[], GhostMode::GHOST_MODE_FROZEN, 0).await.unwrap();
    let alley = offset(home, 30.0, 4000.0);
    ctx.at(&aoi, alley).await;
    assert_at!(pin_point(&ctx.seen_by(&ren, &aoi).await.unwrap()), home);
    let mut rens = ctx.app.listen_user(ren.id).await;
    let mut mios = ctx.app.listen_user(mio.id).await;

    let request = TriggerSosRequest {
        message: " 助けて ".into(),
        notify_user_ids: vec![ren.id.to_string()],
        ..Default::default()
    };
    let response: TriggerSosResponse = ctx.safety("TriggerSos", &aoi, &request).await.unwrap();
    let sos = response.sos.into_option().unwrap();
    assert_eq!((sos.message.as_str(), sos.notified_user_ids.clone()), ("助けて", vec![ren.id.to_string()]));
    assert!(sos.started_at.as_option().is_some() && sos.resolved_at.as_option().is_none());
    assert_at!(pin_point(sos.location.as_option().unwrap()), alley);

    // An urgent notification, and the precise live pin, for the friend alerted — and only for them.
    let urgent = ctx.notifications_for(ren.id, "geo.sos").await;
    assert_eq!(urgent.len(), 1);
    assert_eq!(
        (urgent[0].data["urgent"].as_bool(), urgent[0].data["body"]["ja"].as_str()),
        (Some(true), Some("助けて"))
    );
    assert!(urgent[0].data["title"]["ja"].as_str().unwrap().contains("SOS"));
    assert!(ctx.notifications_for(mio.id, "geo.sos").await.is_empty());
    let events = pushes(&mut rens, QUIET).await;
    let pushed = events
        .iter()
        .find_map(|push| if let Push::SosUpdated(sos) = push { Some(sos.clone()) } else { None })
        .expect("an SOS event");
    assert_at!(pin_point(pushed.location.as_option().unwrap()), alley);
    assert!(pushed.notified_user_ids.is_empty(), "who else was alerted is the sender's business");
    assert!(
        events.iter().any(|push| matches!(push, Push::Location(pin) if at(pin_point(pin), alley))),
        "the map pin jumps to the true position"
    );
    assert!(pushes(&mut mios, QUIET).await.is_empty());
    assert_at!(pin_point(&ctx.seen_by(&ren, &aoi).await.unwrap()), alley);
    assert_at!(pin_point(&ctx.seen_by(&mio, &aoi).await.unwrap()), home, "still frozen for everyone else");

    // Live from here on.
    let moved = offset(alley, 30.0, 300.0);
    ctx.at(&aoi, moved).await;
    assert_at!(pin_point(&next_pin(&mut rens).await.unwrap()), moved);
    assert!(pins(&mut mios, QUIET).await.is_empty());

    let mine = active_sos(&ctx, &aoi).await;
    assert_eq!(mine.mine.as_option().unwrap().id, sos.id);
    assert!(mine.friends.is_empty());
    let seen = active_sos(&ctx, &ren).await;
    assert!(seen.mine.as_option().is_none());
    assert_eq!(seen.friends.iter().map(|s| s.id.clone()).collect::<Vec<_>>(), vec![sos.id.clone()]);
    assert_at!(pin_point(seen.friends[0].location.as_option().unwrap()), moved);
    assert!(active_sos(&ctx, &mio).await.friends.is_empty());

    // Pressing again while it is active returns the same SOS and alerts nobody twice.
    let again: TriggerSosResponse = ctx.safety("TriggerSos", &aoi, &TriggerSosRequest::default()).await.unwrap();
    assert_eq!(again.sos.as_option().unwrap().id, sos.id);
    assert_eq!(ctx.notifications_for(ren.id, "geo.sos").await.len(), 1);

    // Only the sender resolves it; afterwards ghost mode applies again.
    let resolve = |user: &TestUser, sos_id: &str| {
        let (user, request) = (user.clone(), ResolveSosRequest { sos_id: sos_id.to_owned(), ..Default::default() });
        let ctx = &ctx;
        async move { ctx.safety::<_, ResolveSosResponse>("ResolveSos", &user, &request).await }
    };
    assert_eq!(resolve(&ren, &sos.id).await.unwrap_err().reason, "SOS_NOT_FOUND");
    assert_eq!(resolve(&aoi, &Uuid::now_v7().to_string()).await.unwrap_err().reason, "SOS_NOT_FOUND");
    resolve(&aoi, &sos.id).await.unwrap();
    let events = pushes(&mut rens, QUIET).await;
    assert!(events.iter().any(
        |push| matches!(push, Push::SosUpdated(done) if done.id == sos.id && done.resolved_at.as_option().is_some())
    ));
    assert!(
        events.iter().any(|push| matches!(push, Push::Location(pin) if at(pin_point(pin), home))),
        "back to the frozen pin"
    );
    assert_at!(pin_point(&ctx.seen_by(&ren, &aoi).await.unwrap()), home);
    assert_eq!(ctx.notifications_for(ren.id, "geo.sos_resolved").await.len(), 1);
    assert!(active_sos(&ctx, &aoi).await.mine.as_option().is_none() && active_sos(&ctx, &ren).await.friends.is_empty());
    resolve(&aoi, &sos.id).await.unwrap(); // resolving twice is fine
}

#[tokio::test]
async fn sos_requests_are_validated_and_unresolved_ones_end_by_themselves() {
    let ctx = start().await;
    let (aoi, ren, mio, loner, stranger) = (
        ctx.app.user("aoi").await,
        ctx.app.user("ren").await,
        ctx.app.user("mio").await,
        ctx.app.user("loner").await,
        ctx.app.user("stranger").await,
    );
    ctx.befriend(&aoi, &ren).await;
    ctx.befriend(&aoi, &mio).await;
    let reason = |user: &TestUser, request: TriggerSosRequest| {
        let user = user.clone();
        let ctx = &ctx;
        async move { ctx.safety::<_, TriggerSosResponse>("TriggerSos", &user, &request).await.unwrap_err().reason }
    };
    assert_eq!(
        reason(&aoi, TriggerSosRequest { message: "あ".repeat(201), ..Default::default() }).await,
        "MESSAGE_TOO_LONG"
    );
    assert_eq!(
        reason(&aoi, TriggerSosRequest { notify_user_ids: vec![stranger.id.to_string()], ..Default::default() }).await,
        "NOT_FRIENDS"
    );
    assert_eq!(
        reason(&aoi, TriggerSosRequest { notify_user_ids: vec!["x".into()], ..Default::default() }).await,
        "INVALID_ID"
    );
    assert_eq!(
        reason(
            &aoi,
            TriggerSosRequest {
                notify_user_ids: (0..101).map(|_| Uuid::now_v7().to_string()).collect(),
                ..Default::default()
            }
        )
        .await,
        "TOO_MANY_USERS"
    );
    assert_eq!(reason(&loner, TriggerSosRequest::default()).await, "NO_RECIPIENTS");
    assert!(active_sos(&ctx, &aoi).await.mine.as_option().is_none());

    // No list: every friend is alerted. An SOS works even before any location is known.
    let response: TriggerSosResponse = ctx.safety("TriggerSos", &aoi, &TriggerSosRequest::default()).await.unwrap();
    let sos = response.sos.into_option().unwrap();
    let mut alerted = sos.notified_user_ids.clone();
    alerted.sort();
    let mut friends = vec![ren.id.to_string(), mio.id.to_string()];
    friends.sort();
    assert_eq!(alerted, friends);
    assert!(sos.location.as_option().is_none());
    for friend in [&ren, &mio] {
        let urgent = ctx.notifications_for(friend.id, "geo.sos").await;
        assert_eq!((urgent.len(), urgent[0].data["urgent"].as_bool()), (1, Some(true)));
    }

    // Nobody resolves it: after the maximum time it ends, so ghost mode cannot stay overridden forever.
    assert_eq!(active_sos(&ctx, &ren).await.friends.len(), 1);
    sqlx::query("UPDATE geo_sos SET started_at = now() - interval '25 hours' WHERE id = $1")
        .bind(Uuid::parse_str(&sos.id).unwrap())
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    assert!(ctx.geo.expire_sos().await.unwrap() >= 1);
    assert!(active_sos(&ctx, &aoi).await.mine.as_option().is_none() && active_sos(&ctx, &ren).await.friends.is_empty());

    // A former friend is taken off an active SOS.
    let response: TriggerSosResponse = ctx.safety("TriggerSos", &aoi, &TriggerSosRequest::default()).await.unwrap();
    assert_eq!(response.sos.as_option().unwrap().notified_user_ids.len(), 2);
    ctx.unfriend(&aoi, &mio).await;
    assert!(active_sos(&ctx, &mio).await.friends.is_empty());
    assert_eq!(active_sos(&ctx, &aoi).await.mine.as_option().unwrap().notified_user_ids, vec![ren.id.to_string()]);
    let anonymous = ctx
        .app
        .call::<_, GetActiveSosResponse>(
            "tokuly.geo.v1.SafetyService/GetActiveSos",
            None,
            &GetActiveSosRequest::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(anonymous.reason, "SIGN_IN_REQUIRED");
}
