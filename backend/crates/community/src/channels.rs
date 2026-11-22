//! Channels: the boards of a community.
use std::collections::HashSet;

use tokuly_core::prelude::*;
use tokuly_proto::community::{self as pb, ModerationAction, Permission};

use crate::{
    Community,
    access::{Access, Caller, permission_denied},
    model::{ChannelRow, channel_columns, state},
    modlog::{self, LogEntry},
    validate,
    workers::SweepJob,
};

pub fn channel_not_found() -> AppError {
    AppError::not_found("CHANNEL_NOT_FOUND", "channel not found")
}

/// What an update changes; `None` leaves a field alone.
#[derive(Debug, Default)]
pub struct ChannelEdit {
    pub name: Option<String>,
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub posting: Option<i32>,
    pub anonymous: Option<bool>,
}

impl Community {
    pub(crate) async fn channel_by_id(&self, channel_id: Uuid) -> AppResult<Option<ChannelRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            channel_columns!(),
            " FROM community_channels ch WHERE ch.id = $1 AND ch.deleted_at IS NULL"
        ))
        .bind(channel_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// The channel named by `channel_id` and the caller's standing in its
    /// community. A channel of a community the caller may not know about
    /// does not exist.
    pub(crate) async fn open_channel(&self, caller: &Caller, channel_id: &str) -> AppResult<(ChannelRow, Access)> {
        let id = Uuid::parse_str(channel_id.trim()).map_err(|_| channel_not_found())?;
        let channel = self.channel_by_id(id).await?.ok_or_else(channel_not_found)?;
        match self.access_by_id(caller, channel.community_id).await {
            Ok(access) => Ok((channel, access)),
            Err(error) if error.is_not_found() => Err(channel_not_found()),
            Err(error) => Err(error),
        }
    }

    async fn channel_rows(&self, community_id: Uuid) -> AppResult<Vec<ChannelRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            channel_columns!(),
            " FROM community_channels ch WHERE ch.community_id = $1 AND ch.deleted_at IS NULL ORDER BY ch.position, ch.id"
        ))
        .bind(community_id)
        .fetch_all(&self.core.db)
        .await?)
    }

    /// A channel as the caller of `access` sees it.
    pub(crate) fn channel_proto(&self, access: &Access, row: &ChannelRow, has_unread: bool) -> pb::Channel {
        let can_post = access.timeout().is_none() && access.role().is_some_and(|role| row.open_to(role));
        pb::Channel {
            id: row.id.to_string(),
            community_id: row.community_id.to_string(),
            name: row.name.clone(),
            description: row.description.clone(),
            emoji: row.emoji.clone(),
            position: row.position,
            posting: row.posting.into(),
            anonymous: row.anonymous,
            slow_mode_seconds: row.slow_mode_seconds,
            thread_count: row.thread_count,
            last_activity_at: ts(row.last_activity_at),
            has_unread,
            can_post,
            ..Default::default()
        }
    }

    /// The channels of a community in display order, with the caller's
    /// unread markers.
    pub(crate) async fn list_channels(&self, access: &Access) -> AppResult<Vec<pb::Channel>> {
        access.require_read()?;
        let rows = self.channel_rows(access.community.id).await?;
        let unread: HashSet<Uuid> = match &access.member {
            Some(member) => {
                let ids: Vec<Uuid> = sqlx::query_scalar(
                    "SELECT ch.id FROM community_channels ch
                     LEFT JOIN community_channel_reads cr ON cr.channel_id = ch.id AND cr.user_id = $2
                     WHERE ch.community_id = $1 AND ch.deleted_at IS NULL AND EXISTS (
                         SELECT 1 FROM community_threads t
                         LEFT JOIN community_thread_reads tr ON tr.thread_id = t.id AND tr.user_id = $2
                         WHERE t.channel_id = ch.id AND t.deleted_at IS NULL AND t.state = 1
                           AND t.last_activity_at > greatest($3, coalesce(cr.read_at, $3))
                           AND (tr.read_at IS NULL OR t.last_activity_at > tr.read_at))",
                )
                .bind(access.community.id)
                .bind(member.user_id)
                .bind(member.read_floor())
                .fetch_all(&self.core.db)
                .await?;
                ids.into_iter().collect()
            }
            None => HashSet::new(),
        };
        Ok(rows.iter().map(|row| self.channel_proto(access, row, unread.contains(&row.id))).collect())
    }

    /// Adds a channel at the end of the list. Needs MANAGE_CHANNELS.
    pub(crate) async fn create_channel(
        &self,
        access: &Access,
        req: &pb::CreateChannelRequest,
    ) -> AppResult<pb::Channel> {
        access.require(Permission::PERMISSION_MANAGE_CHANNELS)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        let name = validate::channel_name(&req.name)?;
        let description = validate::description(&req.description, validate::MAX_CHANNEL_DESCRIPTION_CHARS)?;
        let emoji = validate::emoji(&req.emoji)?;
        let posting = validate::posting(req.posting)?;
        let community_id = access.community.id;
        self.staff_limit(actor).await?;

        let mut tx = self.core.db.begin().await?;
        // Locking the community row serializes channel creation: the limit
        // and the positions hold.
        sqlx::query("SELECT 1 FROM communities WHERE id = $1 FOR UPDATE").bind(community_id).execute(&mut *tx).await?;
        let (count, last): (i64, Option<i32>) = sqlx::query_as(
            "SELECT count(*), max(position) FROM community_channels WHERE community_id = $1 AND deleted_at IS NULL",
        )
        .bind(community_id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            count < i64::from(self.cfg.max_channels),
            precondition,
            "TOO_MANY_CHANNELS",
            "a community has at most {} channels",
            self.cfg.max_channels
        );
        let channel: ChannelRow = sqlx::query_as(concat!(
            "INSERT INTO community_channels AS ch (community_id, name, description, emoji, position, posting, anonymous)
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING ",
            channel_columns!()
        ))
        .bind(community_id)
        .bind(&name)
        .bind(&description)
        .bind(&emoji)
        .bind(last.map_or(0, |last| last + 1))
        .bind(posting)
        .bind(req.anonymous)
        .fetch_one(&mut *tx)
        .await?;
        let entry = LogEntry::new(community_id, ModerationAction::MODERATION_ACTION_CHANNEL_CREATED)
            .by(actor)
            .channel(channel.id)
            .detail("name", &channel.name);
        modlog::record(&mut *tx, &entry).await?;
        tx.commit().await?;
        self.push_topic(
            community_id,
            pb::ChannelsChanged { community_id: community_id.to_string(), ..Default::default() },
        )
        .await;
        Ok(self.channel_proto(access, &channel, false))
    }

    /// Edits a channel. Needs MANAGE_CHANNELS.
    pub(crate) async fn update_channel(
        &self,
        access: &Access,
        channel: &ChannelRow,
        edit: ChannelEdit,
    ) -> AppResult<pb::Channel> {
        access.require(Permission::PERMISSION_MANAGE_CHANNELS)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        self.staff_limit(actor).await?;
        let mut tx = self.core.db.begin().await?;
        if edit.anonymous.is_some_and(|anonymous| anonymous != channel.anonymous) {
            // Threads keep the mode they were written in; a channel showing
            // both would be confusing about who is named and who is not.
            let has_threads: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM community_threads WHERE channel_id = $1 AND deleted_at IS NULL)",
            )
            .bind(channel.id)
            .fetch_one(&mut *tx)
            .await?;
            ensure!(
                !has_threads,
                precondition,
                "CHANNEL_NOT_EMPTY",
                "anonymous mode can only be changed while the channel has no threads"
            );
        }
        let updated: ChannelRow = sqlx::query_as(concat!(
            "UPDATE community_channels AS ch SET
                 name = coalesce($2, name), description = coalesce($3, description), emoji = coalesce($4, emoji),
                 posting = coalesce($5, posting), anonymous = coalesce($6, anonymous)
             WHERE ch.id = $1 AND ch.deleted_at IS NULL RETURNING ",
            channel_columns!()
        ))
        .bind(channel.id)
        .bind(&edit.name)
        .bind(&edit.description)
        .bind(&edit.emoji)
        .bind(edit.posting)
        .bind(edit.anonymous)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(channel_not_found)?;
        let entry = LogEntry::new(channel.community_id, ModerationAction::MODERATION_ACTION_CHANNEL_UPDATED)
            .by(actor)
            .channel(channel.id)
            .detail("name", &updated.name);
        modlog::record(&mut *tx, &entry).await?;
        tx.commit().await?;
        self.push_channels_changed(channel.community_id).await;
        Ok(self.channel_proto(access, &updated, false))
    }

    pub(crate) async fn push_channels_changed(&self, community_id: Uuid) {
        self.push_topic(
            community_id,
            pb::ChannelsChanged { community_id: community_id.to_string(), ..Default::default() },
        )
        .await;
    }

    /// Deletes a channel with everything in it. Needs MANAGE_CHANNELS; a
    /// community keeps at least one channel.
    pub(crate) async fn delete_channel(&self, access: &Access, channel: &ChannelRow) -> AppResult<()> {
        access.require(Permission::PERMISSION_MANAGE_CHANNELS)?;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        let community_id = channel.community_id;
        self.staff_limit(actor).await?;
        let mut tx = self.core.db.begin().await?;
        sqlx::query("SELECT 1 FROM communities WHERE id = $1 FOR UPDATE").bind(community_id).execute(&mut *tx).await?;
        let remaining: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM community_channels WHERE community_id = $1 AND deleted_at IS NULL AND id <> $2",
        )
        .bind(community_id)
        .bind(channel.id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(remaining > 0, precondition, "LAST_CHANNEL", "a community keeps at least one channel");
        let deleted =
            sqlx::query("UPDATE community_channels SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL")
                .bind(channel.id)
                .execute(&mut *tx)
                .await?;
        if deleted.rows_affected() == 0 {
            return Err(channel_not_found());
        }
        // The threads go with the channel: they leave the search index in
        // the background (the sweep job) and stay on file, like any removed
        // thread, until their retention is over.
        let states: Vec<i32> = sqlx::query_scalar(
            "UPDATE community_threads SET deleted_at = now() WHERE channel_id = $1 AND deleted_at IS NULL RETURNING state",
        )
        .bind(channel.id)
        .fetch_all(&mut *tx)
        .await?;
        let visible = states.iter().filter(|s| **s == state::VISIBLE).count() as i32;
        sqlx::query("UPDATE communities SET thread_count = greatest(thread_count - $2, 0) WHERE id = $1")
            .bind(community_id)
            .bind(visible)
            .execute(&mut *tx)
            .await?;
        let entry = LogEntry::new(community_id, ModerationAction::MODERATION_ACTION_CHANNEL_DELETED)
            .by(actor)
            .channel(channel.id)
            .detail("name", &channel.name)
            .detail("threads", states.len());
        modlog::record(&mut *tx, &entry).await?;
        tx.commit().await?;
        self.queue_sweep(SweepJob { community_id, channel_id: Some(channel.id) }).await;
        self.push_channels_changed(community_id).await;
        Ok(())
    }

    /// Sets the order of the channels. Needs MANAGE_CHANNELS.
    pub(crate) async fn reorder_channels(&self, access: &Access, channel_ids: &[Uuid]) -> AppResult<Vec<pb::Channel>> {
        access.require(Permission::PERMISSION_MANAGE_CHANNELS)?;
        let community_id = access.community.id;
        self.staff_limit(access.user_id.ok_or_else(permission_denied)?).await?;
        let mut tx = self.core.db.begin().await?;
        let current: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM community_channels WHERE community_id = $1 AND deleted_at IS NULL FOR UPDATE",
        )
        .bind(community_id)
        .fetch_all(&mut *tx)
        .await?;
        let requested: HashSet<Uuid> = channel_ids.iter().copied().collect();
        let same = requested.len() == channel_ids.len()
            && current.len() == channel_ids.len()
            && current.iter().all(|id| requested.contains(id));
        ensure!(same, invalid, "CHANNEL_ORDER_MISMATCH", "name every channel of the community exactly once");
        sqlx::query(
            "UPDATE community_channels ch SET position = o.position::int - 1
             FROM unnest($1::uuid[]) WITH ORDINALITY AS o(id, position) WHERE ch.id = o.id",
        )
        .bind(channel_ids)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.push_channels_changed(community_id).await;
        self.list_channels(access).await
    }
}
