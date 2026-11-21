//! Notifications about comments: `engagement.comment` to the subject owner,
//! `engagement.reply` to the people answered, `engagement.mention` to the
//! people mentioned, `engagement.heart` to the author of a hearted comment.
//!
//! People are told when a comment enters public view — for a held comment
//! that is its approval, not its writing — and once per comment
//! (`comments.notified_at`). Nobody hears about their own comment, about a
//! comment of someone they are in a block with, or as an owner of content
//! that belongs to nobody.
use sqlx::PgConnection;
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};

use super::CommentRow;
use crate::{
    Engagement,
    subjects::{Subject, comment_path, is_absent},
    text,
};

pub(crate) const KIND_COMMENT: &str = "engagement.comment";
pub(crate) const KIND_REPLY: &str = "engagement.reply";
pub(crate) const KIND_MENTION: &str = "engagement.mention";
pub(crate) const KIND_HEART: &str = "engagement.heart";

/// Why someone is told about a comment. One person, one notification: the
/// first reason that applies, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    /// It answers their comment.
    Reply,
    /// It mentions them.
    Mention,
    /// It is a new top-level comment on their subject.
    Comment,
}

/// The people a comment concerns, before any of them is ruled out.
#[derive(Debug, Clone, Default)]
pub(crate) struct Concerned {
    /// The author of the top-level comment a reply was added to.
    pub thread_author: Option<Uuid>,
    /// The author of the reply it answers, if it answers a reply.
    pub replied_author: Option<Uuid>,
    pub mentioned: Vec<Uuid>,
    /// The subject owner, for a top-level comment.
    pub subject_owner: Option<Uuid>,
}

/// Who is told about a comment by `author_id`, and why. `excluded` are the
/// people in a block with the author.
pub(crate) fn audience(author_id: Uuid, concerned: &Concerned, excluded: &[Uuid]) -> Vec<(Uuid, Reason)> {
    let candidates =
        (concerned.thread_author.into_iter().chain(concerned.replied_author).map(|id| (id, Reason::Reply)))
            .chain(concerned.mentioned.iter().map(|id| (*id, Reason::Mention)))
            .chain(concerned.subject_owner.map(|id| (id, Reason::Comment)));
    let mut audience: Vec<(Uuid, Reason)> = Vec::new();
    for (user_id, reason) in candidates {
        let told = user_id != author_id
            && !user_id.is_nil()
            && !excluded.contains(&user_id)
            && !audience.iter().any(|(seen, _)| *seen == user_id);
        if told {
            audience.push((user_id, reason));
        }
    }
    audience
}

/// Who a notification about a comment says it is from.
#[derive(Debug, Clone)]
pub(crate) struct Sender {
    /// `None` for a comment written as a channel: the account behind a
    /// channel is not revealed, so the notification carries no actor.
    pub actor_id: Option<Uuid>,
    pub name: String,
    /// The channel's icon, for a comment written as a channel.
    pub image_url: Option<String>,
}

/// What a notification shows of the comment itself.
fn body_of(row: &CommentRow) -> Localized {
    let excerpt = text::excerpt(&row.text, 100);
    if !excerpt.is_empty() {
        Localized::same(excerpt)
    } else if row.stamp_id.is_some() {
        Localized::new("スタンプを送ったよ", "Sent a stamp")
    } else {
        Localized::new("画像を送ったよ", "Sent an image")
    }
}

fn title_of(reason: Reason, sender: &str, subject_title: &str) -> Localized {
    let subject_title = text::excerpt(subject_title, 30);
    match reason {
        Reason::Reply => Localized::new(format!("{sender}が返信したよ!"), format!("{sender} replied to your comment")),
        Reason::Mention => Localized::new(
            format!("{sender}がコメントであなたをメンションしたよ!"),
            format!("{sender} mentioned you in a comment"),
        ),
        Reason::Comment if subject_title.is_empty() => {
            Localized::new(format!("{sender}がコメントしたよ!"), format!("{sender} commented"))
        }
        Reason::Comment => Localized::new(
            format!("{sender}が「{subject_title}」にコメントしたよ!"),
            format!("{sender} commented on \"{subject_title}\""),
        ),
    }
}

fn kind_of(reason: Reason) -> &'static str {
    match reason {
        Reason::Reply => KIND_REPLY,
        Reason::Mention => KIND_MENTION,
        Reason::Comment => KIND_COMMENT,
    }
}

fn about(request: NotificationRequest, row: &CommentRow, subject: &Subject) -> NotificationRequest {
    request
        .subject(subject.key.subject_type(), subject.key.id)
        .data(serde_json::json!({ "comment_id": row.id, "parent_id": row.parent_id }))
}

