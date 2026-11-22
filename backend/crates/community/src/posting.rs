//! What threads and replies share when they are written: uploads, stamps,
//! mentions, the posting limits and slow mode, the word filter — and the
//! extra rules of anonymous channels, which exist so that nothing but the
//! pseudonym says anything about who wrote a post:
//!
//! - the staff gets no exemption there (not from slow mode, not from the
//!   word filter, not from auto-hide): an exemption would be visible and
//!   would say "this was written by the staff";
//! - a stamp must be one anyone could send (a personal stamp names its
//!   maker), an upload one that is used nowhere else (the same picture on a
//!   named post would name the author);
//! - enough members must be able to post: with two people, "anonymous" is
//!   the other one.
use std::time::Duration;

use tokuly_core::prelude::*;
use tokuly_media::assets;
use tokuly_proto::{
    common::Image,
    community as pb,
    media::{AssetKind, AssetPurpose},
};
use tokuly_social::graph;

use crate::{
    Community,
    access::Access,
    model::{ChannelRow, CommunityRow, hidden, level, posting, state},
    text, validate,
};

/// Upload purposes accepted for the images of a post.
const IMAGE_PURPOSES: [AssetPurpose; 2] =
    [AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE, AssetPurpose::ASSET_PURPOSE_CHAT_IMAGE];
/// Mention notifications one author may cause for one member per hour, and
/// for everyone together. Mentions beyond that stay in the text and tell
/// nobody: a mention is not a way to ring someone's phone all day — least
/// of all from behind a pseudonym, where the member cannot block the author.
const MENTIONS_PER_PAIR_PER_HOUR: u32 = 5;
const MENTIONS_PER_AUTHOR_PER_HOUR: u32 = 200;

pub fn thread_holder(thread_id: Uuid) -> String {
    format!("community:thread:{thread_id}")
}

pub fn reply_holder(reply_id: Uuid) -> String {
    format!("community:reply:{reply_id}")
}

/// The images of a post: what is stored, and the uploads to hold for it.
#[derive(Debug, Default)]
pub(crate) struct PostImages {
    pub images: Vec<Image>,
    pub asset_ids: Vec<Uuid>,
}

/// The slow-mode slot a post took. Give it back with
/// [`Community::release_slot`] when the post was not stored after all.
#[derive(Debug, Default)]
pub(crate) struct Slot(Option<String>);

/// Whether the caller's role exempts them from a channel's safety tools.
/// It never does in an anonymous channel.
pub(crate) fn staff_exempt(access: &Access, anonymous: bool) -> bool {
    access.is_staff() && !anonymous
}

/// Whether a new post is held for review: `(state, hidden_reason)`.
pub(crate) fn screening(community: &CommunityRow, access: &Access, anonymous: bool, texts: &[&str]) -> (i32, i32) {
    if staff_exempt(access, anonymous) {
        return (state::VISIBLE, hidden::NONE);
    }
    let held = texts.iter().any(|text| text::find_muted(text, &community.muted_words).is_some());
    if held { (state::HIDDEN, hidden::MUTED_WORD) } else { (state::VISIBLE, hidden::NONE) }
}

