//! Drawing the friends of a poll, fairly.
//!
//! "Fair" means everyone comes up over time: a poll shows the friends who
//! were shown least so far, and chance only decides between friends who were
//! shown equally often. Pure chance would leave some friends out for a long
//! time, and being left out of compliments is exactly what this feature must
//! not do to anyone.
//!
//! Every friend who can be shown has a count of how often they were. A
//! friend gets that count the first time they are eligible for a draw
//! ([`enrollment`]), and it starts at the lowest count among the others, not
//! at zero: someone who becomes a friend after a hundred polls joins the
//! rotation as an equal instead of being in every poll until they have
//! caught up. In a rotation that has only just begun the lowest count is
//! zero, so friends who have not come up yet are simply next.
use std::collections::HashMap;

use rand::{Rng, seq::SliceRandom as _};
use uuid::Uuid;

/// Friends shown in one poll.
pub(crate) const CHOICES: usize = 4;

/// The friends among `eligible` who have no count yet, with the count each
/// starts from: the lowest count among the eligible friends who have one
/// (zero when nobody has). The caller stores these before it draws.
pub(crate) fn enrollment(eligible: &[Uuid], shown: &HashMap<Uuid, i32>) -> (Vec<Uuid>, i32) {
    let start = eligible.iter().filter_map(|id| shown.get(id)).copied().min().unwrap_or(0);
    let mut newcomers: Vec<Uuid> = eligible.iter().copied().filter(|id| !shown.contains_key(id)).collect();
    newcomers.sort_unstable();
    newcomers.dedup();
    (newcomers, start)
}

/// Picks up to `count` of `eligible`, leaving out `exclude`: those shown
/// least often first (`shown` holds everyone's count, see [`enrollment`]),
/// ties decided by chance. The result is in random order, so a position on
/// the screen says nothing about how often someone was shown.
pub(crate) fn draw<R: Rng + ?Sized>(
    eligible: &[Uuid],
    shown: &HashMap<Uuid, i32>,
    exclude: &[Uuid],
    count: usize,
    rng: &mut R,
) -> Vec<Uuid> {
    let mut candidates: Vec<Uuid> = eligible.iter().copied().filter(|id| !exclude.contains(id)).collect();
    candidates.sort_unstable();
    candidates.dedup();
    // Shuffle, then sort stably by count: equal counts stay in random order.
    candidates.shuffle(rng);
    candidates.sort_by_key(|id| shown.get(id).copied().unwrap_or(0));
    candidates.truncate(count);
    candidates.shuffle(rng);
    candidates
}

#[cfg(test)]
mod tests {
    use rand::{SeedableRng as _, rngs::StdRng};

    use super::*;

    fn friends(n: usize) -> Vec<Uuid> {
        (0..n).map(|_| Uuid::now_v7()).collect()
    }

    /// One poll, counted the way `rounds.rs` does it: enroll whoever has no
    /// count, draw, count one appearance for each friend drawn.
    fn poll(eligible: &[Uuid], shown: &mut HashMap<Uuid, i32>, rng: &mut StdRng) -> Vec<Uuid> {
        let (newcomers, start) = enrollment(eligible, shown);
        for id in newcomers {
            shown.insert(id, start);
        }
        let drawn = draw(eligible, shown, &[], CHOICES, rng);
        for id in &drawn {
            *shown.get_mut(id).expect("everyone eligible is enrolled") += 1;
        }
        drawn
    }

    fn polls(eligible: &[Uuid], shown: &mut HashMap<Uuid, i32>, rounds: usize, rng: &mut StdRng) {
        for _ in 0..rounds {
            poll(eligible, shown, rng);
        }
    }

    #[test]
    fn everyone_comes_up_equally_often_over_time() {
        let mut rng = StdRng::seed_from_u64(7);
        for size in [4, 5, 9, 23] {
            let eligible = friends(size);
            let mut shown = HashMap::new();
            polls(&eligible, &mut shown, 200, &mut rng);
            let counts: Vec<i32> = eligible.iter().map(|id| shown[id]).collect();
            let (min, max) = (counts.iter().min().unwrap(), counts.iter().max().unwrap());
            assert!(max - min <= 1, "{size} friends: shown between {min} and {max} times");
            assert_eq!(
                counts.iter().sum::<i32>(),
                200 * CHOICES as i32,
                "{size} friends: every appearance is counted once"
            );
        }
    }

