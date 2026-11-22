//! Senders: the rules every anonymous submission passes through, and
//! "block whoever sent this".
//!
//! # What must hold
//!
//! 1. **A blocked sender cannot tell.** Whatever a blocked sender submits is
//!    accepted exactly like anything else: the same limits are counted, the
//!    same row and the same delivery event are written, the same response
//!    is returned. The only difference is the `dropped` flag on the row,
//!    which makes the delivery worker and every recipient-side query skip
//!    it. Limits are counted before the block is even looked up, and they
//!    count attempts, so a second account cannot measure whether the first
//!    one's questions "arrived".
//! 2. **The recipient learns nothing by blocking.** A block is made from an
//!    item ("whoever sent this"); the sender's ID is copied inside the
//!    database and never passes through a response. Blocks are stored per
//!    item, so the list of blocks does not show that two items came from
//!    one person, and a block never disappears on its own (not even when
//!    the sender deletes their account).
//! 3. **Existing items stay.** Blocking does not remove what the sender sent
//!    before: if it did, the recipient could tell which other anonymous
//!    items were theirs.
//! 4. **What happens to the sender's account shows on nothing anonymous.**
//!    If a suspension hid everything an account had sent, two anonymous
//!    questions vanishing together with someone's profile would name their
//!    sender. So a suspension, a deletion or a change of settings changes
//!    nothing a recipient sees of anonymous items. Only signed items, whose
//!    sender is known anyway, are hidden while the sender is suspended
//!    ([`signed_by_suspended`]); the one thing a suspension stops is the
//!    delivery of what has not been delivered yet (`delivery.rs`). A single
//!    item is removed by a takedown.
//! 5. **An anonymous item needs a crowd.** Where only the recipient's
//!    friends can send (a friends-only box, a prompt, a compliment poll), a
//!    recipient with one friend knows who it was. Such a recipient needs
//!    [`MIN_ANONYMITY_SET`] friends before anything anonymous is accepted
//!    for them. Their number of friends is on their profile, so being
//!    refused tells the sender nothing new.
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokuly_core::prelude::*;
use tokuly_media::assets;
use tokuly_proto::{
    ask::{BlockSource, SenderBlock},
    common::PageInfo,
    media::AssetKind,
};
use tokuly_social::graph;

use crate::{
    Ask,
    answers::IMAGE_PURPOSES,
    filter::{self, WordFilter},
};

/// Friends a recipient needs before friends can send them something
/// anonymously.
pub(crate) const MIN_ANONYMITY_SET: usize = 3;

/// SQL condition for "a signed item whose sender is suspended", for a table
/// aliased `$alias`. The only way the state of a sender's account shows in
/// what a recipient sees, and it never applies to an anonymous item.
macro_rules! signed_by_suspended {
    ($alias:literal) => {
        concat!(
            "(",
            $alias,
            ".signed AND EXISTS (SELECT 1 FROM users suspended WHERE suspended.id = ",
            $alias,
            ".sender_id AND suspended.banned_at IS NOT NULL))"
        )
    };
}
pub(crate) use signed_by_suspended;

/// SQL condition for "this sender's account is suspended", for a table
/// aliased `$alias`. Used where an item is about to be delivered, and for
/// what moderators are shown; never for what a recipient sees.
macro_rules! sender_suspended {
    ($alias:literal) => {
        concat!(
            "EXISTS (SELECT 1 FROM users suspended WHERE suspended.id = ",
            $alias,
            ".sender_id AND suspended.banned_at IS NOT NULL)"
        )
    };
}
pub(crate) use sender_suspended;

/// What becomes of something a sender submits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Intake {
    /// The recipient blocked this sender: store it, show it to nobody.
    pub dropped: bool,
    /// A muted word matched: store it in the FILTERED folder, tell nobody.
    pub held: bool,
}

const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(86_400);

impl Ask {
    /// Counts one submission from `sender` to `recipient` against the three
    /// limits: per sender, per pair, and per recipient (many accounts
    /// flooding one person). Blocked senders are counted like everyone.
    pub(crate) async fn limit_sends(&self, sender: Uuid, recipient: Uuid) -> AppResult<()> {
        let cache = &self.core.cache;
        cache.rate_limit(&format!("ask:send:{sender}"), self.cfg.sends_per_hour, HOUR).await?;
        cache.rate_limit(&format!("ask:pair:{sender}:{recipient}"), self.cfg.pair_sends_per_day, DAY).await?;
        match cache.rate_limit(&format!("ask:box:{recipient}"), self.cfg.box_receives_per_hour, HOUR).await {
            Err(e) if e.reason == "RATE_LIMITED" => {
                Err(AppError::rate_limited("BOX_BUSY", "this person is receiving a lot right now; try again later"))
            }
            other => other,
        }
    }

