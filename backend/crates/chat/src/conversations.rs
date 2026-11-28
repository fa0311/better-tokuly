//! Conversations: opening direct conversations, creating and editing
//! groups, and rendering conversations for the list and the chat screen.
use std::collections::{HashMap, HashSet};

use sqlx::{Postgres, Transaction};
use tokuly_core::{events, outbound, prelude::*};
use tokuly_identity::{settings, users};
use tokuly_media::assets;
use tokuly_proto::{chat as pb, common::SubjectType, identity::Audience as PrivacyAudience, media::AssetPurpose};
use tokuly_social::graph;

use crate::{
    Chat,
    access::{self, Access, conversation_not_found},
    content,
    events::{CONVERSATION_CREATED, ConversationCreated, MEMBER_JOINED, MemberJoined},
    model::{CONV, ConversationRow, MEMBER, MSG, MemberRow, MessageRow, SystemPayload, kind, request, role},
    notices,
    render::View,
    send::{Draft, Posted},
};

/// Conversations a user may pin to the top of their list.
pub(crate) const MAX_PINNED_CONVERSATIONS: i64 = 20;
/// Members shown in a group's avatar collage.
const PREVIEW_MEMBERS: i64 = 4;

/// The asset holder of a group's icon.
pub(crate) fn icon_holder(conversation_id: Uuid) -> String {
    format!("chat:icon:{conversation_id}")
}

pub(crate) fn not_a_group() -> AppError {
    AppError::precondition("NOT_A_GROUP", "this is not a group conversation")
}

pub(crate) fn not_allowed() -> AppError {
    AppError::denied("NOT_ALLOWED", "you are not allowed to do this")
}

/// Orders two distinct users the way `chat_conversations.user_lo/user_hi`
/// store them. (`Uuid` and PostgreSQL's `uuid` both compare byte-wise.)
pub(crate) fn pair_of(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a < b { (a, b) } else { (b, a) }
}

/// The two people of a direct conversation, whether or not both are still
/// members; `None` for groups and for conversations that do not exist.
///
/// For domains that hear about a message with its conversation and need to
/// know who it went to (the server bridges it to them; they do not depend
/// on this crate).
pub async fn direct_members(core: &Core, conversation_id: Uuid) -> AppResult<Option<(Uuid, Uuid)>> {
    let pair: Option<(Option<Uuid>, Option<Uuid>)> =
        sqlx::query_as("SELECT user_lo, user_hi FROM chat_conversations WHERE id = $1 AND kind = $2")
            .bind(conversation_id)
            .bind(kind::DIRECT)
            .fetch_optional(&core.db)
            .await?;
    Ok(pair.and_then(|(lo, hi)| lo.zip(hi)))
}

#[derive(sqlx::FromRow)]
pub(crate) struct ListedConversation {
    #[sqlx(flatten)]
    pub conv: ConversationRow,
    #[sqlx(flatten)]
    pub me: MemberRow,
}

/// Position of a conversation in a user's list: pinned first, then by
/// latest activity.
#[derive(serde::Serialize, serde::Deserialize)]
struct ListCursor(i32, Time, Uuid);

/// Position of a member in the member list: owner, admins, then by join time.
#[derive(serde::Serialize, serde::Deserialize)]
struct MemberCursor(i16, Time, Uuid);

impl Chat {
    // ───────────────────────────── direct conversations ─────────────────────────────

