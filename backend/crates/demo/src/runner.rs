//! Plays the script against the server: every step becomes the API call a
//! person's app would make, with the bot's own token.
//!
//! The runner also answers people. Someone who asks a bot to be friends is
//! accepted and greeted, a few more bots ask to be their friends, and a
//! direct message to a bot gets a (canned) answer. The bots never write to
//! anybody who did not come to them first.
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::sync::mpsc;
use tokuly_proto::{
    Timestamp,
    chat::{
        Conversation, ConversationFilter, ConversationKind, CreateGroupRequest, CreateGroupResponse,
        ListConversationsRequest, ListConversationsResponse, MarkReadRequest, MarkReadResponse, OpenDirectRequest,
        OpenDirectResponse, SendMessageRequest, SendMessageResponse,
    },
    common::{LatLng as Coordinate, PageRequest, SubjectRef, SubjectType, Visibility},
    engagement::{Reaction, ReactionKind, SetReactionRequest, SetReactionResponse},
    geo::{
        Activity, Destination, GetDirectionsRequest, GetDirectionsResponse, LocationSample, ReportLocationsRequest,
        ReportLocationsResponse, StopSharingRequest, StopSharingResponse, TravelMode, destination,
    },
    identity::{SetStatusRequest, SetStatusResponse},
    media::AssetPurpose,
    notes::{
        CreateNoteRequest, CreateNoteResponse, CreateStoryRequest, CreateStoryResponse, LikeNoteRequest,
        LikeNoteResponse, PlaceTag, ReactToStoryRequest, ReactToStoryResponse, StoryItemInput,
    },
    shorts::{CreateShortRequest, CreateShortResponse},
    social::{
        FriendRequestDirection, ListFriendRequestsRequest, ListFriendRequestsResponse, RespondFriendRequestRequest,
        RespondFriendRequestResponse,
    },
    video::{CreateChannelRequest, CreateChannelResponse, ListMyChannelsRequest, ListMyChannelsResponse},
};

use crate::{
    api::{Api, ApiError, ApiResult},
    cast::Group,
    lines::{ANSWERS, GREETINGS, INTRODUCTIONS},
    media::{self, Upload},
    posts::Post,
    rng::Rng,
    script::{Action, Place, Script, Step},
    setup::{Bot, befriend},
    talk::{Line, Room},
    town::{LatLng, decode_polyline},
    walk::Fix,
};

/// How often the script moves on, and how often the bots look for visitors.
const TICK: Duration = Duration::from_secs(5);
const VISITOR_ROUND: Duration = Duration::from_secs(15);
const REPORT_EVERY: Duration = Duration::from_secs(60);

/// What the simulator did so far.
#[derive(Debug, Default)]
pub struct Tally {
    pub fixes: AtomicU64,
    pub messages: AtomicU64,
    pub notes: AtomicU64,
    pub stories: AtomicU64,
    pub shorts: AtomicU64,
    pub reactions: AtomicU64,
    pub statuses: AtomicU64,
    pub visitors: AtomicU64,
    /// Calls the server refused or that did not get through.
    pub refused: AtomicU64,
}

impl std::fmt::Display for Tally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        write!(
            f,
            "{} fixes, {} messages, {} notes, {} stories, {} shorts, {} reactions, {} statuses, {} visitors, {} refused",
            count(&self.fixes),
            count(&self.messages),
            count(&self.notes),
            count(&self.stories),
            count(&self.shorts),
            count(&self.reactions),
            count(&self.statuses),
            count(&self.visitors),
            count(&self.refused)
        )
    }
}

/// The newest thing each bot posted, for the others to react to.
#[derive(Default)]
struct Newest {
    notes: HashMap<usize, String>,
    stories: HashMap<usize, String>,
    shorts: HashMap<usize, String>,
}

/// A route a routing engine gave for a bot's journey.
type Route = (usize, u32, Vec<LatLng>);