    #[test]
    fn nobody_comes_up_twice_before_everyone_came_up_once() {
        let mut rng = StdRng::seed_from_u64(11);
        let eligible = friends(23);
        let mut shown = HashMap::new();
        // 23 friends, four per poll: after five polls twenty different
        // friends were shown; the sixth brings the last three.
        let mut seen: Vec<Uuid> = Vec::new();
        for _ in 0..5 {
            seen.extend(poll(&eligible, &mut shown, &mut rng));
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 20, "the first five polls show twenty different friends");
        let sixth = poll(&eligible, &mut shown, &mut rng);
        assert_eq!(
            sixth.iter().filter(|id| !seen.contains(id)).count(),
            3,
            "the three friends not shown yet come next"
        );
    }

    #[test]
    fn a_draw_has_four_distinct_friends() {
        let mut rng = StdRng::seed_from_u64(1);
        let eligible = friends(10);
        for _ in 0..50 {
            let mut drawn = draw(&eligible, &HashMap::new(), &[], CHOICES, &mut rng);
            assert_eq!(drawn.len(), CHOICES);
            drawn.sort_unstable();
            drawn.dedup();
            assert_eq!(drawn.len(), CHOICES, "no friend twice in one poll");
            assert!(drawn.iter().all(|id| eligible.contains(id)));
        }
    }

    #[test]
    fn excluded_friends_are_left_out() {
        let mut rng = StdRng::seed_from_u64(2);
        let eligible = friends(8);
        let current = &eligible[..4];
        for _ in 0..20 {
            let reshuffled = draw(&eligible, &HashMap::new(), current, CHOICES, &mut rng);
            assert_eq!(reshuffled.len(), CHOICES);
            assert!(reshuffled.iter().all(|id| !current.contains(id)), "a shuffle shows four others");
        }
    }

    #[test]
    fn fewer_candidates_than_asked_returns_what_there_is() {
        let mut rng = StdRng::seed_from_u64(3);
        let eligible = friends(5);
        let drawn = draw(&eligible, &HashMap::new(), &eligible[..4], CHOICES, &mut rng);
        assert_eq!(drawn, vec![eligible[4]]);
        assert!(draw(&[], &HashMap::new(), &[], CHOICES, &mut rng).is_empty());
    }

    #[test]
    fn a_new_friend_joins_the_rotation_without_taking_it_over() {
        let mut rng = StdRng::seed_from_u64(4);
        let mut eligible = friends(8);
        let mut shown = HashMap::new();
        polls(&eligible, &mut shown, 100, &mut rng);
        assert!(eligible.iter().all(|id| shown[id] == 50), "eight friends, four hundred appearances");

        let newcomer = Uuid::now_v7();
        eligible.push(newcomer);
        polls(&eligible, &mut shown, 9, &mut rng);
        // Enrolled at the others' 50; nine polls of four among nine friends
        // are four appearances each.
        assert_eq!(shown[&newcomer], 54, "the newcomer came up as often as everyone else");

        // Starting the newcomer from zero is the mistake this guards
        // against: they would be the least shown in every one of the polls.
        let mut naive = shown.clone();
        naive.insert(newcomer, 0);
        let mut monopolized = 0;
        for _ in 0..9 {
            let drawn = draw(&eligible, &naive, &[], CHOICES, &mut rng);
            monopolized += i32::from(drawn.contains(&newcomer));
            for id in drawn {
                *naive.entry(id).or_insert(0) += 1;
            }
        }
        assert_eq!(monopolized, 9, "a count that starts at zero puts the newcomer in every poll");
    }

    #[test]
    fn enrollment_starts_newcomers_at_the_lowest_count_of_the_others() {
        let eligible = friends(3);
        let mut shown = HashMap::new();
        let (newcomers, start) = enrollment(&eligible, &shown);
        assert_eq!((newcomers.len(), start), (3, 0), "a rotation that has not begun starts at zero");

        shown.insert(eligible[0], 7);
        shown.insert(eligible[1], 5);
        // Someone who is not eligible does not pull the start down.
        shown.insert(Uuid::now_v7(), 1);
        let (newcomers, start) = enrollment(&eligible, &shown);
        assert_eq!(newcomers, vec![eligible[2]]);
        assert_eq!(start, 5);

        shown.insert(eligible[2], 5);
        assert!(enrollment(&eligible, &shown).0.is_empty(), "nobody is enrolled twice");
    }

    #[test]
    fn duplicates_in_the_input_do_not_double_a_friends_chances() {
        let mut rng = StdRng::seed_from_u64(5);
        let base = friends(4);
        let doubled: Vec<Uuid> = base.iter().chain(base.iter()).copied().collect();
        let mut drawn = draw(&doubled, &HashMap::new(), &[], CHOICES, &mut rng);
        drawn.sort_unstable();
        let mut expected = base.clone();
        expected.sort_unstable();
        assert_eq!(drawn, expected);
        assert_eq!(enrollment(&doubled, &HashMap::new()).0.len(), 4);
    }
}
