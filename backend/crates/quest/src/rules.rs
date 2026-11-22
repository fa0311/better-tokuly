//! The rule table: which events are worth what.
//!
//! This file is the whole policy of the quest domain, as data. A [`Rule`]
//! ties one event (a domain event or an analytics event, optionally one
//! `kind` of a generic request) to its effects:
//!
//! - **XP**, counted against the daily cap of the rule's [`Source`];
//! - **metrics** advanced — which is what missions, counted badges and the
//!   metric leaderboards read;
//! - whether it is something done **towards a friend**, which keeps the
//!   streak between the two alive.
//!
//! Adding a way to earn XP means adding a row here (and, when the payload is
//! new, a reader in `payloads.rs`); nothing else changes. The same table is
//! served to clients (`ListEarningRules`) and documented in
//! `docs/events/quest.md` — keep the three in step.
//!
//! One action must be counted by exactly one rule. Several domains announce
//! the same thing in several ways (an event, an analytics event, an activity
//! entry); pick one.
use crate::{
    metrics::Metric,
    payloads::{self, Reader},
    text::{Text, t},
};

/// Where a rule's events come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feed {
    /// Domain events (`tokuly_core::events`, the `EVENTS` stream).
    Events,
    /// Product analytics (`core.analytics.track`, the `ANALYTICS` stream):
    /// how domains announce things that happen too often for a domain
    /// event, such as chat messages and stamp sends.
    Analytics,
}

/// A group of rules sharing one daily XP cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Source {
    Friends,
    Chat,
    Stamps,
    Notes,
    Creators,
    Reactions,
    Map,
    Study,
    Together,
    Streaks,
    Daily,
    Missions,
}

pub struct SourceDef {
    pub source: Source,
    pub key: &'static str,
    pub name: Text,
    pub emoji: &'static str,
    /// Most XP per local day; 0 = no cap.
    pub daily_cap: i32,
}

/// Every XP source, in display order.
pub const SOURCES: &[SourceDef] = &[
    SourceDef { source: Source::Friends, key: "friends", name: t("友達", "Friends"), emoji: "🤝", daily_cap: 100 },
    SourceDef { source: Source::Chat, key: "chat", name: t("チャット", "Chat"), emoji: "💬", daily_cap: 40 },
    SourceDef {
        source: Source::Stamps, key: "stamps", name: t("スタンプ", "Stamps"), emoji: "🎨", daily_cap: 20
    },
    SourceDef {
        source: Source::Notes,
        key: "notes",
        name: t("ノート・記憶", "Notes & stories"),
        emoji: "📝",
        daily_cap: 40,
    },
    SourceDef {
        source: Source::Creators, key: "creators", name: t("つくる", "Creating"), emoji: "🎬", daily_cap: 80
    },
    SourceDef {
        source: Source::Reactions,
        key: "reactions",
        name: t("いいね・コメント", "Likes & comments"),
        emoji: "❤️",
        daily_cap: 30,
    },
    SourceDef { source: Source::Map, key: "map", name: t("マップ", "Map"), emoji: "🗺️", daily_cap: 60 },
    SourceDef { source: Source::Study, key: "study", name: t("勉強", "Study"), emoji: "📚", daily_cap: 60 },
    SourceDef {
        source: Source::Together,
        key: "together",
        name: t("みんなと", "Together"),
        emoji: "🫶",
        daily_cap: 60,
    },
    SourceDef {
        source: Source::Streaks, key: "streaks", name: t("ストリーク", "Streaks"), emoji: "🔥", daily_cap: 25
    },
    SourceDef {
        source: Source::Daily, key: "daily", name: t("ログイン", "Showing up"), emoji: "☀️", daily_cap: 20
    },
    // Mission rewards are limited by the number of missions, not by a cap.
    SourceDef {
        source: Source::Missions,
        key: "missions",
        name: t("ミッション", "Missions"),
        emoji: "🎯",
        daily_cap: 0,
    },
];

impl Source {
    pub fn def(self) -> &'static SourceDef {
        SOURCES.iter().find(|def| def.source == self).expect("every source has a definition")
    }

    pub fn key(self) -> &'static str {
        self.def().key
    }

    pub fn from_key(key: &str) -> Option<Source> {
        SOURCES.iter().find(|def| def.key == key).map(|def| def.source)
    }
}

/// How much XP a deed is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Xp {
    /// The same for every deed.
    Flat(i32),
    /// Per unit of the deed's amount (a minute studied, a map cell).
    PerUnit(i32),
}