struct Stage {
    api: Api,
    bots: Vec<Bot>,
    groups: Vec<Group>,
    friendships: Vec<(usize, usize)>,
    companions: usize,
    rng: Rng,
    rooms: Mutex<HashMap<Room, String>>,
    newest: Mutex<Newest>,
    /// Bots that have a channel to publish shorts on.
    channels: Mutex<HashSet<usize>>,
    /// People a bot has befriended since the simulator started.
    visitors: Mutex<HashSet<String>>,
    /// Refusals already written to the log, so each kind is said once.
    warned: Mutex<HashSet<String>>,
    routes: mpsc::UnboundedSender<Route>,
    tally: Arc<Tally>,
}

/// Runs the demo until `stop` completes. The bots stop sharing their
/// location on the way out, so their pins go stale at once.
pub async fn run(
    api: Api,
    script: Script,
    bots: Vec<Bot>,
    pace: f64,
    companions: usize,
    seed: u64,
    stop: impl Future<Output = ()>,
) -> Arc<Tally> {
    let (routes, mut found_routes) = mpsc::unbounded_channel();
    let mut script = script;
    let stage = Arc::new(Stage {
        api,
        bots,
        groups: script.groups().to_vec(),
        friendships: script.friendships().to_vec(),
        companions,
        rng: Rng::new(seed),
        rooms: Mutex::default(),
        newest: Mutex::default(),
        channels: Mutex::default(),
        visitors: Mutex::default(),
        warned: Mutex::default(),
        routes,
        tally: Arc::default(),
    });
    let interval = |every| {
        let mut interval = tokio::time::interval(every);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval
    };
    let (mut tick, mut visitor_round, mut report) = (interval(TICK), interval(VISITOR_ROUND), interval(REPORT_EVERY));
    let looking = Arc::new(tokio::sync::Mutex::new(()));
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = &mut stop => break,
            _ = tick.tick() => {
                while let Ok((bot, leg, route)) = found_routes.try_recv() {
                    script.follow(bot, leg, &route);
                }
                for step in script.advance(TICK.as_secs_f64() * pace) {
                    let stage = stage.clone();
                    tokio::spawn(async move { stage.play(step).await });
                }
            }
            _ = visitor_round.tick() => {
                // One round at a time: a slow server must not make them pile up.
                if let Ok(turn) = looking.clone().try_lock_owned() {
                    let stage = stage.clone();
                    tokio::spawn(async move {
                        stage.look_for_visitors().await;
                        drop(turn);
                    });
                }
            }
            _ = report.tick() => tracing::info!("demo: {}", stage.tally),
        }
    }
    let stopped = StopSharingRequest::default();
    let leaving = stage.bots.iter().map(|bot| {
        stage.api.call::<_, StopSharingResponse>(
            Some(&bot.token),
            "tokuly.geo.v1.LocationService/StopSharing",
            &stopped,
        )
    });
    futures::future::join_all(leaving).await;
    stage.tally.clone()
}

impl Stage {
    fn token(&self, bot: usize) -> Option<&str> {
        Some(&self.bots[bot].token)
    }

    /// Does what the step says and counts it; a refusal is logged (each
    /// kind once) and the demo goes on.
    async fn play(&self, step: Step) {
        let what = match &step.action {
            Action::Locate { .. } => "location",
            Action::Depart { .. } => "directions",
            Action::Status { .. } => "status",
            Action::Post { post: Post::Note { .. } | Post::Reply { .. }, .. } => "note",
            Action::Post { post: Post::Story { .. }, .. } => "story",
            Action::Post { post: Post::Short { .. }, .. } => "short",
            Action::Post { .. } => "reaction",
            Action::Say(_) => "message",
        };
        let bot = step.bot;
        let done = match step.action {
            Action::Locate { at, fix, cycling } => {
                self.locate(bot, at, fix, cycling).await.map(|()| Some(&self.tally.fixes))
            }
            Action::Depart { leg, from, to, cycling } => {
                self.ask_the_way(bot, leg, from, to, cycling).await.map(|()| None)
            }
            Action::Status { emoji, text } => {
                self.show_status(bot, emoji, text).await.map(|()| Some(&self.tally.statuses))
            }
            Action::Post { post, place } => self.post(bot, post, place).await,
            Action::Say(line) => self.say(line).await.map(|()| Some(&self.tally.messages)),
        };
        match done {
            Ok(Some(counter)) => drop(counter.fetch_add(1, Ordering::Relaxed)),
            Ok(None) => {}
            Err(refused) => self.refused(what, &refused),
        }
    }

