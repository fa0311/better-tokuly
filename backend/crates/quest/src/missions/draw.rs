//! Drawing missions for a player: pure, and repeatable from a seed.
//!
//! A mission is only offered when the player can do it: they have the
//! friends it takes, they have done it before where that matters, and
//! something on this server actually feeds its metric (a mission of a
//! domain that does not exist yet, or that nobody uses, is never drawn).
//! Within one set no two missions count the same thing, and a day mixes
//! difficulties: one easy, one normal, one harder.
use std::collections::HashSet;

use tokuly_proto::quest::{MissionDifficulty, MissionPeriod};
use uuid::Uuid;

use super::pool::{MissionDef, POOL};
use crate::{metrics::Metric, rules};

/// What the draw knows about a player and the server.
#[derive(Debug, Clone, Default)]
pub struct Player {
    pub friends: i32,
    /// Metrics the player has a lifetime count for.
    pub done: HashSet<Metric>,
    /// Keys of rules that fired recently for anyone.
    pub live_rules: HashSet<&'static str>,
}

impl Player {
    /// Whether anything feeds `metric` on this server right now. Metrics no
    /// rule counts are the quest domain's own (XP, active days, missions
    /// claimed...) and always move.
    pub fn is_live(&self, metric: Metric) -> bool {
        let mut rules = rules::rules_counting(metric).peekable();
        rules.peek().is_none() || rules.any(|rule| self.live_rules.contains(rule.key))
    }

    /// Whether `mission` suits the player.
    pub fn suits(&self, mission: &MissionDef) -> bool {
        self.friends >= mission.friends
            && self.is_live(mission.metric)
            && mission.experienced.is_none_or(|metric| self.done.contains(&metric))
    }
}

/// SplitMix64: a small generator, so a draw is a function of its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            let other = (self.next() % (index as u64 + 1)) as usize;
            items.swap(index, other);
        }
    }
}

/// A seed for one draw: the same player, period and attempt always draw the same.
pub fn seed(user_id: Uuid, period_start: chrono::NaiveDate, period: MissionPeriod, attempt: u32) -> u64 {
    use chrono::Datelike as _;
    let (high, low) = user_id.as_u64_pair();
    let mut rng = Rng(high
        ^ low.rotate_left(17)
        ^ ((period_start.num_days_from_ce() as u64) << 20)
        ^ ((period as u64) << 8)
        ^ attempt as u64);
    rng.next()
}

use MissionDifficulty::{
    MISSION_DIFFICULTY_EASY as EASY, MISSION_DIFFICULTY_HARD as HARD, MISSION_DIFFICULTY_NORMAL as NORMAL,
};

/// The difficulties each slot of a set prefers, best first.
fn slot_preferences(period: MissionPeriod, slot: usize) -> &'static [MissionDifficulty] {
    match (period, slot) {
        // The one weekly mission is any that suits: the pool decides how often it is a hard one.
        (MissionPeriod::MISSION_PERIOD_WEEKLY, _) => &[],
        (_, 0) => &[EASY, NORMAL, HARD],
        (_, 1) => &[NORMAL, EASY, HARD],
        (_, _) => &[HARD, NORMAL, EASY],
    }
}

/// The missions of `period` that suit `player`, in the order of `seed`.
fn candidates(period: MissionPeriod, player: &Player, excluded: &[&str], seed: u64) -> Vec<&'static MissionDef> {
    let mut candidates: Vec<&'static MissionDef> = POOL
        .iter()
        .filter(|mission| mission.period == period && !excluded.contains(&mission.key) && player.suits(mission))
        .collect();
    Rng(seed).shuffle(&mut candidates);
    candidates
}

