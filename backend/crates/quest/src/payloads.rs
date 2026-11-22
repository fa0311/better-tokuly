//! What the quest domain reads out of other domains' events.
//!
//! Every reader turns one event into the [`Deed`]s it stands for: who did
//! something, how much of it, and towards whom. The payload structs are
//! this crate's own view of the producers' payloads and deliberately
//! tolerant: every field is optional, unknown fields are ignored, and a
//! payload that cannot be read yields no deeds (logged, never an error), so
//! a producer changing its payload can cost XP but cannot stall the
//! consumer.
//!
//! Readers are referenced from the rule table (`rules.rs`), one per rule.
use serde::{Deserialize, de::DeserializeOwned};
use tokuly_core::{analytics::AnalyticsEvent, events::Event, prelude::*};

/// Who a deed was aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    User(Uuid),
    /// The other member of a direct conversation (resolved by the ingest
    /// through `quest_direct_chats`; chat does not name the recipient).
    DirectChat(Uuid),
}

/// One thing one player did, as far as the rules care.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deed {
    pub user: Uuid,
    /// How much of it: minutes, map cells... 1 for plain actions.
    pub amount: i64,
    /// The people it was aimed at. Only friends among them count (for the
    /// streak with each, and as "friends reached").
    pub towards: Vec<Target>,
}

impl Deed {
    pub fn by(user: Uuid) -> Self {
        Self { user, amount: 1, towards: Vec::new() }
    }

    pub fn amount(mut self, amount: i64) -> Self {
        self.amount = amount;
        self
    }

    /// Aims the deed at `other` (ignored when that is the doer).
    pub fn towards(mut self, other: Option<Uuid>) -> Self {
        if let Some(other) = other.filter(|other| *other != self.user) {
            self.towards.push(Target::User(other));
        }
        self
    }
}

/// Reads an event into deeds.
pub type Reader = fn(&Event) -> Vec<Deed>;

/// Deserializes the payload, or logs and gives up.
fn payload<T: DeserializeOwned>(event: &Event) -> Option<T> {
    match T::deserialize(&event.data) {
        Ok(payload) => Some(payload),
        Err(e) => {
            tracing::warn!(topic = %event.topic, id = %event.id, error = %e, "quest: event payload not understood; ignored");
            None
        }
    }
}

/// An analytics event in the shape of a domain event, so both feeds go
/// through the same rules: the name becomes the topic, the tracked user the
/// actor, and the rest the payload ([`Tracked`]).
pub fn analytics_as_event(tracked: AnalyticsEvent) -> Event {
    Event {
        id: tracked.id,
        topic: tracked.name,
        actor_id: tracked.user_id,
        occurred_at: tracked.at,
        data: serde_json::json!({
            "subject_type": tracked.subject_type,
            "subject_id": tracked.subject_id,
            "owner_id": tracked.owner_id,
            "value": tracked.value,
            "props": tracked.props,
        }),
    }
}

/// The payload [`analytics_as_event`] builds.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Tracked {
    subject_id: Option<Uuid>,
    owner_id: Option<Uuid>,
    props: serde_json::Value,
}

// ── identity ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UserRef {
    user_id: Option<Uuid>,
}

/// `identity.user.created { user_id }` and anything else that names one user.
pub fn user(event: &Event) -> Vec<Deed> {
    payload::<UserRef>(event).and_then(|p| p.user_id).map(Deed::by).into_iter().collect()
}

/// Events of domains whose payloads are not known yet: the deed belongs to
/// the event's actor (every producer passes the acting user to
/// `events::emit`).
pub fn actor(event: &Event) -> Vec<Deed> {
    event.actor_id.map(Deed::by).into_iter().collect()
}

// ── social ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FriendAccepted {
    user_id: Option<Uuid>,
    friend_id: Option<Uuid>,
}

