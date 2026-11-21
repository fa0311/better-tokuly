//! Group membership: adding friends, removing members, roles, leaving.
//!
//! Every change locks the conversation first, so the member count, the
//! "exactly one owner" rule and the system messages that tell the story
//! stay consistent under concurrent changes.
use sqlx::{Postgres, Transaction};
use tokuly_core::{events, outbound, prelude::*};
use tokuly_proto::chat as pb;
use tokuly_social::graph;

use crate::{
    Chat,
    access::{self, Access, conversation_not_found},
    calls::CallChange,
    conversations::{delete_conversation, ensure_group, not_allowed},
    events::{MEMBER_JOINED, MEMBER_LEFT, MemberJoined, MemberLeft},
    model::{ConversationRow, MEMBER, MemberRow, SystemPayload, role},
    notices,
    send::{Draft, Posted},
};

fn member_not_found() -> AppError {
    AppError::not_found("MEMBER_NOT_FOUND", "this user is not a member of the group")
}

/// What happened when a member's row was removed; drives the pushes that
/// follow the commit.
pub(crate) struct Departure {
    pub conversation_id: Uuid,
    pub user_id: Uuid,
    /// The group had nobody left and was deleted.
    pub deleted: bool,
    /// The member who inherited the group, when the owner went.
    pub new_owner: Option<Uuid>,
    posted: Vec<Posted>,
    call: Option<CallChange>,
}