    /// Decides what becomes of `text` sent by `sender` to `recipient`, whose
    /// own muted words are `muted_words`.
    pub(crate) async fn intake(
        &self,
        sender: Uuid,
        recipient: Uuid,
        text: &str,
        muted_words: &[String],
    ) -> AppResult<Intake> {
        let dropped: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ask_sender_blocks WHERE owner_id = $1 AND sender_id = $2)")
                .bind(recipient)
                .bind(sender)
                .fetch_one(&self.core.db)
                .await?;
        let prepared = filter::prepare(text);
        let held = self.filter.matches(&prepared) || WordFilter::new(muted_words).matches(&prepared);
        Ok(Intake { dropped, held })
    }

    /// Whether something `recipient`'s friends send anonymously has a crowd
    /// to hide in: the recipient has enough friends.
    pub(crate) async fn has_anonymity_set(&self, recipient: Uuid) -> AppResult<bool> {
        Ok(graph::friend_ids(&self.core, recipient).await?.len() >= MIN_ANONYMITY_SET)
    }

    /// The image `sender` attaches to a question or a prompt response:
    /// its asset ID and its `Image` as stored JSON. `allowed` is whether the
    /// recipient takes images.
    ///
    /// The image must be a fresh upload, signed or not. The file itself
    /// says nothing about its uploader (it is re-encoded without metadata,
    /// its address is made of the asset's ID and a hash of the pixels, and
    /// nobody but its owner is told who uploaded an asset). What would give
    /// an anonymous sender away is the same upload turning up a second
    /// time: on something they signed, or on a second anonymous item.
    pub(crate) async fn sender_image(
        &self,
        sender: Uuid,
        asset_id: &str,
        allowed: bool,
    ) -> AppResult<Option<(Uuid, serde_json::Value)>> {
        let Some(asset_id) = parse_opt_id(asset_id)? else { return Ok(None) };
        ensure!(allowed, precondition, "IMAGES_NOT_ALLOWED", "images are not accepted here");
        let asset = assets::require_ready(&self.core, asset_id, sender, IMAGE_PURPOSES).await?;
        ensure!(
            asset.has_kind(AssetKind::ASSET_KIND_IMAGE) || asset.has_kind(AssetKind::ASSET_KIND_ANIMATION),
            invalid,
            "ASSET_NOT_IMAGE",
            "an image is required"
        );
        ensure!(!asset.is_attached(), precondition, "IMAGE_ALREADY_USED", "an image sent here has to be a new upload");
        let image = asset.image.clone().or_not_found("ASSET_NOT_READY", "the image has not been processed")?;
        Ok(Some((asset_id, image)))
    }

    /// Whether `receiver` blocked `sender` (compliment votes).
    pub(crate) async fn is_sender_blocked<'e, E: sqlx::PgExecutor<'e>>(
        executor: E,
        receiver: Uuid,
        sender: Uuid,
    ) -> AppResult<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ask_sender_blocks WHERE owner_id = $1 AND sender_id = $2)")
            .bind(receiver)
            .bind(sender)
            .fetch_one(executor)
            .await?)
    }
}

// ───────────────────────────── blocks ─────────────────────────────

/// The item a block is made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockTarget {
    Question(Uuid),
    PromptResponse(Uuid),
    Flame(Uuid),
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct BlockRow {
    pub id: Uuid,
    pub source: i16,
    pub preview: String,
    pub created_at: DateTime<Utc>,
}

impl BlockRow {
    pub(crate) fn to_proto(&self) -> SenderBlock {
        SenderBlock {
            id: self.id.to_string(),
            source: i32::from(self.source).into(),
            preview: self.preview.clone(),
            created_at: ts(self.created_at),
            ..Default::default()
        }
    }
}

const QUESTION: i16 = BlockSource::BLOCK_SOURCE_QUESTION as i16;
const PROMPT_RESPONSE: i16 = BlockSource::BLOCK_SOURCE_PROMPT_RESPONSE as i16;
const FLAME: i16 = BlockSource::BLOCK_SOURCE_FLAME as i16;