/// `social.friend.accepted { user_id, friend_id }`: both made a friend.
pub fn friend_accepted(event: &Event) -> Vec<Deed> {
    let Some(FriendAccepted { user_id: Some(user), friend_id: Some(friend) }) = payload(event) else {
        return Vec::new();
    };
    if user == friend {
        return Vec::new();
    }
    vec![Deed::by(user), Deed::by(friend)]
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Bump {
    user_ids: Vec<Uuid>,
    friends: bool,
}

/// `social.bump.created { user_ids: [a, b], friends }`: two friends bumped,
/// each towards the other. Bumps between strangers do not count.
pub fn bump(event: &Event) -> Vec<Deed> {
    let Some(bump) = payload::<Bump>(event) else { return Vec::new() };
    match bump.user_ids.as_slice() {
        [a, b] if bump.friends && a != b => vec![Deed::by(*a).towards(Some(*b)), Deed::by(*b).towards(Some(*a))],
        _ => Vec::new(),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Intro {
    inviter_id: Option<Uuid>,
}

/// `social.intro.created { inviter_id, user_id }`: the inviter brought someone in.
pub fn intro(event: &Event) -> Vec<Deed> {
    payload::<Intro>(event).and_then(|p| p.inviter_id).map(Deed::by).into_iter().collect()
}

// ── the generic requests of `tokuly_core::outbound` ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Notification {
    user_id: Option<Uuid>,
    actor_id: Option<Uuid>,
}

/// `notify.requested` (`NotificationRequest`): the actor did something
/// towards the recipient. The rule names the notification kind.
pub fn notification(event: &Event) -> Vec<Deed> {
    let Some(Notification { user_id: Some(recipient), actor_id: Some(actor) }) = payload(event) else {
        return Vec::new();
    };
    if recipient == actor {
        return Vec::new();
    }
    vec![Deed::by(actor).towards(Some(recipient))]
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Activity {
    user_id: Option<Uuid>,
    audience: String,
    with_user_ids: Vec<Uuid>,
}

/// `ActivityAudience::Private`: an entry only its author may see.
const AUDIENCE_PRIVATE: &str = "private";

/// `activity.recorded` (`ActivityEntry`): the user did it, towards the
/// people the entry names. The rule names the activity kind. Entries
/// recorded as private are not counted.
pub fn activity(event: &Event) -> Vec<Deed> {
    let Some(Activity { user_id: Some(user), audience, with_user_ids }) = payload(event) else { return Vec::new() };
    // What a player keeps to themselves (a check-in in ghost mode, a region
    // while their footprints are hidden) earns nothing: XP, badges and
    // streaks are seen by others, and would give it away.
    if audience == AUDIENCE_PRIVATE {
        return Vec::new();
    }
    let mut deed = Deed::by(user);
    // A check-in can tag many people; more than a handful is not "towards" anyone.
    for other in with_user_ids.into_iter().take(MAX_TARGETS) {
        deed = deed.towards(Some(other));
    }
    vec![deed]
}

/// Most people one deed can be aimed at.
const MAX_TARGETS: usize = 8;

// ── chat ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MessageProps {
    group: bool,
}

/// Analytics `chat.message.send` (user = sender, subject = conversation,
/// props `{ kind, group, forwarded }`): a message, aimed at the other
/// member when the conversation is a direct one.
pub fn chat_message(event: &Event) -> Vec<Deed> {
    let (Some(sender), Some(tracked)) = (event.actor_id, payload::<Tracked>(event)) else { return Vec::new() };
    let mut deed = Deed::by(sender);
    let group = MessageProps::deserialize(&tracked.props).map(|props| props.group).unwrap_or(false);
    if let Some(conversation) = tracked.subject_id.filter(|_| !group) {
        deed.towards.push(Target::DirectChat(conversation));
    }
    vec![deed]
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CallEnded {
    duration_seconds: i64,
    participant_ids: Vec<Uuid>,
}

/// Shortest call that counts.
const MIN_CALL_SECONDS: i64 = 60;

/// `chat.call.ended { duration_seconds, participant_ids, … }`: everyone who
/// was on a call of at least a minute, for its length in minutes. A call
/// between two people is aimed at each other; a group call at nobody.
pub fn call_ended(event: &Event) -> Vec<Deed> {
    let Some(call) = payload::<CallEnded>(event) else { return Vec::new() };
    let mut participants = call.participant_ids;
    participants.sort_unstable();
    participants.dedup();
    if call.duration_seconds < MIN_CALL_SECONDS || participants.len() < 2 {
        return Vec::new();
    }
    let minutes = call.duration_seconds / 60;
    let pair = participants.len() == 2;
    participants
        .iter()
        .map(|user| {
            let other = participants.iter().find(|other| pair && *other != user).copied();
            Deed::by(*user).amount(minutes).towards(other)
        })
        .collect()
}

// ── stamps ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PackPublished {
    creator_id: Option<Uuid>,
    official: bool,
}

/// `stamp.pack.published { pack_id, creator_id, official }`: a creator's
/// pack went live. Official packs belong to nobody.
pub fn pack_published(event: &Event) -> Vec<Deed> {
    let Some(pack) = payload::<PackPublished>(event) else { return Vec::new() };
    pack.creator_id.filter(|_| !pack.official).map(Deed::by).into_iter().collect()
}

// ── notes ──

/// `tokuly.common.v1.Visibility.VISIBILITY_PRIVATE`.
const VISIBILITY_PRIVATE: i32 = 5;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct NoteCreated {
    author_id: Option<Uuid>,
    visibility: i32,
    reply_to_id: Option<Uuid>,
}

/// `notes.note.created { author_id, visibility, reply_to_id, … }`: a note
/// or quote note somebody else can see. Replies are counted by their
/// activity entry (which names who was answered); notes to oneself are not
/// counted at all.
pub fn note_posted(event: &Event) -> Vec<Deed> {
    let Some(note) = payload::<NoteCreated>(event) else { return Vec::new() };
    if note.reply_to_id.is_some() || note.visibility == VISIBILITY_PRIVATE {
        return Vec::new();
    }
    note.author_id.map(Deed::by).into_iter().collect()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Story {
    owner_id: Option<Uuid>,
    audience: i32,
}

/// `tokuly.notes.v1.StoryAudience`: 2 = close friends only.
const STORY_CLOSE_FRIENDS: i32 = 2;

/// `notes.story.created { owner_id, audience, … }`: a story for the
/// owner's friends. A story for chosen close friends is not counted: the
/// friends who were not chosen are not to know it exists.
pub fn story_posted(event: &Event) -> Vec<Deed> {
    let Some(story) = payload::<Story>(event) else { return Vec::new() };
    story.owner_id.filter(|_| story.audience != STORY_CLOSE_FRIENDS).map(Deed::by).into_iter().collect()
}

/// Analytics `note.like` (user = who liked, owner = the note's author): the
/// like given, aimed at the author.
pub fn like_given(event: &Event) -> Vec<Deed> {
    let (Some(liker), Some(tracked)) = (event.actor_id, payload::<Tracked>(event)) else { return Vec::new() };
    vec![Deed::by(liker).towards(tracked.owner_id)]
}

/// Analytics `note.like`: the same like, from the author's side.
pub fn like_received(event: &Event) -> Vec<Deed> {
    let Some(tracked) = payload::<Tracked>(event) else { return Vec::new() };
    tracked.owner_id.filter(|author| Some(*author) != event.actor_id).map(Deed::by).into_iter().collect()
}

// ── video ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct VideoPublished {
    owner_id: Option<Uuid>,
    first_publication: bool,
}

/// `video.video.published { owner_id, first_publication, … }`: a video
/// reached an audience for the first time (republishing does not count).
pub fn video_published(event: &Event) -> Vec<Deed> {
    let Some(video) = payload::<VideoPublished>(event) else { return Vec::new() };
    video.owner_id.filter(|_| video.first_publication).map(Deed::by).into_iter().collect()
}

// ── engagement ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Comment {
    author_id: Option<Uuid>,
    subject_owner_id: Option<Uuid>,
    channel_id: Option<Uuid>,
}

/// `engagement.comment.published { author_id, subject_owner_id, channel_id, … }`:
/// a comment written under the author's own name, aimed at whoever owns
/// what was commented on.
pub fn comment(event: &Event) -> Vec<Deed> {
    let Some(Comment { author_id: Some(author), subject_owner_id, channel_id }) = payload(event) else {
        return Vec::new();
    };
    // A comment written as a channel never reveals the account behind it.
    // Neither may anything it earns: a streak moving with the subject's
    // owner, or XP showing up on a leaderboard, would.
    if channel_id.is_some() {
        return Vec::new();
    }
    vec![Deed::by(author).towards(subject_owner_id)]
}

/// `ReactionKind`: 1 like, 2 dislike, 3 emoji; 0 = none.
const REACTION_NONE: i32 = 0;
const REACTION_DISLIKE: i32 = 2;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Reaction {
    user_id: Option<Uuid>,
    subject_owner_id: Option<Uuid>,
    kind: i32,
    previous_kind: i32,
}

impl Reaction {
    /// A like or emoji where there was no reaction before. Changing or
    /// removing a reaction, and dislikes, earn nothing.
    fn is_new_like(&self) -> bool {
        self.previous_kind == REACTION_NONE && self.kind != REACTION_NONE && self.kind != REACTION_DISLIKE
    }
}

/// `engagement.reaction.changed`: a new like or emoji reaction, aimed at
/// the owner of the subject.
pub fn reaction_given(event: &Event) -> Vec<Deed> {
    let Some(reaction) = payload::<Reaction>(event) else { return Vec::new() };
    match reaction.user_id {
        Some(user) if reaction.is_new_like() => vec![Deed::by(user).towards(reaction.subject_owner_id)],
        _ => Vec::new(),
    }
}

/// `engagement.reaction.changed`: the same reaction, from the owner's side.
pub fn reaction_received(event: &Event) -> Vec<Deed> {
    let Some(reaction) = payload::<Reaction>(event) else { return Vec::new() };
    if !reaction.is_new_like() {
        return Vec::new();
    }
    reaction
        .subject_owner_id
        .filter(|owner| !owner.is_nil() && Some(*owner) != reaction.user_id)
        .map(Deed::by)
        .into_iter()
        .collect()
}

// ── wallet ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BalanceChanged {
    user_id: Option<Uuid>,
    reason: String,
}

