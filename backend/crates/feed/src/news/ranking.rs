//! The recommended news order: pure functions, no I/O.
//!
//! A reader's interest in a tag is a number that grows with clicks and "more
//! like this", shrinks with "less like this" and halves every half-life
//! ([`decayed`]). An article's score is its freshness multiplied by how much
//! the reader cares about its tags ([`score`]).
//!
//! The recommended order is that ranking with a share of *random* recent
//! articles mixed in at regular positions ([`recommend`]), each flagged so
//! clients can say so ("このニュースはあなたの好みとは別にランダムに取得された
//! ニュースです。"). The random choice is driven by a seed, so the same seed
//! and the same candidates always give the same order: a page cursor only has
//! to carry the seed and an offset.
use std::collections::{HashMap, HashSet};

use tokuly_core::prelude::*;

/// Interest added by opening an article, per tag.
pub const CLICK_WEIGHT: f64 = 1.0;
/// Interest added (or removed) by explicit feedback, per tag.
pub const FEEDBACK_WEIGHT: f64 = 3.0;
/// Interests are kept within ±this, so no tag can drown everything else.
pub const MAX_AFFINITY: f64 = 20.0;
/// Below this an interest is noise: not listed, and swept away eventually.
pub const AFFINITY_EPSILON: f64 = 0.05;

/// Hours after which an article is half as fresh.
const FRESHNESS_HALF_LIFE_HOURS: f64 = 36.0;
/// How strongly interests bend the ranking (`exp(interest * gain)`).
const INTEREST_GAIN: f64 = 0.35;
/// The summed interest of an article's tags is clamped to ±this.
const MAX_ARTICLE_INTEREST: f64 = 6.0;
/// Articles the reader already opened sink, but stay reachable.
const CLICKED_FACTOR: f64 = 0.25;

/// The value now of an interest that was `score` `age_seconds` ago.
pub fn decayed(score: f64, age_seconds: f64, half_life_seconds: f64) -> f64 {
    if half_life_seconds <= 0.0 {
        return score;
    }
    score * 0.5_f64.powf(age_seconds.max(0.0) / half_life_seconds)
}

/// An article that may be recommended.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: Uuid,
    pub tags: Vec<String>,
    pub published_at: Time,
    /// The reader opened it before.
    pub clicked: bool,
}

/// One position of the recommended order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ranked {
    pub id: Uuid,
    /// Chosen at random rather than for the reader's interests.
    pub random_pick: bool,
}

/// How much a reader with `affinity` should want to see `candidate` now.
pub fn score(candidate: &Candidate, affinity: &HashMap<String, f64>, now: Time) -> f64 {
    let age_hours = (now - candidate.published_at).num_seconds().max(0) as f64 / 3600.0;
    let freshness = 0.5_f64.powf(age_hours / FRESHNESS_HALF_LIFE_HOURS);
    let interest: f64 = candidate.tags.iter().filter_map(|tag| affinity.get(tag)).sum();
    let interest = interest.clamp(-MAX_ARTICLE_INTEREST, MAX_ARTICLE_INTEREST);
    let seen = if candidate.clicked { CLICKED_FACTOR } else { 1.0 };
    freshness * (interest * INTEREST_GAIN).exp() * seen
}

