//! `QuestionBoxService`: settings, asking, the inbox, answers and the Q&A page.
mod common;

use buffa::EnumValue;
use common::*;
use tokuly_media::{assets, testing::ready_image};
use tokuly_proto::{
    ask::{ask_event::Event as Push, *},
    common::PageRequest,
    media::AssetPurpose,
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

fn by_id(id: &str) -> GetQuestionRequest {
    GetQuestionRequest { question_id: id.to_owned(), ..Default::default() }
}

fn texts(questions: &[Question]) -> Vec<&str> {
    questions.iter().map(|q| q.text.as_str()).collect()
}

async fn get_box(ctx: &Ctx, viewer: Option<&TestUser>, user: &str) -> Result<QuestionBox, tokuly_testkit::RpcError> {
    let response: GetBoxResponse = ctx
        .app
        .call(&format!("{BOX}/GetBox"), viewer, &GetBoxRequest { user: user.to_owned(), ..Default::default() })
        .await?;
    Ok(response.r#box.as_option().cloned().expect("box"))
}

async fn answers_page(ctx: &Ctx, viewer: Option<&TestUser>, owner: &TestUser) -> Vec<Question> {
    let request = ListAnswersRequest { user: owner.id.to_string(), ..Default::default() };
    let response: ListAnswersResponse =
        ctx.app.call(&format!("{BOX}/ListAnswers"), viewer, &request).await.expect("list answers");
    response.questions
}

#[tokio::test]
async fn settings_start_cautious_and_change_one_field_at_a_time() {
    let ctx = start().await;
    let user = ctx.user("Owner").await;
    let defaults: GetSettingsResponse =
        ctx.call(BOX, "GetSettings", &user, &GetSettingsRequest::default()).await.unwrap();
    let defaults = defaults.settings.as_option().unwrap();
    assert!(!defaults.box_open, "a box is closed until its owner opens it");
    assert_eq!(defaults.audience, AskerAudience::ASKER_AUDIENCE_FRIENDS);
    assert!(defaults.allow_signed && !defaults.allow_images && defaults.in_polls && defaults.allow_hints);

    let changed = ctx
        .update_settings(
            &user,
            UpdateSettingsRequest {
                box_open: Some(true),
                box_prompt: Some("  なんでも聞いて!  ".into()),
                muted_words: MutedWords {
                    words: vec!["うざい".into(), " うざい ".into(), "テスト".into()],
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        )
        .await;
    assert!(changed.box_open);
    assert_eq!(changed.box_prompt, "なんでも聞いて!");
    assert_eq!(changed.muted_words, vec!["うざい", "テスト"]);
    assert_eq!(changed.audience, AskerAudience::ASKER_AUDIENCE_FRIENDS, "fields that were not sent keep their value");

    let changed = ctx
        .update_settings(
            &user,
            UpdateSettingsRequest { allow_hints: Some(false), in_polls: Some(false), ..Default::default() },
        )
        .await;
    assert!(!changed.allow_hints && !changed.in_polls && changed.box_open);
    assert_eq!(changed.muted_words.len(), 2);
    let cleared = ctx
        .update_settings(
            &user,
            UpdateSettingsRequest { muted_words: MutedWords::default().into(), ..Default::default() },
        )
        .await;
    assert!(cleared.muted_words.is_empty());

    for (request, reason) in [
        (UpdateSettingsRequest { box_prompt: Some("あ".repeat(81)), ..Default::default() }, "BOX_PROMPT_INVALID"),
        (
            UpdateSettingsRequest {
                audience: Some(AskerAudience::ASKER_AUDIENCE_UNSPECIFIED.into()),
                ..Default::default()
            },
            "AUDIENCE_INVALID",
        ),
        (UpdateSettingsRequest { audience: Some(EnumValue::from(9)), ..Default::default() }, "AUDIENCE_INVALID"),
        (
            UpdateSettingsRequest {
                muted_words: MutedWords { words: vec![" ".into()], ..Default::default() }.into(),
                ..Default::default()
            },
            "MUTED_WORDS_INVALID",
        ),
        (
            UpdateSettingsRequest {
                muted_words: MutedWords { words: (0..101).map(|n| format!("w{n}")).collect(), ..Default::default() }
                    .into(),
                ..Default::default()
            },
            "MUTED_WORDS_INVALID",
        ),
    ] {
        let err = ctx.call::<_, UpdateSettingsResponse>(BOX, "UpdateSettings", &user, &request).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), ("invalid_argument", reason));
    }
    let guest = ctx
        .app
        .call::<_, GetSettingsResponse>(&format!("{BOX}/GetSettings"), None, &GetSettingsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(guest.code, "unauthenticated");
}

#[tokio::test]
async fn a_box_is_public_and_says_whether_the_caller_can_ask() {
    let ctx = start().await;
    let (owner, friend, stranger) = (ctx.user("Owner").await, ctx.user("Friend").await, ctx.user("Stranger").await);
    ctx.befriend(&owner, &friend).await;

    // Closed: visible, even to guests, but nobody can ask.
    let closed = get_box(&ctx, None, &owner.handle).await.unwrap();
    assert!(!closed.open && !closed.can_ask);
    assert_eq!(closed.owner.as_option().unwrap().id, owner.id.to_string());
    assert_eq!(closed.prompt, "なんでも質問してね!");
    assert!(closed.share_url.ends_with(&format!("/u/{}/ask", owner.handle)));
    assert_eq!(
        get_box(&ctx, None, &format!("@{}", owner.handle)).await.unwrap().owner.as_option().unwrap().id,
        owner.id.to_string()
    );

    // Open for friends.
    ctx.update_settings(
        &owner,
        UpdateSettingsRequest {
            box_open: Some(true), box_prompt: Some("なんでもどうぞ".into()), ..Default::default()
        },
    )
    .await;
    let for_friend = get_box(&ctx, Some(&friend), &owner.id.to_string()).await.unwrap();
    assert!(for_friend.open && for_friend.can_ask);
    assert_eq!(
        (for_friend.prompt.as_str(), for_friend.audience.as_known()),
        ("なんでもどうぞ", Some(AskerAudience::ASKER_AUDIENCE_FRIENDS))
    );
    assert!(!get_box(&ctx, Some(&stranger), &owner.handle).await.unwrap().can_ask, "not a friend");
    assert!(!get_box(&ctx, Some(&owner), &owner.handle).await.unwrap().can_ask, "nobody asks themselves");
    assert!(!get_box(&ctx, None, &owner.handle).await.unwrap().can_ask, "guests have to sign in");

    // Open for everyone who is signed in: anyone could have asked, so an
    // anonymous question needs no minimum of friends.
    ctx.open_box(&owner, true).await;
    let for_stranger = get_box(&ctx, Some(&stranger), &owner.handle).await.unwrap();
    assert!(for_stranger.can_ask && for_stranger.anonymous_available);

    // Unknown users and blocks: there is no box.
    for unknown in ["nobody_here_at_all", &Uuid::now_v7().to_string(), ""] {
        assert_eq!(get_box(&ctx, Some(&stranger), unknown).await.unwrap_err().reason, "BOX_NOT_FOUND", "{unknown:?}");
    }
    ctx.social_block(&owner, &stranger).await;
    for (viewer, of) in [(&stranger, &owner), (&owner, &stranger)] {
        let hidden = get_box(&ctx, Some(viewer), &of.handle).await.unwrap_err();
        assert_eq!((hidden.code.as_str(), hidden.reason.as_str()), ("not_found", "BOX_NOT_FOUND"));
    }
    assert_eq!(ctx.try_ask(&stranger, &owner, "まだ聞ける?", false).await.unwrap_err().reason, "BOX_NOT_FOUND");
}

#[tokio::test]
async fn asking_is_refused_with_a_reason() {
    let ctx = start().await;
    let (owner, friend, stranger) = (ctx.user("Owner").await, ctx.user("Friend").await, ctx.user("Stranger").await);
    ctx.befriend(&owner, &friend).await;

    assert_eq!(ctx.try_ask(&friend, &owner, "あいてる?", false).await.unwrap_err().reason, "BOX_CLOSED");
    ctx.open_box(&owner, false).await;
    let not_friend = ctx.try_ask(&stranger, &owner, "だれ?", false).await.unwrap_err();
    assert_eq!((not_friend.code.as_str(), not_friend.reason.as_str()), ("permission_denied", "FRIENDS_ONLY"));
    assert_eq!(ctx.try_ask(&owner, &owner, "自分に", false).await.unwrap_err().reason, "CANNOT_ASK_SELF");

    // A friends-only box whose owner has one friend: an anonymous question
    // could only be from that friend. Refused with a reason of its own,
    // which the box already announced; a signed one is fine.
    let for_friend = get_box(&ctx, Some(&friend), &owner.handle).await.unwrap();
    assert!(for_friend.can_ask && !for_friend.anonymous_available);
    let alone = ctx.try_ask(&friend, &owner, "だれでしょう", false).await.unwrap_err();
    assert_eq!((alone.code.as_str(), alone.reason.as_str()), ("failed_precondition", "AUDIENCE_TOO_SMALL"));
    assert!(!ctx.try_ask(&friend, &owner, "名前つきなら", true).await.unwrap().anonymous);
    ctx.lone_friends_of(&owner, 1).await;
    assert_eq!(ctx.try_ask(&friend, &owner, "まだふたり", false).await.unwrap_err().reason, "AUDIENCE_TOO_SMALL");
    ctx.lone_friends_of(&owner, 1).await;
    assert!(
        get_box(&ctx, Some(&friend), &owner.handle).await.unwrap().anonymous_available,
        "three friends are a crowd"
    );
    assert!(
        !get_box(&ctx, Some(&stranger), &owner.handle).await.unwrap().anonymous_available,
        "not for those who cannot ask"
    );

    for bad in ["", "   \n ", &"あ".repeat(301)] {
        assert_eq!(ctx.try_ask(&friend, &owner, bad, false).await.unwrap_err().reason, "QUESTION_TEXT_INVALID");
    }
    let unknown = AskQuestionRequest { user: "no_such_user_anywhere".into(), text: "?".into(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, AskQuestionResponse>(BOX, "AskQuestion", &friend, &unknown).await.unwrap_err().reason,
        "BOX_NOT_FOUND"
    );
    let guest = AskQuestionRequest { user: owner.id.to_string(), text: "?".into(), ..Default::default() };
    assert_eq!(
        ctx.app.call::<_, AskQuestionResponse>(&format!("{BOX}/AskQuestion"), None, &guest).await.unwrap_err().code,
        "unauthenticated"
    );
    let bad_asset = AskQuestionRequest {
        user: owner.id.to_string(),
        text: "?".into(),
        signed: true,
        image_asset_id: "not-an-id".into(),
        ..Default::default()
    };
    assert_eq!(
        ctx.call::<_, AskQuestionResponse>(BOX, "AskQuestion", &friend, &bad_asset).await.unwrap_err().reason,
        "INVALID_ID"
    );

    // The longest question that fits, by handle.
    let longest = "あ".repeat(300);
    let request =
        AskQuestionRequest { user: owner.handle.clone(), text: format!("  {longest}  "), ..Default::default() };
    let sent: AskQuestionResponse = ctx.call(BOX, "AskQuestion", &friend, &request).await.unwrap();
    let sent = sent.question.as_option().unwrap();
    assert_eq!(sent.text, longest);
    assert!(sent.anonymous && sent.sent_by_me && sent.asker.is_unset());
    assert_eq!(sent.sent_state, SentState::SENT_STATE_WAITING);
    assert_eq!(sent.folder, QuestionFolder::QUESTION_FOLDER_UNSPECIFIED, "the sender is not shown the owner's folders");
    assert_eq!(sent.path, format!("/ask/q/{}", sent.id));
}

#[tokio::test]
async fn the_inbox_has_folders_counts_and_read_state() {
    let ctx = start().await;
    let (owner, sender, other) = (ctx.user("Owner").await, ctx.user("Sender").await, ctx.user("Other").await);
    ctx.open_box(&owner, true).await;
    ctx.open_box(&other, true).await;
    let mut ids = Vec::new();
    for text in ["一", "二", "三"] {
        ids.push(ctx.ask(&sender, &owner, text).await.id);
    }
    let elsewhere = ctx.ask(&sender, &other, "ほかの箱").await.id;

    let counts = ctx.counts(&owner).await;
    assert_eq!((counts.unread, counts.inbox, counts.answered, counts.archived, counts.filtered), (3, 3, 0, 0, 0));
    let inbox = ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await;
    assert_eq!(texts(&inbox), vec!["三", "二", "一"], "newest first");
    assert!(inbox.iter().all(|q| q.folder == QuestionFolder::QUESTION_FOLDER_INBOX && !q.read && !q.sent_by_me));
    assert_eq!(
        texts(&ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_UNSPECIFIED).await),
        vec!["三", "二", "一"],
        "the inbox is the default folder"
    );

    // Paging.
    let page = |cursor: String| ListInboxRequest {
        page: PageRequest { limit: 2, cursor, ..Default::default() }.into(),
        ..Default::default()
    };
    let first: ListInboxResponse = ctx.call(BOX, "ListInbox", &owner, &page(String::new())).await.unwrap();
    assert_eq!(texts(&first.questions), vec!["三", "二"]);
    assert!(first.page.as_option().unwrap().has_more);
    let second: ListInboxResponse =
        ctx.call(BOX, "ListInbox", &owner, &page(first.page.as_option().unwrap().next_cursor.clone())).await.unwrap();
    assert_eq!(texts(&second.questions), vec!["一"]);
    assert!(!second.page.as_option().unwrap().has_more);
    assert_eq!(
        ctx.call::<_, ListInboxResponse>(BOX, "ListInbox", &owner, &page("!!!".into())).await.unwrap_err().reason,
        "INVALID_CURSOR"
    );
    let unknown_folder = ListInboxRequest { folder: EnumValue::from(42), ..Default::default() };
    assert_eq!(
        ctx.call::<_, ListInboxResponse>(BOX, "ListInbox", &owner, &unknown_folder).await.unwrap_err().reason,
        "FOLDER_INVALID"
    );

    // Reading.
    let read: MarkQuestionsReadResponse = ctx
        .call(
            BOX,
            "MarkQuestionsRead",
            &owner,
            &MarkQuestionsReadRequest { question_ids: vec![ids[0].clone(), elsewhere.clone()], ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(read.counts.as_option().unwrap().unread, 2);
    assert_eq!(ctx.counts(&other).await.unread, 1, "someone else's question is not touched");
    let read: MarkQuestionsReadResponse = ctx
        .call(BOX, "MarkQuestionsRead", &owner, &MarkQuestionsReadRequest { all: true, ..Default::default() })
        .await
        .unwrap();
    assert_eq!(read.counts.as_option().unwrap().unread, 0);
    assert!(ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await.iter().all(|q| q.read));
    let too_many = MarkQuestionsReadRequest {
        question_ids: (0..101).map(|_| Uuid::now_v7().to_string()).collect(),
        ..Default::default()
    };
    assert_eq!(
        ctx.call::<_, MarkQuestionsReadResponse>(BOX, "MarkQuestionsRead", &owner, &too_many).await.unwrap_err().reason,
        "TOO_MANY_IDS"
    );

    // Archiving and back.
    let move_to = |id: &str, folder: QuestionFolder| MoveQuestionRequest {
        question_id: id.to_owned(),
        folder: folder.into(),
        ..Default::default()
    };
    let archived: MoveQuestionResponse = ctx
        .call(BOX, "MoveQuestion", &owner, &move_to(&ids[1], QuestionFolder::QUESTION_FOLDER_ARCHIVED))
        .await
        .unwrap();
    assert_eq!(archived.question.as_option().unwrap().folder, QuestionFolder::QUESTION_FOLDER_ARCHIVED);
    assert_eq!(texts(&ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_ARCHIVED).await), vec!["二"]);
    assert_eq!((ctx.counts(&owner).await.inbox, ctx.counts(&owner).await.archived), (2, 1));
    let _: MoveQuestionResponse =
        ctx.call(BOX, "MoveQuestion", &owner, &move_to(&ids[1], QuestionFolder::QUESTION_FOLDER_INBOX)).await.unwrap();
    assert_eq!(ctx.counts(&owner).await.inbox, 3);
    for folder in [
        QuestionFolder::QUESTION_FOLDER_ANSWERED,
        QuestionFolder::QUESTION_FOLDER_FILTERED,
        QuestionFolder::QUESTION_FOLDER_UNSPECIFIED,
    ] {
        assert_eq!(
            ctx.call::<_, MoveQuestionResponse>(BOX, "MoveQuestion", &owner, &move_to(&ids[1], folder))
                .await
                .unwrap_err()
                .reason,
            "FOLDER_INVALID"
        );
    }
    let not_mine = ctx
        .call::<_, MoveQuestionResponse>(
            BOX,
            "MoveQuestion",
            &owner,
            &move_to(&elsewhere, QuestionFolder::QUESTION_FOLDER_ARCHIVED),
        )
        .await
        .unwrap_err();
    assert_eq!((not_mine.code.as_str(), not_mine.reason.as_str()), ("not_found", "QUESTION_NOT_FOUND"));

    // Deleting: gone for the owner, still "waiting" for whoever sent it.
    let delete = |id: &str| DeleteQuestionRequest { question_id: id.to_owned(), ..Default::default() };
    let _: DeleteQuestionResponse = ctx.call(BOX, "DeleteQuestion", &owner, &delete(&ids[2])).await.unwrap();
    assert_eq!(texts(&ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await), vec!["二", "一"]);
    assert_eq!(
        ctx.call::<_, DeleteQuestionResponse>(BOX, "DeleteQuestion", &owner, &delete(&ids[2]))
            .await
            .unwrap_err()
            .reason,
        "QUESTION_NOT_FOUND"
    );
    assert_eq!(
        ctx.call::<_, DeleteQuestionResponse>(BOX, "DeleteQuestion", &owner, &delete(&elsewhere))
            .await
            .unwrap_err()
            .reason,
        "QUESTION_NOT_FOUND"
    );
    assert_eq!(
        ctx.call::<_, GetQuestionResponse>(BOX, "GetQuestion", &owner, &by_id(&ids[2])).await.unwrap_err().reason,
        "QUESTION_NOT_FOUND"
    );
    let asked = ctx.asked(&sender).await;
    assert_eq!(asked.len(), 4);
    assert!(
        asked.iter().all(|q| q.sent_state == SentState::SENT_STATE_WAITING),
        "a deleted question looks like any unanswered one"
    );
}

#[tokio::test]
async fn muted_words_hold_a_question_in_the_filtered_folder() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user("Sender").await);
    ctx.open_box(&owner, true).await;
    ctx.update_settings(
        &owner,
        UpdateSettingsRequest {
            muted_words: MutedWords { words: vec!["ピーマン".into()], ..Default::default() }.into(),
            ..Default::default()
        },
    )
    .await;
    let mut pushes = ctx.app.listen_user(owner.id).await;

    // The owner's own word, and one from the server's list.
    let held = ctx.ask(&sender, &owner, "ピ ー マ ン食べられる?").await;
    let abusive = ctx.ask(&sender, &owner, "おまえなんか死ねばいい").await;
    assert_eq!(held.sent_state, SentState::SENT_STATE_WAITING, "the sender is told nothing about the filter");
    assert_eq!(abusive.sent_state, SentState::SENT_STATE_WAITING);

    let counts = ctx.counts(&owner).await;
    assert_eq!((counts.unread, counts.inbox, counts.filtered), (0, 0, 2));
    assert!(ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await.is_empty());
    let filtered = ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_FILTERED).await;
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().all(|q| q.folder == QuestionFolder::QUESTION_FOLDER_FILTERED));
    assert!(ctx.notifications_for(owner.id, "ask.question").await.is_empty(), "a held question is not announced");
    assert!(stays_quiet(&mut pushes).await);

    // Released into the inbox, or answered straight from the folder.
    let release = MoveQuestionRequest {
        question_id: held.id.clone(),
        folder: QuestionFolder::QUESTION_FOLDER_INBOX.into(),
        ..Default::default()
    };
    let _: MoveQuestionResponse = ctx.call(BOX, "MoveQuestion", &owner, &release).await.unwrap();
    assert_eq!((ctx.counts(&owner).await.inbox, ctx.counts(&owner).await.filtered), (1, 1));
    let answered =
        ctx.answer(&owner, &abusive.id, "そういうのはやめてね", AnswerAudience::ANSWER_AUDIENCE_PRIVATE).await;
    assert_eq!(answered.folder, QuestionFolder::QUESTION_FOLDER_ANSWERED);
    assert_eq!(ctx.counts(&owner).await.filtered, 0);

    // An ordinary question goes straight in.
    ctx.ask(&sender, &owner, "好きな野菜は?").await;
    assert_eq!(ctx.counts(&owner).await.unread, 2);
    assert_eq!(ctx.notifications_for(owner.id, "ask.question").await.len(), 1);
}

