//! `CommunityEventService`: the calendar, RSVP and check-in.
use std::collections::HashMap;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::community::*;

use crate::{Community as Domain, access::Caller, calendar::EventEdit, model::EventRow, validate};

impl Domain {
    /// Events from several communities, each rendered for the caller's
    /// standing in its own community, in the order given.
    async fn render_mixed_events(&self, caller: &Caller, rows: &[EventRow]) -> AppResult<Vec<CalendarEvent>> {
        let mut by_community: HashMap<Uuid, Vec<EventRow>> = HashMap::new();
        for row in rows {
            by_community.entry(row.community_id).or_default().push(row.clone());
        }
        let mut rendered: HashMap<String, CalendarEvent> = HashMap::with_capacity(rows.len());
        for (community_id, group) in by_community {
            let Ok(access) = self.access_by_id(caller, community_id).await else { continue };
            for event in self.render_events(&access, &group).await? {
                rendered.insert(event.id.clone(), event);
            }
        }
        Ok(rows.iter().filter_map(|row| rendered.remove(&row.id.to_string())).collect())
    }
}

impl CommunityEventService for Domain {
    async fn create_event(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateEventRequest>,
    ) -> ServiceResult<CreateEventResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let access = self.access(&caller, &req.community_id).await?;
            let event = self.create_event(&access, &req).await?;
            ok(CreateEventResponse { event: self.render_event(&access, &event).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_event(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateEventRequest>,
    ) -> ServiceResult<UpdateEventResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let (event, access) = self.open_event(&caller, &req.event_id).await?;
            let edit = EventEdit {
                title: req.title.as_deref().map(validate::title).transpose()?,
                description: req
                    .description
                    .as_deref()
                    .map(|d| validate::description(d, validate::MAX_EVENT_DESCRIPTION_CHARS))
                    .transpose()?,
                starts_at: req.starts_at.as_option().map(from_ts),
                ends_at: req.ends_at.as_option().map(from_ts),
                place: req.place.as_option().map(validate::place).transpose()?,
                online: req.online,
                online_url: req.online_url.as_deref().map(validate::online_url).transpose()?,
                cover_asset_id: req.cover_asset_id.as_deref().map(parse_opt_id).transpose()?,
                capacity: req.capacity.map(validate::capacity).transpose()?,
            };
            let updated = self.update_event(&access, &event, edit).await?;
            ok(UpdateEventResponse { event: self.render_event(&access, &updated).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn cancel_event(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CancelEventRequest>,
    ) -> ServiceResult<CancelEventResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (event, access) = self.open_event(&caller, request.event_id).await?;
            let cancelled = self.cancel_event(&access, &event, request.reason).await?;
            ok(CancelEventResponse {
                event: self.render_event(&access, &cancelled).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_event(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteEventRequest>,
    ) -> ServiceResult<DeleteEventResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (event, access) = self.open_event(&caller, request.event_id).await?;
            self.delete_event(&access, &event).await?;
            ok(DeleteEventResponse::default())
        })
        .await
    }

    async fn get_event(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetEventRequest>,
    ) -> ServiceResult<GetEventResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let (event, access) = self.open_event(&caller, request.event_id).await?;
            ok(GetEventResponse { event: self.render_event(&access, &event).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_events(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListEventsRequest>,
    ) -> ServiceResult<ListEventsResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let req = request.to_owned_message();
            let access = self.access(&caller, &req.community_id).await?;
            access.require_read()?;
            let range = match (req.from.as_option().map(from_ts), req.to.as_option().map(from_ts)) {
                (Some(from), Some(to)) => Some((from, to)),
                (None, None) => None,
                _ => bail!(invalid, "RANGE_INVALID", "give both ends of the range, or neither"),
            };
            let page = Page::from_request(req.page.as_option(), 100);
            let (rows, info) = self.list_events(access.community.id, range, req.past, &page).await?;
            ok(ListEventsResponse {
                events: self.render_events(&access, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_my_events(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMyEventsRequest>,
    ) -> ServiceResult<ListMyEventsResponse> {
        handle(async {
            let (caller, user_id) = Caller::signed_in(&ctx)?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (rows, info) = self.my_events(user_id, &page).await?;
            ok(ListMyEventsResponse {
                events: self.render_mixed_events(&caller, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn rsvp(&self, ctx: RequestContext, request: ServiceRequest<'_, RsvpRequest>) -> ServiceResult<RsvpResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (event, access) = self.open_event(&caller, request.event_id).await?;
            let answer = request.answer.as_known().unwrap_or(RsvpAnswer::RSVP_ANSWER_UNSPECIFIED);
            let updated = self.rsvp(&access, &event, answer).await?;
            ok(RsvpResponse { event: self.render_event(&access, &updated).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn list_attendees(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListAttendeesRequest>,
    ) -> ServiceResult<ListAttendeesResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (event, access) = self.open_event(&caller, request.event_id).await?;
            let page = Page::from_view(request.page.as_option(), 100);
            let (attendees, info) =
                self.list_attendees(&access, &event, request.status.to_i32(), request.checked_in_only, &page).await?;
            ok(ListAttendeesResponse { attendees, page: info.into(), ..Default::default() })
        })
        .await
    }

    async fn get_check_in_code(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetCheckInCodeRequest>,
    ) -> ServiceResult<GetCheckInCodeResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (event, access) = self.open_event(&caller, request.event_id).await?;
            let (code, expires_at) = self.current_code(&access, &event)?;
            ok(GetCheckInCodeResponse { code, expires_at: ts(expires_at), ..Default::default() })
        })
        .await
    }

    async fn check_in(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CheckInRequest>,
    ) -> ServiceResult<CheckInResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (event, access) = self.open_event(&caller, request.event_id).await?;
            let updated = self.check_in(&access, &event, request.code).await?;
            ok(CheckInResponse { event: self.render_event(&access, &updated).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn check_in_attendee(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CheckInAttendeeRequest>,
    ) -> ServiceResult<CheckInAttendeeResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (event, access) = self.open_event(&caller, request.event_id).await?;
            let attendee = self.check_in_attendee(&access, &event, parse_id(request.user_id)?).await?;
            ok(CheckInAttendeeResponse { attendee: attendee.into(), ..Default::default() })
        })
        .await
    }
}