    async fn find_direct(&self, a: Uuid, b: Uuid) -> AppResult<Option<ConversationRow>> {
        let (lo, hi) = pair_of(a, b);
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CONV} FROM chat_conversations c WHERE c.kind = {} AND c.user_lo = $1 AND c.user_hi = $2",
            kind::DIRECT
        )))
        .bind(lo)
        .bind(hi)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// Stores the direct conversation of a pair unless it exists, and
    /// returns its ID. `other_state` is where the conversation starts for
    /// `b` (accepted, or a message request).
    async fn create_direct(&self, a: Uuid, b: Uuid, other_state: i16, actor: Option<Uuid>) -> AppResult<Uuid> {
        let (lo, hi) = pair_of(a, b);
        let mut tx = self.core.db.begin().await?;
        let created: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO chat_conversations (kind, user_lo, user_hi, created_by, member_count) VALUES ($1, $2, $3, $4, 2)
             ON CONFLICT (user_lo, user_hi) WHERE kind = 1 DO NOTHING
             RETURNING id",
        )
        .bind(kind::DIRECT)
        .bind(lo)
        .bind(hi)
        .bind(actor)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(conversation_id) = created else {
            // Someone else opened it in the meantime.
            drop(tx);
            return self.find_direct(a, b).await?.map(|conv| conv.id).ok_or_else(conversation_not_found);
        };
        sqlx::query(
            "INSERT INTO chat_members (conversation_id, user_id, request_state) VALUES ($1, $2, $3), ($1, $4, $5)",
        )
        .bind(conversation_id)
        .bind(a)
        .bind(request::ACCEPTED)
        .bind(b)
        .bind(other_state)
        .execute(&mut *tx)
        .await?;
        let created = ConversationCreated { conversation_id, kind: i32::from(kind::DIRECT), member_ids: vec![a, b] };
        events::emit(&mut *tx, CONVERSATION_CREATED, actor, &created).await?;
        tx.commit().await?;
        Ok(conversation_id)
    }

    /// The direct conversation between the caller and `other`, created when
    /// the two never talked and `other`'s privacy settings let the caller in.
    pub(crate) async fn open_direct_as(&self, me: Uuid, other: Uuid) -> AppResult<Access> {
        let core = &self.core;
        ensure!(me != other, invalid, "CANNOT_MESSAGE_SELF", "you cannot message yourself");
        if let Some(existing) = self.find_direct(me, other).await? {
            return self.access(me, existing.id).await;
        }
        if other == self.bot_id().await? {
            return self.open_bot_conversation(me).await;
        }
        let user_not_found = || AppError::not_found("USER_NOT_FOUND", "user not found");
        let target =
            users::find_by_id(core, other).await?.filter(|user| user.is_active()).ok_or_else(user_not_found)?;
        if graph::is_blocked_either(core, me, other).await? {
            return Err(user_not_found());
        }
        let privacy = settings::privacy_from_json(&target.privacy);
        let audience = privacy.direct_messages_from.as_known().unwrap_or(PrivacyAudience::AUDIENCE_UNSPECIFIED);
        ensure!(
            graph::allows(core, other, Some(me), audience).await?,
            denied,
            "DM_NOT_ALLOWED",
            "this user does not accept messages from you"
        );
        core.cache.rate_limit(&format!("chat-open:{me}"), 60, std::time::Duration::from_secs(3600)).await?;
        // Friends talk right away; anyone else arrives as a message request.
        let other_state = if graph::are_friends(core, me, other).await? { request::ACCEPTED } else { request::PENDING };
        let conversation_id = self.create_direct(me, other, other_state, Some(me)).await?;
        self.access(me, conversation_id).await
    }

    /// Returns the direct conversation between two users, creating it when
    /// needed, for flows the server itself starts (a gift thank-you, the demo
    /// simulator). Privacy audiences are not consulted and nobody has to
    /// accept anything: both sides get an ordinary conversation.
    pub async fn open_direct(&self, a: Uuid, b: Uuid) -> AppResult<Uuid> {
        ensure!(a != b, invalid, "CANNOT_MESSAGE_SELF", "a direct conversation needs two different users");
        let conversation_id = match self.find_direct(a, b).await? {
            Some(existing) => existing.id,
            None => {
                let known = users::filter_active(&self.core, &[a, b]).await?;
                ensure!(known.len() == 2, not_found, "USER_NOT_FOUND", "user not found");
                self.create_direct(a, b, request::ACCEPTED, None).await?
            }
        };
        // A request that was waiting is answered by the server's say-so; a
        // declined one stays declined.
        for user_id in [a, b] {
            self.accept_request_of(conversation_id, user_id).await?;
        }
        Ok(conversation_id)
    }

    // ───────────────────────────── groups ─────────────────────────────

    /// Resolves an upload as a group icon: `(image JSON, asset ID)`.
    async fn resolve_icon(&self, user_id: Uuid, asset_id: &str) -> AppResult<(serde_json::Value, Uuid)> {
        let asset_id = parse_id(asset_id)?;
        let image =
            assets::require_image_json(&self.core, asset_id, user_id, &[AssetPurpose::ASSET_PURPOSE_AVATAR]).await?;
        Ok((image, asset_id))
    }

    /// Fails with `NOT_FRIENDS` unless every one of `user_ids` is a friend of `user_id`.
    pub(crate) async fn ensure_friends(&self, user_id: Uuid, user_ids: &[Uuid]) -> AppResult<()> {
        let friends: HashSet<Uuid> = graph::friend_ids(&self.core, user_id).await?.into_iter().collect();
        ensure!(
            user_ids.iter().all(|id| friends.contains(id)),
            precondition,
            "NOT_FRIENDS",
            "you can only add your friends to a group"
        );
        Ok(())
    }

    pub(crate) async fn create_group(
        &self,
        user_id: Uuid,
        name: &str,
        member_ids: &[Uuid],
        icon_asset_id: &str,
    ) -> AppResult<Access> {
        let core = &self.core;
        let name = content::validate_group_name(name)?;
        let mut others: Vec<Uuid> = Vec::new();
        for id in member_ids {
            if *id != user_id && !others.contains(id) {
                others.push(*id);
            }
        }
        ensure!(!others.is_empty(), invalid, "NO_MEMBERS", "a group needs at least one other member");
        ensure!(
            others.len() < self.cfg.max_group_members as usize,
            invalid,
            "TOO_MANY_MEMBERS",
            "a group can have at most {} members",
            self.cfg.max_group_members
        );
        self.ensure_friends(user_id, &others).await?;
        let icon = if icon_asset_id.is_empty() { None } else { Some(self.resolve_icon(user_id, icon_asset_id).await?) };
        core.cache
            .rate_limit(&format!("chat-group-create:{user_id}"), 20, std::time::Duration::from_secs(3600))
            .await?;
        let creator = tokuly_identity::summary(core, user_id).await?;

        let conversation_id = new_id();
        let mut tx = core.db.begin().await?;
        let mut conv: ConversationRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO chat_conversations AS c (id, kind, name, icon, icon_asset_id, created_by, member_count)
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {CONV}"
        )))
        .bind(conversation_id)
        .bind(kind::GROUP)
        .bind(&name)
        .bind(icon.as_ref().map(|(image, _)| image))
        .bind(icon.as_ref().map(|(_, asset_id)| *asset_id))
        .bind(user_id)
        .bind(others.len() as i32 + 1)
        .fetch_one(&mut *tx)
        .await?;
        if let Some((_, asset_id)) = &icon {
            assets::attach(&mut *tx, *asset_id, &icon_holder(conversation_id)).await?;
        }
        sqlx::query("INSERT INTO chat_members (conversation_id, user_id, role) VALUES ($1, $2, $3)")
            .bind(conversation_id)
            .bind(user_id)
            .bind(role::OWNER)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO chat_members (conversation_id, user_id, role, added_by) SELECT $1, member, $3, $4 FROM unnest($2::uuid[]) AS member")
            .bind(conversation_id)
            .bind(&others)
            .bind(role::MEMBER)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        let payload = SystemPayload::new(pb::SystemEventType::SYSTEM_EVENT_TYPE_GROUP_CREATED, Some(user_id))
            .targets(others.clone())
            .text(&name);
        let posted = self.post(&mut tx, &mut conv, Draft::system(payload)).await?;
        let mut member_ids = vec![user_id];
        member_ids.extend(&others);
        events::emit(
            &mut *tx,
            CONVERSATION_CREATED,
            Some(user_id),
            &ConversationCreated { conversation_id, kind: i32::from(kind::GROUP), member_ids },
        )
        .await?;
        for other in &others {
            events::emit(
                &mut *tx,
                MEMBER_JOINED,
                Some(user_id),
                &MemberJoined { conversation_id, user_id: *other, added_by: user_id },
            )
            .await?;
            outbound::notify(&mut *tx, &notices::added_to_group(*other, &creator, user_id, &conv)).await?;
        }
        tx.commit().await?;
        self.announce(&posted).await;
        self.access(user_id, conversation_id).await
    }

    /// Renames a group and/or changes its icon. `icon_asset_id` of
    /// `Some("")` removes the icon.
    pub(crate) async fn update_group(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        name: Option<&str>,
        icon_asset_id: Option<&str>,
    ) -> AppResult<Access> {
        let access = self.access(user_id, conversation_id).await?;
        ensure_group(&access.conv)?;
        if !access.me.manages() {
            return Err(not_allowed());
        }
        let name = name.map(content::validate_group_name).transpose()?;
        let icon = match icon_asset_id {
            None => None,
            Some("") => Some(None),
            Some(asset_id) => Some(Some(self.resolve_icon(user_id, asset_id).await?)),
        };
        // Every change adds a line to the conversation.
        self.core
            .cache
            .rate_limit(&format!("chat-group-update:{user_id}"), 60, std::time::Duration::from_secs(3600))
            .await?;

        let mut tx = self.core.db.begin().await?;
        let conv = access::lock_conversation(&mut tx, conversation_id).await?;
        let me = access::member_in_tx(&mut tx, conversation_id, user_id).await?.ok_or_else(conversation_not_found)?;
        if !me.manages() {
            return Err(not_allowed());
        }
        let mut announcements: Vec<Posted> = Vec::new();
        let mut current = conv.clone();
        if let Some(name) = name.filter(|name| *name != conv.name) {
            sqlx::query("UPDATE chat_conversations SET name = $2 WHERE id = $1")
                .bind(conversation_id)
                .bind(&name)
                .execute(&mut *tx)
                .await?;
            let payload =
                SystemPayload::new(pb::SystemEventType::SYSTEM_EVENT_TYPE_GROUP_RENAMED, Some(user_id)).text(&name);
            announcements.push(self.post(&mut tx, &mut current, Draft::system(payload)).await?);
        }
        if let Some(icon) = icon {
            let new_asset = icon.as_ref().map(|(_, asset_id)| *asset_id);
            if new_asset != conv.icon_asset_id {
                sqlx::query("UPDATE chat_conversations SET icon = $2, icon_asset_id = $3 WHERE id = $1")
                    .bind(conversation_id)
                    .bind(icon.as_ref().map(|(image, _)| image))
                    .bind(new_asset)
                    .execute(&mut *tx)
                    .await?;
                let holder = icon_holder(conversation_id);
                if let Some(old) = conv.icon_asset_id {
                    assets::detach(&mut *tx, old, &holder).await?;
                }
                if let Some(new) = new_asset {
                    assets::attach(&mut *tx, new, &holder).await?;
                }
                let payload = SystemPayload::new(pb::SystemEventType::SYSTEM_EVENT_TYPE_ICON_CHANGED, Some(user_id));
                announcements.push(self.post(&mut tx, &mut current, Draft::system(payload)).await?);
            }
        }
        tx.commit().await?;
        if !announcements.is_empty() {
            self.push_conversation_shared(conversation_id).await;
            for posted in &announcements {
                self.announce(posted).await;
            }
        }
        self.access(user_id, conversation_id).await
    }

    // ───────────────────────────── rendering ─────────────────────────────

    /// One conversation as its member sees it.
    pub(crate) async fn conversation_for(&self, access: &Access) -> AppResult<pb::Conversation> {
        let mut rendered =
            self.conversations_for(access.me.user_id, vec![(access.conv.clone(), access.me.clone())]).await?;
        rendered.pop().ok_or_else(conversation_not_found)
    }

    /// A conversation without anybody's private state, for events that go
    /// to every member alike. Carries no `last_message` either: what the
    /// latest visible message is differs between members.
    pub(crate) async fn shared_conversation(&self, conv: &ConversationRow) -> AppResult<pb::Conversation> {
        let mut rendered = self.render_conversations(std::slice::from_ref(conv), None).await?;
        rendered.pop().ok_or_else(conversation_not_found)
    }

    /// Conversations as `viewer` sees them, in the order given.
    pub(crate) async fn conversations_for(
        &self,
        viewer: Uuid,
        items: Vec<(ConversationRow, MemberRow)>,
    ) -> AppResult<Vec<pb::Conversation>> {
        let (convs, members): (Vec<ConversationRow>, Vec<MemberRow>) = items.into_iter().unzip();
        let members: HashMap<Uuid, MemberRow> =
            members.into_iter().map(|member| (member.conversation_id, member)).collect();
        self.render_conversations(&convs, Some((viewer, &members))).await
    }

    async fn render_conversations(
        &self,
        convs: &[ConversationRow],
        viewer: Option<(Uuid, &HashMap<Uuid, MemberRow>)>,
    ) -> AppResult<Vec<pb::Conversation>> {
        if convs.is_empty() {
            return Ok(Vec::new());
        }
        let core = &self.core;
        let ids: Vec<Uuid> = convs.iter().map(|conv| conv.id).collect();
        let group_ids: Vec<Uuid> = convs.iter().filter(|conv| conv.is_group()).map(|conv| conv.id).collect();

        // Who to show: both sides of a direct conversation, the longest
        // standing members of a group.
        let mut preview: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        if !group_ids.is_empty() {
            let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT conversation_id, user_id FROM (
                     SELECT conversation_id, user_id, row_number() OVER (PARTITION BY conversation_id ORDER BY joined_at, user_id) AS position
                     FROM chat_members WHERE conversation_id = ANY($1)
                 ) ranked
                 WHERE position <= $2 ORDER BY conversation_id, position",
            )
            .bind(&group_ids)
            .bind(PREVIEW_MEMBERS)
            .fetch_all(&core.db)
            .await?;
            for (conversation_id, user_id) in rows {
                preview.entry(conversation_id).or_default().push(user_id);
            }
        }
        for conv in convs {
            if let Some((lo, hi)) = conv.pair() {
                preview.insert(conv.id, vec![lo, hi]);
            }
        }
        let user_ids: Vec<Uuid> = preview.values().flatten().copied().collect::<HashSet<_>>().into_iter().collect();
        let summaries = tokuly_identity::summaries(core, &user_ids).await?;

        let live_calls: HashMap<Uuid, Uuid> = sqlx::query_as::<_, (Uuid, Uuid)>(
            "SELECT conversation_id, id FROM chat_calls WHERE conversation_id = ANY($1) AND state <> 3",
        )
        .bind(&ids)
        .fetch_all(&core.db)
        .await?
        .into_iter()
        .collect();

        // Per-viewer parts: the latest visible message and the request state
        // of the other side of direct conversations.
        let mut last_messages: HashMap<Uuid, pb::Message> = HashMap::new();
        let mut other_states: HashMap<Uuid, i16> = HashMap::new();
        if let Some((viewer_id, members)) = viewer {
            let visible: Vec<Uuid> = convs
                .iter()
                .filter(|conv| members.get(&conv.id).is_some_and(|me| conv.last_seq > me.history_from_seq))
                .filter_map(|conv| conv.last_message_id)
                .collect();
            if !visible.is_empty() {
                let rows: Vec<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT {MSG} FROM chat_messages x WHERE x.id = ANY($1)"
                )))
                .bind(&visible)
                .fetch_all(&core.db)
                .await?;
                let view = View::user(viewer_id, members.values().map(|me| (me.conversation_id, me.history_from_seq)));
                for (row, message) in rows.iter().zip(self.render_messages(&rows, &view).await?) {
                    last_messages.insert(row.conversation_id, message);
                }
            }
            let direct_ids: Vec<Uuid> = convs.iter().filter(|conv| conv.is_direct()).map(|conv| conv.id).collect();
            if !direct_ids.is_empty() {
                other_states = sqlx::query_as::<_, (Uuid, i16)>(
                    "SELECT conversation_id, request_state FROM chat_members WHERE conversation_id = ANY($1) AND user_id <> $2",
                )
                .bind(&direct_ids)
                .bind(viewer_id)
                .fetch_all(&core.db)
                .await?
                .into_iter()
                .collect();
            }
        }

        let mut rendered = Vec::with_capacity(convs.len());
        for conv in convs {
            let members = preview
                .get(&conv.id)
                .map(|ids| ids.iter().map(|id| users::pick(&summaries, *id)).collect())
                .unwrap_or_default();
            let viewer_state = viewer.and_then(|(_, members)| members.get(&conv.id)).map(|me| {
                // While the other side has not accepted, the viewer's
                // messages are a request with a limited allowance.
                let requesting =
                    conv.is_direct() && other_states.get(&conv.id).is_some_and(|state| *state != request::ACCEPTED);
                let muted = me.muted_now();
                pb::ViewerState {
                    role: i32::from(me.role).into(),
                    request_state: i32::from(me.request_state).into(),
                    unread_count: me.unread_count,
                    unread_mention_count: me.unread_mention_count,
                    read_seq: me.read_seq,
                    pinned: me.pinned_at.is_some(),
                    muted,
                    muted_until: opt_ts(me.muted_until.filter(|_| muted)),
                    archived: me.archived_at.is_some(),
                    hidden: me.hidden_at.is_some(),
                    joined_at: ts(me.joined_at),
                    history_from_seq: me.history_from_seq,
                    request_messages_left: if requesting {
                        (self.cfg.request_message_limit - me.request_messages).max(0)
                    } else {
                        -1
                    },
                    ..Default::default()
                }
            });
            rendered.push(pb::Conversation {
                id: conv.id.to_string(),
                kind: i32::from(conv.kind).into(),
                name: conv.name.clone(),
                icon: assets::image_from_json(conv.icon.as_ref()).into(),
                member_count: conv.member_count,
                members,
                last_message: last_messages.remove(&conv.id).into(),
                last_seq: conv.last_seq,
                last_activity_at: ts(conv.last_activity_at),
                created_at: ts(conv.created_at),
                read_only: conv.read_only,
                active_call_id: live_calls.get(&conv.id).map(|id| id.to_string()).unwrap_or_default(),
                viewer: viewer_state.into(),
                ..Default::default()
            });
        }
        Ok(rendered)
    }

    /// Read cursors of the members who accepted a conversation.
    pub(crate) async fn read_cursors(&self, conversation_id: Uuid) -> AppResult<Vec<pb::ReadCursor>> {
        let rows: Vec<(Uuid, i64, Option<Time>)> = sqlx::query_as(
            "SELECT user_id, read_seq, read_at FROM chat_members WHERE conversation_id = $1 AND request_state = $2 ORDER BY joined_at, user_id LIMIT 100",
        )
        .bind(conversation_id)
        .bind(request::ACCEPTED)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(user_id, read_seq, read_at)| pb::ReadCursor {
                user_id: user_id.to_string(),
                read_seq,
                read_at: opt_ts(read_at),
                ..Default::default()
            })
            .collect())
    }

    // ───────────────────────────── lists ─────────────────────────────

    pub(crate) async fn list_conversations(
        &self,
        user_id: Uuid,
        filter: pb::ConversationFilter,
        page: &Page,
    ) -> AppResult<(Vec<pb::Conversation>, tokuly_proto::common::PageInfo)> {
        let scope = match filter {
            pb::ConversationFilter::CONVERSATION_FILTER_REQUESTS => "m.request_state = 2",
            pb::ConversationFilter::CONVERSATION_FILTER_ARCHIVED => "m.request_state = 1 AND m.archived_at IS NOT NULL",
            _ => "m.request_state = 1 AND m.archived_at IS NULL",
        };
        let cursor: Option<ListCursor> = page.decode()?;
        let (pinned, activity, id) = match cursor {
            Some(ListCursor(pinned, activity, id)) => (Some(pinned), Some(activity), Some(id)),
            None => (None, None, None),
        };
        // Direct conversations nobody wrote in yet are not listed.
        let mut rows: Vec<ListedConversation> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CONV}, {MEMBER}
             FROM chat_members m JOIN chat_conversations c ON c.id = m.conversation_id
             WHERE m.user_id = $1 AND {scope} AND m.hidden_at IS NULL AND (c.kind = {group} OR c.last_seq > 0)
               AND ($2::int IS NULL OR ((m.pinned_at IS NOT NULL)::int, c.last_activity_at, c.id) < ($2, $3, $4))
             ORDER BY (m.pinned_at IS NOT NULL)::int DESC, c.last_activity_at DESC, c.id DESC
             LIMIT $5",
            group = kind::GROUP
        )))
        .bind(user_id)
        .bind(pinned)
        .bind(activity)
        .bind(id)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| {
            ListCursor(i32::from(row.me.pinned_at.is_some()), row.conv.last_activity_at, row.conv.id)
        });
        let conversations =
            self.conversations_for(user_id, rows.into_iter().map(|row| (row.conv, row.me)).collect()).await?;
        Ok((conversations, info))
    }

    pub(crate) async fn list_members(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<pb::Member>, tokuly_proto::common::PageInfo)> {
        self.access(user_id, conversation_id).await?;
        let cursor: Option<MemberCursor> = page.decode()?;
        let (after_role, after_joined, after_id) = match cursor {
            Some(MemberCursor(role, joined, id)) => (Some(role), Some(joined), Some(id)),
            None => (None, None, None),
        };
        let mut rows: Vec<MemberRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MEMBER} FROM chat_members m
             WHERE m.conversation_id = $1
               AND ($2::smallint IS NULL OR (-m.role, m.joined_at, m.user_id) > (-$2, $3, $4))
             ORDER BY m.role DESC, m.joined_at, m.user_id
             LIMIT $5"
        )))
        .bind(conversation_id)
        .bind(after_role)
        .bind(after_joined)
        .bind(after_id)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| MemberCursor(row.role, row.joined_at, row.user_id));
        Ok((self.members_to_proto(&rows).await?, info))
    }

    pub(crate) async fn members_to_proto(&self, rows: &[MemberRow]) -> AppResult<Vec<pb::Member>> {
        let summaries =
            tokuly_identity::summaries(&self.core, &rows.iter().map(|row| row.user_id).collect::<Vec<_>>()).await?;
        Ok(rows
            .iter()
            .map(|row| pb::Member {
                user: users::pick(&summaries, row.user_id).into(),
                role: i32::from(row.role).into(),
                joined_at: ts(row.joined_at),
                added_by_id: row.added_by.map(|id| id.to_string()).unwrap_or_default(),
                ..Default::default()
            })
            .collect())
    }
}

