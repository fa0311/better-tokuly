//! `CommunityEventService`: the calendar, RSVP with a waitlist, reminders
//! and check-in.
mod common;

use chrono::Duration;
use common::*;
use tokuly_proto::community::{community_event::Event as Pushed, *};
use tokuly_testkit::TestUser;

const SVC: &str = "CommunityEventService";

fn by_id(event: &CalendarEvent) -> String {
    event.id.clone()
}

async fn attendees(ctx: &Ctx, viewer: &TestUser, request: &ListAttendeesRequest) -> Vec<Attendee> {
    let response: ListAttendeesResponse = ctx.call(SVC, "ListAttendees", Some(viewer), request).await.unwrap();
    response.attendees
}

/// Lets plain members create events in `community`.
async fn open_events_to_members(ctx: &Ctx, owner: &TestUser, community: &Community) {
    let request = UpdateCommunityRequest {
        community_id: community.id.clone(),
        settings: CommunitySettings { members_can_create_events: true, ..Default::default() }.into(),
        ..Default::default()
    };
    let _: UpdateCommunityResponse =
        ctx.call("CommunityService", "UpdateCommunity", Some(owner), &request).await.unwrap();
}

#[tokio::test]
async fn events_are_put_on_the_calendar_by_those_allowed() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let request = CreateEventRequest {
        place: EventPlace {
            name: "第2体育館".into(),
            address: "東京都".into(),
            location: tokuly_proto::common::LatLng { lat: 35.68, lng: 139.76, ..Default::default() }.into(),
            ..Default::default()
        }
        .into(),
        capacity: 20,
        ..ctx.new_event(&community)
    };
    assert_eq!(ctx.reason(SVC, "CreateEvent", Some(&member), &request).await, "PERMISSION_DENIED");
    assert_eq!(ctx.reason(SVC, "CreateEvent", None, &request).await, "SIGN_IN_REQUIRED");

    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let event = ctx.try_event(&moderator, &request).await.unwrap();
    assert_eq!(event.organizer.id, moderator.id.to_string());
    assert_eq!((event.title.as_str(), event.capacity, event.going_count), ("オフ会", 20, 1));
    assert_eq!((event.place.name.as_str(), event.place.location.lat), ("第2体育館", 35.68));
    assert!(!event.online && !event.cancelled);
    // Two hours unless said otherwise.
    assert_eq!(event.ends_at.seconds - event.starts_at.seconds, 7200);
    assert_eq!(event.path, format!("/c/{}/events/{}", community.handle, event.id));
    // The organiser is going, and runs the event.
    assert_eq!(event.viewer.status, AttendanceStatus::ATTENDANCE_STATUS_GOING);
    assert!(event.viewer.can_manage);
    assert_eq!(ctx.app.events_by("community.event.created", moderator.id).await[0].data["event_id"], event.id);
    assert!(pushes(&mut topic).await.iter().any(|push| matches!(push, Pushed::CalendarEventChanged(changed)
        if changed.event_id == event.id && changed.event.title == "オフ会" && changed.event.viewer.as_option().is_none())));

    // Anyone reads a public community's calendar.
    let seen = ctx.get_event(None, &event.id).await.unwrap();
    assert!(seen.viewer.as_option().is_none());
    let as_member = ctx.get_event(Some(&member), &event.id).await.unwrap();
    assert_eq!(as_member.viewer.status, AttendanceStatus::ATTENDANCE_STATUS_UNSPECIFIED);
    assert!(!as_member.viewer.can_manage);

    // A community may let its members organise.
    open_events_to_members(&ctx, &owner, &community).await;
    assert_eq!(ctx.try_event(&member, &request).await.unwrap().organizer.id, member.id.to_string());
}

