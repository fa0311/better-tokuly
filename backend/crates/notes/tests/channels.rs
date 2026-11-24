//! Channels: creating and editing them, following, moderators, pins,
//! taking notes out, the channel timeline, and how channel notes stay out
//! of the general timelines.
mod common;

use common::*;
use futures::future::join_all;
use serde::{Serialize, de::DeserializeOwned};
use tokuly_core::events::Event;
use tokuly_proto::{
    common::Visibility,
    media::{AssetKind, AssetPurpose},
    notes::*,
};
use tokuly_testkit::{RpcError, TestUser};
use uuid::Uuid;

fn event(topic: &str, data: serde_json::Value) -> Event {
    Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: None, occurred_at: chrono::Utc::now(), data }
}

trait Channels {
    async fn channel<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError>;
    async fn channel_reason<Req: Serialize>(&self, method: &str, user: Option<&TestUser>, request: &Req) -> String;
    async fn create_channel(&self, owner: &TestUser, name: &str) -> NoteChannel;
    async fn get_channel(&self, viewer: Option<&TestUser>, channel: &NoteChannel) -> Result<NoteChannel, RpcError>;
    async fn follow_channel(&self, user: &TestUser, channel: &NoteChannel) -> NoteChannel;
    async fn post_in(&self, user: &TestUser, channel: &NoteChannel, text: &str) -> Note;
    async fn channel_timeline(&self, viewer: Option<&TestUser>, channel: &NoteChannel) -> GetChannelTimelineResponse;
    async fn add_moderator(&self, owner: &TestUser, channel: &NoteChannel, user: &TestUser);
    async fn scalar(&self, sql: &str, about: Uuid) -> i64;
}

impl Channels for Ctx {
    async fn channel<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        method: &str,
        user: Option<&TestUser>,
        request: &Req,
    ) -> Result<Resp, RpcError> {
        self.app.call(&format!("tokuly.notes.v1.NoteChannelService/{method}"), user, request).await
    }

    async fn channel_reason<Req: Serialize>(&self, method: &str, user: Option<&TestUser>, request: &Req) -> String {
        match self.channel::<_, serde_json::Value>(method, user, request).await {
            Ok(response) => panic!("NoteChannelService/{method} should fail, got {response}"),
            Err(error) => error.reason,
        }
    }

    async fn create_channel(&self, owner: &TestUser, name: &str) -> NoteChannel {
        let request = CreateChannelRequest { name: name.to_owned(), ..Default::default() };
        let created: CreateChannelResponse = self.channel("CreateChannel", Some(owner), &request).await.unwrap();
        created.channel.as_option().cloned().unwrap()
    }

    async fn get_channel(&self, viewer: Option<&TestUser>, channel: &NoteChannel) -> Result<NoteChannel, RpcError> {
        let request = GetChannelRequest { channel_id: channel.id.clone(), ..Default::default() };
        let got: GetChannelResponse = self.channel("GetChannel", viewer, &request).await?;
        Ok(got.channel.as_option().cloned().unwrap())
    }

    async fn follow_channel(&self, user: &TestUser, channel: &NoteChannel) -> NoteChannel {
        let request = FollowChannelRequest { channel_id: channel.id.clone(), ..Default::default() };
        let followed: FollowChannelResponse = self.channel("FollowChannel", Some(user), &request).await.unwrap();
        followed.channel.as_option().cloned().unwrap()
    }

    async fn post_in(&self, user: &TestUser, channel: &NoteChannel, text: &str) -> Note {
        self.post_with(user, text, |request| request.channel_id = channel.id.clone()).await
    }

    async fn channel_timeline(&self, viewer: Option<&TestUser>, channel: &NoteChannel) -> GetChannelTimelineResponse {
        let request = GetChannelTimelineRequest { channel_id: channel.id.clone(), ..Default::default() };
        self.channel("GetChannelTimeline", viewer, &request).await.unwrap()
    }

    async fn add_moderator(&self, owner: &TestUser, channel: &NoteChannel, user: &TestUser) {
        let request = AddChannelModeratorRequest {
            channel_id: channel.id.clone(),
            user_id: user.id.to_string(),
            ..Default::default()
        };
        let _: AddChannelModeratorResponse = self.channel("AddChannelModerator", Some(owner), &request).await.unwrap();
    }

    async fn scalar(&self, sql: &str, about: Uuid) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned())).bind(about).fetch_one(&self.app.core.db).await.unwrap()
    }
}

fn create(name: &str, fill: impl FnOnce(&mut CreateChannelRequest)) -> CreateChannelRequest {
    let mut request = CreateChannelRequest { name: name.to_owned(), ..Default::default() };
    fill(&mut request);
    request
}

fn update_of(channel: &NoteChannel, fill: impl FnOnce(&mut UpdateChannelRequest)) -> UpdateChannelRequest {
    let mut request = UpdateChannelRequest {
        channel_id: channel.id.clone(),
        name: channel.name.clone(),
        description: channel.description.clone(),
        color: channel.color.clone(),
        banner_asset_id: String::new(),
        ..Default::default()
    };
    fill(&mut request);
    request
}

fn pin(channel: &NoteChannel, note: &Note) -> PinChannelNoteRequest {
    PinChannelNoteRequest { channel_id: channel.id.clone(), note_id: note.id.clone(), ..Default::default() }
}

fn removal(channel: &NoteChannel, note: &Note) -> RemoveChannelNoteRequest {
    RemoveChannelNoteRequest { channel_id: channel.id.clone(), note_id: note.id.clone(), ..Default::default() }
}

