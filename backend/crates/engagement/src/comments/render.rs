//! Comment rows as `tokuly.engagement.v1.Comment`, for one viewer.
//!
//! What a viewer is shown depends on who they are: the account behind a
//! comment written as a channel is only named to that account itself, the
//! reason a comment is held only to the subject owner, and a comment hidden
//! behind a comment block reads as published to its author.
use std::collections::HashMap;

use tokuly_core::prelude::*;
use tokuly_identity::users;
use tokuly_media::assets::image_from_json;
use tokuly_proto::{
    common::{Author, ChannelSummary, StampRef, UserSummary},
    engagement::{Comment, CommentViewerState, Reaction},
};

use super::CommentRow;
use crate::{Engagement, reactions, subjects::SubjectKey};

/// The current owner of the subjects on a page, as far as the caller
/// resolved them. Comments on other subjects fall back to the owner
/// recorded when they were written.
pub(crate) type Owners = HashMap<SubjectKey, Uuid>;

/// An author as shown to `viewer`. For a comment written as a channel the
/// user is left out unless the viewer is that user; a channel that cannot
/// be looked up any more is shown by its ID alone, never by its account.
pub(crate) fn author_for(
    viewer: Option<Uuid>,
    author_id: Uuid,
    channel_id: Option<Uuid>,
    users: &HashMap<Uuid, UserSummary>,
    channels: &HashMap<Uuid, ChannelSummary>,
) -> Author {
    match channel_id {
        None => Author { user: users::pick(users, author_id).into(), ..Default::default() },
        Some(channel_id) => {
            let channel = channels
                .get(&channel_id)
                .cloned()
                .unwrap_or_else(|| ChannelSummary { id: channel_id.to_string(), ..Default::default() });
            let user = (viewer == Some(author_id)).then(|| users::pick(users, author_id));
            Author { user: user.into(), channel: channel.into(), ..Default::default() }
        }
    }
}

impl Engagement {
    /// Builds the API representation of `rows` for `viewer` (`None`: a
    /// guest, or the anonymous audience of a realtime topic). The caller
    /// has already decided that the viewer may see every row.
    pub(crate) async fn render_comments(
        &self,
        rows: &[CommentRow],
        viewer: Option<Uuid>,
        owners: &Owners,
    ) -> AppResult<Vec<Comment>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let core = &self.core;
        let comment_ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let hidden = self.hidden_from(viewer).await?;

