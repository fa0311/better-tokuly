//! `CommunityBoardService`: channels, threads, replies and polls.
use std::{collections::HashMap, time::Duration};

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::community::{mark_read_request::Target, *};

use crate::{
    Community as Domain,
    access::{Access, Caller, permission_denied},
    channels::{ChannelEdit, channel_not_found},
    model::ThreadRow,
    render::View,
    threads::ThreadFilter,
    validate,
};

impl Domain {
    /// One thread as the caller of `access` sees it.
    async fn thread_response(&self, access: &Access, blocked: &[Uuid], row: &ThreadRow) -> AppResult<Thread> {
        self.render_thread(&access.community, Some(&View { access, blocked }), row).await
    }

    /// Threads from several communities, each rendered for the caller's
    /// standing in its own community, in the order given.
    async fn render_mixed_threads(&self, caller: &Caller, rows: &[ThreadRow]) -> AppResult<Vec<Thread>> {
        let blocked = self.blocked(caller.user_id).await?;
        let mut by_community: HashMap<Uuid, Vec<ThreadRow>> = HashMap::new();
        for row in rows {
            by_community.entry(row.community_id).or_default().push(row.clone());
        }
        let mut rendered: HashMap<String, Thread> = HashMap::with_capacity(rows.len());
        for (community_id, group) in by_community {
            // A community that vanished between the two queries drops its threads.
            let Ok(access) = self.access_by_id(caller, community_id).await else { continue };
            let view = View { access: &access, blocked: &blocked };
            for thread in self.render_threads(&access.community, Some(&view), &group).await? {
                rendered.insert(thread.id.clone(), thread);
            }
        }
        Ok(rows.iter().filter_map(|row| rendered.remove(&row.id.to_string())).collect())
    }
}