impl Engagement {
    /// How the author of a comment appears in notifications: as the user,
    /// or — for a comment written as a channel — as the channel alone.
    pub(crate) async fn sender_of(&self, author_id: Uuid, channel_id: Option<Uuid>) -> AppResult<Sender> {
        match channel_id {
            None => {
                let user = tokuly_identity::summary(&self.core, author_id).await?;
                let name = if user.display_name.is_empty() { format!("@{}", user.handle) } else { user.display_name };
                Ok(Sender { actor_id: Some(author_id), name, image_url: None })
            }
            Some(channel_id) => {
                let channel = self.channel_summaries(&[channel_id]).await?.remove(&channel_id).unwrap_or_default();
                let name = if channel.name.is_empty() { "チャンネル".to_owned() } else { channel.name };
                let image_url = channel.icon.as_option().map(|icon| icon.url.clone()).filter(|url| !url.is_empty());
                Ok(Sender { actor_id: None, name, image_url })
            }
        }
    }

    /// The people an existing comment concerns, from what is stored: used
    /// when a held comment is approved.
    pub(crate) async fn concerned_by(
        &self,
        row: &CommentRow,
        parent: Option<&CommentRow>,
        subject: &Subject,
    ) -> AppResult<Concerned> {
        let replied_author: Option<Uuid> = match row.reply_to_id {
            Some(reply_to_id) => {
                sqlx::query_scalar("SELECT author_id FROM comments WHERE id = $1")
                    .bind(reply_to_id)
                    .fetch_optional(&self.core.db)
                    .await?
            }
            None => None,
        };
        let mentioned: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM comment_mentions WHERE comment_id = $1")
            .bind(row.id)
            .fetch_all(&self.core.db)
            .await?;
        Ok(Concerned {
            thread_author: parent.map(|parent| parent.author_id),
            replied_author,
            mentioned,
            subject_owner: subject.owner().filter(|_| !row.is_reply()),
        })
    }

    /// Who is to be told about a comment of `author_id` on `subject`.
    /// People mentioned in a comment on something they cannot see are left
    /// out.
    pub(crate) async fn comment_audience(
        &self,
        author_id: Uuid,
        concerned: &Concerned,
        subject: &Subject,
    ) -> AppResult<Vec<(Uuid, Reason)>> {
        let excluded = self.hidden_from(Some(author_id)).await?;
        let mut told = Vec::new();
        for (user_id, reason) in audience(author_id, concerned, &excluded) {
            if reason == Reason::Mention {
                match self.subject(subject.key.subject_type(), &subject.key.id.to_string(), Some(user_id)).await {
                    Ok(_) => {}
                    Err(error) if is_absent(&error) => continue,
                    Err(error) => return Err(error),
                }
            }
            told.push((user_id, reason));
        }
        Ok(told)
    }

    /// Sends the notifications of a comment that entered public view and
    /// marks it as announced.
    pub(crate) async fn announce_published(
        &self,
        conn: &mut PgConnection,
        row: &CommentRow,
        subject: &Subject,
        audience: &[(Uuid, Reason)],
        sender: &Sender,
    ) -> AppResult<()> {
        self.announce(conn, row, subject, audience, sender).await?;
        sqlx::query("UPDATE comments SET notified_at = now() WHERE id = $1").bind(row.id).execute(conn).await?;
        Ok(())
    }

    /// Sends one notification per member of `audience`. Runs on the
    /// caller's transaction and touches nothing else.
    pub(crate) async fn announce(
        &self,
        conn: &mut PgConnection,
        row: &CommentRow,
        subject: &Subject,
        audience: &[(Uuid, Reason)],
        sender: &Sender,
    ) -> AppResult<()> {
        let path = comment_path(&subject.info.path, row.id);
        for (user_id, reason) in audience {
            let mut request = about(
                NotificationRequest::new(
                    *user_id,
                    kind_of(*reason),
                    title_of(*reason, &sender.name, &subject.info.title),
                    body_of(row),
                    &path,
                ),
                row,
                subject,
            );
            if let Some(actor_id) = sender.actor_id {
                request = request.actor(actor_id);
            }
            if let Some(image_url) = &sender.image_url {
                request = request.image(image_url);
            }
            outbound::notify(&mut *conn, &request).await?;
        }
        Ok(())
    }

