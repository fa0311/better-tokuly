//! `MessageService`: thin RPC handlers over [`crate::messages`],
//! [`crate::reactions`] and [`crate::state`].
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::{page::encode_cursor, prelude::*};
use tokuly_proto::chat::*;

use crate::{Chat, conversation_svc::page_of};

impl MessageService for Chat {
    async fn send_message(&self, ctx: RequestContext, request: ServiceRequest<'_, SendMessageRequest>) -> ServiceResult<SendMessageResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let (access, row, duplicate) = Chat::send_message(self, user_id, &req).await?;
            let message = self.render_for(&access, std::slice::from_ref(&row)).await?.pop();
            ok(SendMessageResponse { message: message.into(), duplicate, ..Default::default() })
        })
        .await
    }

    async fn send_hello(&self, ctx: RequestContext, request: ServiceRequest<'_, SendHelloRequest>) -> ServiceResult<SendHelloResponse> {
        handle(async {
            let (access, row) = Chat::send_hello(self, ctx.user_id()?, parse_id(request.user_id)?).await?;
            let message = self.render_for(&access, std::slice::from_ref(&row)).await?.pop();
            ok(SendHelloResponse { message: message.into(), conversation: self.conversation_for(&access).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn edit_message(&self, ctx: RequestContext, request: ServiceRequest<'_, EditMessageRequest>) -> ServiceResult<EditMessageResponse> {
        handle(async {
            let (access, row) = Chat::edit_message(self, ctx.user_id()?, parse_id(request.message_id)?, request.text).await?;
            let message = self.render_for(&access, std::slice::from_ref(&row)).await?.pop();
            ok(EditMessageResponse { message: message.into(), ..Default::default() })
        })
        .await
    }

    async fn delete_message(&self, ctx: RequestContext, request: ServiceRequest<'_, DeleteMessageRequest>) -> ServiceResult<DeleteMessageResponse> {
        handle(async {
            let (access, row) = Chat::delete_message(self, ctx.user_id()?, parse_id(request.message_id)?).await?;
            let message = self.render_for(&access, std::slice::from_ref(&row)).await?.pop();
            ok(DeleteMessageResponse { message: message.into(), ..Default::default() })
        })
        .await
    }

    async fn get_message(&self, ctx: RequestContext, request: ServiceRequest<'_, GetMessageRequest>) -> ServiceResult<GetMessageResponse> {
        handle(async {
            let (access, row) = self.message_access(ctx.user_id()?, parse_id(request.message_id)?).await?;
            let message = self.render_for(&access, std::slice::from_ref(&row)).await?.pop();
            ok(GetMessageResponse { message: message.into(), ..Default::default() })
        })
        .await
    }

    async fn list_messages(&self, ctx: RequestContext, request: ServiceRequest<'_, ListMessagesRequest>) -> ServiceResult<ListMessagesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let access = self.access(user_id, parse_id(&req.conversation_id)?).await?;
            let window = self.message_window(&access, req.position.as_ref(), req.limit).await?;
            ok(ListMessagesResponse {
                messages: self.render_for(&access, &window.rows).await?,
                older_cursor: encode_cursor(&window.older),
                has_older: window.has_older,
                newer_cursor: encode_cursor(&window.newer),
                has_newer: window.has_newer,
                read_cursors: self.read_cursors(access.conv.id).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn search_messages(&self, ctx: RequestContext, request: ServiceRequest<'_, SearchMessagesRequest>) -> ServiceResult<SearchMessagesResponse> {
        handle(async {
            let access = self.access(ctx.user_id()?, parse_id(request.conversation_id)?).await?;
            let page = page_of(request.page.as_option(), 30, 50);
            let (rows, page) = Chat::search_messages(self, &access, request.query, &page).await?;
            ok(SearchMessagesResponse { messages: self.render_for(&access, &rows).await?, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn forward_messages(&self, ctx: RequestContext, request: ServiceRequest<'_, ForwardMessagesRequest>) -> ServiceResult<ForwardMessagesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            let forwarded = Chat::forward_messages(self, user_id, &parse_ids(&req.message_ids)?, &parse_ids(&req.conversation_ids)?, &req.client_id).await?;
            let mut messages = Vec::with_capacity(forwarded.len());
            for (access, row) in &forwarded {
                messages.extend(self.render_for(access, std::slice::from_ref(row)).await?);
            }
            ok(ForwardMessagesResponse { messages, ..Default::default() })
        })
        .await
    }

    async fn add_reaction(&self, ctx: RequestContext, request: ServiceRequest<'_, AddReactionRequest>) -> ServiceResult<AddReactionResponse> {
        handle(async {
            let reactions = Chat::add_reaction(self, ctx.user_id()?, parse_id(request.message_id)?, request.emoji).await?;
            ok(AddReactionResponse { reactions, ..Default::default() })
        })
        .await
    }

    async fn remove_reaction(&self, ctx: RequestContext, request: ServiceRequest<'_, RemoveReactionRequest>) -> ServiceResult<RemoveReactionResponse> {
        handle(async {
            let reactions = Chat::remove_reaction(self, ctx.user_id()?, parse_id(request.message_id)?, request.emoji).await?;
            ok(RemoveReactionResponse { reactions, ..Default::default() })
        })
        .await
    }

    async fn list_reactions(&self, ctx: RequestContext, request: ServiceRequest<'_, ListReactionsRequest>) -> ServiceResult<ListReactionsResponse> {
        handle(async {
            let page = page_of(request.page.as_option(), 50, 100);
            let (reactions, page) = Chat::list_reactions(self, ctx.user_id()?, parse_id(request.message_id)?, request.emoji, &page).await?;
            ok(ListReactionsResponse { reactions, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn pin_message(&self, ctx: RequestContext, request: ServiceRequest<'_, PinMessageRequest>) -> ServiceResult<PinMessageResponse> {
        handle(async {
            let (access, row) = Chat::pin_message(self, ctx.user_id()?, parse_id(request.message_id)?).await?;
            let message = self.render_for(&access, std::slice::from_ref(&row)).await?.pop();
            ok(PinMessageResponse { message: message.into(), ..Default::default() })
        })
        .await
    }

    async fn unpin_message(&self, ctx: RequestContext, request: ServiceRequest<'_, UnpinMessageRequest>) -> ServiceResult<UnpinMessageResponse> {
        handle(async {
            let (access, row) = Chat::unpin_message(self, ctx.user_id()?, parse_id(request.message_id)?).await?;
            let message = self.render_for(&access, std::slice::from_ref(&row)).await?.pop();
            ok(UnpinMessageResponse { message: message.into(), ..Default::default() })
        })
        .await
    }

    async fn list_pinned_messages(&self, ctx: RequestContext, request: ServiceRequest<'_, ListPinnedMessagesRequest>) -> ServiceResult<ListPinnedMessagesResponse> {
        handle(async {
            let access = self.access(ctx.user_id()?, parse_id(request.conversation_id)?).await?;
            let page = page_of(request.page.as_option(), 30, 50);
            let (rows, page) = self.pinned_messages(&access, &page).await?;
            ok(ListPinnedMessagesResponse { messages: self.render_for(&access, &rows).await?, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn list_read_receipts(&self, ctx: RequestContext, request: ServiceRequest<'_, ListReadReceiptsRequest>) -> ServiceResult<ListReadReceiptsResponse> {
        handle(async {
            let (_, row) = self.message_access(ctx.user_id()?, parse_id(request.message_id)?).await?;
            let (receipts, unread_member_count) = self.read_receipts(row.conversation_id, row.seq, row.sender_id).await?;
            ok(ListReadReceiptsResponse { receipts, unread_member_count, ..Default::default() })
        })
        .await
    }

    async fn list_shared_media(&self, ctx: RequestContext, request: ServiceRequest<'_, ListSharedMediaRequest>) -> ServiceResult<ListSharedMediaResponse> {
        handle(async {
            let access = self.access(ctx.user_id()?, parse_id(request.conversation_id)?).await?;
            let filter = request.filter.as_known().unwrap_or(SharedMediaFilter::SHARED_MEDIA_FILTER_UNSPECIFIED);
            let page = page_of(request.page.as_option(), 30, 100);
            let (items, page) = self.shared_media(&access, filter, &page).await?;
            ok(ListSharedMediaResponse { items, page: page.into(), ..Default::default() })
        })
        .await
    }
}