#[tokio::test]
async fn an_event_is_validated() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let base = ctx.new_event(&community);
    let cases: Vec<(&str, CreateEventRequest)> = vec![
        ("TITLE_INVALID", CreateEventRequest { title: " ".into(), ..base.clone() }),
        ("DESCRIPTION_TOO_LONG", CreateEventRequest { description: "あ".repeat(3001), ..base.clone() }),
        ("TIME_INVALID", CreateEventRequest { starts_at: Default::default(), ..base.clone() }),
        ("TIME_INVALID", CreateEventRequest { starts_at: ts(now() - Duration::hours(1)), ..base.clone() }),
        ("TIME_INVALID", CreateEventRequest { ends_at: ts(now() + Duration::hours(1)), ..base.clone() }),
        ("TIME_INVALID", CreateEventRequest { ends_at: ts(now() + Duration::days(40)), ..base.clone() }),
        ("PLACE_INVALID", CreateEventRequest { place: Default::default(), ..base.clone() }),
        ("PLACE_INVALID", CreateEventRequest { place: EventPlace::default().into(), ..base.clone() }),
        ("URL_INVALID", CreateEventRequest { online: true, online_url: "javascript:alert(1)".into(), ..base.clone() }),
        ("CAPACITY_INVALID", CreateEventRequest { capacity: -1, ..base.clone() }),
        ("COMMUNITY_NOT_FOUND", CreateEventRequest { community_id: "no-such-community".into(), ..base.clone() }),
    ];
    for (reason, request) in cases {
        assert_eq!(ctx.reason(SVC, "CreateEvent", Some(&owner), &request).await, reason);
    }
}

#[tokio::test]
async fn the_link_of_an_online_event_is_for_those_who_come() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let request = CreateEventRequest {
        online: true,
        online_url: "https://meet.example.com/room".into(),
        place: Default::default(),
        ..ctx.new_event(&community)
    };
    let event = ctx.try_event(&owner, &request).await.unwrap();
    assert!(event.online && event.place.as_option().is_none());
    assert_eq!(event.online_url, "https://meet.example.com/room");
    assert!(ctx.get_event(Some(&member), &event.id).await.unwrap().online_url.is_empty());
    assert!(ctx.get_event(None, &event.id).await.unwrap().online_url.is_empty());
    let going = ctx.rsvp(&member, &event, RsvpAnswer::RSVP_ANSWER_GOING).await;
    assert_eq!(going.online_url, "https://meet.example.com/room");
}