/// `wallet.balance.changed { user_id, reason, … }` with reason
/// `daily_bonus`: the daily login bonus was collected.
pub fn daily_bonus(event: &Event) -> Vec<Deed> {
    let Some(change) = payload::<BalanceChanged>(event) else { return Vec::new() };
    change.user_id.filter(|_| change.reason == "daily_bonus").map(Deed::by).into_iter().collect()
}

// ── ask ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AnswerPublished {
    owner_id: Option<Uuid>,
    audience: i32,
    first: bool,
}

/// `tokuly.ask.v1.AnswerAudience`: 3 = private (only the asker and the owner).
const ANSWER_PRIVATE: i32 = 3;

/// `ask.answer.published { owner_id, first, … }`: the owner of a question
/// box answered a question. `first: false` is an edit of an answer's
/// audience, not another answer, and an answer only the asker may see is
/// not counted (nobody else is to know it exists). The payload says nothing
/// about who asked, and neither does the deed.
pub fn answer_published(event: &Event) -> Vec<Deed> {
    let Some(answer) = payload::<AnswerPublished>(event) else { return Vec::new() };
    answer.owner_id.filter(|_| answer.first && answer.audience != ANSWER_PRIVATE).map(Deed::by).into_iter().collect()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PollRound {
    user_id: Option<Uuid>,
    votes: i64,
}

/// Most votes one round is counted for.
const MAX_ROUND_VOTES: i64 = 50;

/// `ask.round.completed { user_id, votes, … }`: a finished round of
/// compliment polls, for the votes cast in it (skipped questions are not
/// votes). Who was picked is not in the payload.
pub fn poll_round(event: &Event) -> Vec<Deed> {
    let Some(round) = payload::<PollRound>(event) else { return Vec::new() };
    round
        .user_id
        .filter(|_| round.votes > 0)
        .map(|user| Deed::by(user).amount(round.votes.min(MAX_ROUND_VOTES)))
        .into_iter()
        .collect()
}

// ── domains that are still being built ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Footprint {
    user_id: Option<Uuid>,
    new_cells: i64,
}

