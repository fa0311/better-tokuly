//! Notices other domains post into conversations (`Chat::post_notice`): no
//! sender, nothing unread, no notification, not reportable, once per key,
//! and a bounded number per conversation.
mod common;

use common::*;
use futures::future::join_all;
use tokuly_chat::{NOTICES_PER_CONVERSATION_PER_DAY, direct_conversations_of};
use tokuly_core::outbound::Localized;
use tokuly_proto::{chat::*, common::SubjectType};
use uuid::Uuid;

fn new_owner() -> Localized {
    Localized::new("このボットの持ち主が変わったよ", "This bot has a new owner")
}

fn notices(messages: &[Message]) -> Vec<&Message> {
    messages
        .iter()
        .filter(|message| message.system.r#type.as_known() == Some(SystemEventType::SYSTEM_EVENT_TYPE_NOTICE))
        .collect()
}

#[tokio::test]
async fn a_notice_is_shown_to_the_members_and_nothing_else() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    ctx.text(&taro, &direct.id, "おはよ").await;
    ctx.mark_read(&hanako, &direct.id).await;
    let notified_before = ctx.notifications(&hanako).await.len();
    let mut pushes = ctx.listen(&hanako).await;

    let conversation_id: Uuid = direct.id.parse().unwrap();
    assert!(ctx.chat.post_notice(conversation_id, "test:owner", new_owner()).await.unwrap());

    let latest = ctx.latest(&hanako, &direct.id).await;
    let shown = notices(&latest);
    assert_eq!(shown.len(), 1);
    let notice = shown[0];
    assert_eq!(notice.kind, MessageKind::MESSAGE_KIND_SYSTEM);
    assert!(notice.sender.as_option().is_none(), "nobody sent it");
    let text = notice.system.notice.as_option().expect("the notice's wording");
    assert_eq!((text.ja.as_str(), text.en.as_str()), ("このボットの持ち主が変わったよ", "This bot has a new owner"));
    assert!(notice.system.actor.as_option().is_none() && notice.system.targets.is_empty());
    // Pushed like any message, so open threads show it at once.
    assert!(created_messages(&ctx.events(&mut pushes).await).iter().any(|message| message.id == notice.id));
    // Nothing to read, nothing to be told about.
    assert_eq!(ctx.unread(&hanako).await.badge, 0);
    assert_eq!(ctx.unread(&taro).await.badge, 0);
    assert_eq!(ctx.notifications(&hanako).await.len(), notified_before);
    // Nobody is behind it, so there is nobody to report.
    let subjects = &ctx.app.core.subjects;
    let reported =
        subjects.resolve_for_report(SubjectType::SUBJECT_TYPE_CHAT_MESSAGE, &notice.id, Some(hanako.id)).await;
    assert_eq!(reported.unwrap_err().reason, "SUBJECT_NOT_FOUND");
    // Nor anything to react to.
    let react = AddReactionRequest { message_id: notice.id.clone(), emoji: "👍".into(), ..Default::default() };
    assert!(ctx.call::<_, AddReactionResponse>("Message", "AddReaction", &hanako, &react).await.is_err());

    // The same key again (an event delivered twice) posts nothing.
    assert!(!ctx.chat.post_notice(conversation_id, "test:owner", new_owner()).await.unwrap());
    assert_eq!(notices(&ctx.latest(&taro, &direct.id).await).len(), 1);
}

#[tokio::test]
async fn the_same_notice_posted_at_once_is_stored_once() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let conversation_id: Uuid = direct.id.parse().unwrap();
    let chat = &ctx.chat;
    let posted = join_all((0..5).map(|_| chat.post_notice(conversation_id, "test:race", new_owner()))).await;
    assert_eq!(posted.into_iter().map(|posted| posted.unwrap()).filter(|posted| *posted).count(), 1);
    assert_eq!(notices(&ctx.latest(&taro, &direct.id).await).len(), 1);
    // The copies that found it there used up nothing of the day's share.
    for i in 1..NOTICES_PER_CONVERSATION_PER_DAY {
        assert!(chat.post_notice(conversation_id, &format!("test:after-{i}"), new_owner()).await.unwrap());
    }
}

#[tokio::test]
async fn a_conversation_takes_a_few_notices_a_day() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let direct = ctx.direct(&taro, &hanako).await;
    let conversation_id: Uuid = direct.id.parse().unwrap();
    for i in 0..NOTICES_PER_CONVERSATION_PER_DAY {
        assert!(ctx.chat.post_notice(conversation_id, &format!("test:{i}"), new_owner()).await.unwrap());
    }
    assert!(!ctx.chat.post_notice(conversation_id, "test:one-more", new_owner()).await.unwrap());
    assert_eq!(notices(&ctx.latest(&taro, &direct.id).await).len(), NOTICES_PER_CONVERSATION_PER_DAY as usize);
    // Other conversations have their own share.
    let jiro = ctx.app.user("jiro").await;
    ctx.befriend(&taro, &jiro).await;
    let other: Uuid = ctx.direct(&taro, &jiro).await.id.parse().unwrap();
    assert!(ctx.chat.post_notice(other, "test:one-more", new_owner()).await.unwrap());
}

#[tokio::test]
async fn notices_are_checked_and_need_a_conversation() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let conversation_id: Uuid = ctx.direct(&taro, &hanako).await.id.parse().unwrap();
    let chat = &ctx.chat;
    assert_eq!(chat.post_notice(conversation_id, "", new_owner()).await.unwrap_err().reason, "NOTICE_KEY_INVALID");
    assert_eq!(
        chat.post_notice(conversation_id, "k", Localized::same(" ")).await.unwrap_err().reason,
        "NOTICE_INVALID"
    );
    let long = Localized::same("あ".repeat(301));
    assert_eq!(chat.post_notice(conversation_id, "k", long).await.unwrap_err().reason, "NOTICE_INVALID");
    assert_eq!(chat.post_notice(Uuid::now_v7(), "k", new_owner()).await.unwrap_err().reason, "CONVERSATION_NOT_FOUND");
}

#[tokio::test]
async fn the_direct_conversations_of_a_user_leave_groups_out() {
    let ctx = start().await;
    let (taro, hanako) = ctx.friends("taro", "hanako").await;
    let jiro = ctx.app.user("jiro").await;
    ctx.befriend(&taro, &jiro).await;
    let mut expected = vec![ctx.direct(&taro, &hanako).await.id, ctx.direct(&jiro, &taro).await.id];
    ctx.group(&taro, "部活", &[&hanako, &jiro]).await;
    let mut found: Vec<String> =
        direct_conversations_of(&ctx.app.core, taro.id).await.unwrap().iter().map(Uuid::to_string).collect();
    expected.sort();
    found.sort();
    assert_eq!(found, expected);
    assert!(direct_conversations_of(&ctx.app.core, Uuid::now_v7()).await.unwrap().is_empty());
}
