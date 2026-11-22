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
//! Only XP has a daily cap; the counters behind missions and badges do not.
//! So a reader also says when a deed counts only **once** ([`Once`]): a
//! like can be taken back and given again, a friendship ended and renewed,
//! a pack removed and added, each time with a new event. Such deeds carry
//! a key (the note liked, the friend made), and the ingest counts a key
//! once per player — for good, or once a day where repeating it tomorrow
//! is honest. And nothing a player does to their own content counts.
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

/// When a deed counts only once for its doer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Once {
    /// What it is about: `like:<note>`, `friend:<user>`, ...
    pub key: String,
    /// Once per local day rather than once ever.
    pub daily: bool,
    /// The other person the key names, if any (their deletion removes it).
    pub about: Option<Uuid>,
}

/// One thing one player did, as far as the rules care.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deed {
    pub user: Uuid,
    /// How much of it: minutes, votes... 1 for plain actions.
    pub amount: i64,
    /// The people it was aimed at. Only friends among them count (for the
    /// streak with each, and as "friends reached").
    pub towards: Vec<Target>,
    /// Set when repeating the deed (after undoing it, or just again) must
    /// not count again.
    pub once: Option<Once>,
}

impl Deed {
    pub fn by(user: Uuid) -> Self {
        Self { user, amount: 1, towards: Vec::new(), once: None }
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

    /// Counts once ever per `key`.
    pub fn once(mut self, key: String) -> Self {
        self.once = Some(Once { key, daily: false, about: None });
        self
    }

    /// Says which other person the once-key names.
    pub fn about(mut self, person: Uuid) -> Self {
        if let Some(once) = &mut self.once {
            once.about = Some(person);
        }
        self
    }

    /// Counts once per local day per `key`.
    pub fn once_a_day(mut self, key: String) -> Self {
        self.once = Some(Once { key, daily: true, about: None });
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

/// Analytics events that name nothing but their user: the deed is the
/// tracked user's (`stamp.send`).
pub fn actor(event: &Event) -> Vec<Deed> {
    event.actor_id.map(Deed::by).into_iter().collect()
}

// ── identity ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UserRef {
    user_id: Option<Uuid>,
}

/// `identity.user.created { user_id }`: once per account.
pub fn joined(event: &Event) -> Vec<Deed> {
    payload::<UserRef>(event)
        .and_then(|p| p.user_id)
        .map(|user| Deed::by(user).once("joined".to_owned()))
        .into_iter()
        .collect()
}

// ── social ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FriendAccepted {
    user_id: Option<Uuid>,
    friend_id: Option<Uuid>,
}

/// `social.friend.accepted { user_id, friend_id }`: both made a friend.
/// Each friend counts once for the other, ever: unfriending and making up
/// is not a new friend.
pub fn friend_accepted(event: &Event) -> Vec<Deed> {
    let Some(FriendAccepted { user_id: Some(user), friend_id: Some(friend) }) = payload(event) else {
        return Vec::new();
    };
    if user == friend {
        return Vec::new();
    }
    vec![
        Deed::by(user).once(format!("friend:{friend}")).about(friend),
        Deed::by(friend).once(format!("friend:{user}")).about(user),
    ]
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Bump {
    user_ids: Vec<Uuid>,
    friends: bool,
}

/// `social.bump.created { user_ids: [a, b], friends }`: two friends bumped,
/// each towards the other. Bumps between strangers do not count, and the
/// same two count once a day.
pub fn bump(event: &Event) -> Vec<Deed> {
    let Some(bump) = payload::<Bump>(event) else { return Vec::new() };
    match bump.user_ids.as_slice() {
        [a, b] if bump.friends && a != b => {
            vec![
                Deed::by(*a).towards(Some(*b)).once_a_day(format!("bump:{b}")).about(*b),
                Deed::by(*b).towards(Some(*a)).once_a_day(format!("bump:{a}")).about(*a),
            ]
        }
        _ => Vec::new(),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Intro {
    inviter_id: Option<Uuid>,
    user_id: Option<Uuid>,
}

/// `social.intro.created { inviter_id, user_id }`: the inviter brought
/// someone in (each newcomer once).
pub fn intro(event: &Event) -> Vec<Deed> {
    let Some(Intro { inviter_id: Some(inviter), user_id: Some(newcomer) }) = payload(event) else { return Vec::new() };
    if inviter == newcomer {
        return Vec::new();
    }
    vec![Deed::by(inviter).once(format!("intro:{newcomer}")).about(newcomer)]
}

// ── the generic requests of `tokuly_core::outbound` ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Notification {
    kind: String,
    user_id: Option<Uuid>,
    actor_id: Option<Uuid>,
}

/// `notify.requested` (`NotificationRequest`): the actor did something
/// towards the recipient. The rule names the notification kind. One
/// recipient counts once a day per kind, however often they are poked.
pub fn notification(event: &Event) -> Vec<Deed> {
    let Some(Notification { kind, user_id: Some(recipient), actor_id: Some(actor) }) = payload(event) else {
        return Vec::new();
    };
    if recipient == actor {
        return Vec::new();
    }
    vec![Deed::by(actor).towards(Some(recipient)).once_a_day(format!("{kind}:{recipient}")).about(recipient)]
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Activity {
    user_id: Option<Uuid>,
    audience: String,
    /// `(SubjectType as i32, id)`.
    subject: Option<(i32, Uuid)>,
    with_user_ids: Vec<Uuid>,
    data: serde_json::Value,
}

/// `ActivityAudience::Private`: an entry only its author may see.
const AUDIENCE_PRIVATE: &str = "private";
/// Most people one deed can be aimed at.
const MAX_TARGETS: usize = 8;

/// An activity entry somebody else may see, with its author. What a player
/// keeps to themselves (a check-in in ghost mode, a region while their
/// footprints are hidden, studying they do not show) earns nothing: XP,
/// badges and streaks are seen by others, and would give it away.
fn shown_activity(event: &Event) -> Option<(Uuid, Activity)> {
    let activity = payload::<Activity>(event)?;
    let user = activity.user_id?;
    (activity.audience != AUDIENCE_PRIVATE).then_some((user, activity))
}

/// The deed of an activity entry, aimed at the people it names.
fn activity_deed(user: Uuid, activity: &Activity) -> Deed {
    let mut deed = Deed::by(user);
    // An entry can name many people; more than a handful is not "towards" anyone.
    for other in activity.with_user_ids.iter().take(MAX_TARGETS) {
        deed = deed.towards(Some(*other));
    }
    deed
}

/// `activity.recorded` (`ActivityEntry`): the user did it, towards the
/// people the entry names. The rule names the activity kind. Entries
/// recorded as private are not counted.
pub fn activity(event: &Event) -> Vec<Deed> {
    shown_activity(event).map(|(user, activity)| activity_deed(user, &activity)).into_iter().collect()
}

/// `activity.recorded`, kind `geo.checkin`: a check-in the user shows their
/// friends, aimed at the friends tagged. One place counts once a day.
pub fn check_in(event: &Event) -> Vec<Deed> {
    let Some((user, activity)) = shown_activity(event) else { return Vec::new() };
    let place = activity.subject.map(|(_, id)| id.to_string()).unwrap_or_else(|| "somewhere".to_owned());
    vec![activity_deed(user, &activity).once_a_day(format!("checkin:{place}"))]
}

/// `activity.recorded`, kind `geo.new_region` (`data.region_id`): a region
/// entered for the first time; each region once.
pub fn new_region(event: &Event) -> Vec<Deed> {
    let Some((user, activity)) = shown_activity(event) else { return Vec::new() };
    let Some(region) = activity.data.get("region_id").map(|id| id.to_string()) else { return Vec::new() };
    vec![Deed::by(user).once(format!("region:{region}"))]
}

/// Longest study session counted, in minutes.
const MAX_STUDY_MINUTES: i64 = 12 * 60;

/// `activity.recorded`, kind `school.studied` (`data = { session_id,
/// minutes, … }`): a timed study session, for its minutes.
///
/// The school domain's own event (`school.study.session_ended`) says
/// neither whether the player shows their studying to friends nor is it
/// limited to timed sessions. This entry is recorded exactly for what may
/// count here: sessions the server timed (not time logged by hand), of
/// players who show that they study.
pub fn studied(event: &Event) -> Vec<Deed> {
    let Some((user, activity)) = shown_activity(event) else { return Vec::new() };
    let minutes = activity.data.get("minutes").and_then(|minutes| minutes.as_i64()).unwrap_or(0).min(MAX_STUDY_MINUTES);
    let Some(session) = activity.data.get("session_id").map(|id| id.to_string()) else { return Vec::new() };
    if minutes < 1 {
        return Vec::new();
    }
    vec![Deed::by(user).amount(minutes).once(format!("study:{session}"))]
}

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
struct Pack {
    pack_id: Option<Uuid>,
    user_id: Option<Uuid>,
    creator_id: Option<Uuid>,
    official: bool,
}

/// `stamp.pack.installed { pack_id, user_id }`: a pack added to the
/// library; each pack once, however often it is removed and added again.
pub fn pack_installed(event: &Event) -> Vec<Deed> {
    let Some(Pack { pack_id: Some(pack), user_id: Some(user), .. }) = payload(event) else { return Vec::new() };
    vec![Deed::by(user).once(format!("pack:{pack}"))]
}

/// `stamp.pack.published { pack_id, creator_id, official }`: a creator's
/// pack went live (each pack once; official packs belong to nobody).
pub fn pack_published(event: &Event) -> Vec<Deed> {
    let Some(Pack { pack_id: Some(pack), creator_id: Some(creator), official: false, .. }) = payload(event) else {
        return Vec::new();
    };
    vec![Deed::by(creator).once(format!("published:{pack}"))]
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

/// Analytics `note.like` (user = who liked, subject = the note, owner =
/// its author): the like given, aimed at the author. A note is liked once,
/// whatever happens to the like afterwards; liking one's own note is not a
/// like.
pub fn like_given(event: &Event) -> Vec<Deed> {
    let (Some(liker), Some(Tracked { subject_id: Some(note), owner_id, .. })) =
        (event.actor_id, payload::<Tracked>(event))
    else {
        return Vec::new();
    };
    if owner_id == Some(liker) {
        return Vec::new();
    }
    vec![Deed::by(liker).towards(owner_id).once(format!("like:{note}"))]
}

/// Analytics `note.like`: the same like, from the author's side (once per
/// note and person liking it).
pub fn like_received(event: &Event) -> Vec<Deed> {
    let (Some(liker), Some(Tracked { subject_id: Some(note), owner_id: Some(author), .. })) =
        (event.actor_id, payload::<Tracked>(event))
    else {
        return Vec::new();
    };
    if author == liker || author.is_nil() {
        return Vec::new();
    }
    vec![Deed::by(author).once(format!("liked:{note}:{liker}")).about(liker)]
}

// ── video ──

/// `tokuly.common.v1.Visibility.VISIBILITY_PUBLIC`.
const VISIBILITY_PUBLIC: i32 = 1;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct VideoPublished {
    video_id: Option<Uuid>,
    owner_id: Option<Uuid>,
    visibility: i32,
    first_publication: bool,
}

/// `video.video.published { video_id, owner_id, visibility,
/// first_publication }`: a public video reached its audience for the first
/// time. Republishing does not count, and neither does a video that is not
/// public: a badge dated to the day of an unlisted or friends-only upload
/// would tell that it exists.
pub fn video_published(event: &Event) -> Vec<Deed> {
    let Some(VideoPublished {
        video_id: Some(video),
        owner_id: Some(owner),
        visibility: VISIBILITY_PUBLIC,
        first_publication: true,
    }) = payload(event)
    else {
        return Vec::new();
    };
    vec![Deed::by(owner).once(format!("video:{video}"))]
}

// ── engagement ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Comment {
    author_id: Option<Uuid>,
    subject_id: Option<Uuid>,
    subject_owner_id: Option<Uuid>,
    channel_id: Option<Uuid>,
}

/// `engagement.comment.published { author_id, subject_id,
/// subject_owner_id, channel_id }`: a comment written under the author's
/// own name on somebody else's content, aimed at its owner. One subject
/// counts once a day (deleting a comment and writing it again is not two).
pub fn comment(event: &Event) -> Vec<Deed> {
    let Some(Comment { author_id: Some(author), subject_id: Some(subject), subject_owner_id, channel_id }) =
        payload(event)
    else {
        return Vec::new();
    };
    // A comment written as a channel never reveals the account behind it.
    // Neither may anything it earns: a streak moving with the subject's
    // owner, or XP showing up on a leaderboard, would.
    if channel_id.is_some() || subject_owner_id == Some(author) {
        return Vec::new();
    }
    vec![Deed::by(author).towards(subject_owner_id).once_a_day(format!("comment:{subject}"))]
}

/// `ReactionKind`: 1 like, 2 dislike, 3 emoji; 0 = none.
const REACTION_NONE: i32 = 0;
const REACTION_DISLIKE: i32 = 2;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Reaction {
    user_id: Option<Uuid>,
    subject_id: Option<Uuid>,
    subject_owner_id: Option<Uuid>,
    kind: i32,
    previous_kind: i32,
}

impl Reaction {
    /// Who reacted to what of whom, when the event is a like or emoji where
    /// there was no reaction before, on somebody else's content. Changing
    /// or removing a reaction, dislikes and reactions to one's own content
    /// earn nothing.
    fn new_like(&self) -> Option<(Uuid, Uuid, Option<Uuid>)> {
        let (user, subject) = (self.user_id?, self.subject_id?);
        let fresh = self.previous_kind == REACTION_NONE && self.kind != REACTION_NONE && self.kind != REACTION_DISLIKE;
        (fresh && self.subject_owner_id != Some(user)).then_some((user, subject, self.subject_owner_id))
    }
}

/// `engagement.reaction.changed`: a new like or emoji reaction, aimed at
/// the owner of the subject. A subject is reacted to once: taking the
/// reaction back and giving it again does not count again.
pub fn reaction_given(event: &Event) -> Vec<Deed> {
    let Some((user, subject, owner)) = payload::<Reaction>(event).and_then(|reaction| reaction.new_like()) else {
        return Vec::new();
    };
    vec![Deed::by(user).towards(owner).once(format!("react:{subject}"))]
}

/// `engagement.reaction.changed`: the same reaction, from the owner's side
/// (once per subject and person reacting).
pub fn reaction_received(event: &Event) -> Vec<Deed> {
    let Some((user, subject, Some(owner))) = payload::<Reaction>(event).and_then(|reaction| reaction.new_like()) else {
        return Vec::new();
    };
    if owner.is_nil() {
        return Vec::new();
    }
    vec![Deed::by(owner).once(format!("reacted:{subject}:{user}")).about(user)]
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Share {
    user_id: Option<Uuid>,
    subject_id: Option<Uuid>,
}

/// `engagement.share.recorded { user_id, subject_id, … }`: something
/// shared; each thing once.
pub fn share(event: &Event) -> Vec<Deed> {
    let Some(Share { user_id: Some(user), subject_id: Some(subject) }) = payload(event) else { return Vec::new() };
    vec![Deed::by(user).once(format!("share:{subject}"))]
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
    change
        .user_id
        .filter(|_| change.reason == "daily_bonus")
        .map(|user| Deed::by(user).once_a_day("bonus".to_owned()))
        .into_iter()
        .collect()
}

// ── ask ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AnswerPublished {
    question_id: Option<Uuid>,
    owner_id: Option<Uuid>,
    audience: i32,
    first: bool,
}

/// `tokuly.ask.v1.AnswerAudience`: 3 = private (only the asker and the owner).
const ANSWER_PRIVATE: i32 = 3;

/// `ask.answer.published { question_id, owner_id, audience, first }`: the
/// owner of a question box answered a question. `first: false` is an edit
/// of an answer's audience, not another answer, and an answer only the
/// asker may see is not counted (nobody else is to know it exists). Each
/// question once. The payload says nothing about who asked, and neither
/// does the deed.
pub fn answer_published(event: &Event) -> Vec<Deed> {
    let Some(AnswerPublished { question_id: Some(question), owner_id: Some(owner), audience, first: true }) =
        payload(event)
    else {
        return Vec::new();
    };
    if audience == ANSWER_PRIVATE {
        return Vec::new();
    }
    vec![Deed::by(owner).once(format!("answer:{question}"))]
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PollRound {
    user_id: Option<Uuid>,
    round_id: Option<Uuid>,
    votes: i64,
}

/// Most votes one round is counted for.
const MAX_ROUND_VOTES: i64 = 50;

/// `ask.round.completed { user_id, round_id, votes, … }`: a finished round
/// of compliment polls, for the votes cast in it (skipped questions are not
/// votes). Who was picked is not in the payload.
pub fn poll_round(event: &Event) -> Vec<Deed> {
    let Some(PollRound { user_id: Some(user), round_id: Some(round), votes }) = payload(event) else {
        return Vec::new();
    };
    if votes < 1 {
        return Vec::new();
    }
    vec![Deed::by(user).amount(votes.min(MAX_ROUND_VOTES)).once(format!("round:{round}"))]
}

// ── plans ──

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PlanCreated {
    host_id: Option<Uuid>,
}

/// `plans.plan.created { plan_id, host_id, … }`: the host made a plan. One
/// a day counts: a plan can be called off and made again.
pub fn plan_created(event: &Event) -> Vec<Deed> {
    payload::<PlanCreated>(event)
        .and_then(|plan| plan.host_id)
        .map(|host| Deed::by(host).once_a_day("plan".to_owned()))
        .into_iter()
        .collect()
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

    /// Who the deeds credit, and the key each counts once for.
    fn keys(deeds: &[Deed]) -> Vec<(Uuid, Option<(String, bool)>)> {
        deeds.iter().map(|deed| (deed.user, deed.once.as_ref().map(|once| (once.key.clone(), once.daily)))).collect()
    }

    #[test]
    fn a_bump_between_friends_is_two_deeds_aimed_at_each_other() {
        let (a, b) = ids();
        let deeds = bump(&event(
            "social.bump.created",
            Some(a),
            json!({ "bump_id": Uuid::now_v7(), "user_ids": [a, b], "friends": true, "lat": 35.0, "lng": 139.0 }),
        ));
        assert_eq!(
            deeds.iter().map(|deed| deed.towards.clone()).collect::<Vec<_>>(),
            vec![vec![Target::User(b)], vec![Target::User(a)]]
        );
        assert_eq!(keys(&deeds), vec![(a, Some((format!("bump:{b}"), true))), (b, Some((format!("bump:{a}"), true)))]);
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
        assert!(joined(&event("identity.user.created", None, json!({ "handle": "x" }))).is_empty());
        assert!(actor(&event("stamp.send", None, json!({}))).is_empty());
        assert!(
            share(&event("engagement.share.recorded", None, json!({ "user_id": null, "subject_id": Uuid::now_v7() })))
                .is_empty()
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let (a, b) = ids();
        let deeds = friend_accepted(&event(
            "social.friend.accepted",
            Some(a),
            json!({ "user_id": a, "friend_id": b, "source": 3, "new_field": { "x": 1 } }),
        ));
        assert_eq!(
            keys(&deeds),
            vec![(a, Some((format!("friend:{b}"), false))), (b, Some((format!("friend:{a}"), false)))]
        );
    }

    #[test]
    fn what_can_be_undone_and_redone_names_what_it_counts_once_for() {
        let (user, other) = ids();
        let thing = Uuid::now_v7();
        let once = |deeds: Vec<Deed>| deeds[0].once.clone().map(|once| (once.key, once.daily));
        // The same key every time the same thing is done again.
        let like = AnalyticsEvent::new("note.like").user(Some(user)).subject("note", thing).owner(other);
        assert_eq!(once(like_given(&analytics_as_event(like.clone()))), Some((format!("like:{thing}"), false)));
        assert_eq!(once(like_received(&analytics_as_event(like))), Some((format!("liked:{thing}:{user}"), false)));
        let react = event(
            "engagement.reaction.changed",
            Some(user),
            json!({ "user_id": user, "subject_id": thing, "subject_owner_id": other, "kind": 1, "previous_kind": 0 }),
        );
        assert_eq!(once(reaction_given(&react)), Some((format!("react:{thing}"), false)));
        assert_eq!(once(reaction_received(&react)), Some((format!("reacted:{thing}:{user}"), false)));
        assert_eq!(
            once(pack_installed(&event(
                "stamp.pack.installed",
                Some(user),
                json!({ "pack_id": thing, "user_id": user })
            ))),
            Some((format!("pack:{thing}"), false))
        );
        assert_eq!(
            once(pack_published(&event(
                "stamp.pack.published",
                Some(user),
                json!({ "pack_id": thing, "creator_id": user, "official": false })
            ))),
            Some((format!("published:{thing}"), false))
        );
        assert_eq!(
            once(share(&event(
                "engagement.share.recorded",
                Some(user),
                json!({ "user_id": user, "subject_id": thing })
            ))),
            Some((format!("share:{thing}"), false))
        );
        assert_eq!(
            once(intro(&event("social.intro.created", Some(other), json!({ "inviter_id": user, "user_id": other })))),
            Some((format!("intro:{other}"), false))
        );
        assert_eq!(
            once(joined(&event("identity.user.created", Some(user), json!({ "user_id": user })))),
            Some(("joined".to_owned(), false))
        );
        // Things that are honest to do again tomorrow count once a day.
        assert_eq!(
            once(plan_created(&event("plans.plan.created", Some(user), json!({ "plan_id": thing, "host_id": user })))),
            Some(("plan".to_owned(), true))
        );
        let poke = event(
            "notify.requested",
            Some(user),
            json!({ "kind": "social.map_reaction", "user_id": other, "actor_id": user }),
        );
        assert_eq!(once(notification(&poke)), Some((format!("social.map_reaction:{other}"), true)));
        let said = event(
            "engagement.comment.published",
            Some(user),
            json!({ "author_id": user, "subject_id": thing, "subject_owner_id": other }),
        );
        assert_eq!(once(comment(&said)), Some((format!("comment:{thing}"), true)));
        assert!(
            pack_published(&event(
                "stamp.pack.published",
                None,
                json!({ "pack_id": thing, "creator_id": null, "official": true })
            ))
            .is_empty()
        );
    }

    #[test]
    fn nothing_done_to_ones_own_content_counts() {
        let me = Uuid::now_v7();
        let thing = Uuid::now_v7();
        let like = AnalyticsEvent::new("note.like").user(Some(me)).subject("note", thing).owner(me);
        assert!(like_given(&analytics_as_event(like.clone())).is_empty());
        assert!(like_received(&analytics_as_event(like)).is_empty());
        let react = event(
            "engagement.reaction.changed",
            Some(me),
            json!({ "user_id": me, "subject_id": thing, "subject_owner_id": me, "kind": 1, "previous_kind": 0 }),
        );
        assert!(reaction_given(&react).is_empty() && reaction_received(&react).is_empty());
        let said = event(
            "engagement.comment.published",
            Some(me),
            json!({ "author_id": me, "subject_id": thing, "subject_owner_id": me }),
        );
        assert!(comment(&said).is_empty());
    }

    #[test]
    fn a_direct_message_is_aimed_at_the_conversation_and_a_group_message_at_nobody() {
        let (sender, conversation) = ids();
        let tracked = AnalyticsEvent::new("chat.message.send")
            .user(Some(sender))
            .subject("conversation", conversation)
            .props(json!({ "kind": 1, "group": false, "forwarded": false }));
        let direct = chat_message(&analytics_as_event(tracked));
        assert_eq!(
            direct,
            vec![Deed { user: sender, amount: 1, towards: vec![Target::DirectChat(conversation)], once: None }]
        );
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
        let thing = Uuid::now_v7();
        let own_name = event(
            "engagement.comment.published",
            Some(author),
            json!({ "comment_id": Uuid::now_v7(), "author_id": author, "subject_id": thing, "subject_owner_id": owner, "channel_id": null }),
        );
        assert_eq!(comment(&own_name)[0].towards, vec![Target::User(owner)]);
        // Written as a channel: the account behind it stays unknown.
        let as_channel = event(
            "engagement.comment.published",
            Some(author),
            json!({ "author_id": author, "subject_id": thing, "subject_owner_id": owner, "channel_id": Uuid::now_v7() }),
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

        // Only a public video counts, the first time it is published.
        let video = |visibility: i32, first: bool| {
            video_published(&event(
                "video.video.published",
                Some(author),
                json!({ "video_id": thing, "owner_id": author, "visibility": visibility, "first_publication": first }),
            ))
        };
        assert_eq!(keys(&video(1, true)), vec![(author, Some((format!("video:{thing}"), false)))]);
        for hidden in [2, 3, 4, 5] {
            assert!(
                video(hidden, true).is_empty(),
                "a video with visibility {hidden} is not everybody's to know about"
            );
        }
        assert!(video(1, false).is_empty());
    }

    #[test]
    fn only_new_likes_count() {
        let (user, owner) = ids();
        let thing = Uuid::now_v7();
        for (kind, previous) in [(0, 1), (3, 1), (2, 0)] {
            let other = event(
                "engagement.reaction.changed",
                Some(user),
                json!({ "user_id": user, "subject_id": thing, "subject_owner_id": owner, "kind": kind, "previous_kind": previous }),
            );
            assert!(reaction_given(&other).is_empty() && reaction_received(&other).is_empty());
        }
        // Official content belongs to nobody: the like counts for who gave it only.
        let official = event(
            "engagement.reaction.changed",
            Some(user),
            json!({ "user_id": user, "subject_id": thing, "subject_owner_id": Uuid::nil(), "kind": 1, "previous_kind": 0 }),
        );
        assert!(reaction_received(&official).is_empty());
        assert_eq!(reaction_given(&official).len(), 1);
    }

    #[test]
    fn only_the_daily_bonus_counts_among_balance_changes() {
        let user = Uuid::now_v7();
        assert_eq!(
            daily_bonus(&event(
                "wallet.balance.changed",
                Some(user),
                json!({ "user_id": user, "coins": 10, "reason": "daily_bonus" })
            ))
            .len(),
            1
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
        let question = Uuid::now_v7();
        let answered = event(
            "ask.answer.published",
            Some(owner),
            json!({ "question_id": question, "owner_id": owner, "audience": 1, "first": true }),
        );
        assert_eq!(keys(&answer_published(&answered)), vec![(owner, Some((format!("answer:{question}"), false)))]);
        // An answer only the asker sees leaves no trace.
        let private = event(
            "ask.answer.published",
            Some(owner),
            json!({ "question_id": question, "owner_id": owner, "audience": 3, "first": true }),
        );
        assert!(answer_published(&private).is_empty());
        // Changing who may see an answer is not answering again.
        let edited = event(
            "ask.answer.published",
            Some(owner),
            json!({ "question_id": question, "owner_id": owner, "audience": 2, "first": false }),
        );
        assert!(answer_published(&edited).is_empty());

        let round = |votes: i64| {
            poll_round(&event(
                "ask.round.completed",
                Some(owner),
                json!({ "user_id": owner, "round_id": Uuid::now_v7(), "votes": votes, "skipped": 2, "coins": 5 }),
            ))
        };
        assert_eq!(round(9)[0].amount, 9);
        assert!(round(0).is_empty(), "a round of nothing but skips is no vote");
        assert_eq!(round(10_000)[0].amount, MAX_ROUND_VOTES);
    }

    #[test]
    fn study_counts_through_what_the_player_shows_of_it() {
        let user = Uuid::now_v7();
        let session = Uuid::now_v7();
        let shown = |audience: &str, minutes: i64| {
            studied(&event(
                "activity.recorded",
                Some(user),
                json!({ "user_id": user, "kind": "school.studied", "audience": audience, "data": { "session_id": session, "minutes": minutes, "subject": "math", "in_room": false } }),
            ))
        };
        let deeds = shown("friends", 45);
        assert_eq!(
            (deeds[0].amount, deeds[0].once.clone().map(|once| once.key)),
            (45, Some(format!("study:\"{session}\"")))
        );
        assert_eq!(shown("friends", 100_000)[0].amount, MAX_STUDY_MINUTES);
        assert!(shown("private", 45).is_empty());
        assert!(shown("friends", 0).is_empty());
    }

    #[test]
    fn activities_name_who_they_were_done_with() {
        let (user, friend) = ids();
        let place = Uuid::now_v7();
        let entry = |audience: &str, with: Vec<Uuid>| {
            event(
                "activity.recorded",
                Some(user),
                json!({ "user_id": user, "kind": "geo.checkin", "audience": audience, "subject": [15, place], "with_user_ids": with, "summary": { "ja": "a", "en": "b" } }),
            )
        };
        let deeds = check_in(&entry("friends", vec![friend, user]));
        assert_eq!(deeds[0].towards, vec![Target::User(friend)]);
        assert_eq!(deeds[0].once, Some(Once { key: format!("checkin:{place}"), daily: true, about: None }));
        // What the player kept to themselves is not counted.
        assert!(check_in(&entry("private", vec![friend])).is_empty());
        assert!(activity(&entry("private", vec![friend])).is_empty());
        let many: Vec<Uuid> = (0..20).map(|_| Uuid::now_v7()).collect();
        assert_eq!(activity(&entry("friends", many))[0].towards.len(), MAX_TARGETS);
        let region = event(
            "activity.recorded",
            Some(user),
            json!({ "user_id": user, "kind": "geo.new_region", "audience": "friends", "data": { "region_id": "jp-13" } }),
        );
        assert_eq!(
            new_region(&region)[0].once,
            Some(Once { key: "region:\"jp-13\"".to_owned(), daily: false, about: None })
        );
    }
}