pub(crate) fn ensure_group(conv: &ConversationRow) -> AppResult<()> {
    if conv.is_group() { Ok(()) } else { Err(not_a_group()) }
}

/// Messages announced per statement when a conversation goes.
const DELETE_BATCH: i64 = 1000;

/// Deletes a conversation nobody is left in (a group everyone left, a direct
/// conversation whose two accounts were erased), releasing every upload its
/// messages and its icon held. The messages go with it (`ON DELETE
/// CASCADE`); those not deleted before get `subject.deleted` here, in
/// batches, so that saves of them elsewhere go too (a deleted message was
/// announced then).
pub(crate) async fn delete_conversation(tx: &mut Transaction<'_, Postgres>, conversation_id: Uuid) -> AppResult<()> {
    let mut after = 0_i64;
    loop {
        let batch: Vec<(i64, Uuid)> = sqlx::query_as(
            "SELECT seq, id FROM chat_messages WHERE conversation_id = $1 AND seq > $2 AND deleted_at IS NULL
             ORDER BY seq LIMIT $3",
        )
        .bind(conversation_id)
        .bind(after)
        .bind(DELETE_BATCH)
        .fetch_all(&mut **tx)
        .await?;
        let message_ids: Vec<Uuid> = batch.iter().map(|(_, id)| *id).collect();
        outbound::subjects_deleted(&mut **tx, SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, &message_ids).await?;
        match batch.last() {
            Some(&(seq, _)) if batch.len() as i64 == DELETE_BATCH => after = seq,
            _ => break,
        }
    }
    // Every hold is released on its own, in upload ID order: a forward holds
    // the original upload under its own message, so two conversations deleted
    // at once can hold the same uploads, and they must lock them in the same order.
    let mut holds: Vec<(Uuid, String)> = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT asset_id, message_id FROM chat_message_assets WHERE conversation_id = $1",
    )
    .bind(conversation_id)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|(asset_id, message_id)| (asset_id, crate::send::message_holder(message_id)))
    .collect();
    let icon: Option<Uuid> = sqlx::query_scalar("SELECT icon_asset_id FROM chat_conversations WHERE id = $1")
        .bind(conversation_id)
        .fetch_one(&mut **tx)
        .await?;
    holds.extend(icon.map(|asset_id| (asset_id, icon_holder(conversation_id))));
    holds.sort();
    for (asset_id, holder) in &holds {
        assets::detach(&mut **tx, *asset_id, holder).await?;
    }
    sqlx::query("DELETE FROM chat_conversations WHERE id = $1").bind(conversation_id).execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_are_ordered_whatever_the_argument_order() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(pair_of(a, b), pair_of(b, a));
        let (lo, hi) = pair_of(a, b);
        assert!(lo < hi);
    }
}
