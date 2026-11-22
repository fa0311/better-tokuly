//! The level curve.
//!
//! Level 2 takes 100 XP, and every further level takes 50 XP more than the
//! one before it (150, 200, 250, ...). So the total XP at which level `L`
//! is reached is
//!
//! ```text
//! total(L) = 100 + 150 + ... = 25 × (L − 1) × (L + 2)
//! ```
//!
//! Early levels come within a day or two; at a couple of hundred XP a day
//! (an ordinary day, far below the caps) level 10 takes about two weeks,
//! level 20 two months and level 50 the better part of a year. There is no
//! top level, and XP is never taken away.
use tokuly_proto::quest::{LevelCurve, LevelProgress};

/// XP from level 1 to level 2.
pub const FIRST_LEVEL_XP: i64 = 100;
/// How much more XP each level takes than the one before.
pub const STEP_XP: i64 = 50;

/// Total XP at which `level` is reached (0 for level 1).
pub fn threshold(level: i32) -> i64 {
    let level = level.max(1) as i64;
    (STEP_XP / 2) * (level - 1) * (level + 2)
}

/// The level a player with `xp` total XP is on.
pub fn level_for(xp: i64) -> i32 {
    if xp <= 0 {
        return 1;
    }
    // Invert the quadratic, then settle the rounding on integers.
    let estimate = ((-1.0 + (9.0 + 4.0 * xp as f64 / (STEP_XP / 2) as f64).sqrt()) / 2.0).floor() as i32;
    let mut level = estimate.max(1);
    while threshold(level + 1) <= xp {
        level += 1;
    }
    while level > 1 && threshold(level) > xp {
        level -= 1;
    }
    level
}

/// XP between `level` and the next one.
pub fn xp_for_next(level: i32) -> i64 {
    threshold(level + 1) - threshold(level)
}

/// Where a player with `xp` total XP stands, as the API reports it.
pub fn progress(xp: i64) -> LevelProgress {
    let xp = xp.max(0);
    let level = level_for(xp);
    LevelProgress {
        level,
        total_xp: xp,
        xp_into_level: xp - threshold(level),
        xp_for_next_level: xp_for_next(level),
        ..Default::default()
    }
}

/// Coins granted for reaching `level`: the small reward, or the milestone
/// reward on every tenth level.
pub fn level_up_coins(level: i32, level_coins: i64, milestone_coins: i64) -> i64 {
    if level % 10 == 0 { milestone_coins } else { level_coins }
}

/// The curve as the API describes it.
pub fn curve(level_coins: i64, milestone_coins: i64) -> LevelCurve {
    LevelCurve {
        first_level_xp: FIRST_LEVEL_XP as i32,
        step_xp: STEP_XP as i32,
        thresholds: (2..=51).map(threshold).collect(),
        level_up_coins: level_coins as i32,
        milestone_level_coins: milestone_coins as i32,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_takes_fifty_xp_more_than_the_one_before() {
        assert_eq!(threshold(1), 0);
        assert_eq!(threshold(2), 100);
        assert_eq!(threshold(3), 250);
        assert_eq!(threshold(4), 450);
        assert_eq!(xp_for_next(1), FIRST_LEVEL_XP);
        for level in 1..200 {
            assert_eq!(xp_for_next(level + 1) - xp_for_next(level), STEP_XP);
        }
    }

    #[test]
    fn levels_change_exactly_at_their_thresholds() {
        assert_eq!(level_for(-5), 1);
        assert_eq!(level_for(0), 1);
        assert_eq!(level_for(99), 1);
        assert_eq!(level_for(100), 2);
        assert_eq!(level_for(249), 2);
        assert_eq!(level_for(250), 3);
        for level in 1..500 {
            assert_eq!(level_for(threshold(level)), level, "at the threshold of level {level}");
            assert_eq!(level_for(threshold(level + 1) - 1), level, "one XP short of level {}", level + 1);
        }
    }

    #[test]
    fn the_level_never_goes_down_as_xp_grows() {
        let mut previous = 1;
        for xp in (0..300_000).step_by(37) {
            let level = level_for(xp);
            assert!(level >= previous);
            previous = level;
        }
        // Far beyond anything reachable: still consistent.
        let huge = level_for(i64::MAX / 4);
        assert!(threshold(huge) <= i64::MAX / 4 && threshold(huge + 1) > i64::MAX / 4);
    }

    #[test]
    fn progress_splits_total_xp_into_the_current_level() {
        let p = progress(300);
        assert_eq!((p.level, p.total_xp, p.xp_into_level, p.xp_for_next_level), (3, 300, 50, 200));
        let p = progress(0);
        assert_eq!((p.level, p.xp_into_level, p.xp_for_next_level), (1, 0, 100));
    }

    #[test]
    fn every_tenth_level_pays_the_milestone_reward() {
        assert_eq!(level_up_coins(2, 20, 100), 20);
        assert_eq!(level_up_coins(10, 20, 100), 100);
        assert_eq!(level_up_coins(11, 20, 100), 20);
        let curve = curve(20, 100);
        assert_eq!(curve.thresholds.len(), 50);
        assert_eq!(curve.thresholds[0], 100);
    }
}
