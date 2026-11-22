//! Metrics: the things the quest domain counts about a player.
//!
//! A metric is a named counter (messages sent, minutes studied, friends
//! added). The rule table says which events advance which metric; missions
//! ("send 5 stamps today"), counted badges ("1,000 messages") and the metric
//! leaderboards all read the same counters: per local day in
//! `quest_metric_days`, for all time in `quest_counters`.

macro_rules! metrics {
    ($($(#[$doc:meta])* $name:ident => $key:literal,)+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Metric {
            $($(#[$doc])* $name,)+
        }

        impl Metric {
            /// Every metric.
            pub const ALL: &'static [Metric] = &[$(Metric::$name,)+];

            /// The stable key stored in the database and in Valkey.
            pub const fn key(self) -> &'static str {
                match self {
                    $(Metric::$name => $key,)+
                }
            }

            pub fn from_key(key: &str) -> Option<Metric> {
                match key {
                    $($key => Some(Metric::$name),)+
                    _ => None,
                }
            }
        }
    };
}

metrics! {
    // ── friends ──
    /// Friendships made.
    FriendsAdded => "friends_added",
    Bumps => "bumps",
    /// People who joined through the player's invite.
    Intros => "intros",
    MapReactions => "map_reactions",
    /// Friends the player did something towards, counted once per friend
    /// and day (so over a week it counts friend-days).
    FriendsReached => "friends_reached",

    // ── chat ──
    Messages => "messages",
    Calls => "calls",
    CallMinutes => "call_minutes",

    // ── stamps ──
    StampsSent => "stamps_sent",
    PacksInstalled => "packs_installed",
    PacksPublished => "packs_published",

    // ── making things ──
    NotesPosted => "notes_posted",
    StoriesPosted => "stories_posted",
    VideosPublished => "videos_published",
    ShortsPosted => "shorts_posted",
    LivesStarted => "lives_started",
    /// Likes and emoji reactions other people gave the player's content.
    LikesReceived => "likes_received",

    // ── reacting ──
    LikesGiven => "likes_given",
    Comments => "comments",
    Shares => "shares",

    // ── map ──
    CheckIns => "check_ins",
    /// Prefectures / states and countries entered for the first time.
    RegionsExplored => "regions_explored",
    /// Map cells uncovered on the scratch map.
    ExploredCells => "explored_cells",
    /// Places visited for the first time.
    PlacesVisited => "places_visited",

    // ── study ──
    StudyMinutes => "study_minutes",
    StudySessions => "study_sessions",

    // ── together ──
    QuestionsAnswered => "questions_answered",
    ComplimentVotes => "compliment_votes",
    PlansCreated => "plans_created",
    SpacesJoined => "spaces_joined",
    SpacesHosted => "spaces_hosted",
    ThreadsPosted => "threads_posted",

    // ── the quest domain's own ──
    /// XP earned from events (mission rewards excluded, so "earn 50 XP"
    /// cannot be completed by claiming missions).
    Xp => "xp",
    /// Days on which the player earned any XP.
    ActiveDays => "active_days",
    /// Daily login bonuses collected.
    DailyBonuses => "daily_bonuses",
    MissionsClaimed => "missions_claimed",
    /// Days on which all three daily missions were claimed.
    PerfectDays => "perfect_days",
    /// Streak days completed with any friend.
    StreakDays => "streak_days",
    /// Seasons finished in the top third among friends.
    SeasonsWon => "seasons_won",
    /// 1 once the account exists (the welcome badge).
    Joined => "joined",
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn keys_are_unique_and_round_trip() {
        let mut seen = HashSet::new();
        for metric in Metric::ALL {
            assert!(seen.insert(metric.key()), "duplicate metric key {}", metric.key());
            assert_eq!(Metric::from_key(metric.key()), Some(*metric));
            assert!(metric.key().bytes().all(|b| b.is_ascii_lowercase() || b == b'_'));
        }
        assert_eq!(Metric::from_key("no_such_metric"), None);
    }
}
