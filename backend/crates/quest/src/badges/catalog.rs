//! The badge catalogue, as data.
//!
//! Badges come in two shapes: **lines** of three tiers (bronze, silver,
//! gold) for one achievement counted at three thresholds, and **single**
//! badges. A badge says how it is earned through its [`Criterion`]; counted
//! criteria are evaluated by the quest domain whenever the number behind
//! them moves, the rest is awarded by the code path that knows (a season
//! ending, a mission claimed, a streak restored) or by staff.
//!
//! Adding a badge is adding a row. Keys are stored with what players own,
//! so never rename one; a removed key is simply not shown any more.
use std::{collections::HashMap, sync::LazyLock};

use tokuly_proto::quest::BadgeTier;

use crate::{
    metrics::Metric,
    text::{Text, t},
};

/// A group of badges shown together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Friends,
    Explore,
    Chat,
    Stamps,
    Creators,
    Study,
    Streaks,
    Seasons,
    Journey,
    Missions,
    Together,
    Secret,
    Special,
}

pub struct FamilyDef {
    pub family: Family,
    pub key: &'static str,
    pub name: Text,
    pub emoji: &'static str,
}

/// Every family, in display order.
pub const FAMILIES: &[FamilyDef] = &[
    FamilyDef { family: Family::Friends, key: "friends", name: t("友達", "Friends"), emoji: "🤝" },
    FamilyDef { family: Family::Explore, key: "explore", name: t("マップ探検", "Exploring"), emoji: "🧭" },
    FamilyDef { family: Family::Chat, key: "chat", name: t("チャット", "Chat"), emoji: "💬" },
    FamilyDef { family: Family::Stamps, key: "stamps", name: t("スタンプ", "Stamps"), emoji: "🎨" },
    FamilyDef { family: Family::Creators, key: "creators", name: t("クリエイター", "Creators"), emoji: "🎬" },
    FamilyDef { family: Family::Study, key: "study", name: t("勉強", "Study"), emoji: "📚" },
    FamilyDef { family: Family::Streaks, key: "streaks", name: t("ストリーク", "Streaks"), emoji: "🔥" },
    FamilyDef { family: Family::Seasons, key: "seasons", name: t("シーズン", "Seasons"), emoji: "🏆" },
    FamilyDef { family: Family::Journey, key: "journey", name: t("あゆみ", "Journey"), emoji: "🎖️" },
    FamilyDef { family: Family::Missions, key: "missions", name: t("ミッション", "Missions"), emoji: "🎯" },
    FamilyDef { family: Family::Together, key: "together", name: t("みんなと", "Together"), emoji: "🫶" },
    FamilyDef { family: Family::Secret, key: "secret", name: t("シークレット", "Secret"), emoji: "❓" },
    FamilyDef { family: Family::Special, key: "special", name: t("スペシャル", "Special"), emoji: "✨" },
];

impl Family {
    pub fn def(self) -> &'static FamilyDef {
        FAMILIES.iter().find(|def| def.family == self).expect("every family has a definition")
    }
}

/// How a badge is earned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criterion {
    /// A lifetime counter reached a number.
    Count(Metric, i64),
    /// The player reached a level.
    Level(i32),
    /// A streak with one friend reached this many days.
    StreakDays(i32),
    /// This many streaks of at least `days` days running at once.
    Streaks { days: i32, count: i32 },
    /// Did something between two local hours (`from` inclusive, `to` exclusive).
    Hours { from: u32, to: u32 },
    /// Did something on this day of the year (local date).
    Date { month: u32, day: u32 },
    /// Did something on their birthday.
    Birthday,
    /// Earned XP from this many sources in one day.
    SourcesInDay(i32),
    /// Came back after this many days without earning anything.
    Comeback(i32),
    /// Awarded by the code path that knows, or by staff.
    Manual,
}

