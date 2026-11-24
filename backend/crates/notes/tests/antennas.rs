//! Antennas: managing them, what the background match collects (keywords,
//! sources, replies, media, visibility, blocks), idempotency, trimming,
//! notifications, limits under concurrency and erasure.
//!
//! Antennas of every test in this binary share the database, so each test
//! matches on words of its own ([`unique`]).
mod common;

use common::*;
use serde::{Serialize, de::DeserializeOwned};
use tokuly_proto::{common::Visibility, notes::*};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

async fn call<Req: Serialize, Resp: DeserializeOwned>(
    ctx: &Ctx,
    method: &str,
    user: Option<&TestUser>,
    request: &Req,
) -> Result<Resp, RpcError> {
    ctx.app.call(&format!("tokuly.notes.v1.AntennaService/{method}"), user, request).await
}

async fn reason<Req: Serialize>(ctx: &Ctx, method: &str, user: Option<&TestUser>, request: &Req) -> String {
    match call::<_, serde_json::Value>(ctx, method, user, request).await {
        Ok(response) => panic!("AntennaService/{method} should fail, got {response}"),
        Err(error) => error.reason,
    }
}

fn group(words: &[&str]) -> KeywordGroup {
    KeywordGroup { words: words.iter().map(|word| word.to_string()).collect(), ..Default::default() }
}

fn input(name: &str, keywords: &[&[&str]]) -> AntennaInput {
    AntennaInput {
        name: name.to_owned(),
        keywords: keywords.iter().map(|words| group(words)).collect(),
        ..Default::default()
    }
}

async fn create(ctx: &Ctx, owner: &TestUser, antenna: AntennaInput) -> Antenna {
    let response: CreateAntennaResponse = call(
        ctx,
        "CreateAntenna",
        Some(owner),
        &CreateAntennaRequest { antenna: antenna.into(), ..Default::default() },
    )
    .await
    .expect("create antenna");
    response.antenna.as_option().cloned().expect("the antenna")
}

async fn create_reason(ctx: &Ctx, owner: &TestUser, antenna: AntennaInput) -> String {
    reason(ctx, "CreateAntenna", Some(owner), &CreateAntennaRequest { antenna: antenna.into(), ..Default::default() })
        .await
}

async fn collected(ctx: &Ctx, owner: &TestUser, antenna: &Antenna) -> Vec<String> {
    let response: GetAntennaTimelineResponse = call(
        ctx,
        "GetAntennaTimeline",
        Some(owner),
        &GetAntennaTimelineRequest { antenna_id: antenna.id.clone(), page: page(100, ""), ..Default::default() },
    )
    .await
    .expect("antenna timeline");
    texts(&response.entries).iter().map(|text| text.to_string()).collect()
}

/// Runs the `notes.antenna_match` job for a note, as the worker would.
async fn matched(ctx: &Ctx, note: &Note) -> usize {
    ctx.notes.match_antennas(id(&note.id)).await.expect("antenna match")
}

/// Posts a note and runs the match for it.
async fn post_matched(ctx: &Ctx, author: &TestUser, text: &str) -> Note {
    let note = ctx.post(author, text).await;
    matched(ctx, &note).await;
    note
}