impl Ask {
    /// Blocks whoever sent `target` to `owner`. The sender's ID goes from
    /// the item's row into the block's row without leaving the database.
    ///
    /// Anything that reached the owner can be blocked, whatever became of it
    /// since: also after they deleted it, and also after its sender took it
    /// back (otherwise sending abuse and withdrawing it would be a way to
    /// stay unblockable). Items that were dropped or not delivered cannot:
    /// the owner never had them, and a block that "worked" on a guessed ID
    /// would confirm that the item exists.
    pub(crate) async fn block_sender_of(&self, owner: Uuid, target: BlockTarget) -> AppResult<BlockRow> {
        // `ON CONFLICT DO UPDATE` with a no-op makes a repeated block return
        // the existing row.
        let (sql, id) = match target {
            BlockTarget::Question(id) => (
                "INSERT INTO ask_sender_blocks (owner_id, sender_id, source, source_id, preview)
                 SELECT q.owner_id, q.sender_id, $3, q.id, left(q.text, $4)
                 FROM ask_questions q
                 WHERE q.id = $1 AND q.owner_id = $2 AND q.delivered_at IS NOT NULL AND NOT q.dropped
                 ON CONFLICT (owner_id, source, source_id) DO UPDATE SET source = EXCLUDED.source
                 RETURNING id, source, preview, created_at",
                id,
            ),
            BlockTarget::PromptResponse(id) => (
                "INSERT INTO ask_sender_blocks (owner_id, sender_id, source, source_id, preview)
                 SELECT r.owner_id, r.sender_id, $3, r.id, left(r.text, $4)
                 FROM ask_prompt_responses r
                 WHERE r.id = $1 AND r.owner_id = $2 AND r.delivered_at IS NOT NULL AND NOT r.dropped
                 ON CONFLICT (owner_id, source, source_id) DO UPDATE SET source = EXCLUDED.source
                 RETURNING id, source, preview, created_at",
                id,
            ),
            BlockTarget::Flame(id) => (
                "INSERT INTO ask_sender_blocks (owner_id, sender_id, source, source_id, preview)
                 SELECT f.receiver_id, f.sender_id, $3, f.id, left(p.text_ja, $4)
                 FROM ask_flames f JOIN ask_poll_questions p ON p.id = f.question_id
                 WHERE f.id = $1 AND f.receiver_id = $2 AND f.delivered_at IS NOT NULL AND NOT f.dropped
                 ON CONFLICT (owner_id, source, source_id) DO UPDATE SET source = EXCLUDED.source
                 RETURNING id, source, preview, created_at",
                id,
            ),
        };
        let source = match target {
            BlockTarget::Question(_) => QUESTION,
            BlockTarget::PromptResponse(_) => PROMPT_RESPONSE,
            BlockTarget::Flame(_) => FLAME,
        };
        let block: Option<BlockRow> = sqlx::query_as(sql)
            .bind(id)
            .bind(owner)
            .bind(source)
            .bind(crate::text::PREVIEW_CHARS as i32)
            .fetch_optional(&self.core.db)
            .await?;
        block.or_not_found("BLOCK_TARGET_NOT_FOUND", "there is nothing of that kind sent to you")
    }

    /// `owner`'s sender blocks, newest first.
    pub(crate) async fn sender_blocks_page(&self, owner: Uuid, page: &Page) -> AppResult<(Vec<BlockRow>, PageInfo)> {
        let cursor: Option<Uuid> = page.decode()?;
        let mut rows: Vec<BlockRow> = sqlx::query_as(
            "SELECT id, source, preview, created_at FROM ask_sender_blocks
             WHERE owner_id = $1 AND ($2::uuid IS NULL OR id < $2)
             ORDER BY id DESC LIMIT $3",
        )
        .bind(owner)
        .bind(cursor)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        Ok((rows, info))
    }

    /// Lifts one of `owner`'s blocks.
    pub(crate) async fn lift_block(&self, owner: Uuid, block_id: Uuid) -> AppResult<()> {
        let lifted = sqlx::query("DELETE FROM ask_sender_blocks WHERE id = $1 AND owner_id = $2")
            .bind(block_id)
            .bind(owner)
            .execute(&self.core.db)
            .await?;
        ensure!(lifted.rows_affected() == 1, not_found, "BLOCK_NOT_FOUND", "block not found");
        Ok(())
    }
}