#[tokio::test]
async fn an_answer_is_written_edited_and_deleted() {
    let ctx = start().await;
    let (owner, sender, other) = (ctx.user("Owner").await, ctx.user("Sender").await, ctx.user("Other").await);
    ctx.open_box(&owner, true).await;
    let question = ctx.ask(&sender, &owner, "好きな教科は?").await;
    let answer = |text: &str| AnswerQuestionRequest {
        question_id: question.id.clone(),
        text: text.to_owned(),
        ..Default::default()
    };

    for (request, reason) in [
        (answer("  "), "ANSWER_EMPTY"),
        (answer(&"あ".repeat(1001)), "ANSWER_TEXT_INVALID"),
        (AnswerQuestionRequest { audience: EnumValue::from(77), ..answer("数学") }, "AUDIENCE_INVALID"),
        (AnswerQuestionRequest { stamp_id: Uuid::now_v7().to_string(), ..answer("数学") }, "STAMP_NOT_USABLE"),
        (AnswerQuestionRequest { image_asset_id: Uuid::now_v7().to_string(), ..answer("数学") }, "ASSET_NOT_FOUND"),
    ] {
        assert_eq!(
            ctx.call::<_, AnswerQuestionResponse>(BOX, "AnswerQuestion", &owner, &request).await.unwrap_err().reason,
            reason
        );
    }
    assert_eq!(
        ctx.call::<_, AnswerQuestionResponse>(BOX, "AnswerQuestion", &other, &answer("数学")).await.unwrap_err().reason,
        "QUESTION_NOT_FOUND"
    );
    let not_yet =
        UpdateAnswerRequest { question_id: question.id.clone(), text: Some("?".into()), ..Default::default() };
    assert_eq!(
        ctx.call::<_, UpdateAnswerResponse>(BOX, "UpdateAnswer", &owner, &not_yet).await.unwrap_err().reason,
        "QUESTION_NOT_ANSWERED"
    );

    // Answering: public unless said otherwise, and it tells everyone it should.
    let mut asker_pushes = ctx.app.listen_user(sender.id).await;
    ctx.new_events().await;
    let answered: AnswerQuestionResponse =
        ctx.call(BOX, "AnswerQuestion", &owner, &answer(&"あ".repeat(1000))).await.unwrap();
    let answered = answered.question.as_option().unwrap().clone();
    assert_eq!(answered.folder, QuestionFolder::QUESTION_FOLDER_ANSWERED);
    assert!(answered.read, "answering reads the question");
    let written = answered.answer.as_option().unwrap();
    assert_eq!(written.audience, AnswerAudience::ANSWER_AUDIENCE_PUBLIC);
    assert!(written.answered_at.is_set() && written.edited_at.is_unset());

    let events = ctx.deliver().await;
    let published = events.iter().find(|e| e.topic == "ask.answer.published").expect("ask.answer.published");
    assert_eq!(published.actor_id, Some(owner.id));
    assert_eq!(
        published.data,
        serde_json::json!({ "question_id": question.id, "owner_id": owner.id, "audience": 1, "first": true })
    );
    let indexed = events.iter().find(|e| e.topic == "search.changed").expect("a search document");
    assert_eq!(
        (indexed.data["kind"].as_str(), indexed.data["doc"]["title"].as_str()),
        (Some("answer"), Some("好きな教科は?"))
    );
    assert_eq!(indexed.data["doc"]["owner_id"], serde_json::json!(owner.id));
    assert_eq!(indexed.data["doc"]["path"], serde_json::json!(format!("/ask/q/{}", question.id)));
    let activity = events.iter().find(|e| e.topic == "activity.recorded").expect("an activity entry");
    assert_eq!(
        (activity.data["kind"].as_str(), activity.data["audience"].as_str(), activity.actor_id),
        (Some("ask.answered"), Some("public"), Some(owner.id))
    );
    let told = ctx.notifications_for(sender.id, "ask.answered").await;
    assert_eq!(told.len(), 1, "the asker is told");
    assert_eq!(told[0].actor_id, Some(owner.id));
    assert_eq!(told[0].data["path"], serde_json::json!(format!("/ask/q/{}", question.id)));
    match next_push(&mut asker_pushes).await.expect("the asker is pushed the answer") {
        Push::QuestionAnswered(pushed) => {
            let pushed = pushed.question.as_option().unwrap();
            assert_eq!(
                (pushed.id.as_str(), pushed.sent_state.as_known()),
                (question.id.as_str(), Some(SentState::SENT_STATE_ANSWERED))
            );
            assert!(pushed.sent_by_me && pushed.answer.is_set());
        }
        other => panic!("unexpected push {other:?}"),
    }

    // An answered question is not answered again, nor moved.
    assert_eq!(
        ctx.call::<_, AnswerQuestionResponse>(BOX, "AnswerQuestion", &owner, &answer("もう一回"))
            .await
            .unwrap_err()
            .reason,
        "QUESTION_ALREADY_ANSWERED"
    );
    let moving = MoveQuestionRequest {
        question_id: question.id.clone(),
        folder: QuestionFolder::QUESTION_FOLDER_ARCHIVED.into(),
        ..Default::default()
    };
    assert_eq!(
        ctx.call::<_, MoveQuestionResponse>(BOX, "MoveQuestion", &owner, &moving).await.unwrap_err().reason,
        "QUESTION_ALREADY_ANSWERED"
    );
    let withdraw = WithdrawQuestionRequest { question_id: question.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, WithdrawQuestionResponse>(BOX, "WithdrawQuestion", &sender, &withdraw).await.unwrap_err().reason,
        "QUESTION_ALREADY_ANSWERED"
    );

    // Editing the text marks the answer as edited and refreshes the index.
    let edit = |change: UpdateAnswerRequest| UpdateAnswerRequest { question_id: question.id.clone(), ..change };
    let edited: UpdateAnswerResponse = ctx
        .call(
            BOX,
            "UpdateAnswer",
            &owner,
            &edit(UpdateAnswerRequest { text: Some("数学!".into()), ..Default::default() }),
        )
        .await
        .unwrap();
    let edited = edited.question.as_option().unwrap().answer.as_option().unwrap().clone();
    assert_eq!(edited.text, "数学!");
    assert!(edited.edited_at.is_set());
    assert_eq!(edited.audience, AnswerAudience::ANSWER_AUDIENCE_PUBLIC);
    let events = ctx.deliver().await;
    assert!(events.iter().any(|e| e.topic == "search.changed" && e.data["doc"]["text"] == serde_json::json!("数学!")));
    assert!(
        !events.iter().any(|e| e.topic == "ask.answer.published"),
        "an edit that keeps the audience is not a publication"
    );

    // Narrowing the audience takes it out of search; the asker is not told again.
    let narrowed: UpdateAnswerResponse = ctx
        .call(
            BOX,
            "UpdateAnswer",
            &owner,
            &edit(UpdateAnswerRequest {
                audience: Some(AnswerAudience::ANSWER_AUDIENCE_FRIENDS.into()),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        narrowed.question.as_option().unwrap().answer.as_option().unwrap().audience,
        AnswerAudience::ANSWER_AUDIENCE_FRIENDS
    );
    let events = ctx.deliver().await;
    assert!(events.iter().any(|e| e.topic == "search.changed" && e.data["doc"].is_null()));
    let republished =
        events.iter().find(|e| e.topic == "ask.answer.published").expect("the change of audience is announced");
    assert_eq!((republished.data["audience"].as_i64(), republished.data["first"].as_bool()), (Some(2), Some(false)));
    assert_eq!(ctx.notifications_for(sender.id, "ask.answered").await.len(), 1);
    for (change, reason) in [
        (UpdateAnswerRequest { text: Some(String::new()), ..Default::default() }, "ANSWER_EMPTY"),
        (
            UpdateAnswerRequest {
                audience: Some(AnswerAudience::ANSWER_AUDIENCE_UNSPECIFIED.into()),
                ..Default::default()
            },
            "AUDIENCE_INVALID",
        ),
        (UpdateAnswerRequest { text: Some("あ".repeat(1001)), ..Default::default() }, "ANSWER_TEXT_INVALID"),
    ] {
        assert_eq!(
            ctx.call::<_, UpdateAnswerResponse>(BOX, "UpdateAnswer", &owner, &edit(change)).await.unwrap_err().reason,
            reason
        );
    }

    // Deleting the answer puts the question back.
    let delete = DeleteAnswerRequest { question_id: question.id.clone(), ..Default::default() };
    let back: DeleteAnswerResponse = ctx.call(BOX, "DeleteAnswer", &owner, &delete).await.unwrap();
    let back = back.question.as_option().unwrap();
    assert_eq!(back.folder, QuestionFolder::QUESTION_FOLDER_INBOX);
    assert!(back.answer.is_unset());
    let events = ctx.deliver().await;
    assert!(
        events
            .iter()
            .any(|e| e.topic == "ask.answer.deleted" && e.data["question_id"] == serde_json::json!(question.id))
    );
    assert_eq!(ctx.asked(&sender).await[0].sent_state, SentState::SENT_STATE_WAITING);
    assert_eq!(
        ctx.call::<_, DeleteAnswerResponse>(BOX, "DeleteAnswer", &owner, &delete).await.unwrap_err().reason,
        "QUESTION_NOT_ANSWERED"
    );
    assert_eq!(
        ctx.call::<_, DeleteAnswerResponse>(BOX, "DeleteAnswer", &other, &delete).await.unwrap_err().reason,
        "QUESTION_NOT_FOUND"
    );
}

#[tokio::test]
async fn an_answer_carries_an_image_or_a_stamp_but_not_both() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user("Sender").await);
    ctx.open_box(&owner, true).await;
    let core = &ctx.app.core;
    let image = ready_image(core, owner.id, AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE).await.unwrap();
    let art = ready_image(core, owner.id, AssetPurpose::ASSET_PURPOSE_STAMP).await.unwrap();
    let source = tokuly_stamp::stamps::StampSource::from_asset(&art).unwrap();
    let text = tokuly_stamp::validate::stamp_text("やったー", "", &[] as &[&str]).unwrap();
    let stamp_id = tokuly_stamp::personal::create(core, owner.id, &source, &text, 10).await.unwrap();

    let with_image = ctx.ask(&sender, &owner, "写真で答えて").await;
    let with_stamp = ctx.ask(&sender, &owner, "スタンプで答えて").await;
    let both = AnswerQuestionRequest {
        question_id: with_image.id.clone(),
        image_asset_id: image.id.to_string(),
        stamp_id: stamp_id.to_string(),
        ..Default::default()
    };
    assert_eq!(
        ctx.call::<_, AnswerQuestionResponse>(BOX, "AnswerQuestion", &owner, &both).await.unwrap_err().reason,
        "ANSWER_ATTACHMENT_CONFLICT"
    );

    // An image alone is an answer; the question holds the upload.
    let request = AnswerQuestionRequest {
        question_id: with_image.id.clone(),
        image_asset_id: image.id.to_string(),
        ..Default::default()
    };
    let answered: AnswerQuestionResponse = ctx.call(BOX, "AnswerQuestion", &owner, &request).await.unwrap();
    let answer = answered.question.as_option().unwrap().answer.as_option().unwrap().clone();
    assert_eq!(answer.image.as_option().expect("the image").asset_id, image.id.to_string());
    assert!(answer.text.is_empty() && answer.stamp.is_unset());
    assert!(assets::is_attached(core, image.id).await.unwrap());
    let wrong_purpose = AnswerQuestionRequest {
        question_id: with_stamp.id.clone(),
        image_asset_id: art.id.to_string(),
        ..Default::default()
    };
    assert_eq!(
        ctx.call::<_, AnswerQuestionResponse>(BOX, "AnswerQuestion", &owner, &wrong_purpose).await.unwrap_err().reason,
        "ASSET_WRONG_PURPOSE"
    );

    // A stamp alone is an answer too.
    let request = AnswerQuestionRequest {
        question_id: with_stamp.id.clone(),
        stamp_id: stamp_id.to_string(),
        ..Default::default()
    };
    let answered: AnswerQuestionResponse = ctx.call(BOX, "AnswerQuestion", &owner, &request).await.unwrap();
    let stamp = answered
        .question
        .as_option()
        .unwrap()
        .answer
        .as_option()
        .unwrap()
        .stamp
        .as_option()
        .cloned()
        .expect("the stamp");
    assert_eq!(stamp.id, stamp_id.to_string());
    let page = answers_page(&ctx, None, &owner).await;
    assert_eq!(page.len(), 2);
    assert!(page.iter().any(|q| q.answer.as_option().unwrap().stamp.is_set()), "readers see the stamp");

    // Swapping the image for text releases the upload.
    let swap = UpdateAnswerRequest {
        question_id: with_image.id.clone(),
        text: Some("やっぱり文字で".into()),
        image_asset_id: Some(String::new()),
        ..Default::default()
    };
    let swapped: UpdateAnswerResponse = ctx.call(BOX, "UpdateAnswer", &owner, &swap).await.unwrap();
    assert!(swapped.question.as_option().unwrap().answer.as_option().unwrap().image.is_unset());
    assert!(!assets::is_attached(core, image.id).await.unwrap());
    // Removing the only content is refused.
    let empty =
        UpdateAnswerRequest { question_id: with_stamp.id.clone(), stamp_id: Some(String::new()), ..Default::default() };
    assert_eq!(
        ctx.call::<_, UpdateAnswerResponse>(BOX, "UpdateAnswer", &owner, &empty).await.unwrap_err().reason,
        "ANSWER_EMPTY"
    );
}