    /// Tells the subject owner that a comment waits for their review. One
    /// entry per subject, however many comments wait.
    pub(crate) async fn announce_held(
        &self,
        conn: &mut PgConnection,
        row: &CommentRow,
        subject: &Subject,
    ) -> AppResult<()> {
        let Some(owner_id) = subject.owner().filter(|owner_id| *owner_id != row.author_id) else { return Ok(()) };
        let request = about(
            NotificationRequest::new(
                owner_id,
                KIND_COMMENT,
                Localized::new("承認待ちのコメントがあるよ!", "A comment is waiting for your review"),
                body_of(row),
                comment_path(&subject.info.path, row.id),
            ),
            row,
            subject,
        )
        .collapse(format!("engagement.review:{}:{}", subject.key.kind, subject.key.id));
        outbound::notify(conn, &request).await
    }

    /// Tells the author that the creator gave their comment a heart. The
    /// notification names no actor: the owner of a channel's content is not
    /// necessarily known to its audience as a person.
    pub(crate) async fn announce_heart(
        &self,
        conn: &mut PgConnection,
        row: &CommentRow,
        subject: &Subject,
    ) -> AppResult<()> {
        if subject.is_owned_by(row.author_id) {
            return Ok(());
        }
        let mut request = about(
            NotificationRequest::new(
                row.author_id,
                KIND_HEART,
                Localized::new("コメントに❤️がついたよ!", "Your comment got a heart"),
                body_of(row),
                comment_path(&subject.info.path, row.id),
            ),
            row,
            subject,
        )
        .collapse(format!("engagement.heart:{}", row.id));
        if let Some(image_url) = &subject.info.image_url {
            request = request.image(image_url);
        }
        outbound::notify(conn, &request).await
    }
}

#[cfg(test)]
mod tests {
    use super::{super::testing::comment, *};

    #[test]
    fn a_reply_tells_the_people_it_answers_before_those_it_mentions() {
        let (author, thread, replied, mentioned, owner) =
            (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let concerned = Concerned {
            thread_author: Some(thread),
            replied_author: Some(replied),
            mentioned: vec![thread, mentioned, author],
            subject_owner: None,
        };
        assert_eq!(
            audience(author, &concerned, &[]),
            [(thread, Reason::Reply), (replied, Reason::Reply), (mentioned, Reason::Mention)]
        );
        // The owner of the subject hears of top-level comments only, and once.
        let top_level = Concerned { mentioned: vec![owner], subject_owner: Some(owner), ..Default::default() };
        assert_eq!(audience(author, &top_level, &[]), [(owner, Reason::Mention)]);
    }

    #[test]
    fn nobody_hears_of_their_own_comment_or_across_a_block() {
        let (author, blocked, owner) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let concerned = Concerned {
            thread_author: Some(author),
            replied_author: Some(blocked),
            mentioned: vec![blocked],
            subject_owner: Some(owner),
        };
        assert_eq!(audience(author, &concerned, &[blocked]), [(owner, Reason::Comment)]);
        // Content that belongs to nobody has nobody to tell.
        let ownerless = Concerned { subject_owner: Some(Uuid::nil()), ..Default::default() };
        assert!(audience(author, &ownerless, &[]).is_empty());
        let own = Concerned { subject_owner: Some(author), ..Default::default() };
        assert!(audience(author, &own, &[]).is_empty());
    }

    #[test]
    fn bodies_describe_comments_without_text() {
        let author = Uuid::now_v7();
        assert_eq!(body_of(&comment(author)).ja, "こんにちは");
        let stamp = CommentRow { text: String::new(), stamp_id: Some(Uuid::now_v7()), ..comment(author) };
        assert_eq!((body_of(&stamp).ja.as_str(), body_of(&stamp).en.as_str()), ("スタンプを送ったよ", "Sent a stamp"));
        let image = CommentRow { text: String::new(), image_asset_id: Some(Uuid::now_v7()), ..comment(author) };
        assert_eq!(body_of(&image).en, "Sent an image");
    }

    #[test]
    fn titles_name_the_sender_and_the_subject() {
        assert_eq!(title_of(Reason::Comment, "アリス", "今日のランチ").ja, "アリスが「今日のランチ」にコメントしたよ!");
        assert_eq!(title_of(Reason::Comment, "Alice", "").en, "Alice commented");
        assert_eq!(title_of(Reason::Reply, "Alice", "x").en, "Alice replied to your comment");
        assert_eq!(title_of(Reason::Mention, "アリス", "x").ja, "アリスがコメントであなたをメンションしたよ!");
        assert_eq!(
            (kind_of(Reason::Reply), kind_of(Reason::Mention), kind_of(Reason::Comment)),
            (KIND_REPLY, KIND_MENTION, KIND_COMMENT)
        );
    }
}