#[tokio::test]
async fn answers_fill_the_places_and_the_waitlist_moves_up() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let mut users = Vec::new();
    for name in ["A", "B", "C", "D"] {
        users.push(ctx.member(&owner, &community, name).await);
    }
    let [a, b, c, d] = &users[..] else { unreachable!() };
    let outsider = ctx.app.user("Outsider").await;
    let event = ctx.try_event(&owner, &CreateEventRequest { capacity: 2, ..ctx.new_event(&community) }).await.unwrap();
    use AttendanceStatus::*;
    use RsvpAnswer::*;

    // Two places: the organiser has one, A takes the other, B and C queue up.
    assert_eq!(ctx.rsvp(a, &event, RSVP_ANSWER_GOING).await.viewer.status, ATTENDANCE_STATUS_GOING);
    let b_waits = ctx.rsvp(b, &event, RSVP_ANSWER_GOING).await;
    assert_eq!((b_waits.viewer.status, b_waits.viewer.waitlist_position), (ATTENDANCE_STATUS_WAITLISTED.into(), 1));
    let c_waits = ctx.rsvp(c, &event, RSVP_ANSWER_GOING).await;
    assert_eq!((c_waits.viewer.status, c_waits.viewer.waitlist_position), (ATTENDANCE_STATUS_WAITLISTED.into(), 2));
    let d_interested = ctx.rsvp(d, &event, RSVP_ANSWER_INTERESTED).await;
    assert_eq!(d_interested.viewer.status, ATTENDANCE_STATUS_INTERESTED);
    assert_eq!((d_interested.going_count, d_interested.waitlist_count, d_interested.interested_count), (2, 2, 1));

    // A cannot make it: B, who waited longest, moves up and is told.
    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let a_out = ctx.rsvp(a, &event, RSVP_ANSWER_NOT_GOING).await;
    assert_eq!(a_out.viewer.status, ATTENDANCE_STATUS_NOT_GOING);
    assert_eq!((a_out.going_count, a_out.waitlist_count), (2, 1));
    assert_eq!(ctx.get_event(Some(b), &event.id).await.unwrap().viewer.status, ATTENDANCE_STATUS_GOING);
    let told = ctx.notifications(b.id, "community.event_update").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["data"]["update"], "promoted");
    assert!(ctx.notifications(c.id, "community.event_update").await.is_empty());
    assert_eq!(ctx.get_event(Some(c), &event.id).await.unwrap().viewer.waitlist_position, 1);
    assert!(
        pushes(&mut topic)
            .await
            .iter()
            .any(|push| matches!(push, Pushed::CalendarEventChanged(changed) if changed.event.waitlist_count == 1))
    );

    // Answering "going" again keeps what one has; A, back again, queues behind C.
    assert_eq!(ctx.rsvp(b, &event, RSVP_ANSWER_GOING).await.viewer.status, ATTENDANCE_STATUS_GOING);
    assert_eq!(ctx.rsvp(c, &event, RSVP_ANSWER_GOING).await.viewer.waitlist_position, 1);
    assert_eq!(ctx.rsvp(a, &event, RSVP_ANSWER_GOING).await.viewer.waitlist_position, 2);

    // More room lets the whole queue in.
    let roomier = UpdateEventRequest { event_id: by_id(&event), capacity: Some(4), ..Default::default() };
    let updated: UpdateEventResponse = ctx.call(SVC, "UpdateEvent", Some(&owner), &roomier).await.unwrap();
    assert_eq!((updated.event.going_count, updated.event.waitlist_count), (4, 0));
    assert_eq!(ctx.notifications(c.id, "community.event_update").await[0]["data"]["update"], "promoted");
    // Full again: D, deciding to come after all, waits.
    assert_eq!(ctx.rsvp(d, &event, RSVP_ANSWER_GOING).await.viewer.status, ATTENDANCE_STATUS_WAITLISTED);

    // Someone who leaves the community gives up their place.
    let leave = LeaveCommunityRequest { community_id: community.id.clone(), ..Default::default() };
    let _: LeaveCommunityResponse = ctx.call("CommunityService", "LeaveCommunity", Some(c), &leave).await.unwrap();
    let after = ctx.get_event(Some(d), &event.id).await.unwrap();
    assert_eq!(after.viewer.status, ATTENDANCE_STATUS_GOING);
    assert_eq!((after.going_count, after.waitlist_count, after.interested_count), (4, 0, 0));

    assert_eq!(ctx.try_rsvp(a, &event.id, RSVP_ANSWER_UNSPECIFIED).await.unwrap_err().reason, "ANSWER_INVALID");
    assert_eq!(ctx.try_rsvp(&outsider, &event.id, RSVP_ANSWER_GOING).await.unwrap_err().reason, "NOT_A_MEMBER");
    assert_eq!(
        ctx.try_rsvp(a, &uuid::Uuid::now_v7().to_string(), RSVP_ANSWER_GOING).await.unwrap_err().reason,
        "EVENT_NOT_FOUND"
    );
    let anonymous = RsvpRequest { event_id: by_id(&event), answer: RSVP_ANSWER_GOING.into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "Rsvp", None, &anonymous).await, "SIGN_IN_REQUIRED");
    // An event that is over takes no answers.
    ctx.move_event(&event.id, Duration::hours(-5), Duration::hours(2)).await;
    assert_eq!(ctx.try_rsvp(a, &event.id, RSVP_ANSWER_INTERESTED).await.unwrap_err().reason, "EVENT_ENDED");
}