impl CommunityBoardService for Domain {
    async fn create_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateChannelRequest>,
    ) -> ServiceResult<CreateChannelResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let access = self.access(&caller, &req.community_id).await?;
            let channel = self.create_channel(&access, &req).await?;
            ok(CreateChannelResponse { channel: channel.into(), ..Default::default() })
        })
        .await
    }

    async fn update_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateChannelRequest>,
    ) -> ServiceResult<UpdateChannelResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let (channel, access) = self.open_channel(&caller, &req.channel_id).await?;
            let edit = ChannelEdit {
                name: req.name.as_deref().map(validate::channel_name).transpose()?,
                description: req
                    .description
                    .as_deref()
                    .map(|d| validate::description(d, validate::MAX_CHANNEL_DESCRIPTION_CHARS))
                    .transpose()?,
                emoji: req.emoji.as_deref().map(validate::emoji).transpose()?,
                posting: req.posting.map(validate::posting).transpose()?,
                anonymous: req.anonymous,
            };
            let channel = self.update_channel(&access, &channel, edit).await?;
            ok(UpdateChannelResponse { channel: channel.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_channel(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteChannelRequest>,
    ) -> ServiceResult<DeleteChannelResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (channel, access) = self.open_channel(&caller, request.channel_id).await?;
            self.delete_channel(&access, &channel).await?;
            ok(DeleteChannelResponse::default())
        })
        .await
    }

    async fn reorder_channels(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ReorderChannelsRequest>,
    ) -> ServiceResult<ReorderChannelsResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let access = self.access(&caller, &req.community_id).await?;
            let channel_ids = parse_ids(&req.channel_ids).map_err(|_| {
                AppError::invalid("CHANNEL_ORDER_MISMATCH", "name every channel of the community exactly once")
            })?;
            let channels = self.reorder_channels(&access, &channel_ids).await?;
            ok(ReorderChannelsResponse { channels, ..Default::default() })
        })
        .await
    }

    async fn list_channels(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListChannelsRequest>,
    ) -> ServiceResult<ListChannelsResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            ok(ListChannelsResponse { channels: self.list_channels(&access).await?, ..Default::default() })
        })
        .await
    }

    async fn create_thread(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateThreadRequest>,
    ) -> ServiceResult<CreateThreadResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let (channel, access) = self.open_channel(&caller, &req.channel_id).await?;
            let thread = self.create_thread(&access, &channel, &req).await?;
            let blocked = self.blocked(caller.user_id).await?;
            ok(CreateThreadResponse {
                thread: self.thread_response(&access, &blocked, &thread).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_thread(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetThreadRequest>,
    ) -> ServiceResult<GetThreadResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let (thread, access, blocked) = self.open_thread(&caller, request.thread_id).await?;
            ok(GetThreadResponse {
                thread: self.thread_response(&access, &blocked, &thread).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_thread(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateThreadRequest>,
    ) -> ServiceResult<UpdateThreadResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let (thread, access, blocked) = self.open_thread(&caller, &req.thread_id).await?;
            let title = req.title.as_deref().map(validate::title).transpose()?;
            let body = req.body.as_deref().map(|b| validate::body(b, validate::MAX_THREAD_BODY_CHARS)).transpose()?;
            let tags = req
                .tags
                .into_option()
                .map(|list| validate::tags(&list.values, validate::MAX_THREAD_TAGS))
                .transpose()?;
            let updated = self.update_thread(&access, &thread, title, body, tags).await?;
            ok(UpdateThreadResponse {
                thread: self.thread_response(&access, &blocked, &updated).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_thread(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteThreadRequest>,
    ) -> ServiceResult<DeleteThreadResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (thread, access, _) = self.open_thread(&caller, request.thread_id).await?;
            self.delete_thread(&access, &thread, request.reason).await?;
            ok(DeleteThreadResponse::default())
        })
        .await
    }

    async fn list_threads(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListThreadsRequest>,
    ) -> ServiceResult<ListThreadsResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let req = request.to_owned_message();
            let access = self.access(&caller, &req.community_id).await?;
            access.require_read()?;
            let channel_id = match req.channel_id.trim() {
                "" => None,
                id => {
                    let id = Uuid::parse_str(id).map_err(|_| channel_not_found())?;
                    let channel =
                        self.channel_by_id(id).await?.filter(|channel| channel.community_id == access.community.id);
                    Some(channel.ok_or_else(channel_not_found)?.id)
                }
            };
            let sort = match req.sort.as_known() {
                Some(ThreadSort::THREAD_SORT_UNSPECIFIED) => ThreadSort::THREAD_SORT_HOT,
                Some(sort) => sort,
                None => bail!(invalid, "SORT_INVALID", "choose hot, new or top"),
            };
            let window = req.window.as_known().unwrap_or(TopWindow::TOP_WINDOW_ALL);
            let tag = if req.tag.trim().is_empty() { None } else { Some(validate::tag(&req.tag)?) };
            let filter = ThreadFilter { channel_id, sort, window, tag };
            let blocked = self.blocked(caller.user_id).await?;
            let view = View { access: &access, blocked: &blocked };
            let page = Page::from_request(req.page.as_option(), 50);
            let pinned = if page.cursor.is_none() { self.pinned_threads(&view, &filter).await? } else { Vec::new() };
            let (rows, info) = self.list_threads(&view, &filter, &page).await?;
            ok(ListThreadsResponse {
                pinned: self.render_threads(&access.community, Some(&view), &pinned).await?,
                threads: self.render_threads(&access.community, Some(&view), &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_announcements(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListAnnouncementsRequest>,
    ) -> ServiceResult<ListAnnouncementsResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let access = self.access(&caller, request.community_id).await?;
            access.require_read()?;
            let blocked = self.blocked(caller.user_id).await?;
            let view = View { access: &access, blocked: &blocked };
            let page = Page::from_view(request.page.as_option(), 50);
            let (rows, info) = self.list_announcements(&view, &page).await?;
            ok(ListAnnouncementsResponse {
                threads: self.render_threads(&access.community, Some(&view), &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_followed_threads(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFollowedThreadsRequest>,
    ) -> ServiceResult<ListFollowedThreadsResponse> {
        handle(async {
            let (caller, user_id) = Caller::signed_in(&ctx)?;
            let community_id = match request.community_id.trim() {
                "" => None,
                reference => Some(self.access(&caller, reference).await?.community.id),
            };
            let page = Page::from_view(request.page.as_option(), 50);
            let (rows, info) = self.followed_threads(user_id, community_id, request.unread_only, &page).await?;
            ok(ListFollowedThreadsResponse {
                threads: self.render_mixed_threads(&caller, &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn pin_thread(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PinThreadRequest>,
    ) -> ServiceResult<PinThreadResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (thread, access, blocked) = self.open_thread(&caller, request.thread_id).await?;
            let updated = self.pin_thread(&access, &thread, request.pinned).await?;
            ok(PinThreadResponse {
                thread: self.thread_response(&access, &blocked, &updated).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn lock_thread(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LockThreadRequest>,
    ) -> ServiceResult<LockThreadResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (thread, access, blocked) = self.open_thread(&caller, request.thread_id).await?;
            let updated = self.lock_thread(&access, &thread, request.locked).await?;
            ok(LockThreadResponse {
                thread: self.thread_response(&access, &blocked, &updated).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn set_announcement(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetAnnouncementRequest>,
    ) -> ServiceResult<SetAnnouncementResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (thread, access, blocked) = self.open_thread(&caller, request.thread_id).await?;
            let updated = self.set_announcement(&access, &thread, request.announcement).await?;
            ok(SetAnnouncementResponse {
                thread: self.thread_response(&access, &blocked, &updated).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn set_thread_follow(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetThreadFollowRequest>,
    ) -> ServiceResult<SetThreadFollowResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (thread, access, _) = self.open_thread(&caller, request.thread_id).await?;
            self.set_thread_follow(&access, &thread, request.following).await?;
            ok(SetThreadFollowResponse { following: request.following, ..Default::default() })
        })
        .await
    }

    async fn mark_read(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, MarkReadRequest>,
    ) -> ServiceResult<MarkReadResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            match req.target {
                Some(Target::ThreadId(thread_id)) => {
                    let (thread, access, _) = self.open_thread(&caller, &thread_id).await?;
                    let member = access.require_member()?;
                    self.mark_thread_read(thread.id, member.user_id).await?;
                }
                Some(Target::ChannelId(channel_id)) => {
                    let (channel, access) = self.open_channel(&caller, &channel_id).await?;
                    let member = access.require_member()?;
                    self.mark_channel_read(channel.id, member.user_id).await?;
                }
                Some(Target::CommunityId(community_id)) => {
                    let access = self.access(&caller, &community_id).await?;
                    let member = access.require_member()?;
                    self.mark_community_read(access.community.id, member.user_id).await?;
                }
                None => bail!(invalid, "TARGET_REQUIRED", "name a thread, a channel or a community"),
            }
            ok(MarkReadResponse::default())
        })
        .await
    }

    async fn vote_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, VotePollRequest>,
    ) -> ServiceResult<VotePollResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let (thread, access, _) = self.open_thread(&caller, &req.thread_id).await?;
            let member = access.require_active()?;
            let option_ids = parse_ids(&req.option_ids)
                .map_err(|_| AppError::not_found("OPTION_NOT_FOUND", "this poll has no such option"))?;
            // Every vote is pushed to everyone reading the thread.
            self.limit("vote", member.user_id, 60, Duration::from_secs(60)).await?;
            self.vote(thread.id, member.user_id, &option_ids).await?;
            let poll = self.load_polls(&[thread.id], Some(member.user_id)).await?.remove(&thread.id);
            let poll = poll.ok_or_else(|| AppError::not_found("POLL_NOT_FOUND", "this thread has no poll"))?;
            self.push_thread_updated(&access.community, &thread).await;
            ok(VotePollResponse {
                poll: poll.to_proto(thread.author_id == member.user_id).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn close_poll(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ClosePollRequest>,
    ) -> ServiceResult<ClosePollResponse> {
        handle(async {
            let (caller, user_id) = Caller::signed_in(&ctx)?;
            let (thread, access, _) = self.open_thread(&caller, request.thread_id).await?;
            let own = access.is_member() && thread.author_id == user_id;
            if !own && !access.moderates() {
                return Err(permission_denied());
            }
            self.limit("vote", user_id, 60, Duration::from_secs(60)).await?;
            self.close_poll(thread.id).await?;
            let poll = self.load_polls(&[thread.id], Some(user_id)).await?.remove(&thread.id);
            let poll = poll.ok_or_else(|| AppError::not_found("POLL_NOT_FOUND", "this thread has no poll"))?;
            self.push_thread_updated(&access.community, &thread).await;
            ok(ClosePollResponse { poll: poll.to_proto(thread.author_id == user_id).into(), ..Default::default() })
        })
        .await
    }

    async fn create_reply(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateReplyRequest>,
    ) -> ServiceResult<CreateReplyResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let req = request.to_owned_message();
            let (thread, access, blocked) = self.open_thread(&caller, &req.thread_id).await?;
            let view = View { access: &access, blocked: &blocked };
            let reply = self.create_reply(&view, &thread, &req).await?;
            ok(CreateReplyResponse {
                reply: self.render_reply(Some(&view), &reply).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_reply(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetReplyRequest>,
    ) -> ServiceResult<GetReplyResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let (reply, access, blocked) = self.open_reply(&caller, request.reply_id).await?;
            let view = View { access: &access, blocked: &blocked };
            ok(GetReplyResponse { reply: self.render_reply(Some(&view), &reply).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn update_reply(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateReplyRequest>,
    ) -> ServiceResult<UpdateReplyResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (reply, access, blocked) = self.open_reply(&caller, request.reply_id).await?;
            let updated = self.update_reply(&access, &reply, request.body).await?;
            let view = View { access: &access, blocked: &blocked };
            ok(UpdateReplyResponse {
                reply: self.render_reply(Some(&view), &updated).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_reply(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteReplyRequest>,
    ) -> ServiceResult<DeleteReplyResponse> {
        handle(async {
            let (caller, _) = Caller::signed_in(&ctx)?;
            let (reply, access, _) = self.open_reply(&caller, request.reply_id).await?;
            self.delete_reply(&access, &reply, request.reason).await?;
            ok(DeleteReplyResponse::default())
        })
        .await
    }

    async fn list_replies(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListRepliesRequest>,
    ) -> ServiceResult<ListRepliesResponse> {
        handle(async {
            let caller = Caller::of(&ctx)?;
            let (thread, access, blocked) = self.open_thread(&caller, request.thread_id).await?;
            let view = View { access: &access, blocked: &blocked };
            let page = Page::from_view(request.page.as_option(), 100);
            let (rows, info) = self.list_replies(&view, &thread, request.newest_first, &page).await?;
            ok(ListRepliesResponse {
                replies: self.render_replies(Some(&view), &rows).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }
}