impl Community {
    /// Resolves the uploads a post carries: the author's own, finished,
    /// images (or animations), at most four. In an anonymous channel each
    /// must be an upload nothing else uses.
    pub(crate) async fn resolve_images(
        &self,
        author: Uuid,
        asset_ids: &[String],
        anonymous: bool,
    ) -> AppResult<PostImages> {
        let mut ids: Vec<Uuid> = Vec::with_capacity(asset_ids.len());
        for id in parse_ids(asset_ids)? {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ensure!(
            ids.len() <= validate::MAX_IMAGES,
            invalid,
            "TOO_MANY_IMAGES",
            "a post carries at most {} images",
            validate::MAX_IMAGES
        );
        let mut images = Vec::with_capacity(ids.len());
        for id in &ids {
            let asset = assets::require_ready(&self.core, *id, author, &IMAGE_PURPOSES).await?;
            let is_image =
                asset.has_kind(AssetKind::ASSET_KIND_IMAGE) || asset.has_kind(AssetKind::ASSET_KIND_ANIMATION);
            let image = asset.image().filter(|_| is_image);
            let image = image.ok_or_else(|| AppError::invalid("ASSET_NOT_IMAGE", "posts carry images only"))?;
            ensure!(
                !anonymous || !asset.is_attached(),
                precondition,
                "ASSET_ALREADY_USED",
                "in an anonymous channel a picture must be uploaded for the post: the same upload elsewhere would name you"
            );
            images.push(image);
        }
        Ok(PostImages { images, asset_ids: ids })
    }

    /// Resolves the stamp of a post: one the author may send. In an
    /// anonymous channel it must also be one anyone could send — a stamp of
    /// a published pack — because a personal stamp, or one of a pack that
    /// only its maker holds, names the sender.
    pub(crate) async fn resolve_stamp(&self, author: Uuid, stamp_id: &str, anonymous: bool) -> AppResult<Option<Uuid>> {
        let Some(id) = parse_opt_id(stamp_id)? else { return Ok(None) };
        tokuly_stamp::ensure_usable(&self.core, author, id).await?;
        if anonymous {
            let view = tokuly_stamp::stamps::view(&self.core, Some(author), id).await?;
            let anyone_could = view.is_some_and(|view| {
                view.approved_at.is_some()
                    && view.pack().is_some_and(|pack| pack.status == tokuly_stamp::model::status::PUBLISHED)
            });
            ensure!(
                anyone_could,
                precondition,
                "STAMP_NOT_ALLOWED",
                "in an anonymous channel only stamps of published packs can be sent: your own stamps would name you"
            );
        }
        Ok(Some(id))
    }

    /// Records that the author sent a stamp, once the post is stored.
    pub(crate) async fn record_stamp(&self, author: Uuid, stamp_id: Option<Uuid>) {
        if let Some(stamp_id) = stamp_id
            && let Err(error) =
                tokuly_stamp::record_send(&self.core, author, stamp_id, tokuly_stamp::SendContext::Note).await
        {
            tracing::warn!(%error, "recording a stamp sent in a community post failed");
        }
    }

    /// Refuses a post in an anonymous channel while too few members could
    /// have written it for a pseudonym to hide anyone.
    pub(crate) async fn ensure_anonymity(&self, community: &CommunityRow, channel: &ChannelRow) -> AppResult<()> {
        if !channel.anonymous {
            return Ok(());
        }
        let could_post: i64 = if channel.posting == posting::MODERATORS {
            sqlx::query_scalar("SELECT count(*) FROM community_members WHERE community_id = $1 AND role >= 2")
                .bind(community.id)
                .fetch_one(&self.core.db)
                .await?
        } else {
            i64::from(community.member_count)
        };
        ensure!(
            could_post >= i64::from(self.cfg.anonymous_min_members),
            precondition,
            "TOO_FEW_MEMBERS",
            "an anonymous channel takes posts once {} members can post in it",
            self.cfg.anonymous_min_members
        );
        Ok(())
    }

    /// The members of the community `@mentioned` in `body`, the author aside.
    /// Only members can be mentioned: nobody is pinged into a community they
    /// are not part of.
    pub(crate) async fn resolve_mentions(
        &self,
        community_id: Uuid,
        author: Uuid,
        body: &str,
    ) -> AppResult<Vec<pb::Mention>> {
        let handles = text::mentioned_handles(body);
        if handles.is_empty() {
            return Ok(Vec::new());
        }
        let found: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT u.id, u.handle::text FROM users u
             JOIN community_members m ON m.user_id = u.id AND m.community_id = $1
             WHERE u.handle = ANY($2::citext[]) AND u.deleted_at IS NULL AND u.banned_at IS NULL AND u.id <> $3",
        )
        .bind(community_id)
        .bind(&handles)
        .bind(author)
        .fetch_all(&self.core.db)
        .await?;
        Ok(handles
            .iter()
            .filter_map(|handle| {
                found.iter().find(|(_, found)| found.eq_ignore_ascii_case(handle)).map(|(id, _)| pb::Mention {
                    handle: handle.clone(),
                    user_id: id.to_string(),
                    ..Default::default()
                })
            })
            .collect())
    }