#[tokio::test]
async fn events_are_edited_cancelled_and_removed_by_those_who_run_them() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    open_events_to_members(&ctx, &owner, &community).await;
    let organizer = ctx.member(&owner, &community, "Organizer").await;
    let guest = ctx.member(&owner, &community, "Guest").await;
    let moderator = ctx.moderator(&owner, &community).await;
    let event = ctx.event(&organizer, &community).await;
    ctx.rsvp(&guest, &event, RsvpAnswer::RSVP_ANSWER_GOING).await;

    let retitle =
        UpdateEventRequest { event_id: by_id(&event), title: Some("オフ会 (改)".into()), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateEvent", Some(&guest), &retitle).await, "PERMISSION_DENIED");
    let updated: UpdateEventResponse = ctx.call(SVC, "UpdateEvent", Some(&organizer), &retitle).await.unwrap();
    assert_eq!(updated.event.title, "オフ会 (改)");
    // Nothing moved, so nobody is told.
    assert!(ctx.notifications(guest.id, "community.event_update").await.is_empty());

    // A new start keeps the length and tells everyone who answered — not the one who moved it.
    let new_start = now() + Duration::days(3);
    let moved = UpdateEventRequest { event_id: by_id(&event), starts_at: ts(new_start), ..Default::default() };
    let updated: UpdateEventResponse = ctx.call(SVC, "UpdateEvent", Some(&organizer), &moved).await.unwrap();
    assert_eq!(updated.event.starts_at.seconds, new_start.timestamp());
    assert_eq!(updated.event.ends_at.seconds - updated.event.starts_at.seconds, 7200);
    assert_eq!(ctx.notifications(guest.id, "community.event_update").await[0]["data"]["update"], "rescheduled");
    assert!(ctx.notifications(organizer.id, "community.event_update").await.is_empty());
    let into_the_past =
        UpdateEventRequest { event_id: by_id(&event), starts_at: ts(now() - Duration::hours(1)), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateEvent", Some(&organizer), &into_the_past).await, "TIME_INVALID");

    // Going online drops the place; a place brings it back.
    let online = UpdateEventRequest {
        event_id: by_id(&event),
        online: Some(true),
        online_url: Some("https://meet.example.com/x".into()),
        ..Default::default()
    };
    let updated: UpdateEventResponse = ctx.call(SVC, "UpdateEvent", Some(&moderator), &online).await.unwrap();
    assert!(updated.event.online && updated.event.place.as_option().is_none());
    let in_person = UpdateEventRequest {
        event_id: by_id(&event),
        place: EventPlace { name: "公園".into(), ..Default::default() }.into(),
        ..Default::default()
    };
    let updated: UpdateEventResponse = ctx.call(SVC, "UpdateEvent", Some(&organizer), &in_person).await.unwrap();
    assert!(!updated.event.online && updated.event.online_url.is_empty());
    assert_eq!(updated.event.place.name, "公園");

    // Called off by a moderator: it stays on the calendar, marked, and everyone who answered hears why.
    let cancel = CancelEventRequest { event_id: by_id(&event), reason: "雨のため".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CancelEvent", Some(&guest), &cancel).await, "PERMISSION_DENIED");
    let cancelled: CancelEventResponse = ctx.call(SVC, "CancelEvent", Some(&moderator), &cancel).await.unwrap();
    assert!(cancelled.event.cancelled);
    assert_eq!(cancelled.event.cancel_reason, "雨のため");
    for user in [&guest, &organizer] {
        let told = ctx.notifications(user.id, "community.event_update").await;
        assert_eq!(
            (told.last().unwrap()["data"]["update"].as_str(), told.last().unwrap()["body"]["ja"].as_str()),
            (Some("cancelled"), Some("雨のため"))
        );
    }
    assert_eq!(
        ctx.try_rsvp(&guest, &event.id, RsvpAnswer::RSVP_ANSWER_NOT_GOING).await.unwrap_err().reason,
        "EVENT_CANCELLED"
    );
    assert_eq!(ctx.reason(SVC, "UpdateEvent", Some(&organizer), &retitle).await, "EVENT_CANCELLED");
    let log = ctx.moderation_log(&owner, &community).await;
    let entry = log.iter().find(|entry| entry.action == ModerationAction::MODERATION_ACTION_EVENT_REMOVED).unwrap();
    assert_eq!(
        (entry.event_id.as_str(), entry.details["how"].as_str(), entry.target_user.id.as_str()),
        (event.id.as_str(), "cancelled", organizer.id.to_string().as_str())
    );

    // Removed: gone from the calendar.
    let delete = DeleteEventRequest { event_id: by_id(&event), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "DeleteEvent", Some(&guest), &delete).await, "PERMISSION_DENIED");
    let mut topic = ctx.app.listen_topic(&format!("community:{}", community.id)).await;
    let _: DeleteEventResponse = ctx.call(SVC, "DeleteEvent", Some(&organizer), &delete).await.unwrap();
    assert!(pushes(&mut topic).await.iter().any(|push| matches!(push, Pushed::CalendarEventChanged(changed)
        if changed.event_id == event.id && changed.event.as_option().is_none())));
    assert_eq!(ctx.get_event(Some(&guest), &event.id).await.unwrap_err().reason, "EVENT_NOT_FOUND");
    assert_eq!(ctx.reason(SVC, "DeleteEvent", Some(&organizer), &delete).await, "EVENT_NOT_FOUND");
}

#[tokio::test]
async fn the_calendar_lists_by_time() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.restricted(&owner).await;
    let member = ctx.member(&owner, &community, "Member").await;
    let outsider = ctx.app.user("Outsider").await;
    let named = |title: &str, starts_in: Duration| CreateEventRequest {
        title: title.into(),
        starts_at: ts(now() + starts_in),
        ..ctx.new_event(&community)
    };
    let over = ctx.try_event(&owner, &named("over", Duration::hours(1))).await.unwrap();
    ctx.move_event(&over.id, Duration::days(-1), Duration::hours(2)).await;
    let soon = ctx.try_event(&owner, &named("soon", Duration::days(1))).await.unwrap();
    let later = ctx.try_event(&owner, &named("later", Duration::days(8))).await.unwrap();
    let list = ListEventsRequest { community_id: community.id.clone(), ..Default::default() };
    let titles = |request: ListEventsRequest| {
        let ctx = &ctx;
        let member = &member;
        async move {
            let response: ListEventsResponse = ctx.call(SVC, "ListEvents", Some(member), &request).await.unwrap();
            response.events.into_iter().map(|event| event.title).collect::<Vec<_>>()
        }
    };

    assert_eq!(titles(list.clone()).await, vec!["soon", "later"]);
    assert_eq!(titles(ListEventsRequest { past: true, ..list.clone() }).await, vec!["over"]);
    let this_week = ListEventsRequest { from: ts(now()), to: ts(now() + Duration::days(7)), ..list.clone() };
    assert_eq!(titles(this_week).await, vec!["soon"]);
    let everything =
        ListEventsRequest { from: ts(now() - Duration::days(2)), to: ts(now() + Duration::days(30)), ..list.clone() };
    assert_eq!(titles(everything).await, vec!["over", "soon", "later"]);
    let paged: ListEventsResponse = ctx
        .call(SVC, "ListEvents", Some(&member), &ListEventsRequest { page: page(1, ""), ..list.clone() })
        .await
        .unwrap();
    assert!(paged.page.has_more);
    assert_eq!(
        titles(ListEventsRequest { page: page(1, &paged.page.next_cursor), ..list.clone() }).await,
        vec!["later"]
    );

    for bad in [
        ListEventsRequest { from: ts(now()), ..list.clone() },
        ListEventsRequest { from: ts(now()), to: ts(now() - Duration::days(1)), ..list.clone() },
        ListEventsRequest { from: ts(now()), to: ts(now() + Duration::days(93)), ..list.clone() },
    ] {
        assert_eq!(ctx.reason(SVC, "ListEvents", Some(&member), &bad).await, "RANGE_INVALID");
    }
    // A restricted community's calendar is for its members.
    assert_eq!(ctx.reason(SVC, "ListEvents", Some(&outsider), &list).await, "NOT_A_MEMBER");
    assert_eq!(ctx.get_event(Some(&outsider), &soon.id).await.unwrap_err().reason, "NOT_A_MEMBER");

    // My calendar: what I answered, across communities, soonest first.
    let elsewhere = ctx.public(&owner).await;
    ctx.join(&member, &elsewhere).await;
    let between = ctx
        .try_event(
            &owner,
            &CreateEventRequest {
                title: "between".into(),
                starts_at: ts(now() + Duration::days(4)),
                ..ctx.new_event(&elsewhere)
            },
        )
        .await
        .unwrap();
    ctx.rsvp(&member, &later, RsvpAnswer::RSVP_ANSWER_GOING).await;
    ctx.rsvp(&member, &between, RsvpAnswer::RSVP_ANSWER_INTERESTED).await;
    ctx.rsvp(&member, &soon, RsvpAnswer::RSVP_ANSWER_NOT_GOING).await;
    let mine: ListMyEventsResponse =
        ctx.call(SVC, "ListMyEvents", Some(&member), &ListMyEventsRequest::default()).await.unwrap();
    assert_eq!(mine.events.iter().map(|event| event.title.as_str()).collect::<Vec<_>>(), vec!["between", "later"]);
    assert_eq!(mine.events[1].viewer.status, AttendanceStatus::ATTENDANCE_STATUS_GOING);
    assert_eq!(ctx.reason(SVC, "ListMyEvents", None, &ListMyEventsRequest::default()).await, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn attendees_check_in_with_the_code_on_the_organisers_screen() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let (going, interested, silent) = (
        ctx.member(&owner, &community, "Going").await,
        ctx.member(&owner, &community, "Interested").await,
        ctx.member(&owner, &community, "Silent").await,
    );
    let outsider = ctx.app.user("Outsider").await;
    let event = ctx.event(&owner, &community).await;
    ctx.rsvp(&going, &event, RsvpAnswer::RSVP_ANSWER_GOING).await;
    ctx.rsvp(&interested, &event, RsvpAnswer::RSVP_ANSWER_INTERESTED).await;
    let code_request = GetCheckInCodeRequest { event_id: by_id(&event), ..Default::default() };
    let check_in = |code: &str| CheckInRequest { event_id: by_id(&event), code: code.into(), ..Default::default() };

    // Tomorrow's event is not open for check-in yet.
    assert_eq!(ctx.reason(SVC, "GetCheckInCode", Some(&owner), &code_request).await, "CHECK_IN_CLOSED");
    assert_eq!(ctx.reason(SVC, "CheckIn", Some(&going), &check_in("123456")).await, "CHECK_IN_CLOSED");
    ctx.move_event(&event.id, Duration::minutes(30), Duration::hours(2)).await;

    assert_eq!(ctx.reason(SVC, "GetCheckInCode", Some(&going), &code_request).await, "PERMISSION_DENIED");
    let shown: GetCheckInCodeResponse = ctx.call(SVC, "GetCheckInCode", Some(&owner), &code_request).await.unwrap();
    assert_eq!(shown.code.len(), 6);
    assert!(shown.code.bytes().all(|b| b.is_ascii_digit()));
    assert!(shown.expires_at.seconds > now().timestamp() - 1);
    let wrong = if shown.code == "000000" { "000001" } else { "000000" };

    assert_eq!(ctx.reason(SVC, "CheckIn", Some(&going), &check_in(wrong)).await, "CODE_INVALID");
    assert_eq!(ctx.reason(SVC, "CheckIn", Some(&interested), &check_in(&shown.code)).await, "NOT_GOING");
    assert_eq!(ctx.reason(SVC, "CheckIn", Some(&silent), &check_in(&shown.code)).await, "NOT_GOING");
    assert_eq!(ctx.reason(SVC, "CheckIn", Some(&outsider), &check_in(&shown.code)).await, "NOT_A_MEMBER");

    // The code may have rotated since it was shown; ask again like a screen would.
    let shown: GetCheckInCodeResponse = ctx.call(SVC, "GetCheckInCode", Some(&owner), &code_request).await.unwrap();
    let checked: CheckInResponse = ctx.call(SVC, "CheckIn", Some(&going), &check_in(&shown.code)).await.unwrap();
    assert!(checked.event.viewer.checked_in_at.as_option().is_some());
    assert_eq!(checked.event.attended_count, 1);
    // Twice is once.
    let again: CheckInResponse = ctx.call(SVC, "CheckIn", Some(&going), &check_in(&shown.code)).await.unwrap();
    assert_eq!(again.event.attended_count, 1);
    let attended = ctx.app.events_by("community.event.attended", going.id).await;
    assert_eq!(attended.len(), 1);
    assert_eq!(attended[0].data["event_id"], event.id);
    assert_eq!(
        ctx.app
            .events_by("activity.recorded", going.id)
            .await
            .iter()
            .filter(|e| e.data["kind"] == "community.event_attended")
            .count(),
        1
    );

    // A dead phone: the organiser checks in by hand someone who said they
    // would come. Nobody is recorded as having been somewhere on the
    // organiser's word alone.
    let late = ctx.member(&owner, &community, "Late").await;
    ctx.rsvp(&late, &event, RsvpAnswer::RSVP_ANSWER_GOING).await;
    let by_hand = |user: &TestUser| CheckInAttendeeRequest {
        event_id: by_id(&event),
        user_id: user.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.reason(SVC, "CheckInAttendee", Some(&going), &by_hand(&late)).await, "PERMISSION_DENIED");
    assert_eq!(ctx.reason(SVC, "CheckInAttendee", Some(&owner), &by_hand(&outsider)).await, "MEMBER_NOT_FOUND");
    assert_eq!(ctx.reason(SVC, "CheckInAttendee", Some(&owner), &by_hand(&silent)).await, "NOT_GOING");
    assert_eq!(ctx.reason(SVC, "CheckInAttendee", Some(&owner), &by_hand(&interested)).await, "NOT_GOING");
    assert!(ctx.app.events_by("community.event.attended", silent.id).await.is_empty());
    let manual: CheckInAttendeeResponse =
        ctx.call(SVC, "CheckInAttendee", Some(&owner), &by_hand(&late)).await.unwrap();
    assert_eq!(manual.attendee.status, AttendanceStatus::ATTENDANCE_STATUS_GOING);
    assert!(manual.attendee.checked_in_at.as_option().is_some());
    let after = ctx.get_event(Some(&owner), &event.id).await.unwrap();
    assert_eq!((after.going_count, after.attended_count, after.interested_count), (3, 2, 1));

    // The attendance list is the organiser's; members see who comes, not who showed up.
    let list = ListAttendeesRequest { event_id: by_id(&event), ..Default::default() };
    let as_organizer = attendees(&ctx, &owner, &list).await;
    assert_eq!(as_organizer.len(), 3);
    assert_eq!(as_organizer.iter().filter(|attendee| attendee.checked_in_at.as_option().is_some()).count(), 2);
    let as_member = attendees(&ctx, &going, &list).await;
    assert_eq!(as_member.len(), 3);
    assert!(as_member.iter().all(|attendee| attendee.checked_in_at.as_option().is_none()));
    let present = ListAttendeesRequest { checked_in_only: true, ..list.clone() };
    let ids: Vec<String> =
        attendees(&ctx, &owner, &present).await.into_iter().map(|attendee| attendee.user.id.clone()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&going.id.to_string()) && ids.contains(&late.id.to_string()));
    assert_eq!(ctx.reason(SVC, "ListAttendees", Some(&going), &present).await, "PERMISSION_DENIED");
    let maybe = ListAttendeesRequest { status: AttendanceStatus::ATTENDANCE_STATUS_INTERESTED.into(), ..list.clone() };
    assert_eq!(attendees(&ctx, &going, &maybe).await[0].user.id, interested.id.to_string());
    assert_eq!(ctx.reason(SVC, "ListAttendees", Some(&outsider), &list).await, "NOT_A_MEMBER");

    // Long after the end the doors are closed.
    ctx.move_event(&event.id, Duration::hours(-10), Duration::hours(2)).await;
    assert_eq!(ctx.reason(SVC, "GetCheckInCode", Some(&owner), &code_request).await, "CHECK_IN_CLOSED");
}