/// The first candidate of a preferred difficulty that counts something the
/// set does not count yet; failing that, any candidate not in the set.
fn pick(
    candidates: &[&'static MissionDef],
    preferences: &[MissionDifficulty],
    metrics: &HashSet<Metric>,
    drawn: &[&'static MissionDef],
) -> Option<&'static MissionDef> {
    let unused = |mission: &&&'static MissionDef| !drawn.iter().any(|other| other.key == mission.key);
    preferences
        .iter()
        .find_map(|difficulty| {
            candidates.iter().find(|mission| {
                mission.difficulty == *difficulty && !metrics.contains(&mission.metric) && unused(mission)
            })
        })
        .or_else(|| candidates.iter().find(|mission| unused(mission)))
        .copied()
}

/// Draws `count` missions of `period` for `player`, one per slot. Fewer
/// come back when the pool holds fewer that suit the player.
pub fn draw(period: MissionPeriod, count: usize, player: &Player, seed: u64) -> Vec<&'static MissionDef> {
    let candidates = candidates(period, player, &[], seed);
    let mut metrics: HashSet<Metric> = HashSet::new();
    let mut drawn: Vec<&'static MissionDef> = Vec::with_capacity(count);
    for slot in 0..count {
        let Some(mission) = pick(&candidates, slot_preferences(period, slot), &metrics, &drawn) else { break };
        metrics.insert(mission.metric);
        drawn.push(mission);
    }
    drawn
}