#[derive(Debug, Clone, Copy)]
pub struct BadgeDef {
    pub key: &'static str,
    /// The line it is a tier of; its own key for single badges.
    pub line: &'static str,
    pub family: Family,
    pub tier: BadgeTier,
    pub emoji: &'static str,
    pub name: Text,
    /// How it is earned; `{n}` stands for `n`.
    pub how: Text,
    /// The number in `how` (a threshold); 0 when there is none.
    pub n: i64,
    /// Hidden from the catalogue until earned.
    pub secret: bool,
    /// The title (称号) it unlocks; empty = none.
    pub title: Text,
    pub criterion: Criterion,
}

/// What a line of three tiers counts.
#[derive(Debug, Clone, Copy)]
enum Counted {
    Metric(Metric),
    Level,
    StreakDays,
    /// Streaks of at least this many days running at once.
    Streaks(i32),
}

impl Counted {
    fn criterion(self, threshold: i64) -> Criterion {
        match self {
            Counted::Metric(metric) => Criterion::Count(metric, threshold),
            Counted::Level => Criterion::Level(threshold as i32),
            Counted::StreakDays => Criterion::StreakDays(threshold as i32),
            Counted::Streaks(days) => Criterion::Streaks { days, count: threshold as i32 },
        }
    }
}