    /// Who is told about being mentioned: mentioned members whose level
    /// lets mentions through, minus `skip` — and minus anyone in a block
    /// with the author, unless the post is anonymous (the absence of a
    /// notification must not tell who wrote it). What one author may send
    /// one member, and everyone, per hour is capped.
    pub(crate) async fn mention_recipients(
        &self,
        community_id: Uuid,
        mentions: &[pb::Mention],
        author: Uuid,
        anonymous: bool,
        skip: &[Uuid],
    ) -> AppResult<Vec<Uuid>> {
        let ids: Vec<Uuid> = mentions
            .iter()
            .filter_map(|mention| Uuid::parse_str(&mention.user_id).ok())
            .filter(|id| *id != author && !skip.contains(id))
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut recipients: Vec<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM community_members
             WHERE community_id = $1 AND user_id = ANY($2) AND notification_level IN ($3, $4)",
        )
        .bind(community_id)
        .bind(&ids)
        .bind(level::ALL)
        .bind(level::MENTIONS)
        .fetch_all(&self.core.db)
        .await?;
        if !anonymous {
            let blocked = graph::blocked_ids_either(&self.core, author).await?;
            recipients.retain(|id| !blocked.contains(id));
        }
        let hour = Duration::from_secs(3600);
        let mut allowed = Vec::with_capacity(recipients.len());
        for recipient in recipients {
            let pair = format!("community:mention:{author}:{recipient}");
            let all = format!("community:mentions:{author}");
            if self.core.cache.rate_limit(&pair, MENTIONS_PER_PAIR_PER_HOUR, hour).await.is_ok()
                && self.core.cache.rate_limit(&all, MENTIONS_PER_AUTHOR_PER_HOUR, hour).await.is_ok()
            {
                allowed.push(recipient);
            }
        }
        Ok(allowed)
    }

    /// The limits in front of a post: the per-minute and per-hour
    /// allowance and the channel's slow mode. The staff is exempt from slow
    /// mode, except in an anonymous channel.
    pub(crate) async fn admit_post(&self, access: &Access, channel: &ChannelRow) -> AppResult<Slot> {
        let member = access.require_member()?;
        let user_id = member.user_id;
        self.limit("post", user_id, self.cfg.posts_per_minute, Duration::from_secs(60)).await?;
        self.limit("post-hour", user_id, self.cfg.posts_per_hour, Duration::from_secs(3600)).await?;
        if channel.slow_mode_seconds <= 0 || staff_exempt(access, channel.anonymous) {
            return Ok(Slot(None));
        }
        // Slow mode is a rule of the channel, not an abuse limit: it holds
        // even where request limits are switched off.
        let key = format!("community:slow:{}:{}", channel.id, user_id);
        let wait = Duration::from_secs(channel.slow_mode_seconds as u64);
        if !self.core.cache.set_nx(&key, "1", wait).await? {
            return Err(AppError::rate_limited(
                "SLOW_MODE",
                format!("slow mode: one post every {} seconds in this channel", channel.slow_mode_seconds),
            ));
        }
        Ok(Slot(Some(key)))
    }

    /// Gives a slow-mode slot back: the post it was taken for failed.
    pub(crate) async fn release_slot(&self, slot: Slot) {
        if let Some(key) = slot.0 {
            let _ = self.core.cache.del(&key).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holders_name_the_post() {
        let id = Uuid::nil();
        assert_eq!(thread_holder(id), format!("community:thread:{id}"));
        assert_eq!(reply_holder(id), format!("community:reply:{id}"));
    }
}