/// A small deterministic generator (SplitMix64). The order of random picks
/// must be reproducible from a seed for as long as cursors live, whatever
/// version of a random-number library is linked.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number in `0..bound` (`bound` > 0).
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// A seed from a cursor token (FNV-1a).
pub fn seed_of(token: &str) -> u64 {
    token.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

fn shuffle<T>(items: &mut [T], rng: &mut SplitMix64) {
    for i in (1..items.len()).rev() {
        items.swap(i, rng.below(i + 1));
    }
}

/// The recommended order of `candidates` for a reader.
///
/// Articles are ranked by [`score`]. `random_percent` (0 to 100) of the
/// positions, evenly spaced, are instead filled from a seeded shuffle of the
/// articles the reader has not opened, and flagged `random_pick`. Every
/// candidate appears exactly once.
pub fn recommend(
    mut candidates: Vec<Candidate>,
    affinity: &HashMap<String, f64>,
    now: Time,
    random_percent: u32,
    seed: u64,
) -> Vec<Ranked> {
    // A stable starting order, so the result depends only on the inputs.
    candidates.sort_by(|a, b| b.published_at.cmp(&a.published_at).then_with(|| b.id.cmp(&a.id)));

    let mut random: Vec<Uuid> =
        candidates.iter().filter(|candidate| !candidate.clicked).map(|candidate| candidate.id).collect();
    shuffle(&mut random, &mut SplitMix64(seed));

    let mut scored: Vec<(f64, Uuid)> =
        candidates.iter().map(|candidate| (score(candidate, affinity, now), candidate.id)).collect();
    // Stable sort: equal scores keep the newest-first order from above.
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    let share = random_percent.min(100);
    let mut used: HashSet<Uuid> = HashSet::with_capacity(candidates.len());
    let mut order = Vec::with_capacity(candidates.len());
    let (mut next_ranked, mut next_random) = (scored.iter().map(|(_, id)| *id), random.into_iter());
    // Whole percents, so the spacing never drifts: 20 % is exactly every fifth position.
    let mut owed = 0;
    while order.len() < candidates.len() {
        owed += share;
        if owed >= 100 {
            owed -= 100;
            if let Some(id) = next_random.by_ref().find(|id| !used.contains(id)) {
                used.insert(id);
                order.push(Ranked { id, random_pick: true });
                continue;
            }
        }
        match next_ranked.by_ref().find(|id| !used.contains(id)) {
            Some(id) => {
                used.insert(id);
                order.push(Ranked { id, random_pick: false });
            }
            // Every article was placed (the ranked list holds them all).
            None => break,
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn candidate(n: u128, tags: &[&str], hours_old: i64, now: Time) -> Candidate {
        Candidate {
            id: Uuid::from_u128(n),
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            published_at: now - Duration::hours(hours_old),
            clicked: false,
        }
    }

    fn affinity(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(tag, value)| ((*tag).to_owned(), *value)).collect()
    }

    #[test]
    fn interests_halve_every_half_life() {
        let day = 86_400.0;
        assert_eq!(decayed(8.0, 0.0, 30.0 * day), 8.0);
        assert!((decayed(8.0, 30.0 * day, 30.0 * day) - 4.0).abs() < 1e-9);
        assert!((decayed(8.0, 90.0 * day, 30.0 * day) - 1.0).abs() < 1e-9);
        assert!((decayed(-6.0, 30.0 * day, 30.0 * day) + 3.0).abs() < 1e-9, "dislikes fade too");
        assert_eq!(decayed(5.0, -100.0, day), 5.0, "a clock running backwards changes nothing");
        assert_eq!(decayed(5.0, 100.0, 0.0), 5.0);
    }

    #[test]
    fn fresher_articles_score_higher_all_else_equal() {
        let now = now();
        let none = HashMap::new();
        let fresh = score(&candidate(1, &["a"], 1, now), &none, now);
        let day_old = score(&candidate(2, &["a"], 24, now), &none, now);
        let old = score(&candidate(3, &["a"], 72, now), &none, now);
        assert!(fresh > day_old && day_old > old);
        assert!((old - 0.25).abs() < 1e-9, "two freshness half-lives");
    }

    #[test]
    fn interests_lift_and_dislikes_sink() {
        let now = now();
        let reader = affinity(&[("ゲーム", 4.0), ("政治", -4.0)]);
        let liked = score(&candidate(1, &["ゲーム"], 48, now), &reader, now);
        let neutral = score(&candidate(2, &["天気"], 2, now), &reader, now);
        let disliked = score(&candidate(3, &["政治"], 1, now), &reader, now);
        assert!(liked > neutral, "a strong interest outweighs two days of age");
        assert!(neutral > disliked);
        // A liked and a disliked tag on the same article cancel out.
        let mixed = score(&candidate(4, &["ゲーム", "政治"], 2, now), &reader, now);
        assert!((mixed - neutral).abs() < 1e-9);
    }

    #[test]
    fn interest_is_bounded_and_opened_articles_sink() {
        let now = now();
        let obsessed = affinity(&[("a", 20.0), ("b", 20.0)]);
        let capped = score(&candidate(1, &["a", "b"], 0, now), &obsessed, now);
        assert!((capped - (MAX_ARTICLE_INTEREST * INTEREST_GAIN).exp()).abs() < 1e-9);

        let mut opened = candidate(2, &["a"], 0, now);
        let before = score(&opened, &HashMap::new(), now);
        opened.clicked = true;
        assert!((score(&opened, &HashMap::new(), now) - before * CLICKED_FACTOR).abs() < 1e-9);
    }

    fn pool(now: Time) -> Vec<Candidate> {
        (1..=50).map(|n| candidate(n, if n % 2 == 0 { &["even"] } else { &["odd"] }, n as i64, now)).collect()
    }

    #[test]
    fn every_article_appears_exactly_once() {
        let now = now();
        for share in [0, 20, 50, 100] {
            let order = recommend(pool(now), &affinity(&[("even", 3.0)]), now, share, 7);
            assert_eq!(order.len(), 50, "share {share}");
            assert_eq!(order.iter().map(|r| r.id).collect::<HashSet<_>>().len(), 50, "share {share}");
        }
        assert!(recommend(Vec::new(), &HashMap::new(), now, 20, 1).is_empty());
    }

    #[test]
    fn the_configured_share_of_positions_is_random() {
        let now = now();
        let order = recommend(pool(now), &HashMap::new(), now, 20, 99);
        let random_positions: Vec<usize> =
            order.iter().enumerate().filter(|(_, r)| r.random_pick).map(|(i, _)| i).collect();
        assert_eq!(random_positions, vec![4, 9, 14, 19, 24, 29, 34, 39, 44, 49], "every fifth position");

        assert!(recommend(pool(now), &HashMap::new(), now, 0, 99).iter().all(|r| !r.random_pick));
        assert!(recommend(pool(now), &HashMap::new(), now, 100, 99).iter().all(|r| r.random_pick));
    }

    #[test]
    fn without_random_picks_the_order_is_the_ranking() {
        let now = now();
        let order = recommend(pool(now), &affinity(&[("even", 5.0)]), now, 0, 1);
        // The interest lifts the freshest "even" articles to the top.
        assert_eq!(order[0].id, Uuid::from_u128(2));
        assert_eq!(order[1].id, Uuid::from_u128(4));
        // Without interests it is simply newest first.
        let plain = recommend(pool(now), &HashMap::new(), now, 0, 1);
        assert_eq!(plain.iter().map(|r| r.id).collect::<Vec<_>>(), (1..=50).map(Uuid::from_u128).collect::<Vec<_>>());
    }

    #[test]
    fn the_same_seed_gives_the_same_order_and_another_seed_another() {
        let now = now();
        let reader = affinity(&[("odd", 2.0)]);
        let first = recommend(pool(now), &reader, now, 20, seed_of("tokenA"));
        let mut reversed_input = pool(now);
        reversed_input.reverse();
        assert_eq!(
            first,
            recommend(reversed_input, &reader, now, 20, seed_of("tokenA")),
            "input order does not matter"
        );
        let other = recommend(pool(now), &reader, now, 20, seed_of("tokenB"));
        assert_ne!(first, other);
        let random_of = |order: &[Ranked]| order.iter().filter(|r| r.random_pick).map(|r| r.id).collect::<Vec<_>>();
        assert_ne!(random_of(&first), random_of(&other));
    }

    #[test]
    fn opened_articles_are_never_random_picks() {
        let now = now();
        let mut candidates = pool(now);
        for candidate in candidates.iter_mut().filter(|c| c.id.as_u128() <= 40) {
            candidate.clicked = true;
        }
        let order = recommend(candidates, &HashMap::new(), now, 50, 5);
        assert_eq!(order.len(), 50);
        let random: Vec<u128> = order.iter().filter(|r| r.random_pick).map(|r| r.id.as_u128()).collect();
        assert!(
            !random.is_empty() && random.len() <= 10,
            "only the ten unopened articles can be picked at random: {random:?}"
        );
        assert!(random.iter().all(|n| *n > 40));
    }

    #[test]
    fn seeds_differ_between_tokens() {
        assert_eq!(seed_of("abc"), seed_of("abc"));
        assert_ne!(seed_of("abc"), seed_of("abd"));
        assert_ne!(seed_of(""), seed_of("a"));
    }
}