async fn create_list(ctx: &Ctx, owner: &TestUser, members: &[&TestUser]) -> String {
    let list: CreateListResponse = ctx
        .app
        .call(
            "tokuly.notes.v1.NoteListService/CreateList",
            Some(owner),
            &CreateListRequest { name: "list".into(), ..Default::default() },
        )
        .await
        .unwrap();
    let list_id = list.list.id.clone();
    let _: AddListMembersResponse = ctx
        .app
        .call(
            "tokuly.notes.v1.NoteListService/AddListMembers",
            Some(owner),
            &AddListMembersRequest {
                list_id: list_id.clone(),
                user_ids: members.iter().map(|member| member.id.to_string()).collect(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    list_id
}

#[tokio::test]
async fn antennas_are_managed_by_their_owner_only() {
    let ctx = start().await;
    let (owner, other) = (ctx.member("owner").await, ctx.member("other").await);
    let word = unique("kw");
    let antenna = create(&ctx, &owner, input(" 推しの話 ", &[&[&word, "ＬＩＶＥ"]])).await;
    assert_eq!(antenna.name, "推しの話");
    assert_eq!(antenna.keywords[0].words, vec![word.clone(), "live".to_owned()], "stored folded");
    assert_eq!(antenna.source, AntennaSource::ANTENNA_SOURCE_EVERYONE);

    let get = GetAntennaRequest { antenna_id: antenna.id.clone(), ..Default::default() };
    let got: GetAntennaResponse = call(&ctx, "GetAntenna", Some(&owner), &get).await.unwrap();
    assert_eq!(got.antenna.id, antenna.id);
    assert_eq!(reason(&ctx, "GetAntenna", Some(&other), &get).await, "ANTENNA_NOT_FOUND");
    assert_eq!(reason(&ctx, "GetAntenna", None, &get).await, "SIGN_IN_REQUIRED");
    let timeline = GetAntennaTimelineRequest { antenna_id: antenna.id.clone(), ..Default::default() };
    assert_eq!(reason(&ctx, "GetAntennaTimeline", Some(&other), &timeline).await, "ANTENNA_NOT_FOUND");
    let listed: ListAntennasResponse =
        call(&ctx, "ListAntennas", Some(&other), &ListAntennasRequest::default()).await.unwrap();
    assert!(listed.antennas.is_empty());

    let mut changed = input("renamed", &[&[&word]]);
    changed.with_replies = true;
    changed.notify = true;
    let update = UpdateAntennaRequest { antenna_id: antenna.id.clone(), antenna: changed.into(), ..Default::default() };
    assert_eq!(reason(&ctx, "UpdateAntenna", Some(&other), &update).await, "ANTENNA_NOT_FOUND");
    let updated: UpdateAntennaResponse = call(&ctx, "UpdateAntenna", Some(&owner), &update).await.unwrap();
    assert_eq!(
        (updated.antenna.name.as_str(), updated.antenna.with_replies, updated.antenna.notify),
        ("renamed", true, true)
    );
    let listed: ListAntennasResponse =
        call(&ctx, "ListAntennas", Some(&owner), &ListAntennasRequest::default()).await.unwrap();
    assert_eq!(listed.antennas.len(), 1);

    let delete = DeleteAntennaRequest { antenna_id: antenna.id.clone(), ..Default::default() };
    assert_eq!(reason(&ctx, "DeleteAntenna", Some(&other), &delete).await, "ANTENNA_NOT_FOUND");
    let _: DeleteAntennaResponse = call(&ctx, "DeleteAntenna", Some(&owner), &delete).await.unwrap();
    assert_eq!(reason(&ctx, "GetAntenna", Some(&owner), &get).await, "ANTENNA_NOT_FOUND");
    assert_eq!(reason(&ctx, "DeleteAntenna", Some(&owner), &delete).await, "ANTENNA_NOT_FOUND");
}

#[tokio::test]
async fn antenna_input_is_validated() {
    let ctx = start().await;
    let (owner, other) = (ctx.member("owner").await, ctx.member("other").await);
    assert_eq!(create_reason(&ctx, &owner, input("", &[&["a"]])).await, "INVALID_NAME");
    assert_eq!(create_reason(&ctx, &owner, input(&"a".repeat(31), &[&["a"]])).await, "INVALID_NAME");
    assert_eq!(create_reason(&ctx, &owner, input("x", &[])).await, "INVALID_KEYWORDS");
    assert_eq!(create_reason(&ctx, &owner, input("x", &[&["1", "2", "3", "4", "5", "6"]])).await, "INVALID_KEYWORDS");
    let mut too_many = input("x", &[&["a"]]);
    too_many.exclude_words = (0..11).map(|i| format!("w{i}")).collect();
    assert_eq!(create_reason(&ctx, &owner, too_many).await, "INVALID_KEYWORDS");

    // Someone else's list, and accounts that do not exist or are blocked.
    let theirs = create_list(&ctx, &other, &[]).await;
    let mut list = input("x", &[]);
    list.source = AntennaSource::ANTENNA_SOURCE_LIST.into();
    list.list_id = theirs;
    assert_eq!(create_reason(&ctx, &owner, list).await, "LIST_NOT_FOUND");
    let mut users = input("x", &[]);
    users.source = AntennaSource::ANTENNA_SOURCE_USERS.into();
    users.user_ids = vec![Uuid::now_v7().to_string()];
    assert_eq!(create_reason(&ctx, &owner, users.clone()).await, "USER_NOT_FOUND");
    ctx.block(&other, &owner).await;
    users.user_ids = vec![other.id.to_string()];
    assert_eq!(create_reason(&ctx, &owner, users.clone()).await, "USER_NOT_FOUND");
    users.user_ids.clear();
    assert_eq!(create_reason(&ctx, &owner, users).await, "INVALID_USERS");
    assert_eq!(
        reason(
            &ctx,
            "CreateAntenna",
            None,
            &CreateAntennaRequest { antenna: input("x", &[&["a"]]).into(), ..Default::default() }
        )
        .await,
        "SIGN_IN_REQUIRED"
    );
}

#[tokio::test]
async fn new_notes_are_collected_by_their_words_once() {
    let ctx = start().await;
    let (owner, author) = (ctx.member("owner").await, ctx.member("author").await);
    let (ramen, shibuya, noodle) = (unique("ramen"), unique("shibuya"), unique("noodle"));
    let mut wanted = input("食べ物", &[&[&ramen, &shibuya], &[&noodle]]);
    wanted.exclude_words = vec!["閉店".into()];
    wanted.notify = true;
    let antenna = create(&ctx, &owner, wanted).await;

    let both = ctx.post(&author, &format!("{shibuya}で{}食べた", ramen.to_uppercase())).await;
    assert_eq!(matched(&ctx, &both).await, 1);
    assert_eq!(matched(&ctx, &both).await, 0, "the job running twice collects nothing more");
    post_matched(&ctx, &author, &format!("{ramen}だけ")).await;
    post_matched(&ctx, &author, &format!("{noodle}閉店しちゃった")).await;
    let fullwidth: String = noodle
        .chars()
        .map(|c| if c.is_ascii_lowercase() { char::from_u32(c as u32 - 0x61 + 0xFF41).unwrap() } else { c })
        .collect();
    post_matched(&ctx, &author, &format!("{fullwidth}うまい")).await;
    assert_eq!(
        collected(&ctx, &owner, &antenna).await,
        vec![format!("{fullwidth}うまい"), format!("{shibuya}で{}食べた", ramen.to_uppercase())]
    );
    let got: GetAntennaResponse = call(
        &ctx,
        "GetAntenna",
        Some(&owner),
        &GetAntennaRequest { antenna_id: antenna.id.clone(), ..Default::default() },
    )
    .await
    .unwrap();
    assert!(got.antenna.last_matched_at.as_option().is_some());

    // Told once (collapsed and throttled per antenna), not per note.
    let told = ctx.notifications_for(owner.id, "notes.antenna").await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["collapse_key"], format!("notes.antenna:{}", antenna.id));
    assert_eq!(told[0].data["path"], format!("/notes/antennas/{}", antenna.id));
}

#[tokio::test]
async fn replies_and_media_are_collected_only_when_asked_for() {
    let ctx = start().await;
    let (owner, author) = (ctx.member("owner").await, ctx.member("author").await);
    let word = unique("kw");
    let plain = create(&ctx, &owner, input("plain", &[&[&word]])).await;
    let mut replies = input("replies", &[&[&word]]);
    replies.with_replies = true;
    let replies = create(&ctx, &owner, replies).await;
    let mut media = input("media", &[&[&word]]);
    media.media_only = true;
    let media = create(&ctx, &owner, media).await;

    let first = post_matched(&ctx, &author, &format!("{word} top")).await;
    let reply = ctx.reply(&author, &first, &format!("{word} reply")).await;
    matched(&ctx, &reply).await;
    let image = ctx.note_image(&author).await;
    let photo = ctx.post_with(&author, &format!("{word} photo"), |request| request.asset_ids = vec![image]).await;
    matched(&ctx, &photo).await;

    assert_eq!(collected(&ctx, &owner, &plain).await, vec![format!("{word} photo"), format!("{word} top")]);
    assert_eq!(
        collected(&ctx, &owner, &replies).await,
        vec![format!("{word} photo"), format!("{word} reply"), format!("{word} top")]
    );
    assert_eq!(collected(&ctx, &owner, &media).await, vec![format!("{word} photo")]);
}

#[tokio::test]
async fn an_antenna_only_collects_what_its_owner_may_see() {
    let ctx = start().await;
    let (author, follower, stranger, blocker) = (
        ctx.member("author").await,
        ctx.member("follower").await,
        ctx.member("stranger").await,
        ctx.member("blocker").await,
    );
    ctx.follow(&follower, &author).await;
    let word = unique("kw");
    let for_follower = create(&ctx, &follower, input("a", &[&[&word]])).await;
    let for_stranger = create(&ctx, &stranger, input("a", &[&[&word]])).await;
    let for_blocker = create(&ctx, &blocker, input("a", &[&[&word]])).await;
    ctx.block(&blocker, &author).await;

    let note = ctx.post_visible_to(&author, &format!("{word} followers only"), Visibility::VISIBILITY_FOLLOWERS).await;
    assert_eq!(matched(&ctx, &note).await, 1);
    post_matched(&ctx, &author, &format!("{word} public")).await;
    assert_eq!(
        collected(&ctx, &follower, &for_follower).await,
        vec![format!("{word} public"), format!("{word} followers only")]
    );
    assert_eq!(collected(&ctx, &stranger, &for_stranger).await, vec![format!("{word} public")]);
    assert!(collected(&ctx, &blocker, &for_blocker).await.is_empty(), "nothing across a block");

    // Collected, then hidden: the timeline only shows what is visible now.
    ctx.block(&author, &follower).await;
    assert!(collected(&ctx, &follower, &for_follower).await.is_empty());
}

#[tokio::test]
async fn sources_narrow_where_an_antenna_looks() {
    let ctx = start().await;
    let (owner, followed, friend, member, named, stranger) = (
        ctx.member("owner").await,
        ctx.member("followed").await,
        ctx.member("friend").await,
        ctx.member("member").await,
        ctx.member("named").await,
        ctx.member("stranger").await,
    );
    ctx.follow(&owner, &followed).await;
    ctx.befriend(&owner, &friend).await;
    let word = unique("kw");
    let mut following = input("following", &[&[&word]]);
    following.source = AntennaSource::ANTENNA_SOURCE_FOLLOWING.into();
    let following = create(&ctx, &owner, following).await;
    let mut list = input("list", &[]);
    list.source = AntennaSource::ANTENNA_SOURCE_LIST.into();
    list.list_id = create_list(&ctx, &owner, &[&member]).await;
    let list = create(&ctx, &owner, list).await;
    let mut users = input("users", &[&[&word]]);
    users.source = AntennaSource::ANTENNA_SOURCE_USERS.into();
    users.user_ids = vec![named.id.to_string()];
    let users = create(&ctx, &owner, users).await;
    assert_eq!(users.users.len(), 1);
    assert_eq!(users.users[0].id, named.id.to_string());

    for author in [&followed, &friend, &member, &named, &stranger] {
        post_matched(&ctx, author, &format!("{word} from {}", author.id)).await;
    }
    post_matched(&ctx, &member, "member without the word").await;
    assert_eq!(
        collected(&ctx, &owner, &following).await,
        vec![format!("{word} from {}", friend.id), format!("{word} from {}", followed.id)]
    );
    assert_eq!(
        collected(&ctx, &owner, &list).await,
        vec!["member without the word".to_owned(), format!("{word} from {}", member.id)],
        "a list antenna without words takes everything of its members"
    );
    assert_eq!(collected(&ctx, &owner, &users).await, vec![format!("{word} from {}", named.id)]);
}

#[tokio::test]
async fn changes_apply_to_the_next_notes() {
    let ctx = start().await;
    let (owner, author) = (ctx.member("owner").await, ctx.member("author").await);
    let (first, second) = (unique("first"), unique("second"));
    let antenna = create(&ctx, &owner, input("a", &[&[&first]])).await;
    post_matched(&ctx, &author, &format!("{first} one")).await;
    // The matcher is cached; a new antenna and a changed one are seen at once.
    let other = create(&ctx, &owner, input("b", &[&[&second]])).await;
    let update = UpdateAntennaRequest {
        antenna_id: antenna.id.clone(),
        antenna: input("a", &[&[&second]]).into(),
        ..Default::default()
    };
    let _: UpdateAntennaResponse = call(&ctx, "UpdateAntenna", Some(&owner), &update).await.unwrap();
    post_matched(&ctx, &author, &format!("{first} two")).await;
    post_matched(&ctx, &author, &format!("{second} three")).await;
    assert_eq!(collected(&ctx, &owner, &antenna).await, vec![format!("{second} three"), format!("{first} one")]);
    assert_eq!(collected(&ctx, &owner, &other).await, vec![format!("{second} three")]);
}

#[tokio::test]
async fn an_antenna_keeps_its_latest_thousand_notes() {
    let ctx = start().await;
    let (owner, author) = (ctx.member("owner").await, ctx.member("author").await);
    let word = unique("kw");
    let antenna = create(&ctx, &owner, input("a", &[&[&word]])).await;
    let antenna_id = id(&antenna.id);
    sqlx::query(
        "WITH made AS (INSERT INTO notes (author_id, text, visibility, listed) SELECT $1, 'old ' || g, 1, true FROM generate_series(1, 1000) g RETURNING id)
         INSERT INTO antenna_notes (antenna_id, note_id) SELECT $2, id FROM made",
    )
    .bind(author.id)
    .bind(antenna_id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let oldest: Uuid =
        sqlx::query_scalar("SELECT note_id FROM antenna_notes WHERE antenna_id = $1 ORDER BY note_id LIMIT 1")
            .bind(antenna_id)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    post_matched(&ctx, &author, &format!("{word} newest")).await;
    let (count, still_there): (i64, bool) =
        sqlx::query_as("SELECT count(*), bool_or(note_id = $2) FROM antenna_notes WHERE antenna_id = $1")
            .bind(antenna_id)
            .bind(oldest)
            .fetch_one(&ctx.app.core.db)
            .await
            .unwrap();
    assert_eq!((count, still_there), (1000, false));
    assert_eq!(collected(&ctx, &owner, &antenna).await[0], format!("{word} newest"));
}

#[tokio::test]
async fn the_antenna_limit_holds_under_concurrent_creation() {
    let ctx = start().await;
    let owner = ctx.member("owner").await;
    create(&ctx, &owner, input("one", &[&[&unique("kw")]])).await;
    create(&ctx, &owner, input("two", &[&[&unique("kw")]])).await;
    let request = CreateAntennaRequest { antenna: input("race", &[&[&unique("kw")]]).into(), ..Default::default() };
    // As many calls at once as the test pool has connections.
    let attempts = futures::future::join_all(
        (0..5).map(|_| call::<_, CreateAntennaResponse>(&ctx, "CreateAntenna", Some(&owner), &request)),
    )
    .await;
    assert_eq!(attempts.iter().filter(|attempt| attempt.is_ok()).count(), 3, "{attempts:?}");
    assert!(attempts.iter().filter_map(|attempt| attempt.as_ref().err()).all(|e| e.reason == "TOO_MANY_ANTENNAS"));
    let listed: ListAntennasResponse =
        call(&ctx, "ListAntennas", Some(&owner), &ListAntennasRequest::default()).await.unwrap();
    assert_eq!(listed.antennas.len(), 5);
}

#[tokio::test]
async fn deleting_a_list_or_an_account_takes_its_antennas_along() {
    let ctx = start().await;
    let (owner, member, named) = (ctx.member("owner").await, ctx.member("member").await, ctx.member("named").await);
    let mut list = input("list", &[]);
    list.source = AntennaSource::ANTENNA_SOURCE_LIST.into();
    list.list_id = create_list(&ctx, &owner, &[&member]).await;
    let list_id = list.list_id.clone();
    let on_list = create(&ctx, &owner, list).await;
    let _: DeleteListResponse = ctx
        .app
        .call(
            "tokuly.notes.v1.NoteListService/DeleteList",
            Some(&owner),
            &DeleteListRequest { list_id, ..Default::default() },
        )
        .await
        .unwrap();
    let get = GetAntennaRequest { antenna_id: on_list.id.clone(), ..Default::default() };
    assert_eq!(reason(&ctx, "GetAntenna", Some(&owner), &get).await, "ANTENNA_NOT_FOUND");

    let mut users = input("users", &[&[&unique("kw")]]);
    users.source = AntennaSource::ANTENNA_SOURCE_USERS.into();
    users.user_ids = vec![named.id.to_string(), member.id.to_string()];
    let users = create(&ctx, &owner, users).await;
    let exporter = ctx.app.core.exporters.all().into_iter().find(|exporter| exporter.name() == "notes").unwrap();
    assert_eq!(exporter.export(owner.id).await.unwrap()["antennas"][0]["name"], "users");

    // Someone it names is erased: the antenna stops naming them.
    let erase = |user: Uuid| tokuly_core::events::Event {
        id: Uuid::now_v7(),
        topic: "identity.user.deleted".into(),
        actor_id: None,
        occurred_at: chrono::Utc::now(),
        data: serde_json::json!({ "user_id": user }),
    };
    ctx.notes.handle_event(erase(named.id)).await.unwrap();
    let got: GetAntennaResponse = call(
        &ctx,
        "GetAntenna",
        Some(&owner),
        &GetAntennaRequest { antenna_id: users.id.clone(), ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(got.antenna.users.iter().map(|user| user.id.clone()).collect::<Vec<_>>(), vec![member.id.to_string()]);
    // The owner is erased: the antenna goes.
    ctx.notes.handle_event(erase(owner.id)).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM antennas WHERE owner_id = $1")
        .bind(owner.id)
        .fetch_one(&ctx.app.core.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
}
