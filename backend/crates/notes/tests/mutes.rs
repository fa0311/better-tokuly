//! Muting (words, conversations, renotes), trends, note analytics and post
//! notifications.
mod common;

use common::*;
use tokuly_proto::{
    notes::*,
    social::{FollowNotifyLevel, UpdateFollowPrefsRequest, UpdateFollowPrefsResponse},
};
use tokuly_testkit::TestUser;

async fn mute_call<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
    ctx: &Ctx,
    method: &str,
    user: &TestUser,
    request: &Req,
) -> Result<Resp, String> {
    ctx.app.call(&format!("tokuly.notes.v1.NoteMuteService/{method}"), Some(user), request).await.map_err(|e| e.reason)
}

async fn home(ctx: &Ctx, user: &TestUser) -> Vec<String> {
    let response: GetHomeTimelineResponse = ctx
        .note("GetHomeTimeline", Some(user), &GetHomeTimelineRequest { page: page(50, ""), ..Default::default() })
        .await
        .unwrap();
    texts(&response.entries).iter().map(|t| t.to_string()).collect()
}

fn word(text: &str, regex: bool) -> MutedWord {
    MutedWord { text: text.into(), regex, ..Default::default() }
}

#[tokio::test]
async fn muted_words_and_hidden_renotes_stay_out_of_timelines_but_not_profiles() {
    let ctx = start().await;
    let (me, friend, other) = (ctx.member("me").await, ctx.member("friend").await, ctx.member("other").await);
    ctx.befriend(&me, &friend).await;
    ctx.post(&friend, "映画のネタバレだけど").await;
    ctx.post(&friend, "SPOILERS ahead").await;
    ctx.post(&friend, "ふつうのノート").await;
    let renoted = ctx.post(&other, "renote me").await;
    ctx.renote(&friend, &renoted).await;
    ctx.post(&me, "自分のネタバレは消えない").await;

    let saved: UpdateMutedWordsResponse = mute_call(
        &ctx,
        "UpdateMutedWords",
        &me,
        &UpdateMutedWordsRequest {
            words: vec![word("ネタバレ", false), word("spoil(er)?s?", true)],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(saved.words.len(), 2);
    let _: HideRenotesFromResponse = mute_call(
        &ctx,
        "HideRenotesFrom",
        &me,
        &HideRenotesFromRequest { user_id: friend.id.to_string(), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(home(&ctx, &me).await, vec!["自分のネタバレは消えない", "ふつうのノート"]);
    // The profile shows everything: the person chose to look there.
    let profile: GetUserTimelineResponse = ctx
        .note(
            "GetUserTimeline",
            Some(&me),
            &GetUserTimelineRequest { user_id: friend.id.to_string(), page: page(50, ""), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(profile.entries.len(), 4);

    let settings: GetMuteSettingsResponse =
        mute_call(&ctx, "GetMuteSettings", &me, &GetMuteSettingsRequest::default()).await.unwrap();
    assert_eq!(settings.renotes_hidden_from[0].id, friend.id.to_string());
    let _: ShowRenotesFromResponse = mute_call(
        &ctx,
        "ShowRenotesFrom",
        &me,
        &ShowRenotesFromRequest { user_id: friend.id.to_string(), ..Default::default() },
    )
    .await
    .unwrap();
    assert!(home(&ctx, &me).await.contains(&"renote me".to_string()));

    let bad = mute_call::<_, UpdateMutedWordsResponse>(
        &ctx,
        "UpdateMutedWords",
        &me,
        &UpdateMutedWordsRequest { words: vec![word("(unclosed", true)], ..Default::default() },
    )
    .await;
    assert_eq!(bad.unwrap_err(), "INVALID_MUTED_WORD");
    let myself = mute_call::<_, HideRenotesFromResponse>(
        &ctx,
        "HideRenotesFrom",
        &me,
        &HideRenotesFromRequest { user_id: me.id.to_string(), ..Default::default() },
    )
    .await;
    assert_eq!(myself.unwrap_err(), "USER_NOT_FOUND");
}

#[tokio::test]
async fn a_muted_conversation_sends_no_more_notifications() {
    let ctx = start().await;
    let (me, other) = (ctx.member("me").await, ctx.member("other").await);
    let mine = ctx.post(&me, "話題のノート").await;
    ctx.reply(&other, &mine, "一回目の返信").await;
    assert_eq!(ctx.notifications_for(me.id, "notes.reply").await.len(), 1);
    let reply = ctx.reply(&other, &mine, "二回目").await;
    let _: MuteThreadResponse =
        mute_call(&ctx, "MuteThread", &me, &MuteThreadRequest { note_id: reply.id.clone(), ..Default::default() })
            .await
            .unwrap();
    assert!(ctx.get_note(Some(&me), &mine.id).await.unwrap().viewer.thread_muted);
    ctx.reply(&other, &reply, &format!("三回目 @{}", me.handle)).await;
    ctx.reply(&other, &mine, "四回目").await;
    assert_eq!(ctx.notifications_for(me.id, "notes.reply").await.len(), 2, "nothing after muting");
    assert!(ctx.notifications_for(me.id, "notes.mention").await.is_empty());
    let _: UnmuteThreadResponse =
        mute_call(&ctx, "UnmuteThread", &me, &UnmuteThreadRequest { note_id: mine.id.clone(), ..Default::default() })
            .await
            .unwrap();
    ctx.reply(&other, &mine, "五回目").await;
    assert_eq!(ctx.notifications_for(me.id, "notes.reply").await.len(), 3);
}

#[tokio::test]
async fn trends_resist_fresh_accounts_and_split_by_language() {
    let ctx = start().await;
    let stem = unique("tr");
    // A crowd of new accounts made together pushes nothing up.
    let mut crowd = Vec::new();
    for index in 0..4 {
        crowd.push(ctx.member(&format!("fresh{index}")).await);
    }
    for person in &crowd {
        ctx.post(person, &format!("拡散希望 #{stem}spam")).await;
    }
    // Accounts that joined the same day last week count as one person.
    let mut cluster = Vec::new();
    for index in 0..3 {
        let person = ctx.member(&format!("cluster{index}")).await;
        ctx.age_account(&person, 7).await;
        ctx.post(&person, &format!("みんなで #{stem}cluster")).await;
        cluster.push(person);
    }
    // Two established people talking about the same thing make a trend, in Japanese and in English.
    let (a, b) = (ctx.member("a").await, ctx.member("b").await);
    for person in [&a, &b] {
        ctx.age_account(person, 90).await;
        ctx.post(person, &format!("今日は #{stem}ja の話")).await;
        ctx.post(person, &format!("talking about #{stem}en today")).await;
    }
    ctx.notes.refresh_trends().await.unwrap();
    let ranking = |language: &str| {
        let ctx = &ctx;
        let request = ListTrendingHashtagsRequest { limit: 50, language: language.into(), ..Default::default() };
        let stem = stem.clone();
        async move {
            let response: ListTrendingHashtagsResponse =
                ctx.note("ListTrendingHashtags", None, &request).await.unwrap();
            let mut tags: Vec<String> =
                response.hashtags.iter().map(|t| t.hashtag.tag.clone()).filter(|tag| tag.starts_with(&stem)).collect();
            tags.sort();
            tags
        }
    };
    assert_eq!(ranking("").await, vec![format!("{stem}en"), format!("{stem}ja")]);
    assert_eq!(ranking("ja").await, vec![format!("{stem}ja")]);
    assert_eq!(ranking("en").await, vec![format!("{stem}en")]);
    assert_eq!(
        ctx.note_reason(
            "ListTrendingHashtags",
            None,
            &ListTrendingHashtagsRequest { language: "xx".into(), ..Default::default() }
        )
        .await,
        "INVALID_LANGUAGE"
    );

    // Notes trend by the different established people reacting to them.
    let hot = ctx.post(&a, &format!("話題になるノート {stem}")).await;
    let fans: Vec<TestUser> = {
        let mut fans = Vec::new();
        for index in 0..3 {
            let fan = ctx.member(&format!("fan{index}")).await;
            ctx.age_account(&fan, 100 + index).await;
            fans.push(fan);
        }
        fans
    };
    for fan in &fans {
        ctx.like(fan, &hot).await;
    }
    for person in &crowd {
        ctx.like(person, &ctx.post(&b, &format!("新人が推すノート {stem}")).await).await;
    }
    ctx.notes.refresh_trends().await.unwrap();
    let trending: ListTrendingNotesResponse = ctx
        .note(
            "ListTrendingNotes",
            None,
            &ListTrendingNotesRequest { limit: 50, language: "ja".into(), ..Default::default() },
        )
        .await
        .unwrap();
    let mine: Vec<&str> = trending.notes.iter().map(|n| n.text.as_str()).filter(|t| t.contains(&stem)).collect();
    assert_eq!(mine, vec![hot.text.as_str()]);
}

#[tokio::test]
async fn authors_see_the_numbers_of_their_notes() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let note = ctx.post(&alice, "見てね").await;
    let _: RecordNoteViewsResponse = ctx
        .note(
            "RecordNoteViews",
            Some(&bob),
            &RecordNoteViewsRequest { note_ids: vec![note.id.clone()], ..Default::default() },
        )
        .await
        .unwrap();
    ctx.like(&bob, &note).await;
    ctx.app.flush_analytics().await;
    let numbers: GetNoteAnalyticsResponse = ctx
        .note(
            "GetNoteAnalytics",
            Some(&alice),
            &GetNoteAnalyticsRequest { note_id: note.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!((numbers.impressions, numbers.engagements), (1, 1));
    assert!((numbers.engagement_rate - 1.0).abs() < f64::EPSILON);
    assert_eq!(numbers.impressions_by_hour.iter().map(|hour| hour.count).sum::<i64>(), 1);
    let reason = ctx
        .note_reason(
            "GetNoteAnalytics",
            Some(&bob),
            &GetNoteAnalyticsRequest { note_id: note.id.clone(), ..Default::default() },
        )
        .await;
    assert_eq!(reason, "NOT_NOTE_AUTHOR");
}

#[tokio::test]
async fn followers_who_asked_hear_of_every_post_they_may_see() {
    let ctx = start().await;
    let (author, fan, quiet) = (ctx.member("author").await, ctx.member("fan").await, ctx.member("quiet").await);
    ctx.follow(&fan, &author).await;
    ctx.follow(&quiet, &author).await;
    let prefs = UpdateFollowPrefsRequest {
        user_id: author.id.to_string(),
        notify_level: Some(FollowNotifyLevel::FOLLOW_NOTIFY_LEVEL_ALL.into()),
        ..Default::default()
    };
    let _: UpdateFollowPrefsResponse =
        ctx.app.call("tokuly.social.v1.FollowService/UpdateFollowPrefs", Some(&fan), &prefs).await.unwrap();
    let note = ctx.post(&author, "新しいノート").await;
    let friends_only =
        ctx.post_visible_to(&author, "ともだちだけ", tokuly_proto::common::Visibility::VISIBILITY_FRIENDS).await;
    assert_eq!(ctx.notes.notify_post(id(&note.id)).await.unwrap(), 1);
    assert_eq!(ctx.notes.notify_post(id(&friends_only.id)).await.unwrap(), 0, "not for followers who may not see it");
    assert_eq!(ctx.notes.notify_post(id(&note.id)).await.unwrap(), 0, "told once");
    assert_eq!(ctx.notifications_for(fan.id, "notes.post").await.len(), 1);
    assert!(ctx.notifications_for(quiet.id, "notes.post").await.is_empty());
}