#[tokio::test]
async fn check_in_codes_cannot_be_tried_forever() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let guesser = ctx.member(&owner, &community, "Guesser").await;
    let event = ctx.event(&owner, &community).await;
    ctx.rsvp(&guesser, &event, RsvpAnswer::RSVP_ANSWER_GOING).await;
    ctx.move_event(&event.id, Duration::minutes(10), Duration::hours(1)).await;
    let shown: GetCheckInCodeResponse = ctx
        .call(
            SVC,
            "GetCheckInCode",
            Some(&owner),
            &GetCheckInCodeRequest { event_id: by_id(&event), ..Default::default() },
        )
        .await
        .unwrap();
    let wrong = if shown.code.starts_with('9') { "000000" } else { "999999" };
    let guess = CheckInRequest { event_id: by_id(&event), code: wrong.into(), ..Default::default() };
    for _ in 0..10 {
        assert_eq!(ctx.reason(SVC, "CheckIn", Some(&guesser), &guess).await, "CODE_INVALID");
    }
    assert_eq!(ctx.reason(SVC, "CheckIn", Some(&guesser), &guess).await, "RATE_LIMITED");
}

#[tokio::test]
async fn reminders_go_out_a_day_and_an_hour_before() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    let (going, interested, declined, muted) = (
        ctx.member(&owner, &community, "Going").await,
        ctx.member(&owner, &community, "Interested").await,
        ctx.member(&owner, &community, "Declined").await,
        ctx.member(&owner, &community, "Muted").await,
    );
    let quiet = UpdateMembershipRequest {
        community_id: community.id.clone(),
        notification_level: Some(NotificationLevel::NOTIFICATION_LEVEL_NONE.into()),
        ..Default::default()
    };
    let _: UpdateMembershipResponse =
        ctx.call("CommunityService", "UpdateMembership", Some(&muted), &quiet).await.unwrap();
    let event = ctx.event(&owner, &community).await;
    ctx.rsvp(&going, &event, RsvpAnswer::RSVP_ANSWER_GOING).await;
    ctx.rsvp(&interested, &event, RsvpAnswer::RSVP_ANSWER_INTERESTED).await;
    ctx.rsvp(&declined, &event, RsvpAnswer::RSVP_ANSWER_NOT_GOING).await;
    ctx.rsvp(&muted, &event, RsvpAnswer::RSVP_ANSWER_GOING).await;
    let reminders = |user: &TestUser| {
        let (ctx, id) = (&ctx, user.id);
        async move {
            ctx.notifications(id, "community.event_reminder")
                .await
                .into_iter()
                .map(|notice| notice["data"]["reminder"].as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        }
    };

    // Twenty hours ahead: the day-before reminder, once.
    ctx.move_event(&event.id, Duration::hours(20), Duration::hours(2)).await;
    assert!(ctx.community.send_due_reminders().await.unwrap() >= 1);
    ctx.community.send_due_reminders().await.unwrap();
    for user in [&owner, &going, &interested] {
        assert_eq!(reminders(user).await, vec!["day"], "{}", user.handle);
    }
    for user in [&declined, &muted] {
        assert!(reminders(user).await.is_empty(), "{}", user.handle);
    }

    // Half an hour ahead: the last call.
    ctx.move_event(&event.id, Duration::minutes(30), Duration::hours(2)).await;
    ctx.community.send_due_reminders().await.unwrap();
    ctx.community.send_due_reminders().await.unwrap();
    assert_eq!(reminders(&going).await, vec!["day", "hour"]);
    assert_eq!(reminders(&interested).await, vec!["day", "hour"]);
    assert!(reminders(&declined).await.is_empty());

    // A cancelled event reminds nobody; nor does one put on the calendar at the last minute.
    let called_off = ctx.event(&owner, &community).await;
    ctx.rsvp(&declined, &called_off, RsvpAnswer::RSVP_ANSWER_GOING).await;
    let cancel = CancelEventRequest { event_id: by_id(&called_off), ..Default::default() };
    let _: CancelEventResponse = ctx.call(SVC, "CancelEvent", Some(&owner), &cancel).await.unwrap();
    ctx.move_event(&called_off.id, Duration::minutes(30), Duration::hours(2)).await;
    let last_minute = CreateEventRequest { starts_at: ts(now() + Duration::minutes(45)), ..ctx.new_event(&community) };
    let spontaneous = ctx.try_event(&owner, &last_minute).await.unwrap();
    ctx.rsvp(&declined, &spontaneous, RsvpAnswer::RSVP_ANSWER_GOING).await;
    ctx.community.send_due_reminders().await.unwrap();
    assert!(reminders(&declined).await.is_empty());
}