    fn refused(&self, what: &str, refused: &ApiError) {
        self.tally.refused.fetch_add(1, Ordering::Relaxed);
        if self.warned.lock().expect("not poisoned").insert(format!("{what} {}", refused.reason)) {
            tracing::warn!(%refused, "demo: a {what} was refused (said once per reason)");
        }
    }

    async fn locate(&self, bot: usize, at: LatLng, fix: Fix, cycling: bool) -> ApiResult<()> {
        let activity = match (fix.speed_mps > 0.0, cycling) {
            (false, _) => Activity::ACTIVITY_STILL,
            (true, false) => Activity::ACTIVITY_WALKING,
            (true, true) => Activity::ACTIVITY_CYCLING,
        };
        let sample = LocationSample {
            position: coordinate(at).into(),
            accuracy_meters: 8.0,
            speed_mps: Some(fix.speed_mps),
            heading_degrees: fix.heading,
            recorded_at: now().into(),
            battery_percent: Some(fix.battery_percent),
            activity: activity.into(),
            // The truth: nobody is standing there. The server then keeps
            // the position out of what needs a real one (bumps, nearby).
            mocked: true,
            foreground: true,
            ..Default::default()
        };
        let request = ReportLocationsRequest { samples: vec![sample], ..Default::default() };
        self.api
            .call::<_, ReportLocationsResponse>(
                self.token(bot),
                "tokuly.geo.v1.LocationService/ReportLocations",
                &request,
            )
            .await
            .map(drop)
    }

    /// Asks the server for the way. With a routing engine behind it the
    /// answer follows real streets and the bot takes it; the server's
    /// straight-line estimate is no better than the simulator's own streets.
    async fn ask_the_way(&self, bot: usize, leg: u32, from: LatLng, to: LatLng, cycling: bool) -> ApiResult<()> {
        let request = GetDirectionsRequest {
            origin: coordinate(from).into(),
            destination: Destination {
                target: Some(destination::Target::Position(coordinate(to).into())),
                ..Default::default()
            }
            .into(),
            mode: if cycling { TravelMode::TRAVEL_MODE_BICYCLE } else { TravelMode::TRAVEL_MODE_WALK }.into(),
            ..Default::default()
        };
        let directions: GetDirectionsResponse =
            self.api.call(self.token(bot), "tokuly.geo.v1.TripService/GetDirections", &request).await?;
        if directions.provider != "estimate"
            && let Some(route) = decode_polyline(&directions.polyline).filter(|route| route.len() >= 2)
        {
            let _ = self.routes.send((bot, leg, route));
        }
        Ok(())
    }

    async fn show_status(&self, bot: usize, emoji: &str, text: &str) -> ApiResult<()> {
        let request = SetStatusRequest {
            emoji: emoji.to_owned(),
            text: text.to_owned(),
            expires_in_seconds: 3600,
            ..Default::default()
        };
        self.api
            .call::<_, SetStatusResponse>(self.token(bot), "tokuly.identity.v1.AccountService/SetStatus", &request)
            .await
            .map(drop)
    }

