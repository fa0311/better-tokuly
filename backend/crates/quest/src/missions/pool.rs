//! The mission pool, as data.
//!
//! A mission asks for a number of something within a day or a week: its
//! progress is the period's count of one [`Metric`]. Which missions a player
//! is offered is decided by `draw.rs` from the fields here: how many friends
//! it takes, whether it is for people who have done the thing before, and —
//! through the metric — whether anything on this server feeds it at all.
//!
//! Adding a mission is adding a row. Keys are stored with drawn missions;
//! do not reuse a key for something else.
use tokuly_proto::quest::{MissionDifficulty, MissionPeriod};

use crate::{
    metrics::Metric,
    text::{Text, t},
};

pub struct MissionDef {
    pub key: &'static str,
    pub period: MissionPeriod,
    pub difficulty: MissionDifficulty,
    pub metric: Metric,
    pub target: i64,
    pub emoji: &'static str,
    pub title: Text,
    pub description: Text,
    /// Where in the app it is done; empty = anywhere.
    pub path: &'static str,
    /// Friends a player needs for the mission to be possible.
    pub friends: i32,
    /// Only offered to players who have done this before: nobody is asked
    /// to start streaming or publishing videos for a mission.
    pub experienced: Option<Metric>,
    /// Badge granted with the reward; empty = none.
    pub badge: &'static str,
}

/// Coins and XP for claiming a mission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Payout {
    pub coins: i32,
    pub xp: i32,
}

/// What a mission pays, by period and difficulty.
pub fn payout(period: MissionPeriod, difficulty: MissionDifficulty) -> Payout {
    use MissionDifficulty as D;
    match (period, difficulty) {
        (MissionPeriod::MISSION_PERIOD_WEEKLY, D::MISSION_DIFFICULTY_HARD) => Payout { coins: 50, xp: 100 },
        (MissionPeriod::MISSION_PERIOD_WEEKLY, D::MISSION_DIFFICULTY_EASY) => Payout { coins: 20, xp: 40 },
        (MissionPeriod::MISSION_PERIOD_WEEKLY, _) => Payout { coins: 30, xp: 60 },
        (_, D::MISSION_DIFFICULTY_HARD) => Payout { coins: 20, xp: 40 },
        (_, D::MISSION_DIFFICULTY_EASY) => Payout { coins: 5, xp: 10 },
        (_, _) => Payout { coins: 10, xp: 20 },
    }
}

/// Daily missions per player.
pub const DAILY_MISSIONS: usize = 3;
/// The bonus for claiming all of a day's missions.
pub const DAILY_BONUS: Payout = Payout { coins: 15, xp: 30 };

const fn mission(
    key: &'static str,
    period: MissionPeriod,
    difficulty: MissionDifficulty,
    metric: Metric,
    target: i64,
    emoji: &'static str,
    title: Text,
    description: Text,
) -> MissionDef {
    MissionDef {
        key,
        period,
        difficulty,
        metric,
        target,
        emoji,
        title,
        description,
        path: "",
        friends: 0,
        experienced: None,
        badge: "",
    }
}

impl MissionDef {
    const fn at(mut self, path: &'static str) -> MissionDef {
        self.path = path;
        self
    }

    const fn needs_friends(mut self, friends: i32) -> MissionDef {
        self.friends = friends;
        self
    }

    const fn for_experienced(mut self, metric: Metric) -> MissionDef {
        self.experienced = Some(metric);
        self
    }

    const fn with_badge(mut self, badge: &'static str) -> MissionDef {
        self.badge = badge;
        self
    }

    pub fn payout(&self) -> Payout {
        payout(self.period, self.difficulty)
    }
}

use Metric as M;
use MissionDifficulty::{
    MISSION_DIFFICULTY_EASY as EASY, MISSION_DIFFICULTY_HARD as HARD, MISSION_DIFFICULTY_NORMAL as NORMAL,
};
use MissionPeriod::{MISSION_PERIOD_DAILY as DAILY, MISSION_PERIOD_WEEKLY as WEEKLY};