fn list(filter: ChannelFilter, query: &str) -> ListChannelsRequest {
    ListChannelsRequest { filter: filter.into(), query: query.to_owned(), page: page(50, ""), ..Default::default() }
}

fn ids(channels: &[NoteChannel]) -> Vec<&str> {
    channels.iter().map(|channel| channel.id.as_str()).collect()
}

#[tokio::test]
async fn a_channel_is_created_with_a_banner_and_read_by_anyone() {
    let ctx = start().await;
    let owner = ctx.member("owner").await;
    let banner = ctx.note_image(&owner).await;
    let request = create("  ゲーム   部 ", |r| {
        r.description = "ゲームの話をしよう!".into();
        r.color = "#FF8800".into();
        r.banner_asset_id = banner.clone();
    });
    let created: CreateChannelResponse = ctx.channel("CreateChannel", Some(&owner), &request).await.unwrap();
    let channel = created.channel.as_option().cloned().unwrap();
    assert_eq!(channel.name, "ゲーム 部");
    assert_eq!(channel.color, "#ff8800");
    assert!(channel.is_owner && !channel.archived && !channel.following);
    assert!(channel.banner.as_option().is_some_and(|image| !image.url.is_empty()));
    assert_eq!(channel.owner.id, owner.id.to_string());
    assert!(tokuly_media::assets::is_attached(&ctx.app.core, id(&banner)).await.unwrap());
    assert!(ctx.is_indexed("note_channel", id(&channel.id)).await);

    let seen = ctx.get_channel(None, &channel).await.unwrap();
    assert_eq!((seen.name.as_str(), seen.is_owner), ("ゲーム 部", false));
    let missing = NoteChannel { id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.get_channel(None, &missing).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");
}

#[tokio::test]
async fn creating_a_channel_checks_its_input() {
    let ctx = start().await;
    let (owner, other) = (ctx.member("owner").await, ctx.member("other").await);
    let others_image = ctx.note_image(&other).await;
    let story_image = ctx.story_image(&owner).await;
    let cases = [
        (create(" ", |_| {}), "INVALID_NAME"),
        (create(&"名".repeat(51), |_| {}), "INVALID_NAME"),
        (create("ok", |r| r.description = "x".repeat(501)), "DESCRIPTION_TOO_LONG"),
        (create("ok", |r| r.color = "orange".into()), "INVALID_COLOR"),
        (create("ok", |r| r.banner_asset_id = others_image.clone()), "ASSET_NOT_FOUND"),
        (create("ok", |r| r.banner_asset_id = story_image.clone()), "ASSET_WRONG_PURPOSE"),
    ];
    for (request, reason) in cases {
        assert_eq!(ctx.channel_reason("CreateChannel", Some(&owner), &request).await, reason);
    }
    let processing = tokuly_media::testing::ready_asset(
        &ctx.app.core,
        owner.id,
        AssetPurpose::ASSET_PURPOSE_NOTE_IMAGE,
        AssetKind::ASSET_KIND_IMAGE,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE media_assets SET status = 2 WHERE id = $1")
        .bind(processing.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let request = create("ok", |r| r.banner_asset_id = processing.id.to_string());
    assert_eq!(ctx.channel_reason("CreateChannel", Some(&owner), &request).await, "ASSET_NOT_READY");
    let error =
        ctx.channel::<_, CreateChannelResponse>("CreateChannel", None, &create("ok", |_| {})).await.unwrap_err();
    assert_eq!(error.code, "unauthenticated");
}

#[tokio::test]
async fn one_person_owns_at_most_ten_channels_even_when_creating_at_once() {
    let ctx = start().await;
    let owner = ctx.member("owner").await;
    sqlx::query("INSERT INTO note_channels (owner_id, name) SELECT $1, 'old ' || g FROM generate_series(1, 9) g")
        .bind(owner.id)
        .execute(&ctx.app.core.db)
        .await
        .unwrap();
    let results = join_all((0..4).map(|i| {
        let request = create(&format!("new {i}"), |_| {});
        let (ctx, owner) = (&ctx, &owner);
        async move { ctx.channel::<_, CreateChannelResponse>("CreateChannel", Some(owner), &request).await }
    }))
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    for error in results.into_iter().filter_map(Result::err) {
        assert_eq!(error.reason, "TOO_MANY_CHANNELS");
    }
    assert_eq!(ctx.scalar("SELECT count(*) FROM note_channels WHERE owner_id = $1", owner.id).await, 10);
}

#[tokio::test]
async fn owners_and_moderators_edit_a_channel_and_its_banner() {
    let ctx = start().await;
    let (owner, moderator, stranger) =
        (ctx.member("owner").await, ctx.member("moderator").await, ctx.member("stranger").await);
    let first = ctx.note_image(&owner).await;
    let request = create("art", |r| r.banner_asset_id = first.clone());
    let created: CreateChannelResponse = ctx.channel("CreateChannel", Some(&owner), &request).await.unwrap();
    let channel = created.channel.as_option().cloned().unwrap();
    ctx.add_moderator(&owner, &channel, &moderator).await;

    // A moderator replaces the banner with their own upload.
    let second = ctx.note_image(&moderator).await;
    let request = update_of(&channel, |r| {
        r.name = "art club".into();
        r.description = "絵を見せ合おう!".into();
        r.banner_asset_id = second.clone();
        r.color = "#123abc".into();
    });
    let updated: UpdateChannelResponse = ctx.channel("UpdateChannel", Some(&moderator), &request).await.unwrap();
    let updated = updated.channel.as_option().cloned().unwrap();
    assert_eq!((updated.name.as_str(), updated.color.as_str()), ("art club", "#123abc"));
    assert!(updated.is_moderator && !updated.is_owner);
    assert!(!tokuly_media::assets::is_attached(&ctx.app.core, id(&first)).await.unwrap());
    assert!(tokuly_media::assets::is_attached(&ctx.app.core, id(&second)).await.unwrap());

    // The owner keeps it (same asset, someone else's upload) and then removes it.
    let keep = update_of(&updated, |r| r.banner_asset_id = second.clone());
    let _: UpdateChannelResponse = ctx.channel("UpdateChannel", Some(&owner), &keep).await.unwrap();
    let clear = update_of(&updated, |_| {});
    let cleared: UpdateChannelResponse = ctx.channel("UpdateChannel", Some(&owner), &clear).await.unwrap();
    assert!(cleared.channel.banner.as_option().is_none());
    assert!(!tokuly_media::assets::is_attached(&ctx.app.core, id(&second)).await.unwrap());

    assert_eq!(ctx.channel_reason("UpdateChannel", Some(&stranger), &clear).await, "NOT_CHANNEL_MODERATOR");
    assert_eq!(
        ctx.channel_reason("UpdateChannel", Some(&owner), &update_of(&updated, |r| r.name = String::new())).await,
        "INVALID_NAME"
    );
    let missing =
        update_of(&NoteChannel { id: Uuid::now_v7().to_string(), name: "x".into(), ..Default::default() }, |_| {});
    assert_eq!(ctx.channel_reason("UpdateChannel", Some(&owner), &missing).await, "CHANNEL_NOT_FOUND");

    let archive = ArchiveChannelRequest { channel_id: channel.id.clone(), archived: true, ..Default::default() };
    let _: ArchiveChannelResponse = ctx.channel("ArchiveChannel", Some(&owner), &archive).await.unwrap();
    assert_eq!(ctx.channel_reason("UpdateChannel", Some(&owner), &clear).await, "CHANNEL_ARCHIVED");
}

#[tokio::test]
async fn an_archived_channel_takes_no_new_notes_until_it_is_opened_again() {
    let ctx = start().await;
    let (owner, moderator, member) =
        (ctx.member("owner").await, ctx.member("moderator").await, ctx.member("member").await);
    let channel = ctx.create_channel(&owner, "quiet").await;
    ctx.add_moderator(&owner, &channel, &moderator).await;
    ctx.post_in(&member, &channel, "before").await;

    let archive = ArchiveChannelRequest { channel_id: channel.id.clone(), archived: true, ..Default::default() };
    assert_eq!(ctx.channel_reason("ArchiveChannel", Some(&moderator), &archive).await, "NOT_CHANNEL_OWNER");
    assert_eq!(ctx.channel_reason("ArchiveChannel", Some(&member), &archive).await, "NOT_CHANNEL_OWNER");
    let archived: ArchiveChannelResponse = ctx.channel("ArchiveChannel", Some(&owner), &archive).await.unwrap();
    assert!(archived.channel.archived);
    assert!(!ctx.is_indexed("note_channel", id(&channel.id)).await, "archived channels leave search");
    let reason = ctx.post_reason(&member, "after", |r| r.channel_id = channel.id.clone()).await;
    assert_eq!(reason, "CHANNEL_ARCHIVED");
    // Still readable.
    assert_eq!(texts(&ctx.channel_timeline(None, &channel).await.entries), ["before"]);

    let reopen = ArchiveChannelRequest { channel_id: channel.id.clone(), archived: false, ..Default::default() };
    let reopened: ArchiveChannelResponse = ctx.channel("ArchiveChannel", Some(&owner), &reopen).await.unwrap();
    assert!(!reopened.channel.archived);
    assert!(ctx.is_indexed("note_channel", id(&channel.id)).await);
    ctx.post_in(&member, &channel, "after").await;
    assert_eq!(ctx.get_channel(None, &channel).await.unwrap().note_count, 2);
    let missing =
        ArchiveChannelRequest { channel_id: Uuid::now_v7().to_string(), archived: true, ..Default::default() };
    assert_eq!(ctx.channel_reason("ArchiveChannel", Some(&owner), &missing).await, "CHANNEL_NOT_FOUND");
}

#[tokio::test]
async fn channel_notes_stay_out_of_general_timelines_unless_the_reader_follows_the_channel() {
    let ctx = start().await;
    let (author, friend, follower) =
        (ctx.member("author").await, ctx.member("friend").await, ctx.member("follower").await);
    ctx.befriend(&author, &friend).await;
    ctx.follow(&follower, &author).await;
    let channel = ctx.create_channel(&author, "topic").await;
    let plain = ctx.post(&author, "plain note").await;
    let in_channel = ctx.post_in(&author, &channel, "channel note").await;
    assert_eq!(in_channel.channel.id, channel.id);
    assert_eq!(in_channel.channel.name, "topic");
    assert_eq!(in_channel.visibility, Visibility::VISIBILITY_PUBLIC);

    let home = |viewer: &TestUser| {
        let viewer = viewer.clone();
        let ctx = &ctx;
        async move {
            let response: GetHomeTimelineResponse =
                ctx.note("GetHomeTimeline", Some(&viewer), &GetHomeTimelineRequest::default()).await.unwrap();
            response.entries.iter().map(|entry| entry.note.id.clone()).collect::<Vec<_>>()
        }
    };
    let following = |viewer: &TestUser| {
        let viewer = viewer.clone();
        let ctx = &ctx;
        async move {
            let response: GetFollowingTimelineResponse =
                ctx.note("GetFollowingTimeline", Some(&viewer), &GetFollowingTimelineRequest::default()).await.unwrap();
            response.entries.iter().map(|entry| entry.note.id.clone()).collect::<Vec<_>>()
        }
    };
    let global = || async {
        let response: GetGlobalTimelineResponse = ctx
            .note("GetGlobalTimeline", None, &GetGlobalTimelineRequest { page: page(100, ""), ..Default::default() })
            .await
            .unwrap();
        response.entries.iter().map(|entry| entry.note.id.clone()).collect::<Vec<_>>()
    };
    assert!(home(&friend).await.contains(&plain.id) && !home(&friend).await.contains(&in_channel.id));
    assert!(!following(&follower).await.contains(&in_channel.id));
    assert!(global().await.contains(&plain.id) && !global().await.contains(&in_channel.id));
    // Not even the author's own home shows it unless they follow the channel.
    assert!(!home(&author).await.contains(&in_channel.id));

    let stranger = ctx.member("stranger").await;
    let followed = ctx.follow_channel(&stranger, &channel).await;
    assert!(followed.following && followed.follower_count == 1);
    assert!(home(&stranger).await.contains(&in_channel.id), "a followed channel's notes come home");
    ctx.follow_channel(&friend, &channel).await;
    assert!(home(&friend).await.contains(&in_channel.id));
    ctx.follow_channel(&follower, &channel).await;
    assert!(following(&follower).await.contains(&in_channel.id));

    // The channel timeline and the author's profile have it all along.
    let timeline = ctx.channel_timeline(None, &channel).await;
    assert_eq!(texts(&timeline.entries), ["channel note"]);
    let profile: GetUserTimelineResponse = ctx
        .note("GetUserTimeline", None, &GetUserTimelineRequest { user_id: author.id.to_string(), ..Default::default() })
        .await
        .unwrap();
    assert!(texts(&profile.entries).contains(&"channel note"));
}

#[tokio::test]
async fn the_channel_timeline_shows_what_the_reader_may_see_and_pins_on_the_first_page() {
    let ctx = start().await;
    let (owner, locked, stranger, blocked) = (
        ctx.member("owner").await,
        ctx.member("locked").await,
        ctx.member("stranger").await,
        ctx.member("blocked").await,
    );
    let channel = ctx.create_channel(&owner, "mixed").await;
    let first = ctx.post_in(&owner, &channel, "first").await;
    let reply = ctx.reply(&stranger, &first, "a reply stays in the channel").await;
    assert_eq!(reply.channel.id, channel.id);
    // A locked author's channel note reaches only their followers.
    ctx.set_locked(&locked, true).await;
    ctx.post_in(&locked, &channel, "locked author").await;
    let last = ctx.post_in(&owner, &channel, "last").await;
    let _: PinChannelNoteResponse = ctx.channel("PinChannelNote", Some(&owner), &pin(&channel, &first)).await.unwrap();

    let timeline = ctx.channel_timeline(Some(&stranger), &channel).await;
    assert_eq!(texts(&timeline.entries), ["last", "a reply stays in the channel", "first"]);
    assert_eq!(note_texts(&timeline.pinned_notes), ["first"]);
    assert_eq!(ctx.get_channel(None, &channel).await.unwrap().pinned_note_ids, vec![first.id.clone()]);

    // Pages: pins only on the first.
    let request = GetChannelTimelineRequest { channel_id: channel.id.clone(), page: page(2, ""), ..Default::default() };
    let one: GetChannelTimelineResponse = ctx.channel("GetChannelTimeline", Some(&stranger), &request).await.unwrap();
    assert_eq!(texts(&one.entries), ["last", "a reply stays in the channel"]);
    assert!(one.page.has_more);
    let request = GetChannelTimelineRequest {
        channel_id: channel.id.clone(),
        page: page(2, &one.page.next_cursor),
        ..Default::default()
    };
    let two: GetChannelTimelineResponse = ctx.channel("GetChannelTimeline", Some(&stranger), &request).await.unwrap();
    assert_eq!(texts(&two.entries), ["first"]);
    assert!(two.pinned_notes.is_empty());
    assert_eq!(last.channel.name, "mixed");

    // A block with the owner hides the channel altogether.
    ctx.block(&owner, &blocked).await;
    let request = GetChannelTimelineRequest { channel_id: channel.id.clone(), ..Default::default() };
    assert_eq!(ctx.channel_reason("GetChannelTimeline", Some(&blocked), &request).await, "CHANNEL_NOT_FOUND");
    assert_eq!(ctx.get_channel(Some(&blocked), &channel).await.unwrap_err().reason, "CHANNEL_NOT_FOUND");
    let reason = ctx.post_reason(&blocked, "let me in", |r| r.channel_id = channel.id.clone()).await;
    assert_eq!(reason, "CHANNEL_NOT_FOUND");
}

#[tokio::test]
async fn following_counts_stay_exact_and_are_capped() {
    let ctx = start().await;
    let owner = ctx.member("owner").await;
    let channel = ctx.create_channel(&owner, "popular").await;
    let mut fans = Vec::new();
    for i in 0..6 {
        fans.push(ctx.member(&format!("fan{i}")).await);
    }
    // Many people at once, each twice.
    join_all(fans.iter().chain(fans.iter()).map(|fan| ctx.follow_channel(fan, &channel))).await;
    assert_eq!(ctx.get_channel(None, &channel).await.unwrap().follower_count, 6);
    let unfollow = |fan: &TestUser| {
        let request = UnfollowChannelRequest { channel_id: channel.id.clone(), ..Default::default() };
        let (ctx, fan) = (&ctx, fan.clone());
        async move {
            let _: UnfollowChannelResponse = ctx.channel("UnfollowChannel", Some(&fan), &request).await.unwrap();
        }
    };
    join_all(fans.iter().take(3).chain(fans.iter().take(3)).map(unfollow)).await;
    let seen = ctx.get_channel(Some(&fans[0]), &channel).await.unwrap();
    assert_eq!((seen.follower_count, seen.following), (3, false));
    assert!(ctx.get_channel(Some(&fans[5]), &channel).await.unwrap().following);

    // The 500th followed channel is the last, also when asked for at once.
    let busy = ctx.member("busy").await;
    let filler = ctx.app.user("filler").await;
    sqlx::query(
        "WITH made AS (INSERT INTO note_channels (owner_id, name) SELECT $1, 'filler ' || g FROM generate_series(1, 499) g RETURNING id)
         INSERT INTO note_channel_follows (channel_id, user_id) SELECT id, $2 FROM made",
    )
    .bind(filler.id)
    .bind(busy.id)
    .execute(&ctx.app.core.db)
    .await
    .unwrap();
    let extra = [ctx.create_channel(&owner, "x1").await, ctx.create_channel(&owner, "x2").await, channel.clone()];
    let results = join_all(extra.iter().map(|channel| {
        let request = FollowChannelRequest { channel_id: channel.id.clone(), ..Default::default() };
        let (ctx, busy) = (&ctx, &busy);
        async move { ctx.channel::<_, FollowChannelResponse>("FollowChannel", Some(busy), &request).await }
    }))
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(results.into_iter().filter_map(Result::err).all(|e| e.reason == "TOO_MANY_FOLLOWED_CHANNELS"));
    assert_eq!(ctx.scalar("SELECT count(*) FROM note_channel_follows WHERE user_id = $1", busy.id).await, 500);
    let missing = FollowChannelRequest { channel_id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(ctx.channel_reason("FollowChannel", Some(&owner), &missing).await, "CHANNEL_NOT_FOUND");
}

#[tokio::test]
async fn the_owner_names_moderators_who_may_step_down() {
    let ctx = start().await;
    let (owner, moderator, other, blocked) = (
        ctx.member("owner").await,
        ctx.member("moderator").await,
        ctx.member("other").await,
        ctx.member("blocked").await,
    );
    let channel = ctx.create_channel(&owner, "team").await;
    ctx.add_moderator(&owner, &channel, &moderator).await;
    ctx.add_moderator(&owner, &channel, &moderator).await; // twice: nothing new
    assert_eq!(ctx.notifications_for(moderator.id, "notes.channel_moderator").await.len(), 1);

    let add = |target: &TestUser| AddChannelModeratorRequest {
        channel_id: channel.id.clone(),
        user_id: target.id.to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.channel_reason("AddChannelModerator", Some(&moderator), &add(&other)).await, "NOT_CHANNEL_OWNER");
    ctx.block(&blocked, &owner).await;
    assert_eq!(ctx.channel_reason("AddChannelModerator", Some(&owner), &add(&blocked)).await, "USER_NOT_FOUND");
    let nobody = AddChannelModeratorRequest {
        channel_id: channel.id.clone(),
        user_id: Uuid::now_v7().to_string(),
        ..Default::default()
    };
    assert_eq!(ctx.channel_reason("AddChannelModerator", Some(&owner), &nobody).await, "USER_NOT_FOUND");
    ctx.add_moderator(&owner, &channel, &other).await;

    let listed: ListChannelModeratorsResponse = ctx
        .channel(
            "ListChannelModerators",
            None,
            &ListChannelModeratorsRequest { channel_id: channel.id.clone(), ..Default::default() },
        )
        .await
        .unwrap();
    let names: Vec<String> = listed.moderators.iter().map(|user| user.id.clone()).collect();
    assert_eq!(names, [moderator.id.to_string(), other.id.to_string()]);

    let remove = |target: &TestUser| RemoveChannelModeratorRequest {
        channel_id: channel.id.clone(),
        user_id: target.id.to_string(),
        ..Default::default()
    };
    assert_eq!(
        ctx.channel_reason("RemoveChannelModerator", Some(&moderator), &remove(&other)).await,
        "NOT_CHANNEL_OWNER"
    );
    let _: RemoveChannelModeratorResponse =
        ctx.channel("RemoveChannelModerator", Some(&moderator), &remove(&moderator)).await.unwrap();
    let _: RemoveChannelModeratorResponse =
        ctx.channel("RemoveChannelModerator", Some(&owner), &remove(&other)).await.unwrap();
    assert!(!ctx.get_channel(Some(&moderator), &channel).await.unwrap().is_moderator);
    let managed: ListChannelsResponse =
        ctx.channel("ListChannels", Some(&other), &list(ChannelFilter::CHANNEL_FILTER_MANAGED, "")).await.unwrap();
    assert!(managed.channels.is_empty());

    // A block between owner and moderator ends the seat.
    ctx.add_moderator(&owner, &channel, &other).await;
    ctx.block(&other, &owner).await;
    ctx.notes
        .handle_event(event("social.user.blocked", serde_json::json!({ "user_id": other.id, "blocked_id": owner.id })))
        .await
        .unwrap();
    assert_eq!(ctx.scalar("SELECT count(*) FROM note_channel_moderators WHERE user_id = $1", other.id).await, 0);
}

#[tokio::test]
async fn a_channel_has_at_most_twenty_moderators_even_when_named_at_once() {
    let ctx = start().await;
    let owner = ctx.member("owner").await;
    let channel = ctx.create_channel(&owner, "big team").await;
    let mut seated = Vec::new();
    for i in 0..19 {
        seated.push(ctx.app.user(&format!("seat{i}")).await);
    }
    for user in &seated {
        sqlx::query("INSERT INTO note_channel_moderators (channel_id, user_id) VALUES ($1, $2)")
            .bind(id(&channel.id))
            .bind(user.id)
            .execute(&ctx.app.core.db)
            .await
            .unwrap();
    }
    let mut candidates = Vec::new();
    for i in 0..3 {
        candidates.push(ctx.app.user(&format!("candidate{i}")).await);
    }
    let results =
        join_all(
            candidates.iter().map(|user| {
                let request = AddChannelModeratorRequest {
                    channel_id: channel.id.clone(),
                    user_id: user.id.to_string(),
                    ..Default::default()
                };
                let (ctx, owner) = (&ctx, &owner);
                async move {
                    ctx.channel::<_, AddChannelModeratorResponse>("AddChannelModerator", Some(owner), &request).await
                }
            }),
        )
        .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(results.into_iter().filter_map(Result::err).all(|e| e.reason == "TOO_MANY_MODERATORS"));
    assert_eq!(
        ctx.scalar("SELECT count(*) FROM note_channel_moderators WHERE channel_id = $1", id(&channel.id)).await,
        20
    );
}

#[tokio::test]
async fn moderators_pin_up_to_five_notes_of_their_channel() {
    let ctx = start().await;
    let (owner, moderator, member) =
        (ctx.member("owner").await, ctx.member("moderator").await, ctx.member("member").await);
    let channel = ctx.create_channel(&owner, "pins").await;
    let elsewhere = ctx.create_channel(&owner, "elsewhere").await;
    ctx.add_moderator(&owner, &channel, &moderator).await;
    let mut notes = Vec::new();
    for i in 0..8 {
        notes.push(ctx.post_in(&member, &channel, &format!("note {i}")).await);
    }
    let outside = ctx.post_in(&member, &elsewhere, "another channel").await;
    let plain = ctx.post(&member, "no channel").await;

    assert_eq!(
        ctx.channel_reason("PinChannelNote", Some(&member), &pin(&channel, &notes[0])).await,
        "NOT_CHANNEL_MODERATOR"
    );
    assert_eq!(
        ctx.channel_reason("PinChannelNote", Some(&moderator), &pin(&channel, &outside)).await,
        "NOTE_NOT_FOUND"
    );
    assert_eq!(ctx.channel_reason("PinChannelNote", Some(&moderator), &pin(&channel, &plain)).await, "NOTE_NOT_FOUND");
    let missing = NoteChannel { id: Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(
        ctx.channel_reason("PinChannelNote", Some(&moderator), &pin(&missing, &notes[0])).await,
        "CHANNEL_NOT_FOUND"
    );

    for note in &notes[..4] {
        let _: PinChannelNoteResponse =
            ctx.channel("PinChannelNote", Some(&moderator), &pin(&channel, note)).await.unwrap();
    }
    // Pinning again changes nothing.
    let _: PinChannelNoteResponse =
        ctx.channel("PinChannelNote", Some(&owner), &pin(&channel, &notes[0])).await.unwrap();
    // The fifth, asked for four times at once.
    let results = join_all(notes[4..].iter().map(|note| {
        let request = pin(&channel, note);
        let (ctx, owner) = (&ctx, &owner);
        async move { ctx.channel::<_, PinChannelNoteResponse>("PinChannelNote", Some(owner), &request).await }
    }))
    .await;
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(results.into_iter().filter_map(Result::err).all(|e| e.reason == "TOO_MANY_PINNED"));
    assert_eq!(ctx.get_channel(None, &channel).await.unwrap().pinned_note_ids.len(), 5);

    let unpin =
        UnpinChannelNoteRequest { channel_id: channel.id.clone(), note_id: notes[3].id.clone(), ..Default::default() };
    assert_eq!(ctx.channel_reason("UnpinChannelNote", Some(&member), &unpin).await, "NOT_CHANNEL_MODERATOR");
    let _: UnpinChannelNoteResponse = ctx.channel("UnpinChannelNote", Some(&moderator), &unpin).await.unwrap();
    let pinned = ctx.get_channel(None, &channel).await.unwrap().pinned_note_ids;
    assert_eq!(pinned.len(), 4);
    assert_eq!(pinned[3], notes[0].id, "latest pinned first");
}

#[tokio::test]
async fn a_moderator_takes_a_note_out_of_the_channel_and_its_author_is_told() {
    let ctx = start().await;
    let (owner, moderator, author, reader) = (
        ctx.member("owner").await,
        ctx.member("moderator").await,
        ctx.member("author").await,
        ctx.member("reader").await,
    );
    let channel = ctx.create_channel(&owner, "rules").await;
    ctx.add_moderator(&owner, &channel, &moderator).await;
    ctx.follow_channel(&reader, &channel).await;
    let off_topic = ctx.post_in(&author, &channel, "off topic").await;
    let kept = ctx.post_in(&author, &channel, "on topic").await;
    let _: PinChannelNoteResponse =
        ctx.channel("PinChannelNote", Some(&owner), &pin(&channel, &off_topic)).await.unwrap();
    assert_eq!(ctx.get_channel(None, &channel).await.unwrap().note_count, 2);

    assert_eq!(
        ctx.channel_reason("RemoveChannelNote", Some(&reader), &removal(&channel, &off_topic)).await,
        "NOT_CHANNEL_MODERATOR"
    );
    let _: RemoveChannelNoteResponse =
        ctx.channel("RemoveChannelNote", Some(&moderator), &removal(&channel, &off_topic)).await.unwrap();
    assert_eq!(
        ctx.channel_reason("RemoveChannelNote", Some(&moderator), &removal(&channel, &off_topic)).await,
        "NOTE_NOT_FOUND"
    );

    let seen = ctx.get_channel(None, &channel).await.unwrap();
    assert_eq!(seen.note_count, 1);
    assert!(seen.pinned_note_ids.is_empty());
    assert_eq!(texts(&ctx.channel_timeline(None, &channel).await.entries), ["on topic"]);
    // The note lives on, outside the channel.
    let note = ctx.get_note(None, &off_topic.id).await.unwrap();
    assert!(note.channel.as_option().is_none());
    assert_eq!(ctx.notifications_for(author.id, "notes.channel_removed").await.len(), 1);
    assert_eq!(ctx.get_note(None, &kept.id).await.unwrap().channel.id, channel.id);
    // A moderator taking out their own note is not told about it.
    let own = ctx.post_in(&moderator, &channel, "mine").await;
    let _: RemoveChannelNoteResponse =
        ctx.channel("RemoveChannelNote", Some(&moderator), &removal(&channel, &own)).await.unwrap();
    assert!(ctx.notifications_for(moderator.id, "notes.channel_removed").await.is_empty());
}

#[tokio::test]
async fn channels_are_listed_featured_followed_managed_and_by_name() {
    let ctx = start().await;
    let (owner, fan, moderator) = (ctx.member("owner").await, ctx.member("fan").await, ctx.member("moderator").await);
    let word = unique("chan");
    let big = ctx.create_channel(&owner, &format!("{word} big")).await;
    let small = ctx.create_channel(&owner, &format!("{word} small")).await;
    let odd = ctx.create_channel(&owner, &format!("{word} 100%_off")).await;
    let shut = ctx.create_channel(&owner, &format!("{word} shut")).await;
    let archive = ArchiveChannelRequest { channel_id: shut.id.clone(), archived: true, ..Default::default() };
    let _: ArchiveChannelResponse = ctx.channel("ArchiveChannel", Some(&owner), &archive).await.unwrap();
    ctx.follow_channel(&fan, &big).await;
    ctx.follow_channel(&moderator, &big).await;
    ctx.follow_channel(&fan, &shut).await;
    ctx.post_in(&fan, &small, "recent").await;
    ctx.add_moderator(&owner, &odd, &moderator).await;

    let featured: ListChannelsResponse =
        ctx.channel("ListChannels", None, &list(ChannelFilter::CHANNEL_FILTER_UNSPECIFIED, &word)).await.unwrap();
    // Active first (a recent note), then by followers; archived ones left out.
    assert_eq!(ids(&featured.channels), [small.id.as_str(), big.id.as_str(), odd.id.as_str()]);
    let literal: ListChannelsResponse =
        ctx.channel("ListChannels", None, &list(ChannelFilter::CHANNEL_FILTER_FEATURED, "0%_O")).await.unwrap();
    assert!(ids(&literal.channels).contains(&odd.id.as_str()));
    let wildcard: ListChannelsResponse = ctx
        .channel("ListChannels", None, &list(ChannelFilter::CHANNEL_FILTER_FEATURED, &format!("{word}%big")))
        .await
        .unwrap();
    assert!(wildcard.channels.is_empty(), "% matches only itself");

    let followed: ListChannelsResponse =
        ctx.channel("ListChannels", Some(&fan), &list(ChannelFilter::CHANNEL_FILTER_FOLLOWED, "")).await.unwrap();
    assert_eq!(ids(&followed.channels), [shut.id.as_str(), big.id.as_str()]);
    assert!(followed.channels.iter().all(|channel| channel.following));
    let managed: ListChannelsResponse =
        ctx.channel("ListChannels", Some(&moderator), &list(ChannelFilter::CHANNEL_FILTER_MANAGED, "")).await.unwrap();
    assert_eq!(ids(&managed.channels), [odd.id.as_str()]);
    assert!(managed.channels[0].is_moderator);
    let owned: ListChannelsResponse =
        ctx.channel("ListChannels", Some(&owner), &list(ChannelFilter::CHANNEL_FILTER_MANAGED, "")).await.unwrap();
    assert_eq!(owned.channels.len(), 4);

    // Paging the featured list.
    let mut request = list(ChannelFilter::CHANNEL_FILTER_FEATURED, &word);
    request.page = page(2, "");
    let first: ListChannelsResponse = ctx.channel("ListChannels", None, &request).await.unwrap();
    assert_eq!(first.channels.len(), 2);
    request.page = page(2, &first.page.next_cursor);
    let second: ListChannelsResponse = ctx.channel("ListChannels", None, &request).await.unwrap();
    assert_eq!(ids(&second.channels), [odd.id.as_str()]);
    assert!(!second.page.has_more);

    let error = ctx
        .channel::<_, ListChannelsResponse>("ListChannels", None, &list(ChannelFilter::CHANNEL_FILTER_FOLLOWED, ""))
        .await
        .unwrap_err();
    assert_eq!(error.code, "unauthenticated");
}

#[tokio::test]
async fn deleting_an_account_takes_its_channels_follows_and_seats() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let banner = ctx.note_image(&alice).await;
    let request = create("alice's", |r| r.banner_asset_id = banner.clone());
    let created: CreateChannelResponse = ctx.channel("CreateChannel", Some(&alice), &request).await.unwrap();
    let alices = created.channel.as_option().cloned().unwrap();
    let bobs = ctx.create_channel(&bob, "bob's").await;
    let bobs_note = ctx.post_in(&bob, &alices, "bob posted in alice's channel").await;
    ctx.follow_channel(&alice, &bobs).await;
    ctx.add_moderator(&bob, &bobs, &alice).await;
    assert_eq!(ctx.get_channel(None, &bobs).await.unwrap().follower_count, 1);

    let deleted = event("identity.user.deleted", serde_json::json!({ "user_id": alice.id }));
    ctx.notes.handle_event(deleted.clone()).await.unwrap();
    ctx.notes.handle_event(deleted).await.unwrap();

    assert_eq!(ctx.scalar("SELECT count(*) FROM note_channels WHERE owner_id = $1", alice.id).await, 0);
    assert_eq!(ctx.scalar("SELECT count(*) FROM note_channel_follows WHERE user_id = $1", alice.id).await, 0);
    assert_eq!(ctx.scalar("SELECT count(*) FROM note_channel_moderators WHERE user_id = $1", alice.id).await, 0);
    assert_eq!(ctx.get_channel(None, &bobs).await.unwrap().follower_count, 0);
    assert!(!tokuly_media::assets::is_attached(&ctx.app.core, id(&banner)).await.unwrap());
    assert!(!ctx.is_indexed("note_channel", id(&alices.id)).await);
    let note = ctx.get_note(None, &bobs_note.id).await.unwrap();
    assert!(note.channel.as_option().is_none(), "the note stays, outside any channel");
}

#[tokio::test]
async fn a_deleted_banner_upload_leaves_the_channel_without_a_banner() {
    let ctx = start().await;
    let owner = ctx.member("owner").await;
    let banner = ctx.note_image(&owner).await;
    let request = create("banner", |r| r.banner_asset_id = banner.clone());
    let created: CreateChannelResponse = ctx.channel("CreateChannel", Some(&owner), &request).await.unwrap();
    let channel = created.channel.as_option().cloned().unwrap();
    let gone = event("media.asset.deleted", serde_json::json!({ "asset_id": banner }));
    ctx.notes.handle_event(gone.clone()).await.unwrap();
    ctx.notes.handle_event(gone).await.unwrap();
    assert!(ctx.get_channel(None, &channel).await.unwrap().banner.as_option().is_none());
}

#[tokio::test]
async fn channels_and_follows_are_in_the_data_export() {
    let ctx = start().await;
    let (alice, bob) = (ctx.member("alice").await, ctx.member("bob").await);
    let mine = ctx.create_channel(&alice, "exported").await;
    let theirs = ctx.create_channel(&bob, "followed").await;
    ctx.follow_channel(&alice, &theirs).await;
    let exporter = ctx.app.core.exporters.all().into_iter().find(|exporter| exporter.name() == "notes").unwrap();
    let export = exporter.export(alice.id).await.unwrap();
    assert_eq!(export["channels"][0]["id"], serde_json::json!(mine.id));
    assert_eq!(export["channels"][0]["name"], "exported");
    assert_eq!(export["channel_follows"][0]["channel_id"], serde_json::json!(theirs.id));
}

#[tokio::test]
async fn locking_channel_calls_from_one_account_do_not_starve_the_pool() {
    let ctx = start().await;
    let (owner, member) = (ctx.member("owner").await, ctx.member("member").await);
    let channel = ctx.create_channel(&owner, "busy").await;
    let mut notes = Vec::new();
    for i in 0..3 {
        notes.push(ctx.post_in(&member, &channel, &format!("n{i}")).await);
    }
    let mut others = Vec::new();
    for i in 0..3 {
        others.push(ctx.create_channel(&member, &format!("c{i}")).await);
    }
    // More concurrent locking calls than the test pool has connections (5).
    let pins =
        notes.iter().map(|note| {
            let request = pin(&channel, note);
            let (ctx, owner) = (&ctx, &owner);
            async move {
                ctx.channel::<_, PinChannelNoteResponse>("PinChannelNote", Some(owner), &request).await.map(|_| ())
            }
        });
    let follows = others.iter().map(|other| {
        let request = FollowChannelRequest { channel_id: other.id.clone(), ..Default::default() };
        let (ctx, owner) = (&ctx, &owner);
        async move { ctx.channel::<_, FollowChannelResponse>("FollowChannel", Some(owner), &request).await.map(|_| ()) }
    });
    let creates = (0..3).map(|i| {
        let request = create(&format!("more {i}"), |_| {});
        let (ctx, owner) = (&ctx, &owner);
        async move { ctx.channel::<_, CreateChannelResponse>("CreateChannel", Some(owner), &request).await.map(|_| ()) }
    });
    let removals = notes.iter().map(|note| {
        let request = removal(&channel, note);
        let (ctx, owner) = (&ctx, &owner);
        async move {
            ctx.channel::<_, RemoveChannelNoteResponse>("RemoveChannelNote", Some(owner), &request).await.map(|_| ())
        }
    });
    let started = std::time::Instant::now();
    let (pins, follows, creates, removals) =
        tokio::join!(join_all(pins), join_all(follows), join_all(creates), join_all(removals));
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "calls waited on the pool");
    assert!(follows.iter().chain(creates.iter()).all(Result::is_ok));
    // Pins and removals of the same notes race; each one either applied or found the note gone.
    for result in pins.iter().chain(removals.iter()) {
        if let Err(error) = result {
            assert_eq!(error.reason, "NOTE_NOT_FOUND");
        }
    }
    assert_eq!(ctx.get_channel(None, &channel).await.unwrap().note_count, 0);
}