impl Chat {
    /// Adds friends of `user_id` to a group; returns the members actually added.
    pub(crate) async fn add_members(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        user_ids: &[Uuid],
    ) -> AppResult<(Access, Vec<pb::Member>)> {
        let core = &self.core;
        let access = self.access(user_id, conversation_id).await?;
        ensure_group(&access.conv)?;
        self.ensure_accepted(&access)?;
        let mut wanted: Vec<Uuid> = Vec::new();
        for id in user_ids {
            if *id != user_id && !wanted.contains(id) {
                wanted.push(*id);
            }
        }
        ensure!(!wanted.is_empty(), invalid, "NO_MEMBERS", "name at least one user to add");
        core.cache.rate_limit(&format!("chat-add-members:{user_id}"), 60, std::time::Duration::from_secs(3600)).await?;
        let adder = tokuly_identity::summary(core, user_id).await?;
        let friends = graph::friend_ids(core, user_id).await?;

        let mut tx = core.db.begin().await?;
        let mut conv = access::lock_conversation(&mut tx, conversation_id).await?;
        access::member_in_tx(&mut tx, conversation_id, user_id).await?.ok_or_else(conversation_not_found)?;
        let existing: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM chat_members WHERE conversation_id = $1 AND user_id = ANY($2)")
                .bind(conversation_id)
                .bind(&wanted)
                .fetch_all(&mut *tx)
                .await?;
        let new: Vec<Uuid> = wanted.into_iter().filter(|id| !existing.contains(id)).collect();
        if new.is_empty() {
            return Ok((access, Vec::new()));
        }
        ensure!(
            new.iter().all(|id| friends.contains(id)),
            precondition,
            "NOT_FRIENDS",
            "you can only add your friends to a group"
        );
        ensure!(
            conv.member_count as usize + new.len() <= self.cfg.max_group_members as usize,
            precondition,
            "TOO_MANY_MEMBERS",
            "a group can have at most {} members",
            self.cfg.max_group_members
        );
        // New members see the conversation from here on: nothing before
        // this point is visible to them, and nothing counts as unread.
        let rows: Vec<MemberRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "INSERT INTO chat_members AS m (conversation_id, user_id, role, added_by, read_seq, history_from_seq)
             SELECT $1, member, $3, $4, $5, $5 FROM unnest($2::uuid[]) AS member
             RETURNING {MEMBER}"
        )))
        .bind(conversation_id)
        .bind(&new)
        .bind(role::MEMBER)
        .bind(user_id)
        .bind(conv.last_seq)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query("UPDATE chat_conversations SET member_count = member_count + $2 WHERE id = $1")
            .bind(conversation_id)
            .bind(new.len() as i32)
            .execute(&mut *tx)
            .await?;
        conv.member_count += new.len() as i32;
        let payload = SystemPayload::new(pb::SystemEventType::SYSTEM_EVENT_TYPE_MEMBERS_ADDED, Some(user_id))
            .targets(new.clone());
        let posted = self.post(&mut tx, &mut conv, Draft::system(payload)).await?;
        for member in &new {
            events::emit(
                &mut *tx,
                MEMBER_JOINED,
                Some(user_id),
                &MemberJoined { conversation_id, user_id: *member, added_by: user_id },
            )
            .await?;
            outbound::notify(&mut *tx, &notices::added_to_group(*member, &adder, user_id, &conv)).await?;
        }
        tx.commit().await?;

        self.push_conversation_shared(conversation_id).await;
        for member in &new {
            self.push_conversation_to(conversation_id, *member).await;
        }
        self.announce(&posted).await;
        let added = self.members_to_proto(&rows).await?;
        Ok((self.access(user_id, conversation_id).await?, added))
    }

    /// Removes a member's row from a group inside `tx` (the conversation is
    /// locked): the count drops, a live call loses them, the story is told
    /// in a system message, an orphaned group gets a new owner and an empty
    /// one is deleted. `removed_by` is the member who removed them; `None`
    /// when they left by themselves (or their account was deleted).
    pub(crate) async fn remove_member_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        conv: &mut ConversationRow,
        member: &MemberRow,
        removed_by: Option<Uuid>,
    ) -> AppResult<Departure> {
        let conversation_id = conv.id;
        let user_id = member.user_id;
        sqlx::query("DELETE FROM chat_members WHERE conversation_id = $1 AND user_id = $2")
            .bind(conversation_id)
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE chat_conversations SET member_count = member_count - 1 WHERE id = $1")
            .bind(conversation_id)
            .execute(&mut **tx)
            .await?;
        conv.member_count -= 1;
        let call = self.drop_from_live_call(tx, conv, user_id).await?;
        events::emit(
            &mut **tx,
            MEMBER_LEFT,
            removed_by.or(Some(user_id)),
            &MemberLeft { conversation_id, user_id, removed_by },
        )
        .await?;
        let mut departure =
            Departure { conversation_id, user_id, deleted: false, new_owner: None, posted: Vec::new(), call };

        if conv.member_count <= 0 {
            delete_conversation(tx, conversation_id).await?;
            departure.deleted = true;
            return Ok(departure);
        }
        let payload = match removed_by {
            Some(actor) => SystemPayload::new(pb::SystemEventType::SYSTEM_EVENT_TYPE_MEMBER_REMOVED, Some(actor))
                .targets([user_id]),
            None => SystemPayload::new(pb::SystemEventType::SYSTEM_EVENT_TYPE_MEMBER_LEFT, Some(user_id)),
        };
        departure.posted.push(self.post(tx, conv, Draft::system(payload)).await?);

        if member.role == role::OWNER {
            // The group goes to the longest standing admin, or else the
            // longest standing member.
            let heir: Option<Uuid> = sqlx::query_scalar(
                "UPDATE chat_members SET role = $2
                 WHERE conversation_id = $1 AND user_id = (
                     SELECT user_id FROM chat_members WHERE conversation_id = $1
                     ORDER BY (role = $3) DESC, joined_at, user_id LIMIT 1)
                 RETURNING user_id",
            )
            .bind(conversation_id)
            .bind(role::OWNER)
            .bind(role::ADMIN)
            .fetch_optional(&mut **tx)
            .await?;
            if let Some(heir) = heir {
                let payload =
                    SystemPayload::new(pb::SystemEventType::SYSTEM_EVENT_TYPE_OWNER_CHANGED, None).targets([heir]);
                departure.posted.push(self.post(tx, conv, Draft::system(payload)).await?);
                departure.new_owner = Some(heir);
            }
        }
        Ok(departure)
    }

    /// The pushes that follow a committed [`Chat::remove_member_tx`].
    pub(crate) async fn after_departure(&self, departure: &Departure, reason: pb::ConversationRemovalReason) {
        let conversation_id = departure.conversation_id;
        self.forget_presence(conversation_id, departure.user_id).await;
        self.push_conversation_removed(conversation_id, departure.user_id, reason).await;
        if let Some(call) = &departure.call {
            self.after_call_change(call).await;
        }
        if departure.deleted {
            return;
        }
        self.push_conversation_shared(conversation_id).await;
        if let Some(owner) = departure.new_owner {
            self.push_conversation_to(conversation_id, owner).await;
        }
        for posted in &departure.posted {
            self.announce(posted).await;
        }
    }

    pub(crate) async fn remove_member(&self, user_id: Uuid, conversation_id: Uuid, target: Uuid) -> AppResult<Access> {
        let access = self.access(user_id, conversation_id).await?;
        ensure_group(&access.conv)?;
        ensure!(target != user_id, invalid, "CANNOT_REMOVE_SELF", "leave the group instead of removing yourself");

        let mut tx = self.core.db.begin().await?;
        let mut conv = access::lock_conversation(&mut tx, conversation_id).await?;
        let me = access::member_in_tx(&mut tx, conversation_id, user_id).await?.ok_or_else(conversation_not_found)?;
        let member = access::member_in_tx(&mut tx, conversation_id, target).await?;
        // The owner may remove anyone; admins may remove plain members.
        // Plain members learn nothing about who else is in the group this way.
        if !me.manages() {
            return Err(not_allowed());
        }
        let member = member.ok_or_else(member_not_found)?;
        if me.role <= member.role {
            return Err(not_allowed());
        }
        let departure = self.remove_member_tx(&mut tx, &mut conv, &member, Some(user_id)).await?;
        tx.commit().await?;
        self.after_departure(&departure, pb::ConversationRemovalReason::CONVERSATION_REMOVAL_REASON_REMOVED).await;
        self.access(user_id, conversation_id).await
    }

    pub(crate) async fn leave_conversation(&self, user_id: Uuid, conversation_id: Uuid) -> AppResult<()> {
        let access = self.access(user_id, conversation_id).await?;
        ensure_group(&access.conv)?;
        let mut tx = self.core.db.begin().await?;
        let mut conv = access::lock_conversation(&mut tx, conversation_id).await?;
        let me = access::member_in_tx(&mut tx, conversation_id, user_id).await?.ok_or_else(conversation_not_found)?;
        let departure = self.remove_member_tx(&mut tx, &mut conv, &me, None).await?;
        tx.commit().await?;
        self.after_departure(&departure, pb::ConversationRemovalReason::CONVERSATION_REMOVAL_REASON_LEFT).await;
        Ok(())
    }

    /// Changes a member's role (owner only). Making someone the owner hands
    /// the group over: the caller becomes an admin.
    pub(crate) async fn set_member_role(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        target: Uuid,
        new_role: pb::MemberRole,
    ) -> AppResult<pb::Member> {
        let access = self.access(user_id, conversation_id).await?;
        ensure_group(&access.conv)?;
        let new_role = match new_role {
            pb::MemberRole::MEMBER_ROLE_MEMBER => role::MEMBER,
            pb::MemberRole::MEMBER_ROLE_ADMIN => role::ADMIN,
            pb::MemberRole::MEMBER_ROLE_OWNER => role::OWNER,
            _ => bail!(invalid, "ROLE_INVALID", "the role must be member, admin or owner"),
        };
        ensure!(target != user_id, invalid, "CANNOT_CHANGE_OWN_ROLE", "you cannot change your own role");
        // Every change adds a line to the conversation.
        self.core.cache.rate_limit(&format!("chat-role:{user_id}"), 60, std::time::Duration::from_secs(3600)).await?;

        let mut tx = self.core.db.begin().await?;
        let mut conv = access::lock_conversation(&mut tx, conversation_id).await?;
        let me = access::member_in_tx(&mut tx, conversation_id, user_id).await?.ok_or_else(conversation_not_found)?;
        if me.role != role::OWNER {
            return Err(not_allowed());
        }
        let member = access::member_in_tx(&mut tx, conversation_id, target).await?.ok_or_else(member_not_found)?;
        let mut posted = None;
        if member.role != new_role {
            sqlx::query("UPDATE chat_members SET role = $3 WHERE conversation_id = $1 AND user_id = $2")
                .bind(conversation_id)
                .bind(target)
                .bind(new_role)
                .execute(&mut *tx)
                .await?;
            let event = match new_role {
                role::OWNER => {
                    sqlx::query("UPDATE chat_members SET role = $3 WHERE conversation_id = $1 AND user_id = $2")
                        .bind(conversation_id)
                        .bind(user_id)
                        .bind(role::ADMIN)
                        .execute(&mut *tx)
                        .await?;
                    pb::SystemEventType::SYSTEM_EVENT_TYPE_OWNER_CHANGED
                }
                role::ADMIN => pb::SystemEventType::SYSTEM_EVENT_TYPE_ADMIN_ADDED,
                _ => pb::SystemEventType::SYSTEM_EVENT_TYPE_ADMIN_REMOVED,
            };
            let payload = SystemPayload::new(event, Some(user_id)).targets([target]);
            posted = Some(self.post(&mut tx, &mut conv, Draft::system(payload)).await?);
        }
        let updated = access::member_in_tx(&mut tx, conversation_id, target).await?.ok_or_else(member_not_found)?;
        tx.commit().await?;
        if let Some(posted) = &posted {
            // Both roles are part of each one's own view of the conversation.
            self.push_conversation_to(conversation_id, target).await;
            self.push_conversation_to(conversation_id, user_id).await;
            self.announce(posted).await;
        }
        let mut members = self.members_to_proto(std::slice::from_ref(&updated)).await?;
        members.pop().ok_or_else(member_not_found)
    }
}