impl Xp {
    /// XP for a deed of `amount` units, before the daily cap.
    pub fn for_amount(self, amount: i64) -> i64 {
        match self {
            Xp::Flat(xp) => xp as i64,
            Xp::PerUnit(xp) => (xp as i64).saturating_mul(amount.max(0)),
        }
    }

    pub fn per_unit(self) -> i32 {
        match self {
            Xp::Flat(xp) | Xp::PerUnit(xp) => xp,
        }
    }
}

/// How a deed advances a metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Count {
    /// By one per deed.
    One(Metric),
    /// By the deed's amount.
    Amount(Metric),
}

impl Count {
    pub fn metric(self) -> Metric {
        match self {
            Count::One(metric) | Count::Amount(metric) => metric,
        }
    }

    pub fn for_amount(self, amount: i64) -> i64 {
        match self {
            Count::One(_) => 1,
            Count::Amount(_) => amount.max(0),
        }
    }
}

pub struct Rule {
    /// Stable key (`chat.message`): the rule's name in the API, the docs and
    /// `quest_rule_activity`.
    pub key: &'static str,
    pub feed: Feed,
    /// Event topic, or analytics event name.
    pub topic: &'static str,
    /// For the generic requests (`notify.requested`, `activity.recorded`):
    /// the `kind` the payload must carry. Empty = any.
    pub kind: &'static str,
    /// Turns the event into deeds; yields nothing when it does not qualify.
    pub read: Reader,
    pub source: Source,
    pub xp: Xp,
    pub counts: &'static [Count],
    /// Doing this towards a friend counts as the doer's part of the streak
    /// day (and as a friend reached).
    pub streak: bool,
    /// The event follows from where the player has been, without them
    /// choosing to share it. It is not counted for a player who shows their
    /// footprints to nobody: XP, badges and boards would trace their steps.
    pub location: bool,
    /// "How to earn XP" line.
    pub text: Text,
    /// What one unit is, for `Xp::PerUnit` rules.
    pub unit: Text,
}

impl Rule {
    const fn on(feed: Feed, key: &'static str, topic: &'static str, read: Reader) -> Rule {
        Rule {
            key,
            feed,
            topic,
            kind: "",
            read,
            source: Source::Daily,
            xp: Xp::Flat(0),
            counts: &[],
            streak: false,
            location: false,
            text: Text::EMPTY,
            unit: Text::EMPTY,
        }
    }

    /// A rule fed by a domain event.
    const fn event(key: &'static str, topic: &'static str, read: Reader) -> Rule {
        Rule::on(Feed::Events, key, topic, read)
    }

    /// A rule fed by an analytics event.
    const fn tracked(key: &'static str, name: &'static str, read: Reader) -> Rule {
        Rule::on(Feed::Analytics, key, name, read)
    }

    /// Only payloads of this `kind` (generic requests).
    const fn kind(mut self, kind: &'static str) -> Rule {
        self.kind = kind;
        self
    }

    const fn earns(mut self, source: Source, xp: i32) -> Rule {
        self.source = source;
        self.xp = Xp::Flat(xp);
        self
    }

    const fn earns_per(mut self, source: Source, xp: i32, unit: Text) -> Rule {
        self.source = source;
        self.xp = Xp::PerUnit(xp);
        self.unit = unit;
        self
    }

    const fn counts(mut self, counts: &'static [Count]) -> Rule {
        self.counts = counts;
        self
    }

    const fn towards_friend(mut self) -> Rule {
        self.streak = true;
        self
    }

    const fn from_location(mut self) -> Rule {
        self.location = true;
        self
    }

    const fn text(mut self, ja: &'static str, en: &'static str) -> Rule {
        self.text = t(ja, en);
        self
    }

    /// Whether clients list the rule under "how to earn XP".
    pub fn listed(&self) -> bool {
        self.xp.per_unit() > 0
    }
}

use Count::{Amount, One};
use Metric as M;
use Source as S;