/// One achievement at three thresholds. The gold tier unlocks `title`.
struct Line {
    line: &'static str,
    keys: [&'static str; 3],
    family: Family,
    emoji: &'static str,
    name: Text,
    how: Text,
    counted: Counted,
    steps: [i64; 3],
    title: Text,
}

macro_rules! line {
    ($key:literal, $family:ident, $emoji:literal, $name:expr, $how:expr, $counted:expr, $steps:expr, $title:expr) => {
        Line {
            line: $key,
            keys: [concat!($key, "_bronze"), concat!($key, "_silver"), concat!($key, "_gold")],
            family: Family::$family,
            emoji: $emoji,
            name: $name,
            how: $how,
            counted: $counted,
            steps: $steps,
            title: $title,
        }
    };
}

const NO_TITLE: Text = Text::EMPTY;

use Counted::Metric as Of;
use Metric as M;

const LINES: &[Line] = &[
    // ── friends ──
    line!(
        "friends_circle",
        Friends,
        "🤝",
        t("友達の輪", "Circle of Friends"),
        t("友達を{n}人つくる", "Make {n} friends"),
        Of(M::FriendsAdded),
        [5, 25, 100],
        t("人気者", "Everybody's Friend")
    ),
    line!(
        "bumper",
        Friends,
        "👊",
        t("バンプ", "Bump"),
        t("友達と{n}回バンプする", "Bump friends {n} times"),
        Of(M::Bumps),
        [1, 10, 50],
        t("バンプマスター", "Bump Master")
    ),
    line!(
        "recruiter",
        Friends,
        "📣",
        t("スカウト", "Scout"),
        t("招待した人が{n}人参加する", "Have {n} people join through your invite"),
        Of(M::Intros),
        [1, 5, 20],
        t("スカウトの鬼", "Head Scout")
    ),
    line!(
        "cheerleader",
        Friends,
        "💖",
        t("マップリアクション", "Map Cheers"),
        t("マップで友達に{n}回リアクションする", "React to friends on the map {n} times"),
        Of(M::MapReactions),
        [10, 100, 500],
        t("応援団長", "Head Cheerleader")
    ),
    line!(
        "caring",
        Friends,
        "🫶",
        t("気くばり", "Keeping in Touch"),
        t(
            "友達に話しかけた回数がのべ{n}人になる（1日1人1回）",
            "Reach out to friends {n} times (once per friend a day)"
        ),
        Of(M::FriendsReached),
        [30, 300, 1500],
        t("気くばり上手", "Always There")
    ),
    // ── exploring ──
    line!(
        "checkin",
        Explore,
        "📍",
        t("チェックイン", "Check-ins"),
        t("{n}回チェックインする", "Check in {n} times"),
        Of(M::CheckIns),
        [5, 30, 150],
        t("おでかけ名人", "Out and About")
    ),
    line!(
        "regions",
        Explore,
        "🗾",
        t("地域めぐり", "Region Hopper"),
        t("新しい地域に{n}か所行く", "Reach {n} new regions"),
        Of(M::RegionsExplored),
        [3, 10, 47],
        t("全国制覇", "Been Everywhere")
    ),
    // ── chat ──
    line!(
        "chatter",
        Chat,
        "💬",
        t("おしゃべり", "Chatterbox"),
        t("メッセージを{n}回送る", "Send {n} messages"),
        Of(M::Messages),
        [100, 1000, 10000],
        t("おしゃべり番長", "Chatterbox-in-Chief")
    ),
    line!(
        "caller",
        Chat,
        "📞",
        t("長電話", "On the Line"),
        t("合計{n}分通話する", "Spend {n} minutes on calls"),
        Of(M::CallMinutes),
        [60, 600, 3000],
        t("長電話の達人", "Marathon Caller")
    ),
    line!(
        "calls",
        Chat,
        "☎️",
        t("もしもし", "Ring Ring"),
        t("{n}回通話する", "Make {n} calls"),
        Of(M::Calls),
        [5, 50, 300],
        NO_TITLE
    ),
    // ── stamps ──
    line!(
        "stamper",
        Stamps,
        "🙌",
        t("スタンプ好き", "Stamp Fan"),
        t("スタンプを{n}回送る", "Send {n} stamps"),
        Of(M::StampsSent),
        [50, 500, 5000],
        t("スタンプ芸人", "Stamp Comedian")
    ),
    line!(
        "collector",
        Stamps,
        "🗂️",
        t("スタンプコレクター", "Stamp Collector"),
        t("スタンプパックを{n}個追加する", "Add {n} stamp packs"),
        Of(M::PacksInstalled),
        [3, 10, 30],
        t("コレクター", "Collector")
    ),
    line!(
        "stamp_artist",
        Stamps,
        "🖌️",
        t("スタンプ職人", "Stamp Artist"),
        t("スタンプパックを{n}個公開する", "Publish {n} stamp packs"),
        Of(M::PacksPublished),
        [1, 3, 10],
        t("スタンプ職人", "Stamp Artist")
    ),
    // ── creators ──
    line!(
        "writer",
        Creators,
        "✏️",
        t("つぶやき", "Note Taker"),
        t("ノートを{n}回投稿する", "Post {n} notes"),
        Of(M::NotesPosted),
        [10, 100, 1000],
        t("つぶやき名人", "Prolific")
    ),
    line!(
        "storyteller",
        Creators,
        "📸",
        t("記憶", "Storyteller"),
        t("記憶（ストーリー）を{n}回投稿する", "Post {n} stories"),
        Of(M::StoriesPosted),
        [5, 50, 200],
        t("思い出メーカー", "Memory Maker")
    ),
    line!(
        "filmmaker",
        Creators,
        "🎬",
        t("動画クリエイター", "Filmmaker"),
        t("動画を{n}本公開する", "Publish {n} videos"),
        Of(M::VideosPublished),
        [1, 10, 50],
        t("映像作家", "Director")
    ),
    line!(
        "popular",
        Creators,
        "⭐",
        t("いいね", "Crowd Pleaser"),
        t("いいねを{n}回もらう", "Receive {n} likes"),
        Of(M::LikesReceived),
        [10, 100, 1000],
        t("人気クリエイター", "Fan Favourite")
    ),
    line!(
        "commenter",
        Creators,
        "💭",
        t("コメント", "Commenter"),
        t("コメントを{n}回書く", "Write {n} comments"),
        Of(M::Comments),
        [10, 100, 500],
        NO_TITLE
    ),
    // ── study ──
    line!(
        "scholar",
        Study,
        "📚",
        t("勉強時間", "Study Time"),
        t("合計{n}分勉強する", "Study for {n} minutes in total"),
        Of(M::StudyMinutes),
        [300, 3000, 30000],
        t("勉強の鬼", "Study Demon")
    ),
    line!(
        "diligent",
        Study,
        "⏱️",
        t("コツコツ", "Steady Does It"),
        t("勉強を{n}回記録する", "Log {n} study sessions"),
        Of(M::StudySessions),
        [10, 100, 500],
        t("コツコツ派", "Slow and Steady")
    ),
    // ── streaks ──
    line!(
        "streak",
        Streaks,
        "🔥",
        t("ストリーク", "Streak"),
        t("ひとりの友達と{n}日連続でやりとりする", "Keep a {n}-day streak with a friend"),
        Counted::StreakDays,
        [7, 30, 100],
        t("相棒", "Partner in Crime")
    ),
    line!(
        "streak_many",
        Streaks,
        "🎪",
        t("同時ストリーク", "Juggler"),
        t("7日以上のストリークを同時に{n}人と続ける", "Keep streaks of 7 days or more with {n} friends at once"),
        Counted::Streaks(7),
        [3, 5, 10],
        t("みんなの相棒", "Everyone's Partner")
    ),
    line!(
        "streak_days",
        Streaks,
        "🕯️",
        t("ストリーク日数", "Flame Keeper"),
        t("ストリークを合計{n}日つなぐ", "Complete {n} streak days in total"),
        Of(M::StreakDays),
        [30, 300, 1000],
        t("火の番人", "Keeper of the Flame")
    ),
    // ── seasons ──
    line!(
        "season_regular",
        Seasons,
        "🏅",
        t("シーズン入賞", "Season Regular"),
        t("シーズンを{n}回、友達の上位3分の1で終える", "Finish {n} seasons in the top third among friends"),
        Of(M::SeasonsWon),
        [1, 3, 6],
        t("シーズンの常連", "Season Regular")
    ),
    // ── journey ──
    line!(
        "level",
        Journey,
        "🎖️",
        t("レベル", "Level Up"),
        t("レベル{n}になる", "Reach level {n}"),
        Counted::Level,
        [5, 10, 25],
        t("ベテラン", "Veteran")
    ),
    line!(
        "loyal",
        Journey,
        "📆",
        t("ログイン日数", "Regular"),
        t("合計{n}日Tokulyを使う", "Use Tokuly on {n} days"),
        Of(M::ActiveDays),
        [7, 30, 365],
        t("常連さん", "Regular")
    ),
    // ── missions ──
    line!(
        "missions",
        Missions,
        "🎯",
        t("ミッション", "Mission Runner"),
        t("ミッションを{n}個クリアする", "Complete {n} missions"),
        Of(M::MissionsClaimed),
        [10, 100, 500],
        t("ミッションハンター", "Mission Hunter")
    ),
    line!(
        "perfect",
        Missions,
        "💯",
        t("パーフェクトデー", "Perfect Day"),
        t("デイリーミッションを全部クリアした日が{n}日になる", "Clear all daily missions on {n} days"),
        Of(M::PerfectDays),
        [1, 10, 50],
        t("完璧主義", "Perfectionist")
    ),
    // ── together ──
    line!(
        "answerer",
        Together,
        "💌",
        t("質問箱", "Answer Box"),
        t("質問に{n}回答える", "Answer {n} questions"),
        Of(M::QuestionsAnswered),
        [5, 50, 200],
        t("なんでも答える人", "Open Book")
    ),
    line!(
        "planner",
        Together,
        "🗓️",
        t("予定", "Planner"),
        t("遊ぶ予定を{n}回立てる", "Make {n} plans"),
        Of(M::PlansCreated),
        [1, 10, 50],
        t("幹事", "Organizer")
    ),
];

const fn single(
    key: &'static str,
    family: Family,
    emoji: &'static str,
    name: Text,
    how: Text,
    criterion: Criterion,
) -> BadgeDef {
    BadgeDef {
        key,
        line: key,
        family,
        tier: BadgeTier::BADGE_TIER_UNSPECIFIED,
        emoji,
        name,
        how,
        n: 0,
        secret: false,
        title: NO_TITLE,
        criterion,
    }
}

impl BadgeDef {
    const fn secret(mut self) -> BadgeDef {
        self.secret = true;
        self
    }