pub const POOL: &[MissionDef] = &[
    // ── daily: always possible ──
    mission(
        "xp_50",
        DAILY,
        EASY,
        M::Xp,
        50,
        "⚡",
        t("50XPかせごう", "Earn 50 XP"),
        t("今日なんでもいいから50XPかせごう!", "Earn 50 XP today, any way you like!"),
    ),
    mission(
        "xp_150",
        DAILY,
        HARD,
        M::Xp,
        150,
        "🌩️",
        t("150XPかせごう", "Earn 150 XP"),
        t("今日は張り切って150XPかせごう!", "Go all in and earn 150 XP today!"),
    ),
    mission(
        "daily_bonus",
        DAILY,
        EASY,
        M::DailyBonuses,
        1,
        "🎁",
        t("ログインボーナスをもらおう", "Collect the login bonus"),
        t("今日のログインボーナスを受け取ろう!", "Collect today's login bonus!"),
    )
    .at("/wallet"),
    // ── daily: friends ──
    mission(
        "add_friend",
        DAILY,
        HARD,
        M::FriendsAdded,
        1,
        "🌱",
        t("友達を追加しよう", "Add a friend"),
        t("新しい友達を1人追加しよう!", "Make one new friend!"),
    )
    .at("/friends/add"),
    mission(
        "reach_3",
        DAILY,
        NORMAL,
        M::FriendsReached,
        3,
        "🫶",
        t("3人に話しかけよう", "Reach out to 3 friends"),
        t("メッセージやリアクションで、3人の友達に話しかけよう!", "Message or react to three different friends!"),
    )
    .at("/chat")
    .needs_friends(3),
    mission(
        "bump_1",
        DAILY,
        NORMAL,
        M::Bumps,
        1,
        "👊",
        t("バンプしよう", "Bump a friend"),
        t("友達と会ってスマホをバンプ!", "Meet a friend and bump phones!"),
    )
    .at("/map")
    .needs_friends(1),
    mission(
        "map_reaction_1",
        DAILY,
        EASY,
        M::MapReactions,
        1,
        "💖",
        t("マップでリアクション", "React on the map"),
        t("マップで友達にリアクションを送ろう!", "Send a friend a reaction on the map!"),
    )
    .at("/map")
    .needs_friends(1),
    // ── daily: chat and stamps ──
    mission(
        "messages_5",
        DAILY,
        EASY,
        M::Messages,
        5,
        "💬",
        t("メッセージを5回送ろう", "Send 5 messages"),
        t("友達にメッセージを5回送ろう!", "Send your friends five messages!"),
    )
    .at("/chat")
    .needs_friends(1),
    mission(
        "messages_20",
        DAILY,
        NORMAL,
        M::Messages,
        20,
        "🗯️",
        t("メッセージを20回送ろう", "Send 20 messages"),
        t("今日はたくさん話そう!メッセージを20回!", "A chatty day: send twenty messages!"),
    )
    .at("/chat")
    .needs_friends(1),
    mission(
        "call_1",
        DAILY,
        NORMAL,
        M::Calls,
        1,
        "📞",
        t("通話しよう", "Make a call"),
        t("友達と1分以上通話しよう!", "Talk to a friend on a call for a minute or more!"),
    )
    .at("/chat")
    .needs_friends(1),
    mission(
        "stamp_1",
        DAILY,
        EASY,
        M::StampsSent,
        1,
        "🙌",
        t("スタンプを送ろう", "Send a stamp"),
        t("お気に入りのスタンプを1回送ろう!", "Send your favourite stamp!"),
    )
    .at("/chat"),
    mission(
        "stamps_5",
        DAILY,
        NORMAL,
        M::StampsSent,
        5,
        "🎨",
        t("スタンプを5回送ろう", "Send 5 stamps"),
        t("スタンプを5回送ろう!", "Send five stamps!"),
    )
    .at("/chat"),
    // ── daily: notes and reactions ──
    mission(
        "note_1",
        DAILY,
        EASY,
        M::NotesPosted,
        1,
        "✏️",
        t("ノートを投稿しよう", "Post a note"),
        t("今なにしてる?ノートを1つ投稿しよう!", "What are you up to? Post a note!"),
    )
    .at("/notes"),
    mission(
        "story_1",
        DAILY,
        NORMAL,
        M::StoriesPosted,
        1,
        "📸",
        t("記憶を投稿しよう", "Post a story"),
        t("今日の記憶（ストーリー）を1つ投稿しよう!", "Post a story of your day!"),
    )
    .at("/notes"),
    mission(
        "likes_3",
        DAILY,
        EASY,
        M::LikesGiven,
        3,
        "❤️",
        t("3回いいねしよう", "Give 3 likes"),
        t("いいなと思ったものに3回いいねしよう!", "Like three things you enjoy!"),
    ),
    mission(
        "comment_1",
        DAILY,
        EASY,
        M::Comments,
        1,
        "💭",
        t("コメントしよう", "Write a comment"),
        t("なにかにコメントを1つ書こう!", "Leave a comment on something!"),
    ),
    mission(
        "share_1",
        DAILY,
        EASY,
        M::Shares,
        1,
        "📤",
        t("シェアしよう", "Share something"),
        t("お気に入りを1つシェアしよう!", "Share something you like!"),
    ),
    // ── daily: map ──
    mission(
        "checkin_1",
        DAILY,
        NORMAL,
        M::CheckIns,
        1,
        "📍",
        t("チェックインしよう", "Check in"),
        t("今いる場所にチェックインしよう!", "Check in where you are!"),
    )
    .at("/map"),
    // ── daily: study ──
    mission(
        "study_30",
        DAILY,
        NORMAL,
        M::StudyMinutes,
        30,
        "📚",
        t("30分勉強しよう", "Study for 30 minutes"),
        t("タイマーをつけて30分勉強しよう!", "Start the timer and study for thirty minutes!"),
    )
    .at("/school/study"),
    mission(
        "study_90",
        DAILY,
        HARD,
        M::StudyMinutes,
        90,
        "🔥",
        t("90分勉強しよう", "Study for 90 minutes"),
        t("今日は集中!合計90分勉強しよう!", "Focus day: study ninety minutes in total!"),
    )
    .at("/school/study"),
    // ── daily: together ──
    mission(
        "answer_1",
        DAILY,
        EASY,
        M::QuestionsAnswered,
        1,
        "💌",
        t("質問に答えよう", "Answer a question"),
        t("質問箱に届いた質問に1つ答えよう!", "Answer one question from your box!"),
    )
    .at("/ask"),
    mission(
        "vote_5",
        DAILY,
        EASY,
        M::ComplimentVotes,
        5,
        "🗳️",
        t("ほめ投票に5回答えよう", "Vote 5 times"),
        t("ほめ投票に5回答えて友達をほめよう!", "Answer five compliment polls!"),
    )
    .at("/ask/polls")
    .needs_friends(4),
    mission(
        "plan_1",
        DAILY,
        NORMAL,
        M::PlansCreated,
        1,
        "🗓️",
        t("予定を立てよう", "Make a plan"),
        t("友達と遊ぶ予定を1つ立てよう!", "Plan something with your friends!"),
    )
    .at("/plans")
    .needs_friends(1),
    // ── weekly ──
    mission(
        "w_xp_500",
        WEEKLY,
        NORMAL,
        M::Xp,
        500,
        "⚡",
        t("今週500XPかせごう", "Earn 500 XP this week"),
        t("今週は合計500XPをめざそう!", "Aim for 500 XP in total this week!"),
    ),
    mission(
        "w_active_5",
        WEEKLY,
        NORMAL,
        M::ActiveDays,
        5,
        "📆",
        t("今週5日あそぼう", "Show up on 5 days"),
        t("今週のうち5日、Tokulyであそぼう!", "Use Tokuly on five days this week!"),
    ),
    mission(
        "w_missions_10",
        WEEKLY,
        NORMAL,
        M::MissionsClaimed,
        10,
        "🎯",
        t("ミッションを10個クリア", "Clear 10 missions"),
        t("今週デイリーミッションを10個クリアしよう!", "Clear ten daily missions this week!"),
    ),
    mission(
        "w_messages_100",
        WEEKLY,
        NORMAL,
        M::Messages,
        100,
        "💬",
        t("今週100回メッセージ", "Send 100 messages"),
        t("今週メッセージを100回送ろう!", "Send a hundred messages this week!"),
    )
    .at("/chat")
    .needs_friends(1),
    mission(
        "w_reach_10",
        WEEKLY,
        NORMAL,
        M::FriendsReached,
        10,
        "🫶",
        t("のべ10人に話しかけよう", "Reach out 10 times"),
        t("今週、のべ10人の友達に話しかけよう!", "Reach out to friends ten times this week (once per friend a day)!"),
    )
    .at("/chat")
    .needs_friends(2),
    mission(
        "w_bumps_3",
        WEEKLY,
        HARD,
        M::Bumps,
        3,
        "👊",
        t("今週3回バンプしよう", "Bump three times this week"),
        t("今週、友達と3回バンプしよう!", "Bump friends three times this week!"),
    )
    .at("/map")
    .needs_friends(2)
    .with_badge("mission_social"),
    mission(
        "w_stamps_30",
        WEEKLY,
        NORMAL,
        M::StampsSent,
        30,
        "🎨",
        t("今週スタンプ30回", "Send 30 stamps"),
        t("今週スタンプを30回送ろう!", "Send thirty stamps this week!"),
    )
    .at("/chat"),
    mission(
        "w_notes_5",
        WEEKLY,
        NORMAL,
        M::NotesPosted,
        5,
        "✏️",
        t("今週ノートを5回", "Post 5 notes"),
        t("今週ノートを5回投稿しよう!", "Post five notes this week!"),
    )
    .at("/notes"),
    mission(
        "w_checkins_3",
        WEEKLY,
        NORMAL,
        M::CheckIns,
        3,
        "📍",
        t("今週3回チェックイン", "Check in 3 times"),
        t("今週3回チェックインしよう!", "Check in three times this week!"),
    )
    .at("/map"),
    mission(
        "w_region_1",
        WEEKLY,
        HARD,
        M::RegionsExplored,
        1,
        "🗾",
        t("新しい地域に行こう", "Reach a new region"),
        t("今週、行ったことのない地域に行ってみよう!", "Reach a region you have never been to this week!"),
    )
    .at("/footprints")
    .with_badge("mission_explorer"),
    mission(
        "w_study_300",
        WEEKLY,
        HARD,
        M::StudyMinutes,
        300,
        "🎓",
        t("今週300分勉強しよう", "Study 300 minutes this week"),
        t("今週、合計300分勉強しよう!", "Study three hundred minutes in total this week!"),
    )
    .at("/school/study")
    .with_badge("mission_scholar"),
    mission(
        "w_answers_5",
        WEEKLY,
        NORMAL,
        M::QuestionsAnswered,
        5,
        "💌",
        t("今週質問に5回答えよう", "Answer 5 questions"),
        t("今週、質問に5回答えよう!", "Answer five questions this week!"),
    )
    .at("/ask"),
    mission(
        "w_video_1",
        WEEKLY,
        HARD,
        M::VideosPublished,
        1,
        "🎬",
        t("動画を公開しよう", "Publish a video"),
        t("今週、動画を1本公開しよう!", "Publish a video this week!"),
    )
    .at("/studio")
    .for_experienced(M::VideosPublished),
];