/// The table. Topics marked "expected" belong to domains that are still
/// being built: the rule is ready, and stays silent (and its missions
/// undrawn) until the topic is emitted.
pub const RULES: &[Rule] = &[
    // ── showing up ──
    Rule::event("daily.joined", "identity.user.created", payloads::user)
        .earns(S::Daily, 10)
        .counts(&[One(M::Joined)])
        .text("Tokulyをはじめる", "Join Tokuly"),
    Rule::event("daily.bonus", "wallet.balance.changed", payloads::daily_bonus)
        .earns(S::Daily, 10)
        .counts(&[One(M::DailyBonuses)])
        .text("ログインボーナスを受け取る", "Collect the daily login bonus"),
    // ── friends ──
    Rule::event("friends.added", "social.friend.accepted", payloads::friend_accepted)
        .earns(S::Friends, 20)
        .counts(&[One(M::FriendsAdded)])
        .text("友達になる", "Make a friend"),
    Rule::event("friends.bump", "social.bump.created", payloads::bump)
        .earns(S::Friends, 15)
        .counts(&[One(M::Bumps)])
        .towards_friend()
        .text("友達とバンプする", "Bump a friend"),
    Rule::event("friends.intro", "social.intro.created", payloads::intro)
        .earns(S::Friends, 30)
        .counts(&[One(M::Intros)])
        .text("招待した人がTokulyに参加する", "Someone joins through your invite"),
    Rule::event("friends.map_reaction", "notify.requested", payloads::notification)
        .kind("social.map_reaction")
        .earns(S::Friends, 3)
        .counts(&[One(M::MapReactions)])
        .towards_friend()
        .text("マップで友達にリアクションする", "React to a friend on the map"),
    Rule::event("friends.buzz", "notify.requested", payloads::notification)
        .kind("social.buzz")
        .earns(S::Friends, 1)
        .towards_friend()
        .text("近くの友達をブルブルさせる", "Buzz a friend nearby"),
    Rule::event("friends.knock", "notify.requested", payloads::notification)
        .kind("social.knock")
        .earns(S::Friends, 1)
        .towards_friend()
        .text("友達にノックする", "Knock on a friend"),
    // ── chat ──
    Rule::tracked("chat.message", "chat.message.send", payloads::chat_message)
        .earns(S::Chat, 2)
        .counts(&[One(M::Messages)])
        .towards_friend()
        .text("メッセージを送る", "Send a message"),
    Rule::event("chat.call", "chat.call.ended", payloads::call_ended)
        .earns(S::Chat, 10)
        .counts(&[One(M::Calls), Amount(M::CallMinutes)])
        .towards_friend()
        .text("1分以上通話する", "Talk on a call for a minute or more"),
    // ── stamps ──
    Rule::tracked("stamps.sent", "stamp.send", payloads::actor)
        .earns(S::Stamps, 2)
        .counts(&[One(M::StampsSent)])
        .text("スタンプを送る", "Send a stamp"),
    Rule::event("stamps.installed", "stamp.pack.installed", payloads::user)
        .earns(S::Stamps, 5)
        .counts(&[One(M::PacksInstalled)])
        .text("スタンプパックを追加する", "Add a stamp pack"),
    // ── notes and stories ──
    Rule::event("notes.posted", "notes.note.created", payloads::note_posted)
        .earns(S::Notes, 8)
        .counts(&[One(M::NotesPosted)])
        .text("ノートを投稿する", "Post a note"),
    Rule::event("notes.replied", "activity.recorded", payloads::activity)
        .kind("note.replied")
        .earns(S::Notes, 3)
        .towards_friend()
        .text("ノートに返信する", "Reply to a note"),
    Rule::event("notes.story", "notes.story.created", payloads::story_posted)
        .earns(S::Notes, 8)
        .counts(&[One(M::StoriesPosted)])
        .text("記憶（ストーリー）を投稿する", "Post a story"),
    // ── creating ──
    Rule::event("creators.video", "video.video.published", payloads::video_published)
        .earns(S::Creators, 40)
        .counts(&[One(M::VideosPublished)])
        .text("動画を公開する", "Publish a video"),
    Rule::event("creators.stamp_pack", "stamp.pack.published", payloads::pack_published)
        .earns(S::Creators, 40)
        .counts(&[One(M::PacksPublished)])
        .text("スタンプパックを公開する", "Publish a stamp pack"),
    // expected
    Rule::event("creators.short", "shorts.short.published", payloads::actor)
        .earns(S::Creators, 20)
        .counts(&[One(M::ShortsPosted)])
        .text("ショートを投稿する", "Post a short"),
    // expected
    Rule::event("creators.live", "live.stream.started", payloads::actor)
        .earns(S::Creators, 30)
        .counts(&[One(M::LivesStarted)])
        .text("ライブ配信をはじめる", "Go live"),
    // ── likes and comments ──
    Rule::tracked("reactions.note_like", "note.like", payloads::like_given)
        .earns(S::Reactions, 1)
        .counts(&[One(M::LikesGiven)])
        .towards_friend()
        .text("ノートにいいねする", "Like a note"),
    Rule::tracked("reactions.note_liked", "note.like", payloads::like_received).counts(&[One(M::LikesReceived)]),
    Rule::event("reactions.reaction", "engagement.reaction.changed", payloads::reaction_given)
        .earns(S::Reactions, 1)
        .counts(&[One(M::LikesGiven)])
        .towards_friend()
        .text("いいね・絵文字でリアクションする", "React with a like or an emoji"),
    Rule::event("reactions.reacted", "engagement.reaction.changed", payloads::reaction_received)
        .counts(&[One(M::LikesReceived)]),
    Rule::event("reactions.comment", "engagement.comment.published", payloads::comment)
        .earns(S::Reactions, 3)
        .counts(&[One(M::Comments)])
        .towards_friend()
        .text("コメントする", "Write a comment"),
    Rule::event("reactions.share", "engagement.share.recorded", payloads::user)
        .earns(S::Reactions, 2)
        .counts(&[One(M::Shares)])
        .text("シェアする", "Share something"),
    // ── map ──
    Rule::event("map.checkin", "activity.recorded", payloads::activity)
        .kind("geo.checkin")
        .earns(S::Map, 10)
        .counts(&[One(M::CheckIns)])
        .towards_friend()
        .text("チェックインする", "Check in somewhere"),
    Rule::event("map.new_region", "activity.recorded", payloads::activity)
        .kind("geo.new_region")
        .earns(S::Map, 30)
        .counts(&[One(M::RegionsExplored)])
        .text("新しい地域に行く", "Reach a new region"),
    // expected
    Rule::event("map.explored", "geo.footprint.expanded", payloads::footprint)
        .earns_per(S::Map, 1, t("1マス", "map cell"))
        .counts(&[Amount(M::ExploredCells)])
        .from_location()
        .text("マップを開拓する", "Uncover the map"),
    // expected
    Rule::event("map.new_place", "geo.visit.started", payloads::first_visit)
        .earns(S::Map, 5)
        .counts(&[One(M::PlacesVisited)])
        .from_location()
        .text("はじめての場所に行く", "Visit a place for the first time"),
    // ── study ── (expected)
    Rule::event("study.session", "school.session.completed", payloads::study_session)
        .earns_per(S::Study, 1, t("1分", "minute"))
        .counts(&[Amount(M::StudyMinutes), One(M::StudySessions)])
        .text("勉強する", "Study"),
    // ── together ──
    // Asking a question earns nothing, on purpose: questions are anonymous
    // and the ask domain does not say who asked. Anything a rule could give
    // the asker (let alone a streak moving) would be a hint.
    Rule::event("together.answer", "ask.answer.published", payloads::answer_published)
        .earns(S::Together, 5)
        .counts(&[One(M::QuestionsAnswered)])
        .text("質問に答える", "Answer a question"),
    Rule::event("together.vote", "ask.round.completed", payloads::poll_round)
        .earns(S::Together, 5)
        .counts(&[Amount(M::ComplimentVotes)])
        .text("ほめ投票を1ラウンド終える", "Finish a round of compliment polls"),
    // expected (the four below)
    Rule::event("together.plan", "plans.plan.created", payloads::actor)
        .earns(S::Together, 10)
        .counts(&[One(M::PlansCreated)])
        .text("遊ぶ予定を立てる", "Make a plan"),
    Rule::event("together.space_join", "spaces.room.joined", payloads::actor)
        .earns(S::Together, 5)
        .counts(&[One(M::SpacesJoined)])
        .text("たまり場に入る", "Join an audio room"),
    Rule::event("together.space_open", "spaces.room.opened", payloads::actor)
        .earns(S::Together, 10)
        .counts(&[One(M::SpacesHosted)])
        .text("たまり場をひらく", "Open an audio room"),
    Rule::event("together.thread", "community.thread.created", payloads::actor)
        .earns(S::Together, 8)
        .counts(&[One(M::ThreadsPosted)])
        .text("コミュニティにスレッドを立てる", "Start a community thread"),
];