    const fn titled(mut self, ja: &'static str, en: &'static str) -> BadgeDef {
        self.title = t(ja, en);
        self
    }

    /// The number shown in the description.
    const fn n(mut self, n: i64) -> BadgeDef {
        self.n = n;
        self
    }
}

/// Keys of badges the code awards by name.
pub const PHOENIX: &str = "phoenix";
pub const SEASON_CHAMPION: &str = "season_champion";
pub const FULL_YEAR: &str = "full_year";
/// The season badges, January to December.
pub const SEASON_MONTHS: [&str; 12] = [
    "season_01",
    "season_02",
    "season_03",
    "season_04",
    "season_05",
    "season_06",
    "season_07",
    "season_08",
    "season_09",
    "season_10",
    "season_11",
    "season_12",
];

const SEASON_HOW: Text =
    t("{n}月のシーズンを友達の上位3分の1で終える", "Finish a season of month {n} in the top third among friends");

const fn season(index: usize, emoji: &'static str, ja: &'static str, en: &'static str) -> BadgeDef {
    single(SEASON_MONTHS[index], Family::Seasons, emoji, t(ja, en), SEASON_HOW, Criterion::Manual).n(index as i64 + 1)
}

const SINGLES: &[BadgeDef] = &[
    // ── friends ──
    single(
        "first_friend",
        Family::Friends,
        "🌱",
        t("はじめての友達", "First Friend"),
        t("最初の友達をつくる", "Make your first friend"),
        Criterion::Count(M::FriendsAdded, 1),
    ),
    // ── streaks: one badge per milestone not covered by the tiers ──
    single(
        "streak_3",
        Family::Streaks,
        "✨",
        t("はじめての炎", "First Flame"),
        t("ひとりの友達と{n}日連続でやりとりする", "Keep a {n}-day streak with a friend"),
        Criterion::StreakDays(3),
    )
    .n(3),
    single(
        "streak_365",
        Family::Streaks,
        "💞",
        t("一年の絆", "A Year Together"),
        t("ひとりの友達と{n}日連続でやりとりする", "Keep a {n}-day streak with a friend"),
        Criterion::StreakDays(365),
    )
    .n(365)
    .titled("一年の絆", "Inseparable"),
    single(
        "streak_500",
        Family::Streaks,
        "🌟",
        t("五百日の絆", "Five Hundred Days"),
        t("ひとりの友達と{n}日連続でやりとりする", "Keep a {n}-day streak with a friend"),
        Criterion::StreakDays(500),
    )
    .n(500),
    single(
        "streak_1000",
        Family::Streaks,
        "💎",
        t("千日の絆", "A Thousand Days"),
        t("ひとりの友達と{n}日連続でやりとりする", "Keep a {n}-day streak with a friend"),
        Criterion::StreakDays(1000),
    )
    .n(1000)
    .titled("伝説の二人", "Legendary Duo"),
    // ── seasons: one badge per month of the year ──
    season(0, "🎍", "睦月シーズン", "January Season"),
    season(1, "⛄", "如月シーズン", "February Season"),
    season(2, "🎎", "弥生シーズン", "March Season"),
    season(3, "🌸", "卯月シーズン", "April Season"),
    season(4, "🎏", "皐月シーズン", "May Season"),
    season(5, "☔", "水無月シーズン", "June Season"),
    season(6, "🎋", "文月シーズン", "July Season"),
    season(7, "🎆", "葉月シーズン", "August Season"),
    season(8, "🎑", "長月シーズン", "September Season"),
    season(9, "🎃", "神無月シーズン", "October Season"),
    season(10, "🍁", "霜月シーズン", "November Season"),
    season(11, "🎄", "師走シーズン", "December Season"),
    single(
        SEASON_CHAMPION,
        Family::Seasons,
        "🏆",
        t("シーズン1位", "Season Champion"),
        t(
            "3人以上が参加したシーズンで、友達の中の1位になる",
            "Finish a season first among friends, with three or more taking part",
        ),
        Criterion::Manual,
    )
    .titled("チャンピオン", "Champion"),
    // ── journey ──
    single(
        "welcome",
        Family::Journey,
        "👋",
        t("ようこそ!", "Welcome!"),
        t("Tokulyをはじめる", "Join Tokuly"),
        Criterion::Count(M::Joined, 1),
    ),
    single(
        "level_50",
        Family::Journey,
        "👑",
        t("レベル50", "Level 50"),
        t("レベル{n}になる", "Reach level {n}"),
        Criterion::Level(50),
    )
    .n(50)
    .titled("達人", "Master"),
    single(
        "level_100",
        Family::Journey,
        "🐉",
        t("レベル100", "Level 100"),
        t("レベル{n}になる", "Reach level {n}"),
        Criterion::Level(100),
    )
    .n(100)
    .titled("レジェンド", "Legend"),
    // ── missions: rewards of particular weekly missions ──
    single(
        "mission_scholar",
        Family::Missions,
        "🎓",
        t("勉強ウィーク", "Study Week"),
        t(
            "週間ミッション「今週300分勉強しよう」をクリアする",
            "Clear the weekly mission \"Study 300 minutes this week\"",
        ),
        Criterion::Manual,
    ),
    single(
        "mission_explorer",
        Family::Missions,
        "🧳",
        t("週末トラベラー", "Weekend Traveller"),
        t("週間ミッション「新しい地域に行こう」をクリアする", "Clear the weekly mission \"Reach a new region\""),
        Criterion::Manual,
    ),
    single(
        "mission_social",
        Family::Missions,
        "🤜",
        t("バンプウィーク", "Bump Week"),
        t(
            "週間ミッション「今週3回バンプしよう」をクリアする",
            "Clear the weekly mission \"Bump three times this week\"",
        ),
        Criterion::Manual,
    ),
    // ── secret ──
    single(
        "early_bird",
        Family::Secret,
        "🐓",
        t("早起き", "Early Bird"),
        t("朝5時〜7時に活動する", "Be active between 5 and 7 in the morning"),
        Criterion::Hours { from: 5, to: 7 },
    )
    .secret()
    .titled("早起きさん", "Early Bird"),
    single(
        "comeback",
        Family::Secret,
        "🏡",
        t("おかえり!", "Welcome Back"),
        t("{n}日以上ぶりに戻ってくる", "Come back after {n} days or more"),
        Criterion::Comeback(30),
    )
    .n(30)
    .secret(),
    single(
        PHOENIX,
        Family::Secret,
        "🕊️",
        t("不死鳥", "Phoenix"),
        t("途切れたストリークを復活させる", "Bring a lost streak back"),
        Criterion::Manual,
    )
    .secret()
    .titled("不死鳥", "Phoenix"),
    single(
        "new_year",
        Family::Secret,
        "🌅",
        t("あけおめ", "Happy New Year"),
        t("元日に活動する", "Be active on New Year's Day"),
        Criterion::Date { month: 1, day: 1 },
    )
    .secret(),
    single(
        "tanabata",
        Family::Secret,
        "🌌",
        t("七夕", "Tanabata"),
        t("7月7日に活動する", "Be active on the seventh of July"),
        Criterion::Date { month: 7, day: 7 },
    )
    .secret(),
    single(
        "halloween",
        Family::Secret,
        "👻",
        t("ハッピーハロウィン", "Happy Halloween"),
        t("10月31日に活動する", "Be active on the 31st of October"),
        Criterion::Date { month: 10, day: 31 },
    )
    .secret(),
    single(
        "christmas",
        Family::Secret,
        "🎅",
        t("メリークリスマス", "Merry Christmas"),
        t("12月25日に活動する", "Be active on the 25th of December"),
        Criterion::Date { month: 12, day: 25 },
    )
    .secret(),
    single(
        "leap_day",
        Family::Secret,
        "🐸",
        t("うるう日", "Leap Day"),
        t("2月29日に活動する", "Be active on the 29th of February"),
        Criterion::Date { month: 2, day: 29 },
    )
    .secret(),
    single(
        "birthday",
        Family::Secret,
        "🎂",
        t("ハッピーバースデー", "Happy Birthday"),
        t("誕生日に活動する", "Be active on your birthday"),
        Criterion::Birthday,
    )
    .secret(),
    single(
        "all_rounder",
        Family::Secret,
        "🃏",
        t("なんでも屋", "Jack of All Trades"),
        t("1日に{n}種類の方法でXPをかせぐ", "Earn XP in {n} different ways in one day"),
        Criterion::SourcesInDay(6),
    )
    .n(6)
    .secret()
    .titled("なんでも屋", "Jack of All Trades"),
    single(
        FULL_YEAR,
        Family::Secret,
        "📅",
        t("皆勤賞", "Full Calendar"),
        t("12か月すべてのシーズンバッジを集める", "Collect the season badge of all twelve months"),
        Criterion::Manual,
    )
    .secret()
    .titled("一年皆勤", "All Year Round"),
    // ── special: given by staff ──
    single(
        "early_adopter",
        Family::Special,
        "🚀",
        t("初期メンバー", "Early Adopter"),
        t("Tokulyを初期から使っている", "Has been here since the early days of Tokuly"),
        Criterion::Manual,
    )
    .titled("古参", "Old Guard"),
    single(
        "bug_hunter",
        Family::Special,
        "🐛",
        t("バグハンター", "Bug Hunter"),
        t("バグを見つけて報告した", "Found and reported a bug"),
        Criterion::Manual,
    )
    .titled("バグハンター", "Bug Hunter"),
    single(
        "event_winner",
        Family::Special,
        "🎉",
        t("イベント入賞", "Event Winner"),
        t("公式イベントで入賞した", "Won a prize at an official event"),
        Criterion::Manual,
    ),
];

const TIERS: [BadgeTier; 3] = [BadgeTier::BADGE_TIER_BRONZE, BadgeTier::BADGE_TIER_SILVER, BadgeTier::BADGE_TIER_GOLD];

/// The whole catalogue: families in display order, lines before singles,
/// tiers in order.
pub static CATALOG: LazyLock<Vec<BadgeDef>> = LazyLock::new(|| {
    let mut badges = Vec::with_capacity(LINES.len() * 3 + SINGLES.len());
    for family in FAMILIES {
        for line in LINES.iter().filter(|line| line.family == family.family) {
            for (index, tier) in TIERS.into_iter().enumerate() {
                badges.push(BadgeDef {
                    key: line.keys[index],
                    line: line.line,
                    family: line.family,
                    tier,
                    emoji: line.emoji,
                    name: line.name,
                    how: line.how,
                    n: line.steps[index],
                    secret: false,
                    title: if index == 2 { line.title } else { NO_TITLE },
                    criterion: line.counted.criterion(line.steps[index]),
                });
            }
        }
        for badge in SINGLES.iter().filter(|badge| badge.family == family.family) {
            badges.push(*badge);
        }
    }
    badges
});

static BY_KEY: LazyLock<HashMap<&'static str, &'static BadgeDef>> =
    LazyLock::new(|| CATALOG.iter().map(|badge| (badge.key, badge)).collect());

/// The badge with this key, if the catalogue has one.
pub fn find(key: &str) -> Option<&'static BadgeDef> {
    BY_KEY.get(key).copied()
}