/// `geo.footprint.expanded { user_id, new_cells }` (expected): map cells
/// uncovered for the first time.
pub fn footprint(event: &Event) -> Vec<Deed> {
    let Some(footprint) = payload::<Footprint>(event) else { return Vec::new() };
    let user = footprint.user_id.or(event.actor_id);
    user.filter(|_| footprint.new_cells > 0)
        .map(|user| Deed::by(user).amount(footprint.new_cells))
        .into_iter()
        .collect()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Visit {
    user_id: Option<Uuid>,
    first_visit: Option<bool>,
}

/// `geo.visit.started { user_id, first_visit }` (expected): a visit to a
/// place the player had not been to (every visit counts when the producer
/// does not say).
pub fn first_visit(event: &Event) -> Vec<Deed> {
    let Some(visit) = payload::<Visit>(event) else { return Vec::new() };
    let user = visit.user_id.or(event.actor_id);
    user.filter(|_| visit.first_visit.unwrap_or(true)).map(Deed::by).into_iter().collect()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StudySession {
    user_id: Option<Uuid>,
    minutes: Option<i64>,
    duration_seconds: Option<i64>,
}

/// Longest study session counted, in minutes.
const MAX_STUDY_MINUTES: i64 = 12 * 60;

/// `school.session.completed { user_id, minutes | duration_seconds }`
/// (expected): a finished study session of at least a minute.
pub fn study_session(event: &Event) -> Vec<Deed> {
    let Some(session) = payload::<StudySession>(event) else { return Vec::new() };
    let minutes =
        session.minutes.or(session.duration_seconds.map(|seconds| seconds / 60)).unwrap_or(0).min(MAX_STUDY_MINUTES);
    let user = session.user_id.or(event.actor_id);
    user.filter(|_| minutes >= 1).map(|user| Deed::by(user).amount(minutes)).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;

    fn event(topic: &str, actor: Option<Uuid>, data: serde_json::Value) -> Event {
        Event { id: Uuid::now_v7(), topic: topic.to_owned(), actor_id: actor, occurred_at: Utc::now(), data }
    }

    fn ids() -> (Uuid, Uuid) {
        (Uuid::now_v7(), Uuid::now_v7())
    }

    #[test]
    fn a_bump_between_friends_is_two_deeds_aimed_at_each_other() {
        let (a, b) = ids();
        let deeds = bump(&event(
            "social.bump.created",
            Some(a),
            json!({ "bump_id": Uuid::now_v7(), "user_ids": [a, b], "friends": true, "lat": 35.0, "lng": 139.0 }),
        ));
        assert_eq!(deeds, vec![Deed::by(a).towards(Some(b)), Deed::by(b).towards(Some(a))]);
        assert!(
            bump(&event("social.bump.created", Some(a), json!({ "user_ids": [a, b], "friends": false }))).is_empty()
        );
        assert!(bump(&event("social.bump.created", Some(a), json!({ "user_ids": [a], "friends": true }))).is_empty());
    }

    #[test]
    fn payloads_that_cannot_be_read_yield_nothing() {
        assert!(friend_accepted(&event("social.friend.accepted", None, json!({ "user_id": "not-a-uuid" }))).is_empty());
        assert!(friend_accepted(&event("social.friend.accepted", None, json!("a string"))).is_empty());
        assert!(friend_accepted(&event("social.friend.accepted", None, json!({}))).is_empty());
        assert!(friend_accepted(&event("social.friend.accepted", None, serde_json::Value::Null)).is_empty());
        assert!(user(&event("identity.user.created", None, json!({ "handle": "x" }))).is_empty());
        assert!(actor(&event("plans.plan.created", None, json!({}))).is_empty());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let (a, b) = ids();
        let deeds = friend_accepted(&event(
            "social.friend.accepted",
            Some(a),
            json!({ "user_id": a, "friend_id": b, "source": 3, "new_field": { "x": 1 } }),
        ));
        assert_eq!(deeds, vec![Deed::by(a), Deed::by(b)]);
    }

    #[test]
    fn a_direct_message_is_aimed_at_the_conversation_and_a_group_message_at_nobody() {
        let (sender, conversation) = ids();
        let tracked = AnalyticsEvent::new("chat.message.send")
            .user(Some(sender))
            .subject("conversation", conversation)
            .props(json!({ "kind": 1, "group": false, "forwarded": false }));
        let direct = chat_message(&analytics_as_event(tracked));
        assert_eq!(direct, vec![Deed { user: sender, amount: 1, towards: vec![Target::DirectChat(conversation)] }]);
        let tracked = AnalyticsEvent::new("chat.message.send")
            .user(Some(sender))
            .subject("conversation", conversation)
            .props(json!({ "kind": 1, "group": true }));
        assert_eq!(chat_message(&analytics_as_event(tracked)), vec![Deed::by(sender)]);
        // Without a sender there is nobody to credit.
        assert!(chat_message(&analytics_as_event(AnalyticsEvent::new("chat.message.send"))).is_empty());
    }

    #[test]
    fn calls_count_in_minutes_and_only_pairs_are_aimed() {
        let (a, b) = ids();
        let c = Uuid::now_v7();
        let pair =
            call_ended(&event("chat.call.ended", None, json!({ "duration_seconds": 185, "participant_ids": [a, b] })));
        assert_eq!(pair.len(), 2);
        assert!(pair.iter().all(|deed| deed.amount == 3 && deed.towards.len() == 1));
        let group = call_ended(&event(
            "chat.call.ended",
            None,
            json!({ "duration_seconds": 600, "participant_ids": [a, b, c, a] }),
        ));
        assert_eq!(group.len(), 3);
        assert!(group.iter().all(|deed| deed.amount == 10 && deed.towards.is_empty()));
        assert!(
            call_ended(&event("chat.call.ended", None, json!({ "duration_seconds": 30, "participant_ids": [a, b] })))
                .is_empty()
        );
        assert!(
            call_ended(&event("chat.call.ended", None, json!({ "duration_seconds": 300, "participant_ids": [a] })))
                .is_empty()
        );
    }

    #[test]
    fn private_notes_and_replies_are_not_posts() {
        let author = Uuid::now_v7();
        assert_eq!(
            note_posted(&event(
                "notes.note.created",
                Some(author),
                json!({ "note_id": Uuid::now_v7(), "author_id": author, "visibility": 1, "reply_to_id": null, "quote_of_id": null })
            )),
            vec![Deed::by(author)]
        );
        assert!(
            note_posted(&event("notes.note.created", Some(author), json!({ "author_id": author, "visibility": 5 })))
                .is_empty()
        );
        assert!(
            note_posted(&event(
                "notes.note.created",
                Some(author),
                json!({ "author_id": author, "visibility": 1, "reply_to_id": Uuid::now_v7() })
            ))
            .is_empty()
        );
    }

    #[test]
    fn what_hides_its_author_or_its_audience_earns_nothing() {
        let (author, owner) = ids();
        let own_name = event(
            "engagement.comment.published",
            Some(author),
            json!({ "comment_id": Uuid::now_v7(), "author_id": author, "subject_owner_id": owner, "channel_id": null }),
        );
        assert_eq!(comment(&own_name), vec![Deed::by(author).towards(Some(owner))]);
        // Written as a channel: the account behind it stays unknown.
        let as_channel = event(
            "engagement.comment.published",
            Some(author),
            json!({ "comment_id": Uuid::now_v7(), "author_id": author, "subject_owner_id": owner, "channel_id": Uuid::now_v7() }),
        );
        assert!(comment(&as_channel).is_empty());

        let story = |audience: i32| {
            story_posted(&event(
                "notes.story.created",
                Some(author),
                json!({ "story_id": Uuid::now_v7(), "owner_id": author, "audience": audience }),
            ))
        };
        assert_eq!(story(1), vec![Deed::by(author)]);
        assert!(story(2).is_empty(), "a close-friends story is not for the other friends to notice");
    }

    #[test]
    fn only_new_likes_count_and_both_sides_see_them() {
        let (user, owner) = ids();
        let new_like = event(
            "engagement.reaction.changed",
            Some(user),
            json!({ "user_id": user, "subject_owner_id": owner, "kind": 1, "previous_kind": 0 }),
        );
        assert_eq!(reaction_given(&new_like), vec![Deed::by(user).towards(Some(owner))]);
        assert_eq!(reaction_received(&new_like), vec![Deed::by(owner)]);
        for (kind, previous) in [(0, 1), (3, 1), (2, 0)] {
            let other = event(
                "engagement.reaction.changed",
                Some(user),
                json!({ "user_id": user, "subject_owner_id": owner, "kind": kind, "previous_kind": previous }),
            );
            assert!(reaction_given(&other).is_empty() && reaction_received(&other).is_empty());
        }
        // Liking one's own content gives the owner nothing; official content belongs to nobody.
        let own = event(
            "engagement.reaction.changed",
            Some(user),
            json!({ "user_id": user, "subject_owner_id": user, "kind": 1, "previous_kind": 0 }),
        );
        assert!(reaction_received(&own).is_empty());
        assert_eq!(reaction_given(&own), vec![Deed::by(user)]);
        let official = event(
            "engagement.reaction.changed",
            Some(user),
            json!({ "user_id": user, "subject_owner_id": Uuid::nil(), "kind": 1, "previous_kind": 0 }),
        );
        assert!(reaction_received(&official).is_empty());
    }

    #[test]
    fn only_the_daily_bonus_counts_among_balance_changes() {
        let user = Uuid::now_v7();
        assert_eq!(
            daily_bonus(&event(
                "wallet.balance.changed",
                Some(user),
                json!({ "user_id": user, "coins": 10, "reason": "daily_bonus" })
            )),
            vec![Deed::by(user)]
        );
        assert!(
            daily_bonus(&event(
                "wallet.balance.changed",
                Some(user),
                json!({ "user_id": user, "coins": 10, "reason": "quest.level_up" })
            ))
            .is_empty()
        );
    }

    #[test]
    fn answers_count_once_and_poll_rounds_by_their_votes() {
        let owner = Uuid::now_v7();
        let answered = event(
            "ask.answer.published",
            Some(owner),
            json!({ "question_id": Uuid::now_v7(), "owner_id": owner, "audience": 1, "first": true }),
        );
        assert_eq!(answer_published(&answered), vec![Deed::by(owner)]);
        // An answer only the asker sees leaves no trace.
        let private = event(
            "ask.answer.published",
            Some(owner),
            json!({ "question_id": Uuid::now_v7(), "owner_id": owner, "audience": 3, "first": true }),
        );
        assert!(answer_published(&private).is_empty());
        // Changing who may see an answer is not answering again.
        let edited = event(
            "ask.answer.published",
            Some(owner),
            json!({ "question_id": Uuid::now_v7(), "owner_id": owner, "audience": 2, "first": false }),
        );
        assert!(answer_published(&edited).is_empty());

        let round = |votes: i64| {
            poll_round(&event(
                "ask.round.completed",
                Some(owner),
                json!({ "user_id": owner, "round_id": Uuid::now_v7(), "votes": votes, "skipped": 2, "coins": 5 }),
            ))
        };
        assert_eq!(round(9), vec![Deed::by(owner).amount(9)]);
        assert!(round(0).is_empty(), "a round of nothing but skips is no vote");
        assert_eq!(round(10_000)[0].amount, MAX_ROUND_VOTES);
    }

    #[test]
    fn study_sessions_accept_minutes_or_seconds() {
        let user = Uuid::now_v7();
        let minutes =
            |data| study_session(&event("school.session.completed", Some(user), data)).first().map(|deed| deed.amount);
        assert_eq!(minutes(json!({ "user_id": user, "minutes": 45 })), Some(45));
        assert_eq!(minutes(json!({ "duration_seconds": 1830 })), Some(30));
        assert_eq!(minutes(json!({ "minutes": 100_000 })), Some(MAX_STUDY_MINUTES));
        assert_eq!(minutes(json!({ "duration_seconds": 20 })), None);
        assert_eq!(minutes(json!({})), None);
    }

    #[test]
    fn activities_name_who_they_were_done_with() {
        let (user, friend) = ids();
        let deeds = activity(&event(
            "activity.recorded",
            Some(user),
            json!({ "user_id": user, "kind": "geo.checkin", "audience": "friends", "with_user_ids": [friend, user], "summary": { "ja": "a", "en": "b" } }),
        ));
        assert_eq!(deeds, vec![Deed::by(user).towards(Some(friend))]);
        // What the player kept to themselves is not counted.
        let hidden = activity(&event(
            "activity.recorded",
            Some(user),
            json!({ "user_id": user, "kind": "geo.checkin", "audience": "private", "with_user_ids": [friend] }),
        ));
        assert!(hidden.is_empty());
        let many: Vec<Uuid> = (0..20).map(|_| Uuid::now_v7()).collect();
        let deeds =
            activity(&event("activity.recorded", Some(user), json!({ "user_id": user, "with_user_ids": many })));
        assert_eq!(deeds[0].towards.len(), MAX_TARGETS);
    }
}