/// What the quest domain credits by itself, outside the table of events:
/// streak days. Listed with the rules so clients can show it.
pub const STREAK_DAY_KEY: &str = "streaks.day";
pub const STREAK_DAY_XP: i32 = 5;
pub const STREAK_DAY_TEXT: Text = t("友達とのストリークを1日つなぐ", "Keep a streak with a friend going for a day");

/// The rules of a topic on a feed.
pub fn rules_for<'a>(feed: Feed, topic: &'a str) -> impl Iterator<Item = &'static Rule> + 'a {
    RULES.iter().filter(move |rule| rule.feed == feed && rule.topic == topic)
}

/// The distinct topics the table consumes from a feed.
pub fn topics(feed: Feed) -> Vec<&'static str> {
    let mut topics: Vec<&'static str> = RULES.iter().filter(|rule| rule.feed == feed).map(|rule| rule.topic).collect();
    topics.sort_unstable();
    topics.dedup();
    topics
}

/// Keys of the rules that advance `metric`.
pub fn rules_counting(metric: Metric) -> impl Iterator<Item = &'static Rule> {
    RULES.iter().filter(move |rule| rule.counts.iter().any(|count| count.metric() == metric))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    #[test]
    fn rule_keys_are_unique_and_namespaced_by_source() {
        let mut seen = HashSet::new();
        for rule in RULES {
            assert!(seen.insert(rule.key), "duplicate rule key {}", rule.key);
            if rule.listed() {
                assert_eq!(
                    rule.key.split('.').next(),
                    Some(rule.source.key()),
                    "{} is not named after its source",
                    rule.key
                );
            }
            assert!(rule.key.bytes().all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_'));
        }
        assert!(!seen.contains(STREAK_DAY_KEY));
    }

    #[test]
    fn every_rule_that_pays_explains_itself_in_both_languages() {
        for rule in RULES.iter().filter(|rule| rule.listed()) {
            assert!(!rule.text.ja.is_empty() && !rule.text.en.is_empty(), "{} has no text", rule.key);
            assert_eq!(rule.source.def().daily_cap > 0, rule.source != Source::Missions);
            if matches!(rule.xp, Xp::PerUnit(_)) {
                assert!(!rule.unit.is_empty(), "{} pays per unit but names no unit", rule.key);
            }
        }
    }

    #[test]
    fn no_single_deed_is_worth_more_than_its_daily_cap() {
        for rule in RULES {
            let cap = rule.source.def().daily_cap;
            if let Xp::Flat(xp) = rule.xp {
                assert!(cap == 0 || xp <= cap, "{} pays {xp} against a cap of {cap}", rule.key);
            }
        }
    }

    #[test]
    fn generic_requests_always_name_a_kind() {
        for rule in RULES {
            let generic = matches!(rule.topic, "notify.requested" | "activity.recorded");
            assert_eq!(generic, !rule.kind.is_empty(), "{}", rule.key);
        }
    }

    #[test]
    fn a_payload_is_never_counted_twice_into_the_same_metric() {
        // Two rules on one (feed, topic, kind) are fine when they credit
        // different people; they must not advance the same metric or both pay.
        let mut by_event: HashMap<(Feed, &str, &str), Vec<&Rule>> = HashMap::new();
        for rule in RULES {
            by_event.entry((rule.feed, rule.topic, rule.kind)).or_default().push(rule);
        }
        for (event, rules) in by_event {
            let paying = rules.iter().filter(|rule| rule.listed()).count();
            assert!(paying <= 1, "{event:?} pays through {paying} rules");
            let mut metrics = HashSet::new();
            for rule in rules {
                for count in rule.counts {
                    assert!(metrics.insert(count.metric()), "{event:?} counts {:?} twice", count.metric());
                }
            }
        }
    }

    #[test]
    fn sources_and_topics_resolve() {
        for def in SOURCES {
            assert_eq!(Source::from_key(def.key), Some(def.source));
            assert_eq!(def.source.def().key, def.key);
        }
        assert_eq!(Source::from_key("nope"), None);
        assert!(topics(Feed::Analytics).contains(&"chat.message.send"));
        assert!(topics(Feed::Events).contains(&"social.bump.created"));
        assert!(!topics(Feed::Events).contains(&"chat.message.send"));
        assert_eq!(rules_for(Feed::Events, "notify.requested").count(), 3);
        assert!(rules_counting(Metric::StudyMinutes).any(|rule| rule.key == "study.session"));
    }

    #[test]
    fn xp_scales_only_for_per_unit_rules() {
        assert_eq!(Xp::Flat(8).for_amount(30), 8);
        assert_eq!(Xp::PerUnit(2).for_amount(30), 60);
        assert_eq!(Xp::PerUnit(2).for_amount(-4), 0);
        assert_eq!(Count::One(Metric::Calls).for_amount(12), 1);
        assert_eq!(Count::Amount(Metric::CallMinutes).for_amount(12), 12);
    }
}