/// What stands in for the key of a secret badge shown to somebody who does
/// not own it: stable, so clients can refer to it, and saying nothing about
/// what the badge is.
pub fn masked_key(badge: &BadgeDef) -> String {
    // FNV-1a over the key.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in badge.key.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("secret_{:012x}", hash & 0xffff_ffff_ffff)
}

/// The secret badge a stand-in key stands for.
pub fn find_masked(key: &str) -> Option<&'static BadgeDef> {
    CATALOG.iter().filter(|badge| badge.secret).find(|badge| masked_key(badge) == key)
}

/// The season badge of a month (1-12).
pub fn season_badge(month: u32) -> &'static BadgeDef {
    find(SEASON_MONTHS[(month.clamp(1, 12) - 1) as usize]).expect("every month has a season badge")
}

impl BadgeDef {
    /// The number a counted badge needs, with the metric it counts.
    pub fn counted(&self) -> Option<(Metric, i64)> {
        match self.criterion {
            Criterion::Count(metric, threshold) => Some((metric, threshold)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn the_catalogue_is_large_and_every_key_is_unique() {
        assert!(CATALOG.len() >= 60, "only {} badges", CATALOG.len());
        let mut keys = HashSet::new();
        for badge in CATALOG.iter() {
            assert!(keys.insert(badge.key), "duplicate badge key {}", badge.key);
            assert!(
                badge.key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{}",
                badge.key
            );
            assert_eq!(find(badge.key).map(|found| found.key), Some(badge.key));
        }
        assert_eq!(keys.len(), LINES.len() * 3 + SINGLES.len(), "a badge belongs to a family that is not listed");
        assert!(find("no_such_badge").is_none());
    }

    #[test]
    fn secret_badges_have_distinct_stand_in_keys() {
        let secrets: Vec<&BadgeDef> = CATALOG.iter().filter(|badge| badge.secret).collect();
        let stand_ins: HashSet<String> = secrets.iter().map(|badge| masked_key(badge)).collect();
        assert_eq!(stand_ins.len(), secrets.len());
        for badge in secrets {
            assert_eq!(find_masked(&masked_key(badge)).map(|found| found.key), Some(badge.key));
            assert!(find(&masked_key(badge)).is_none());
        }
        assert!(find_masked("secret_000000000000").is_none());
        // Only secrets have one.
        assert!(find_masked(&masked_key(find("welcome").unwrap())).is_none());
    }

    #[test]
    fn every_required_family_has_badges_and_tiers() {
        for family in ["friends", "explore", "chat", "stamps", "creators", "study", "streaks", "seasons", "secret"] {
            let def =
                FAMILIES.iter().find(|def| def.key == family).unwrap_or_else(|| panic!("family {family} is missing"));
            let badges: Vec<&BadgeDef> = CATALOG.iter().filter(|badge| badge.family == def.family).collect();
            assert!(!badges.is_empty(), "family {family} has no badges");
            if family != "secret" {
                for tier in TIERS {
                    assert!(badges.iter().any(|badge| badge.tier == tier), "family {family} has no {tier:?} badge");
                }
            }
        }
        assert!(CATALOG.iter().filter(|badge| badge.secret).count() >= 5);
        assert!(CATALOG.iter().filter(|badge| badge.secret).all(|badge| badge.family == Family::Secret));
    }

    #[test]
    fn every_badge_is_described_in_both_languages() {
        for badge in CATALOG.iter() {
            assert!(!badge.name.ja.is_empty() && !badge.name.en.is_empty(), "{} has no name", badge.key);
            assert!(!badge.how.ja.is_empty() && !badge.how.en.is_empty(), "{} has no description", badge.key);
            assert!(!badge.emoji.is_empty(), "{} has no emoji", badge.key);
            assert_eq!(badge.how.ja.contains("{n}"), badge.how.en.contains("{n}"), "{}", badge.key);
            if badge.how.ja.contains("{n}") {
                assert!(badge.n > 0, "{} describes a number it does not have", badge.key);
            }
            assert_eq!(badge.title.ja.is_empty(), badge.title.en.is_empty(), "{}", badge.key);
        }
    }

    #[test]
    fn tiers_of_a_line_get_harder() {
        for line in LINES {
            assert!(
                line.steps[0] < line.steps[1] && line.steps[1] < line.steps[2],
                "{} thresholds do not grow",
                line.line
            );
            let tiers: Vec<&BadgeDef> = CATALOG.iter().filter(|badge| badge.line == line.line).collect();
            assert_eq!(tiers.len(), 3, "{}", line.line);
            assert!(tiers[0].title.is_empty() && tiers[1].title.is_empty());
        }
    }

    #[test]
    fn every_streak_milestone_has_a_badge() {
        for milestone in crate::streaks::days::MILESTONES {
            assert!(
                CATALOG.iter().any(|badge| badge.criterion == Criterion::StreakDays(*milestone)),
                "no badge for a streak of {milestone} days"
            );
        }
    }

    #[test]
    fn the_twelve_season_badges_exist_in_month_order() {
        for month in 1..=12 {
            let badge = season_badge(month);
            assert_eq!(badge.n, month as i64);
            assert_eq!(badge.family, Family::Seasons);
        }
        assert!(find(SEASON_CHAMPION).is_some() && find(FULL_YEAR).is_some() && find(PHOENIX).is_some());
    }
}
