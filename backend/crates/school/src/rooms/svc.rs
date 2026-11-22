//! `StudyRoomService`: thin RPC handlers.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::school::*;

use crate::School;

impl StudyRoomService for School {
    async fn create_study_room(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateStudyRoomRequest>,
    ) -> ServiceResult<CreateStudyRoomResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (room, session) = self.create_room(user_id, &request.to_owned_message()).await?;
            let at = self.now();
            ok(CreateStudyRoomResponse {
                room: self.room_proto(&room, user_id, at).await?.into(),
                session: self.session_proto(&session, Some(&room), at).await?.into(),
                heartbeat_interval_seconds: self.cfg.heartbeat_interval_seconds as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_study_room(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetStudyRoomRequest>,
    ) -> ServiceResult<GetStudyRoomResponse> {
        handle(async {
            let viewer = ctx.user_id()?;
            let room = self.visible_room(parse_id(request.room_id)?, viewer).await?;
            ok(GetStudyRoomResponse {
                room: self.room_proto(&room, viewer, self.now()).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_study_room(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateStudyRoomRequest>,
    ) -> ServiceResult<UpdateStudyRoomResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let room = self.update_room(user_id, &request.to_owned_message()).await?;
            ok(UpdateStudyRoomResponse {
                room: self.room_proto(&room, user_id, self.now()).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn close_study_room(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CloseStudyRoomRequest>,
    ) -> ServiceResult<CloseStudyRoomResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let room = self.visible_room(parse_id(request.room_id)?, user_id).await?;
            ensure!(room.host_id == user_id, denied, "NOT_HOST", "only the host can close the room");
            // Closing a closed room changes nothing.
            self.close_room(room.id, RoomCloseReason::ROOM_CLOSE_REASON_HOST, Some(user_id), self.now()).await?;
            ok(CloseStudyRoomResponse::default())
        })
        .await
    }

    async fn list_study_rooms(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListStudyRoomsRequest>,
    ) -> ServiceResult<ListStudyRoomsResponse> {
        handle(async {
            let scope = request.scope.as_known().unwrap_or(RoomScope::ROOM_SCOPE_UNSPECIFIED);
            let page = Page::from_view(request.page.as_option(), 50);
            let (rooms, page) = self.list_rooms(ctx.user_id()?, scope, &page).await?;
            ok(ListStudyRoomsResponse { rooms, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn join_study_room(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, JoinStudyRoomRequest>,
    ) -> ServiceResult<JoinStudyRoomResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let (room, session) = self.join_room(user_id, parse_id(request.room_id)?, request.subject_id).await?;
            let at = self.now();
            ok(JoinStudyRoomResponse {
                room: self.room_proto(&room, user_id, at).await?.into(),
                session: self.session_proto(&session, Some(&room), at).await?.into(),
                heartbeat_interval_seconds: self.cfg.heartbeat_interval_seconds as i32,
                ..Default::default()
            })
        })
        .await
    }

    async fn leave_study_room(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, LeaveStudyRoomRequest>,
    ) -> ServiceResult<LeaveStudyRoomResponse> {
        handle(async {
            let session = self.leave_room(ctx.user_id()?).await?;
            ok(LeaveStudyRoomResponse { session: session.into(), ..Default::default() })
        })
        .await
    }

    async fn set_study_room_subject(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetStudyRoomSubjectRequest>,
    ) -> ServiceResult<SetStudyRoomSubjectResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.limit_writes(user_id).await?;
            let session = self.set_room_subject(user_id, request.subject_id).await?;
            ok(SetStudyRoomSubjectResponse { session: session.into(), ..Default::default() })
        })
        .await
    }

    async fn control_study_room_timer(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ControlStudyRoomTimerRequest>,
    ) -> ServiceResult<ControlStudyRoomTimerResponse> {
        handle(async {
            let action = request.action.as_known().unwrap_or(TimerAction::TIMER_ACTION_UNSPECIFIED);
            let timer = self.control_room_timer(ctx.user_id()?, parse_id(request.room_id)?, action).await?;
            ok(ControlStudyRoomTimerResponse { timer: timer.into(), ..Default::default() })
        })
        .await
    }

    async fn send_study_room_reaction(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SendStudyRoomReactionRequest>,
    ) -> ServiceResult<SendStudyRoomReactionResponse> {
        handle(async {
            let reaction = request.reaction.as_known().unwrap_or(RoomReaction::ROOM_REACTION_UNSPECIFIED);
            self.send_room_reaction(ctx.user_id()?, reaction, parse_opt_id(request.to_user_id)?).await?;
            ok(SendStudyRoomReactionResponse::default())
        })
        .await
    }

    async fn invite_to_study_room(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, InviteToStudyRoomRequest>,
    ) -> ServiceResult<InviteToStudyRoomResponse> {
        handle(async {
            let request = request.to_owned_message();
            let invitees = self.invite_to_room(ctx.user_id()?, parse_id(&request.room_id)?, &request.user_ids).await?;
            ok(InviteToStudyRoomResponse { invitees, ..Default::default() })
        })
        .await
    }

    async fn uninvite_from_study_room(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UninviteFromStudyRoomRequest>,
    ) -> ServiceResult<UninviteFromStudyRoomResponse> {
        handle(async {
            self.uninvite_from_room(ctx.user_id()?, parse_id(request.room_id)?, parse_id(request.user_id)?).await?;
            ok(UninviteFromStudyRoomResponse::default())
        })
        .await
    }

    async fn remove_study_room_member(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveStudyRoomMemberRequest>,
    ) -> ServiceResult<RemoveStudyRoomMemberResponse> {
        handle(async {
            self.remove_member(ctx.user_id()?, parse_id(request.room_id)?, parse_id(request.user_id)?).await?;
            ok(RemoveStudyRoomMemberResponse::default())
        })
        .await
    }
}