    /// Posts or reacts. `None` when there was nothing to react to yet.
    async fn post(&self, bot: usize, post: Post, place: Option<Place>) -> ApiResult<Option<&AtomicU64>> {
        let token = self.token(bot);
        let newest = |pick: fn(&Newest) -> &HashMap<usize, String>, of: usize| {
            pick(&self.newest.lock().expect("not poisoned")).get(&of).cloned()
        };
        match post {
            Post::Note { text, .. } => {
                let place = place.map(|place| PlaceTag {
                    name: place.name.to_owned(),
                    location: coordinate(place.at).into(),
                    ..Default::default()
                });
                self.write_note(bot, text, place, None).await?;
                Ok(Some(&self.tally.notes))
            }
            Post::Reply { author, text } => {
                let Some(note) = newest(|newest| &newest.notes, author) else { return Ok(None) };
                self.write_note(bot, text, None, Some(note)).await?;
                Ok(Some(&self.tally.notes))
            }
            Post::Like { author } => {
                let Some(note_id) = newest(|newest| &newest.notes, author) else { return Ok(None) };
                let request = LikeNoteRequest { note_id, ..Default::default() };
                self.api.call::<_, LikeNoteResponse>(token, "tokuly.notes.v1.NoteService/LikeNote", &request).await?;
                Ok(Some(&self.tally.reactions))
            }
            Post::Story { caption, colours } => {
                let picture = Upload {
                    purpose: AssetPurpose::ASSET_PURPOSE_STORY,
                    file_name: "story.png",
                    content_type: "image/png",
                    bytes: media::paint(colours, 360, 640),
                };
                let asset_id = media::upload(&self.api, &self.bots[bot].token, picture).await?;
                let request = CreateStoryRequest {
                    items: vec![StoryItemInput { asset_id, caption: caption.to_owned(), ..Default::default() }],
                    idempotency_key: fresh_key(),
                    ..Default::default()
                };
                let created: CreateStoryResponse =
                    self.api.call(token, "tokuly.notes.v1.StoryService/CreateStory", &request).await?;
                if let Some(story) = created.story.as_option() {
                    self.newest.lock().expect("not poisoned").stories.insert(bot, story.id.clone());
                }
                Ok(Some(&self.tally.stories))
            }
            Post::StoryReaction { owner, emoji } => {
                let Some(story_id) = newest(|newest| &newest.stories, owner) else { return Ok(None) };
                let request = ReactToStoryRequest { story_id, emoji: emoji.to_owned(), ..Default::default() };
                self.api
                    .call::<_, ReactToStoryResponse>(token, "tokuly.notes.v1.StoryService/ReactToStory", &request)
                    .await?;
                Ok(Some(&self.tally.reactions))
            }
            Post::Short { caption, clip } => {
                self.open_channel(bot).await?;
                let clip = Upload {
                    purpose: AssetPurpose::ASSET_PURPOSE_SHORT,
                    file_name: "short.mp4",
                    content_type: "video/mp4",
                    bytes: media::CLIPS[clip].to_vec(),
                };
                let asset_id = media::upload(&self.api, &self.bots[bot].token, clip).await?;
                let request = CreateShortRequest {
                    asset_id,
                    caption: caption.to_owned(),
                    visibility: Visibility::VISIBILITY_PUBLIC.into(),
                    ..Default::default()
                };
                let created: CreateShortResponse =
                    self.api.call(token, "tokuly.shorts.v1.ShortService/CreateShort", &request).await?;
                if let Some(short) = created.short.as_option() {
                    self.newest.lock().expect("not poisoned").shorts.insert(bot, short.id.clone());
                }
                Ok(Some(&self.tally.shorts))
            }
            Post::ShortReaction { author, emoji } => {
                let Some(id) = newest(|newest| &newest.shorts, author) else { return Ok(None) };
                let request = SetReactionRequest {
                    subject: SubjectRef { r#type: SubjectType::SUBJECT_TYPE_SHORT.into(), id, ..Default::default() }
                        .into(),
                    reaction: Reaction {
                        kind: ReactionKind::REACTION_KIND_EMOJI.into(),
                        emoji: emoji.to_owned(),
                        ..Default::default()
                    }
                    .into(),
                    ..Default::default()
                };
                self.api
                    .call::<_, SetReactionResponse>(token, "tokuly.engagement.v1.ReactionService/SetReaction", &request)
                    .await?;
                Ok(Some(&self.tally.reactions))
            }
        }
    }

    /// A public note, or an answer to one.
    async fn write_note(
        &self,
        bot: usize,
        text: &str,
        place: Option<PlaceTag>,
        reply_to: Option<String>,
    ) -> ApiResult<()> {
        let answer = reply_to.is_some();
        let request = CreateNoteRequest {
            text: text.to_owned(),
            place: place.into(),
            visibility: Visibility::VISIBILITY_PUBLIC.into(),
            reply_to_id: reply_to.unwrap_or_default(),
            idempotency_key: fresh_key(),
            ..Default::default()
        };
        let created: CreateNoteResponse =
            self.api.call(self.token(bot), "tokuly.notes.v1.NoteService/CreateNote", &request).await?;
        if let Some(note) = created.note.as_option().filter(|_| !answer) {
            self.newest.lock().expect("not poisoned").notes.insert(bot, note.id.clone());
        }
        Ok(())
    }

    /// Shorts are published on a channel; a bot makes one before its first.
    async fn open_channel(&self, bot: usize) -> ApiResult<()> {
        if self.channels.lock().expect("not poisoned").contains(&bot) {
            return Ok(());
        }
        let token = self.token(bot);
        let mine: ListMyChannelsResponse = self
            .api
            .call(token, "tokuly.video.v1.ChannelService/ListMyChannels", &ListMyChannelsRequest::default())
            .await?;
        if mine.channels.is_empty() {
            // The bot's handle is its own; as a channel handle it is told apart by a number.
            let request = CreateChannelRequest {
                handle: format!("demo-ch-{:08x}", self.rng.fork(&self.bots[bot].id).draw() as u32),
                name: self.bots[bot].name.clone(),
                ..Default::default()
            };
            self.api
                .call::<_, CreateChannelResponse>(token, "tokuly.video.v1.ChannelService/CreateChannel", &request)
                .await?;
        }
        self.channels.lock().expect("not poisoned").insert(bot);
        Ok(())
    }

    async fn say(&self, line: Line) -> ApiResult<()> {
        let conversation_id = self.room(line.room).await?;
        self.send(line.speaker, conversation_id, line.text).await
    }

    async fn send(&self, bot: usize, conversation_id: String, text: &str) -> ApiResult<()> {
        let request =
            SendMessageRequest { conversation_id, client_id: fresh_key(), text: text.to_owned(), ..Default::default() };
        self.api
            .call::<_, SendMessageResponse>(self.token(bot), "tokuly.chat.v1.MessageService/SendMessage", &request)
            .await
            .map(drop)
    }

    /// The conversation of a room: the group chat (made by the group's first
    /// member, found again by name after a restart) or the direct messages
    /// of two bots.
    async fn room(&self, room: Room) -> ApiResult<String> {
        if let Some(known) = self.rooms.lock().expect("not poisoned").get(&room) {
            return Ok(known.clone());
        }
        let conversation = match room {
            Room::Direct(a, b) => self.direct(a, &self.bots[b].id).await?,
            Room::Group(index) => {
                let group = &self.groups[index];
                let token = self.token(group.members[0]);
                let existing = self.conversations(group.members[0]).await?.into_iter().find(|conversation| {
                    conversation.kind.as_known() == Some(ConversationKind::CONVERSATION_KIND_GROUP)
                        && conversation.name == group.name
                });
                match existing {
                    Some(conversation) => conversation.id,
                    None => {
                        let request = CreateGroupRequest {
                            name: group.name.to_owned(),
                            member_ids: group.members[1..].iter().map(|&member| self.bots[member].id.clone()).collect(),
                            ..Default::default()
                        };
                        let created: CreateGroupResponse =
                            self.api.call(token, "tokuly.chat.v1.ConversationService/CreateGroup", &request).await?;
                        created.conversation.as_option().map(|conversation| conversation.id.clone()).unwrap_or_default()
                    }
                }
            }
        };
        self.rooms.lock().expect("not poisoned").insert(room, conversation.clone());
        Ok(conversation)
    }

    async fn direct(&self, bot: usize, with: &str) -> ApiResult<String> {
        let request = OpenDirectRequest { user_id: with.to_owned(), ..Default::default() };
        let opened: OpenDirectResponse =
            self.api.call(self.token(bot), "tokuly.chat.v1.ConversationService/OpenDirect", &request).await?;
        Ok(opened.conversation.as_option().map(|conversation| conversation.id.clone()).unwrap_or_default())
    }

    /// The bot's inbox, newest first.
    async fn conversations(&self, bot: usize) -> ApiResult<Vec<Conversation>> {
        let request = ListConversationsRequest {
            filter: ConversationFilter::CONVERSATION_FILTER_INBOX.into(),
            page: PageRequest { limit: 50, ..Default::default() }.into(),
            ..Default::default()
        };
        let listed: ListConversationsResponse =
            self.api.call(self.token(bot), "tokuly.chat.v1.ConversationService/ListConversations", &request).await?;
        Ok(listed.conversations)
    }

    fn is_bot(&self, user_id: &str) -> bool {
        self.bots.iter().any(|bot| bot.id == user_id)
    }

    /// One round of every bot looking at who wants to be friends and who
    /// wrote to it.
    async fn look_for_visitors(&self) {
        for bot in 0..self.bots.len() {
            if let Err(refused) = self.accept_friends(bot).await {
                self.refused("friend request", &refused);
            }
            if let Err(refused) = self.answer_messages(bot).await {
                self.refused("answer", &refused);
            }
        }
    }

    async fn accept_friends(&self, bot: usize) -> ApiResult<()> {
        let token = self.token(bot);
        let request = ListFriendRequestsRequest {
            direction: FriendRequestDirection::FRIEND_REQUEST_DIRECTION_INCOMING.into(),
            ..Default::default()
        };
        let waiting: ListFriendRequestsResponse =
            self.api.call(token, "tokuly.social.v1.FriendService/ListFriendRequests", &request).await?;
        for request in waiting.requests {
            let Some(visitor) = request.from.as_option().map(|from| from.id.clone()) else { continue };
            let accept =
                RespondFriendRequestRequest { request_id: request.id.clone(), accept: true, ..Default::default() };
            self.api
                .call::<_, RespondFriendRequestResponse>(
                    token,
                    "tokuly.social.v1.FriendService/RespondFriendRequest",
                    &accept,
                )
                .await?;
            if self.is_bot(&visitor) {
                continue;
            }
            self.tally.visitors.fetch_add(1, Ordering::Relaxed);
            let mut rng = self.rng.fork(&format!("visitor {visitor} {bot}"));
            let conversation = self.direct(bot, &visitor).await?;
            self.send(bot, conversation, rng.pick::<&str>(GREETINGS)).await?;
            // The first friend among the bots brings a few of its own along.
            if self.visitors.lock().expect("not poisoned").insert(visitor.clone()) {
                let mut friends: Vec<usize> = self
                    .friendships
                    .iter()
                    .filter_map(|&(a, b)| {
                        if a == bot {
                            Some(b)
                        } else if b == bot {
                            Some(a)
                        } else {
                            None
                        }
                    })
                    .collect();
                rng.shuffle(&mut friends);
                for &friend in friends.iter().take(self.companions) {
                    let introduction = rng.pick(INTRODUCTIONS).replace("{friend}", &self.bots[bot].name);
                    // The visitor may not take requests, or may have asked this one too: both are fine.
                    if let Err(refused) = befriend(&self.api, &self.bots[friend], &visitor, &introduction).await {
                        tracing::debug!(%refused, "a companion's friend request was not sent");
                    }
                }
            }
        }
        Ok(())
    }

    /// Answers direct messages people sent to the bot. Message requests of
    /// strangers stay unanswered: the bots talk to their friends.
    async fn answer_messages(&self, bot: usize) -> ApiResult<()> {
        for conversation in self.conversations(bot).await? {
            let unread = conversation.viewer.as_option().is_some_and(|viewer| viewer.unread_count > 0);
            let direct = conversation.kind.as_known() == Some(ConversationKind::CONVERSATION_KIND_DIRECT);
            let Some(message) = conversation.last_message.as_option().filter(|_| unread && direct) else { continue };
            let from_a_person = message.sender.as_option().is_some_and(|sender| !self.is_bot(&sender.id));
            let read = MarkReadRequest { conversation_id: conversation.id.clone(), ..Default::default() };
            self.api
                .call::<_, MarkReadResponse>(self.token(bot), "tokuly.chat.v1.ConversationService/MarkRead", &read)
                .await?;
            if from_a_person && !conversation.read_only {
                let answer = *self.rng.fork(&message.id).pick(ANSWERS);
                self.send(bot, conversation.id.clone(), answer).await?;
                self.tally.messages.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(())
    }
}

fn coordinate(at: LatLng) -> Coordinate {
    Coordinate { lat: at.lat, lng: at.lng, ..Default::default() }
}

fn now() -> Timestamp {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    Timestamp { seconds: since_epoch.as_secs() as i64, nanos: since_epoch.subsec_nanos() as i32, ..Default::default() }
}

/// A key no other request of this or an earlier run has (idempotency keys,
/// client IDs of messages).
fn fresh_key() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let started = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("demo-{started:x}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}
