//! `SpaceService`: thin handlers over [`crate::rooms`],
//! [`crate::participants`], [`crate::invites`] and [`crate::discovery`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::spaces::*;

use crate::{
    Spaces,
    model::{ParticipantRow, Venue},
    notices,
    render::Sight,
    rules,
};

/// Largest page of people (participants, bans, invitations).
const MAX_PEOPLE_PAGE: i64 = 100;
/// Largest page of rooms.
const MAX_ROOM_PAGE: i64 = 50;

impl Spaces {
    /// Credentials for someone who is inside, when audio is configured.
    async fn credentials_for(&self, row: &ParticipantRow) -> AppResult<Option<AudioCredentials>> {
        if !self.audio_enabled() {
            return Ok(None);
        }
        let me = tokuly_identity::summary(&self.core, row.user_id).await?;
        let venue = Venue::Space(row.space_id);
        // New credentials are a way in: the room must be there to take them.
        self.open_room(venue).await?;
        Ok(Some(self.credentials(venue, row.user_id, &notices::name_of(&me), row.role)?))
    }
}

impl SpaceService for Spaces {
    async fn get_spaces_config(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, GetSpacesConfigRequest>,
    ) -> ServiceResult<GetSpacesConfigResponse> {
        handle(async {
            let cfg = &self.cfg;
            let config = SpacesConfig {
                audio_enabled: self.audio_enabled(),
                max_speakers: cfg.max_speakers,
                max_cohosts: cfg.max_cohosts,
                max_listeners: cfg.max_listeners,
                max_title_length: rules::MAX_TITLE_CHARS as i32,
                max_tags: rules::MAX_TAGS as i32,
                max_schedule_days: cfg.max_schedule_days.min(i64::from(i32::MAX)) as i32,
                host_grace_seconds: cfg.host_grace_seconds.min(i64::from(i32::MAX)) as i32,
                max_hangouts: cfg.max_hangouts.min(i64::from(i32::MAX)) as i32,
                max_hangout_members: cfg.max_hangout_members.min(i64::from(i32::MAX)) as i32,
                ..Default::default()
            };
            ok(GetSpacesConfigResponse { config: config.into(), ..Default::default() })
        })
        .await
    }