        let mention_rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT comment_id, user_id FROM comment_mentions WHERE comment_id = ANY($1) ORDER BY comment_id, user_id",
        )
        .bind(&comment_ids)
        .fetch_all(&core.db)
        .await?;
        let mut mentions: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for (comment_id, user_id) in mention_rows {
            mentions.entry(comment_id).or_default().push(user_id);
        }

        // The replies that replies answer: (author, channel) by comment ID.
        let answered_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.reply_to_id).collect();
        let answered: HashMap<Uuid, (Uuid, Option<Uuid>)> = if answered_ids.is_empty() {
            HashMap::new()
        } else {
            let found: Vec<(Uuid, Uuid, Option<Uuid>)> =
                sqlx::query_as("SELECT id, author_id, channel_id FROM comments WHERE id = ANY($1)")
                    .bind(&answered_ids)
                    .fetch_all(&core.db)
                    .await?;
            found.into_iter().map(|(id, author_id, channel_id)| (id, (author_id, channel_id))).collect()
        };

        let identities = rows.iter().map(|row| (row.author_id, row.channel_id)).chain(answered.values().copied());
        let mut user_ids: Vec<Uuid> = mentions.values().flatten().copied().collect();
        let mut channel_ids: Vec<Uuid> = Vec::new();
        for (author_id, channel_id) in identities {
            match channel_id {
                Some(channel_id) => channel_ids.push(channel_id),
                None => user_ids.push(author_id),
            }
            if viewer == Some(author_id) {
                user_ids.push(author_id);
            }
        }
        user_ids.sort();
        user_ids.dedup();
        channel_ids.sort();
        channel_ids.dedup();
        let users = tokuly_identity::summaries(core, &user_ids).await?;
        let channels = self.channel_summaries(&channel_ids).await?;

        let stamp_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.stamp_id).collect();
        let stamps = tokuly_stamp::stamp_refs(core, &stamp_ids).await?;
        let own_reactions: HashMap<Uuid, Reaction> = match viewer {
            Some(viewer) => reactions::of_user_on_comments(core, viewer, &comment_ids).await?,
            None => HashMap::new(),
        };

        Ok(rows
            .iter()
            .map(|row| {
                let owner_id = owners.get(&row.key()).copied().unwrap_or(row.subject_owner_id);
                let is_author = viewer == Some(row.author_id);
                let is_owner = viewer == Some(owner_id) && !owner_id.is_nil();
                let mentioned: Vec<UserSummary> = mentions
                    .get(&row.id)
                    .into_iter()
                    .flatten()
                    .filter(|user_id| !hidden.contains(user_id))
                    .filter_map(|user_id| users.get(user_id).filter(|user| !user.deleted).cloned())
                    .collect();
                let stamp = row.stamp_id.map(|stamp_id| {
                    // A deleted stamp leaves its ID: "this stamp is no longer available".
                    stamps
                        .get(&stamp_id)
                        .cloned()
                        .unwrap_or_else(|| StampRef { id: stamp_id.to_string(), ..Default::default() })
                });
                let reply_to = row
                    .reply_to_id
                    .and_then(|id| answered.get(&id))
                    .filter(|(author_id, _)| !hidden.contains(author_id))
                    .map(|(author_id, channel_id)| author_for(viewer, *author_id, *channel_id, &users, &channels));
                let viewer_state = viewer.map(|_| CommentViewerState {
                    reaction: own_reactions.get(&row.id).cloned().into(),
                    is_author,
                    can_edit: is_author,
                    can_delete: is_author || is_owner,
                    can_moderate: is_owner,
                    ..Default::default()
                });
                Comment {
                    id: row.id.to_string(),
                    subject: row.key().to_ref().into(),
                    parent_id: row.parent_id.map(|id| id.to_string()).unwrap_or_default(),
                    author: author_for(viewer, row.author_id, row.channel_id, &users, &channels).into(),
                    text: row.text.clone(),
                    mentions: mentioned,
                    stamp: stamp.into(),
                    image: image_from_json(row.image.as_ref()).into(),
                    timestamp_seconds: row.timestamp_seconds,
                    status: row.status_for(viewer).into(),
                    like_count: row.like_count,
                    dislike_count: row.dislike_count,
                    reply_count: row.reply_count,
                    pinned: row.pinned_at.is_some(),
                    hearted: row.hearted_at.is_some(),
                    edited: row.edited_at.is_some(),
                    created_at: ts(row.created_at),
                    edited_at: opt_ts(row.edited_at),
                    reply_to: reply_to.into(),
                    viewer: viewer_state.into(),
                    hold_reason: if is_owner { row.hold_reason.into() } else { Default::default() },
                    ..Default::default()
                }
            })
            .collect())
    }

    /// [`Self::render_comments`] for one comment on a subject owned by `owner_id`.
    pub(crate) async fn render_comment(
        &self,
        row: &CommentRow,
        viewer: Option<Uuid>,
        owner_id: Uuid,
    ) -> AppResult<Comment> {
        let owners = Owners::from([(row.key(), owner_id)]);
        let mut comments = self.render_comments(std::slice::from_ref(row), viewer, &owners).await?;
        Ok(comments.pop().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: Uuid) -> UserSummary {
        UserSummary { id: id.to_string(), handle: "alice".into(), ..Default::default() }
    }

    #[test]
    fn a_user_comment_names_the_user() {
        let author = Uuid::now_v7();
        let users = HashMap::from([(author, user(author))]);
        let shown = author_for(None, author, None, &users, &HashMap::new());
        assert_eq!(shown.user.as_option().unwrap().handle, "alice");
        assert!(shown.channel.as_option().is_none());
    }

    #[test]
    fn a_channel_comment_names_the_account_only_to_itself() {
        let (author, channel, stranger) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let users = HashMap::from([(author, user(author))]);
        let channels = HashMap::from([(
            channel,
            ChannelSummary { id: channel.to_string(), name: "Ch".into(), ..Default::default() },
        )]);
        for viewer in [None, Some(stranger)] {
            let shown = author_for(viewer, author, Some(channel), &users, &channels);
            assert!(shown.user.as_option().is_none());
            assert_eq!(shown.channel.as_option().unwrap().name, "Ch");
        }
        let own = author_for(Some(author), author, Some(channel), &users, &channels);
        assert_eq!(own.user.as_option().unwrap().id, author.to_string());
    }

    #[test]
    fn a_vanished_channel_never_falls_back_to_the_account() {
        let (author, channel) = (Uuid::now_v7(), Uuid::now_v7());
        let users = HashMap::from([(author, user(author))]);
        let shown = author_for(None, author, Some(channel), &users, &HashMap::new());
        assert!(shown.user.as_option().is_none());
        assert_eq!(shown.channel.as_option().unwrap().id, channel.to_string());
    }
}