#[tokio::test]
async fn the_audience_of_an_answer_decides_who_reads_it() {
    let ctx = start().await;
    let (owner, asker, friend, stranger) =
        (ctx.user("Owner").await, ctx.user("Asker").await, ctx.user("Friend").await, ctx.user("Stranger").await);
    ctx.befriend(&owner, &friend).await;
    ctx.open_box(&owner, true).await;
    let public = ctx.ask(&asker, &owner, "みんなに").await;
    let friends = ctx.ask(&asker, &owner, "友達に").await;
    let private = ctx.ask(&asker, &owner, "ないしょで").await;
    ctx.answer(&owner, &public.id, "公開のこたえ", AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;
    ctx.answer(&owner, &friends.id, "友達へのこたえ", AnswerAudience::ANSWER_AUDIENCE_FRIENDS).await;
    ctx.answer(&owner, &private.id, "ないしょのこたえ", AnswerAudience::ANSWER_AUDIENCE_PRIVATE).await;

    // The Q&A page, newest answer first.
    assert_eq!(texts(&answers_page(&ctx, None, &owner).await), vec!["みんなに"]);
    assert_eq!(texts(&answers_page(&ctx, Some(&stranger), &owner).await), vec!["みんなに"]);
    assert_eq!(texts(&answers_page(&ctx, Some(&friend), &owner).await), vec!["友達に", "みんなに"]);
    assert_eq!(
        texts(&answers_page(&ctx, Some(&owner), &owner).await),
        vec!["友達に", "みんなに"],
        "private answers are not on the page"
    );
    assert_eq!(
        texts(&ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_ANSWERED).await),
        vec!["ないしょで", "友達に", "みんなに"]
    );
    for (viewer, count) in [(None, 1), (Some(&stranger), 1), (Some(&friend), 2), (Some(&owner), 2)] {
        assert_eq!(get_box(&ctx, viewer, &owner.handle).await.unwrap().answer_count, count);
    }
    let on_page = &answers_page(&ctx, Some(&stranger), &owner).await[0];
    assert!(on_page.anonymous && on_page.asker.is_unset() && !on_page.sent_by_me && !on_page.read);
    assert_eq!(on_page.folder, QuestionFolder::QUESTION_FOLDER_UNSPECIFIED);
    assert_eq!(on_page.answer.as_option().unwrap().text, "公開のこたえ");

    // By ID.
    let open = |viewer: Option<&TestUser>, id: &str| {
        let (ctx, request) = (&ctx, by_id(id));
        let viewer = viewer.cloned();
        async move { ctx.app.call::<_, GetQuestionResponse>(&format!("{BOX}/GetQuestion"), viewer.as_ref(), &request).await }
    };
    for viewer in [None, Some(&stranger), Some(&friend), Some(&owner), Some(&asker)] {
        assert!(open(viewer, &public.id).await.is_ok());
    }
    for (viewer, visible) in
        [(None, false), (Some(&stranger), false), (Some(&friend), true), (Some(&owner), true), (Some(&asker), true)]
    {
        assert_eq!(
            open(viewer, &friends.id).await.is_ok(),
            visible,
            "the friends-only answer for {:?}",
            viewer.map(|v| &v.handle)
        );
    }
    for (viewer, visible) in
        [(None, false), (Some(&stranger), false), (Some(&friend), false), (Some(&owner), true), (Some(&asker), true)]
    {
        assert_eq!(
            open(viewer, &private.id).await.is_ok(),
            visible,
            "the private answer for {:?}",
            viewer.map(|v| &v.handle)
        );
    }
    assert_eq!(open(Some(&stranger), &private.id).await.unwrap_err().reason, "QUESTION_NOT_FOUND");
    assert_eq!(open(None, &Uuid::now_v7().to_string()).await.unwrap_err().reason, "QUESTION_NOT_FOUND");

    // The asker always reads the answer to their own question.
    let asked = ctx.asked(&asker).await;
    assert_eq!(asked.len(), 3);
    assert!(asked.iter().all(|q| q.sent_state == SentState::SENT_STATE_ANSWERED && q.answer.is_set() && q.sent_by_me));
    assert_eq!(ctx.notifications_for(asker.id, "ask.answered").await.len(), 3, "also for a private answer");

    // Only public answers are searchable and only published ones are activity.
    let outbox = ctx.app.events_by("search.changed", owner.id).await;
    assert!(outbox.is_empty(), "search documents carry no actor");
    let indexed: Vec<String> = ctx
        .app
        .events("search.changed")
        .await
        .into_iter()
        .filter(|e| e.data["doc"]["owner_id"] == serde_json::json!(owner.id))
        .map(|e| e.data["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(indexed, vec![public.id.clone()]);
    let audiences: Vec<String> = ctx
        .app
        .events_by("activity.recorded", owner.id)
        .await
        .into_iter()
        .filter(|e| e.data["kind"] == serde_json::json!("ask.answered"))
        .map(|e| e.data["audience"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(audiences, vec!["public", "friends"], "one entry per published answer, none for the private one");
}

#[tokio::test]
async fn the_answers_page_is_paged() {
    let ctx = start().await;
    let (owner, asker) = (ctx.user("Owner").await, ctx.user("Asker").await);
    ctx.open_box(&owner, true).await;
    for n in 1..=5 {
        let question = ctx.ask(&asker, &owner, &format!("質問{n}")).await;
        ctx.answer(&owner, &question.id, &format!("答え{n}"), AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;
    }
    let page = |cursor: String| ListAnswersRequest {
        user: owner.handle.clone(),
        page: PageRequest { limit: 2, cursor, ..Default::default() }.into(),
        ..Default::default()
    };
    let mut seen = Vec::new();
    let mut cursor = String::new();
    loop {
        let response: ListAnswersResponse =
            ctx.app.call(&format!("{BOX}/ListAnswers"), None, &page(cursor)).await.unwrap();
        seen.extend(response.questions.iter().map(|q| q.text.clone()));
        let info = response.page.as_option().unwrap();
        if !info.has_more {
            break;
        }
        cursor = info.next_cursor.clone();
    }
    assert_eq!(seen, vec!["質問5", "質問4", "質問3", "質問2", "質問1"]);
    let unknown = ListAnswersRequest { user: "nobody_at_all_here".into(), ..Default::default() };
    assert_eq!(
        ctx.app.call::<_, ListAnswersResponse>(&format!("{BOX}/ListAnswers"), None, &unknown).await.unwrap_err().reason,
        "BOX_NOT_FOUND"
    );
}

#[tokio::test]
async fn the_asker_can_take_a_question_back_while_it_is_unanswered() {
    let ctx = start().await;
    let (owner, asker, other) = (ctx.user("Owner").await, ctx.user("Asker").await, ctx.user("Other").await);
    ctx.open_box(&owner, true).await;
    let question = ctx.ask(&asker, &owner, "やっぱりなし").await;
    let withdraw = WithdrawQuestionRequest { question_id: question.id.clone(), ..Default::default() };
    assert_eq!(
        ctx.call::<_, WithdrawQuestionResponse>(BOX, "WithdrawQuestion", &other, &withdraw).await.unwrap_err().reason,
        "QUESTION_NOT_FOUND"
    );
    assert_eq!(
        ctx.call::<_, WithdrawQuestionResponse>(BOX, "WithdrawQuestion", &owner, &withdraw).await.unwrap_err().reason,
        "QUESTION_NOT_FOUND"
    );

    let _: WithdrawQuestionResponse = ctx.call(BOX, "WithdrawQuestion", &asker, &withdraw).await.unwrap();
    assert!(ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await.is_empty());
    assert_eq!(ctx.counts(&owner).await.unread, 0);
    assert!(ctx.asked(&asker).await.is_empty());
    assert_eq!(
        ctx.call::<_, WithdrawQuestionResponse>(BOX, "WithdrawQuestion", &asker, &withdraw).await.unwrap_err().reason,
        "QUESTION_NOT_FOUND"
    );
    assert_eq!(
        ctx.call::<_, GetQuestionResponse>(BOX, "GetQuestion", &asker, &by_id(&question.id)).await.unwrap_err().reason,
        "QUESTION_NOT_FOUND"
    );
}

#[tokio::test]
async fn a_social_block_closes_the_box_and_withdraws_only_what_was_signed() {
    let ctx = start().await;
    let (owner, sender) = (ctx.user("Owner").await, ctx.user("Sender").await);
    ctx.open_box(&owner, true).await;
    let anonymous = ctx.ask(&sender, &owner, "匿名の質問").await;
    ctx.try_ask(&sender, &owner, "名前つきの質問", true).await.unwrap();
    let answered = ctx.ask(&sender, &owner, "答えてもらった質問").await;
    ctx.deliver().await;
    ctx.answer(&owner, &answered.id, "こたえ", AnswerAudience::ANSWER_AUDIENCE_PRIVATE).await;
    assert_eq!(ctx.counts(&owner).await.inbox, 2);

    ctx.social_block(&owner, &sender).await;

    // The signed question goes: its sender is known, and blocked. The
    // anonymous one stays: if it went too, blocking people one at a time
    // would show who wrote it.
    assert_eq!(texts(&ctx.inbox(&owner, QuestionFolder::QUESTION_FOLDER_INBOX).await), vec![anonymous.text.as_str()]);
    assert_eq!(ctx.counts(&owner).await.answered, 1);
    // For the blocked sender the box and everything in it are gone.
    assert_eq!(ctx.try_ask(&sender, &owner, "まだ聞ける?", false).await.unwrap_err().reason, "BOX_NOT_FOUND");
    assert!(ctx.asked(&sender).await.is_empty());
    assert_eq!(
        ctx.call::<_, GetQuestionResponse>(BOX, "GetQuestion", &sender, &by_id(&answered.id)).await.unwrap_err().reason,
        "QUESTION_NOT_FOUND"
    );
}

#[tokio::test]
async fn a_signed_askers_name_is_hidden_from_readers_in_a_block_with_them() {
    let ctx = start().await;
    let (owner, asker, reader) = (ctx.user("Owner").await, ctx.user("Asker").await, ctx.user("Reader").await);
    ctx.open_box(&owner, true).await;
    let question = ctx.try_ask(&asker, &owner, "名前つき", true).await.unwrap();
    ctx.deliver().await;
    ctx.answer(&owner, &question.id, "こたえ", AnswerAudience::ANSWER_AUDIENCE_PUBLIC).await;
    let named = &answers_page(&ctx, Some(&reader), &owner).await[0];
    assert_eq!(named.asker.as_option().expect("a signed asker is named").id, asker.id.to_string());
    assert!(!named.anonymous);

    ctx.social_block(&reader, &asker).await;
    let unnamed = &answers_page(&ctx, Some(&reader), &owner).await[0];
    assert!(unnamed.asker.is_unset(), "content of a blocked user does not reach the blocker");
    assert_eq!(unnamed.text, "名前つき");
}