    async fn create_space(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateSpaceRequest>,
    ) -> ServiceResult<CreateSpaceResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (space, credentials) = Spaces::create_space(self, user_id, request.to_owned_message()).await?;
            ok(CreateSpaceResponse {
                space: self.render_space(&space, Some(user_id)).await?.into(),
                credentials: credentials.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_space(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateSpaceRequest>,
    ) -> ServiceResult<UpdateSpaceResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let space = Spaces::update_space(self, user_id, request.to_owned_message()).await?;
            ok(UpdateSpaceResponse {
                space: self.render_space(&space, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn start_space(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartSpaceRequest>,
    ) -> ServiceResult<StartSpaceResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (space, credentials) = Spaces::start_space(self, user_id, parse_id(request.space_id)?).await?;
            ok(StartSpaceResponse {
                space: self.render_space(&space, Some(user_id)).await?.into(),
                credentials: credentials.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn cancel_space(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CancelSpaceRequest>,
    ) -> ServiceResult<CancelSpaceResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let space = Spaces::cancel_space(self, user_id, parse_id(request.space_id)?).await?;
            ok(CancelSpaceResponse {
                space: self.render_space(&space, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn end_space(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, EndSpaceRequest>,
    ) -> ServiceResult<EndSpaceResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let space = Spaces::end_space(self, user_id, parse_id(request.space_id)?).await?;
            ok(EndSpaceResponse { space: self.render_space(&space, Some(user_id)).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_space(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteSpaceRequest>,
    ) -> ServiceResult<DeleteSpaceResponse> {
        handle(async {
            Spaces::delete_space(self, ctx.user_id()?, parse_id(request.space_id)?).await?;
            ok(DeleteSpaceResponse::default())
        })
        .await
    }

    async fn get_space(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetSpaceRequest>,
    ) -> ServiceResult<GetSpaceResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let space = self.visible_space(viewer, parse_id(request.space_id)?).await?;
            ok(GetSpaceResponse { space: self.render_space(&space, viewer).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn join_space(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, JoinSpaceRequest>,
    ) -> ServiceResult<JoinSpaceResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (space, me, credentials) = Spaces::join_space(self, user_id, parse_id(request.space_id)?).await?;
            ok(JoinSpaceResponse {
                space: self.render_space(&space, Some(user_id)).await?.into(),
                credentials: credentials.into(),
                me: self.render_participant(&me, Sight::Full).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn leave_space(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LeaveSpaceRequest>,
    ) -> ServiceResult<LeaveSpaceResponse> {
        handle(async {
            Spaces::leave_space(self, ctx.user_id()?, parse_id(request.space_id)?).await?;
            ok(LeaveSpaceResponse::default())
        })
        .await
    }

    async fn list_participants(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListParticipantsRequest>,
    ) -> ServiceResult<ListParticipantsResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), MAX_PEOPLE_PAGE);
            let (participants, page, hidden_count) = Spaces::list_participants(
                self,
                ctx.user_id()?,
                parse_id(request.space_id)?,
                request.filter.as_known(),
                &page,
            )
            .await?;
            ok(ListParticipantsResponse { participants, page: page.into(), hidden_count, ..Default::default() })
        })
        .await
    }

    async fn set_hand_raised(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetHandRaisedRequest>,
    ) -> ServiceResult<SetHandRaisedResponse> {
        handle(async {
            let me = Spaces::set_hand_raised(self, ctx.user_id()?, parse_id(request.space_id)?, request.raised).await?;
            ok(SetHandRaisedResponse {
                me: self.render_participant(&me, Sight::Full).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn invite_to_stage(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InviteToStageRequest>,
    ) -> ServiceResult<InviteToStageResponse> {
        handle(async {
            let (participant, on_stage) =
                Spaces::invite_to_stage(self, ctx.user_id()?, parse_id(request.space_id)?, parse_id(request.user_id)?)
                    .await?;
            ok(InviteToStageResponse {
                participant: self.render_participant(&participant, Sight::Full).await?.into(),
                on_stage,
                ..Default::default()
            })
        })
        .await
    }

    async fn respond_to_stage_invite(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RespondToStageInviteRequest>,
    ) -> ServiceResult<RespondToStageInviteResponse> {
        handle(async {
            let me = Spaces::respond_to_stage_invite(self, ctx.user_id()?, parse_id(request.space_id)?, request.accept)
                .await?;
            ok(RespondToStageInviteResponse {
                credentials: self.credentials_for(&me).await?.into(),
                me: self.render_participant(&me, Sight::Full).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn move_to_audience(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MoveToAudienceRequest>,
    ) -> ServiceResult<MoveToAudienceResponse> {
        handle(async {
            let target = parse_opt_id(request.user_id)?;
            let participant =
                Spaces::move_to_audience(self, ctx.user_id()?, parse_id(request.space_id)?, target).await?;
            ok(MoveToAudienceResponse {
                participant: self.render_participant(&participant, Sight::Full).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn set_cohost(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetCohostRequest>,
    ) -> ServiceResult<SetCohostResponse> {
        handle(async {
            let participant = Spaces::set_cohost(
                self,
                ctx.user_id()?,
                parse_id(request.space_id)?,
                parse_id(request.user_id)?,
                request.cohost,
            )
            .await?;
            ok(SetCohostResponse {
                participant: self.render_participant(&participant, Sight::Full).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn transfer_host(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, TransferHostRequest>,
    ) -> ServiceResult<TransferHostResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let space =
                Spaces::transfer_host(self, user_id, parse_id(request.space_id)?, parse_id(request.user_id)?).await?;
            ok(TransferHostResponse {
                space: self.render_space(&space, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn mute_speaker(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MuteSpeakerRequest>,
    ) -> ServiceResult<MuteSpeakerResponse> {
        handle(async {
            let muted_tracks =
                Spaces::mute_speaker(self, ctx.user_id()?, parse_id(request.space_id)?, parse_id(request.user_id)?)
                    .await?;
            ok(MuteSpeakerResponse { muted_tracks, ..Default::default() })
        })
        .await
    }

    async fn remove_participant(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveParticipantRequest>,
    ) -> ServiceResult<RemoveParticipantResponse> {
        handle(async {
            Spaces::remove_participant(
                self,
                ctx.user_id()?,
                parse_id(request.space_id)?,
                parse_id(request.user_id)?,
                request.ban,
            )
            .await?;
            ok(RemoveParticipantResponse::default())
        })
        .await
    }

    async fn unban_user(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UnbanUserRequest>,
    ) -> ServiceResult<UnbanUserResponse> {
        handle(async {
            Spaces::unban_user(self, ctx.user_id()?, parse_id(request.space_id)?, parse_id(request.user_id)?).await?;
            ok(UnbanUserResponse::default())
        })
        .await
    }

    async fn list_bans(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListBansRequest>,
    ) -> ServiceResult<ListBansResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), MAX_PEOPLE_PAGE);
            let (bans, page) = Spaces::list_bans(self, ctx.user_id()?, parse_id(request.space_id)?, &page).await?;
            ok(ListBansResponse { bans, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn send_reaction(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SendReactionRequest>,
    ) -> ServiceResult<SendReactionResponse> {
        handle(async {
            Spaces::send_reaction(self, ctx.user_id()?, parse_id(request.space_id)?, request.emoji, request.nonce)
                .await?;
            ok(SendReactionResponse::default())
        })
        .await
    }

    async fn set_pin(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetPinRequest>,
    ) -> ServiceResult<SetPinResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let space = Spaces::set_pin(self, user_id, parse_id(request.space_id)?, request.text, request.url).await?;
            ok(SetPinResponse { space: self.render_space(&space, Some(user_id)).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn invite_to_space(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InviteToSpaceRequest>,
    ) -> ServiceResult<InviteToSpaceResponse> {
        handle(async {
            let request = request.to_owned_message();
            let invited_count =
                Spaces::invite_to_space(self, ctx.user_id()?, parse_id(&request.space_id)?, &request.user_ids).await?;
            ok(InviteToSpaceResponse { invited_count, ..Default::default() })
        })
        .await
    }

    async fn revoke_space_invite(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeSpaceInviteRequest>,
    ) -> ServiceResult<RevokeSpaceInviteResponse> {
        handle(async {
            Spaces::revoke_space_invite(self, ctx.user_id()?, parse_id(request.space_id)?, parse_id(request.user_id)?)
                .await?;
            ok(RevokeSpaceInviteResponse::default())
        })
        .await
    }

    async fn list_space_invites(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListSpaceInvitesRequest>,
    ) -> ServiceResult<ListSpaceInvitesResponse> {
        handle(async {
            let page = Page::from_view(request.page.as_option(), MAX_PEOPLE_PAGE);
            let (invites, page) =
                Spaces::list_space_invites(self, ctx.user_id()?, parse_id(request.space_id)?, &page).await?;
            ok(ListSpaceInvitesResponse { invites, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn set_reminder(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetReminderRequest>,
    ) -> ServiceResult<SetReminderResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let space = Spaces::set_reminder(self, user_id, parse_id(request.space_id)?, request.remind).await?;
            ok(SetReminderResponse {
                space: self.render_space(&space, Some(user_id)).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_live_spaces(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListLiveSpacesRequest>,
    ) -> ServiceResult<ListLiveSpacesResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            self.limit_discovery(viewer, &ctx.client().ip).await?;
            let page = Page::from_view(request.page.as_option(), MAX_ROOM_PAGE);
            let (spaces, page) =
                self.list_live(viewer, request.scope.as_known(), request.tag, request.language, &page).await?;
            ok(ListLiveSpacesResponse { spaces, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn list_upcoming_spaces(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListUpcomingSpacesRequest>,
    ) -> ServiceResult<ListUpcomingSpacesResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            if request.reminders_only {
                ctx.user_id()?;
            }
            self.limit_discovery(viewer, &ctx.client().ip).await?;
            let page = Page::from_view(request.page.as_option(), MAX_ROOM_PAGE);
            let (spaces, page) =
                self.list_upcoming(viewer, request.tag, request.language, request.reminders_only, &page).await?;
            ok(ListUpcomingSpacesResponse { spaces, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn list_my_spaces(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMySpacesRequest>,
    ) -> ServiceResult<ListMySpacesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_discovery(Some(user_id), "").await?;
            let page = Page::from_view(request.page.as_option(), MAX_ROOM_PAGE);
            let (spaces, page) = self.list_mine(user_id, &page).await?;
            ok(ListMySpacesResponse { spaces, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn list_popular_tags(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListPopularTagsRequest>,
    ) -> ServiceResult<ListPopularTagsResponse> {
        handle(async {
            self.limit_discovery(ctx.maybe_user_id()?, &ctx.client().ip).await?;
            let tags = self.popular_tags(request.language, request.limit).await?;
            ok(ListPopularTagsResponse { tags, ..Default::default() })
        })
        .await
    }

    async fn list_friends_in_rooms(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListFriendsInRoomsRequest>,
    ) -> ServiceResult<ListFriendsInRoomsResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_discovery(Some(user_id), "").await?;
            let friends = self.list_friends_in_rooms(user_id).await?;
            ok(ListFriendsInRoomsResponse { friends, ..Default::default() })
        })
        .await
    }
}