/// The mission with this key, if it is still in the pool.
pub fn find(key: &str) -> Option<&'static MissionDef> {
    POOL.iter().find(|mission| mission.key == key)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::{badges::catalog, rules};

    #[test]
    fn keys_are_unique_and_targets_positive() {
        let mut keys = HashSet::new();
        for mission in POOL {
            assert!(keys.insert(mission.key), "duplicate mission key {}", mission.key);
            assert!(mission.target > 0, "{}", mission.key);
            assert!(!mission.title.ja.is_empty() && !mission.title.en.is_empty(), "{}", mission.key);
            assert!(!mission.description.ja.is_empty() && !mission.description.en.is_empty(), "{}", mission.key);
            assert_eq!(mission.key.starts_with("w_"), mission.period == WEEKLY, "{}", mission.key);
        }
    }

    #[test]
    fn every_mission_counts_something_that_can_move() {
        // A metric is fed by a rule, or it is one the quest domain keeps itself.
        let own = [M::Xp, M::ActiveDays, M::MissionsClaimed, M::FriendsReached];
        for mission in POOL {
            assert!(
                rules::rules_counting(mission.metric).next().is_some() || own.contains(&mission.metric),
                "nothing feeds the metric of mission {}",
                mission.key
            );
        }
    }

    #[test]
    fn there_is_always_something_to_draw_without_friends_or_other_domains() {
        let always: Vec<&MissionDef> = POOL
            .iter()
            .filter(|mission| {
                mission.friends == 0
                    && mission.experienced.is_none()
                    && rules::rules_counting(mission.metric).next().is_none()
            })
            .collect();
        assert!(always.iter().any(|mission| mission.period == DAILY));
        assert!(always.iter().any(|mission| mission.period == WEEKLY));
    }

    #[test]
    fn badges_named_by_missions_exist() {
        for mission in POOL.iter().filter(|mission| !mission.badge.is_empty()) {
            assert!(catalog::find(mission.badge).is_some(), "mission {} rewards an unknown badge", mission.key);
        }
    }

    #[test]
    fn harder_and_longer_missions_pay_more() {
        let daily_easy = payout(DAILY, EASY);
        let daily_hard = payout(DAILY, HARD);
        let weekly_hard = payout(WEEKLY, HARD);
        assert!(daily_easy.coins < payout(DAILY, NORMAL).coins && payout(DAILY, NORMAL).coins < daily_hard.coins);
        assert!(daily_hard.coins < weekly_hard.coins && daily_hard.xp < weekly_hard.xp);
    }
}