/// Draws one mission to take the place of `replaced_key` (a reroll): of the
/// same difficulty when possible, counting something none of the `taken`
/// metrics of the rest of the set count, and never the replaced mission
/// itself. `None` when nothing else suits the player.
pub fn replacement(
    period: MissionPeriod,
    difficulty: MissionDifficulty,
    replaced_key: &str,
    replaced_metric: Metric,
    taken: &[Metric],
    player: &Player,
    seed: u64,
) -> Option<&'static MissionDef> {
    let candidates = candidates(period, player, &[replaced_key], seed);
    let mut metrics: HashSet<Metric> = taken.iter().copied().collect();
    metrics.insert(replaced_metric);
    let preferences: Vec<MissionDifficulty> = std::iter::once(difficulty).chain([NORMAL, EASY, HARD]).collect();
    // A mission counting something new is required here: swapping "send 5
    // messages" for "send 20 messages" is not a reroll anybody wants.
    preferences
        .iter()
        .find_map(|wanted| {
            candidates.iter().find(|mission| mission.difficulty == *wanted && !metrics.contains(&mission.metric))
        })
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::missions::pool::DAILY_MISSIONS;

    const DAILY: MissionPeriod = MissionPeriod::MISSION_PERIOD_DAILY;
    const WEEKLY: MissionPeriod = MissionPeriod::MISSION_PERIOD_WEEKLY;

    fn everything_live() -> HashSet<&'static str> {
        rules::RULES.iter().map(|rule| rule.key).collect()
    }

    fn player(friends: i32) -> Player {
        Player { friends, done: HashSet::new(), live_rules: everything_live() }
    }

    #[test]
    fn someone_without_friends_is_never_asked_to_do_something_with_a_friend() {
        let loner = player(0);
        for attempt in 0..500 {
            for mission in draw(DAILY, DAILY_MISSIONS, &loner, attempt) {
                assert_eq!(mission.friends, 0, "drew {} for a player without friends", mission.key);
            }
            for mission in draw(WEEKLY, 1, &loner, attempt) {
                assert_eq!(mission.friends, 0, "drew {} for a player without friends", mission.key);
            }
        }
        // They are offered a way to change that, though.
        assert!((0..500).any(|attempt| {
            draw(DAILY, DAILY_MISSIONS, &loner, attempt).iter().any(|mission| mission.key == "add_friend")
        }));
    }

    #[test]
    fn missions_needing_several_friends_wait_until_there_are_enough() {
        let two_friends = player(2);
        let all: HashSet<&str> = (0..2000)
            .flat_map(|attempt| draw(DAILY, DAILY_MISSIONS, &two_friends, attempt))
            .map(|mission| mission.key)
            .collect();
        assert!(all.contains("messages_5") && all.contains("bump_1"));
        assert!(!all.contains("reach_3") && !all.contains("vote_5"));
    }

    #[test]
    fn nothing_is_drawn_from_domains_that_emit_nothing() {
        // Only chat messages have been seen on this server.
        let quiet = Player { friends: 10, done: HashSet::new(), live_rules: HashSet::from(["chat.message"]) };
        let all: HashSet<&str> = (0..2000)
            .flat_map(|attempt| draw(DAILY, DAILY_MISSIONS, &quiet, attempt))
            .map(|mission| mission.key)
            .collect();
        assert_eq!(all, HashSet::from(["xp_50", "xp_150", "messages_5", "messages_20", "reach_3"]));
        // On a server where nothing happened yet there is still something to do.
        let silent = Player { friends: 0, done: HashSet::new(), live_rules: HashSet::new() };
        let daily = draw(DAILY, DAILY_MISSIONS, &silent, 1);
        assert_eq!(daily.iter().map(|mission| mission.metric).collect::<HashSet<_>>(), HashSet::from([Metric::Xp]));
        assert_eq!(draw(WEEKLY, 1, &silent, 1).len(), 1);
    }

    #[test]
    fn creator_missions_are_only_for_people_who_have_created() {
        let viewer = player(5);
        let streamer = Player { done: HashSet::from([Metric::LivesStarted, Metric::VideosPublished]), ..player(5) };
        let weekly = |player: &Player| -> HashSet<&str> {
            (0..2000).flat_map(|attempt| draw(WEEKLY, 1, player, attempt)).map(|mission| mission.key).collect()
        };
        assert!(!weekly(&viewer).contains("w_live_1") && !weekly(&viewer).contains("w_video_1"));
        assert!(weekly(&streamer).contains("w_live_1") && weekly(&streamer).contains("w_video_1"));
    }

    #[test]
    fn a_daily_set_has_three_different_things_of_mixed_difficulty() {
        let someone = player(10);
        for attempt in 0..500 {
            let set = draw(DAILY, DAILY_MISSIONS, &someone, attempt);
            assert_eq!(set.len(), DAILY_MISSIONS);
            let metrics: HashSet<Metric> = set.iter().map(|mission| mission.metric).collect();
            assert_eq!(metrics.len(), DAILY_MISSIONS, "attempt {attempt}: two missions count the same thing");
            assert_eq!(set[0].difficulty, EASY);
            assert_eq!(set[1].difficulty, NORMAL);
            assert_eq!(set[2].difficulty, HARD);
            assert!(set.iter().all(|mission| mission.period == DAILY));
        }
    }

    #[test]
    fn a_draw_is_repeatable_and_different_players_get_different_sets() {
        let someone = player(10);
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let keys = |user: Uuid, day, attempt| -> Vec<&str> {
            draw(DAILY, DAILY_MISSIONS, &someone, seed(user, day, DAILY, attempt))
                .iter()
                .map(|mission| mission.key)
                .collect()
        };
        assert_eq!(keys(a, day, 0), keys(a, day, 0));
        let distinct: HashSet<Vec<&str>> =
            (0..30).map(|offset| keys(a, day + chrono::Duration::days(offset), 0)).collect();
        assert!(distinct.len() > 10, "the same missions come up day after day");
        let per_user: HashSet<Vec<&str>> = (0..30).map(|_| keys(Uuid::now_v7(), day, 0)).collect();
        assert!(per_user.len() > 10 || keys(a, day, 0) != keys(b, day, 0));
    }

    #[test]
    fn a_reroll_brings_something_else_that_the_set_does_not_count_yet() {
        let someone = player(10);
        for attempt in 0..300 {
            let set = draw(DAILY, DAILY_MISSIONS, &someone, attempt);
            let replaced = set[1];
            let others: Vec<Metric> =
                set.iter().filter(|mission| mission.key != replaced.key).map(|mission| mission.metric).collect();
            let new = replacement(
                DAILY,
                replaced.difficulty,
                replaced.key,
                replaced.metric,
                &others,
                &someone,
                attempt.wrapping_mul(31) + 7,
            )
            .expect("something else suits");
            assert_ne!(new.key, replaced.key);
            assert_ne!(new.metric, replaced.metric, "attempt {attempt}");
            assert!(!others.contains(&new.metric), "attempt {attempt}");
            assert_eq!(new.difficulty, replaced.difficulty, "attempt {attempt}");
        }
    }

    #[test]
    fn a_reroll_fails_when_nothing_else_suits() {
        // Only XP missions exist on a silent server: there is nothing to swap to.
        let silent = Player { friends: 0, done: HashSet::new(), live_rules: HashSet::new() };
        assert!(replacement(DAILY, EASY, "xp_50", Metric::Xp, &[], &silent, 3).is_none());
    }
}