#[tokio::test]
async fn a_timed_out_organiser_does_not_manage_their_event() {
    let ctx = start().await;
    let owner = ctx.app.user("Owner").await;
    let community = ctx.public(&owner).await;
    open_events_to_members(&ctx, &owner, &community).await;
    let organizer = ctx.member(&owner, &community, "Organizer").await;
    let guest = ctx.member(&owner, &community, "Guest").await;
    let event = ctx.event(&organizer, &community).await;
    ctx.rsvp(&guest, &event, RsvpAnswer::RSVP_ANSWER_GOING).await;
    let timeout = TimeoutMemberRequest {
        community_id: community.id.clone(),
        user_id: organizer.id.to_string(),
        duration_seconds: 600,
        reason: "頭を冷やそう".into(),
        ..Default::default()
    };
    let _: TimeoutMemberResponse =
        ctx.call("CommunityModerationService", "TimeoutMember", Some(&owner), &timeout).await.unwrap();

    // Each of these would tell every attendee something.
    let moved =
        UpdateEventRequest { event_id: by_id(&event), starts_at: ts(now() + Duration::days(5)), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateEvent", Some(&organizer), &moved).await, "TIMED_OUT");
    let retitle =
        UpdateEventRequest { event_id: by_id(&event), title: Some("腹いせの題".into()), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "UpdateEvent", Some(&organizer), &retitle).await, "TIMED_OUT");
    let cancel = CancelEventRequest { event_id: by_id(&event), reason: "腹いせ".into(), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "CancelEvent", Some(&organizer), &cancel).await, "TIMED_OUT");
    let delete = DeleteEventRequest { event_id: by_id(&event), ..Default::default() };
    assert_eq!(ctx.reason(SVC, "DeleteEvent", Some(&organizer), &delete).await, "TIMED_OUT");
    assert!(ctx.notifications(guest.id, "community.event_update").await.is_empty());
    assert_eq!(ctx.get_event(Some(&guest), &event.id).await.unwrap().title, "オフ会");
    // The staff still runs it meanwhile.
    let _: UpdateEventResponse = ctx.call(SVC, "UpdateEvent", Some(&owner), &moved).await.unwrap();
    assert_eq!(ctx.notifications(guest.id, "community.event_update").await.len(), 1);
}
