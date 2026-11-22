//! The gift ladder: twelve colour tiers by coin amount.
//!
//! This table is the only place the tiers are defined. Clients read it
//! through `LiveGiftService.GetGiftTiers` instead of hard-coding it, and the
//! server validates gifts against it: the bigger the gift, the longer its
//! message may be and the longer it stays highlighted above the chat.
use tokuly_proto::live::GiftTier;

/// Smallest and largest gift, in coins.
pub const MIN_GIFT_COINS: i64 = 10;
pub const MAX_GIFT_COINS: i64 = 100_000;

pub struct Tier {
    pub level: i32,
    pub min_coins: i64,
    pub color: &'static str,
    pub background: &'static str,
    pub text: &'static str,
    pub max_message_length: usize,
    pub highlight_seconds: i64,
}

const fn tier(
    level: i32,
    min_coins: i64,
    color: &'static str,
    background: &'static str,
    text: &'static str,
    max_message_length: usize,
    highlight_seconds: i64,
) -> Tier {
    Tier { level, min_coins, color, background, text, max_message_length, highlight_seconds }
}

/// Ascending by `min_coins`; a gift belongs to the highest tier it reaches.
pub const TIERS: [Tier; 12] = [
    tier(1, 10, "blue", "#1e88e5", "#ffffff", 0, 0),
    tier(2, 50, "cyan", "#00e5ff", "#000000", 50, 0),
    tier(3, 100, "light-green", "#1de9b6", "#000000", 150, 120),
    tier(4, 300, "yellow", "#ffca28", "#000000", 200, 300),
    tier(5, 500, "orange", "#f57c00", "#ffffff", 225, 600),
    tier(6, 1_000, "magenta", "#e91e63", "#ffffff", 250, 1_800),
    tier(7, 2_000, "red", "#e62117", "#ffffff", 270, 3_600),
    tier(8, 5_000, "red", "#d00000", "#ffffff", 290, 7_200),
    tier(9, 10_000, "red", "#b80000", "#ffffff", 310, 10_800),
    tier(10, 20_000, "red", "#9a0000", "#ffffff", 330, 14_400),
    tier(11, 30_000, "black", "#18181b", "#ffffff", 350, 18_000),
    tier(12, 50_000, "rainbow", "#fae8ff", "#18181b", 350, 18_000),
];

/// Colours the rainbow tier runs through.
const RAINBOW: [&str; 6] = ["#fee2e2", "#fef9c3", "#dcfce7", "#cffafe", "#dbeafe", "#fae8ff"];

/// The tier of a gift of `coins`, or `None` when the amount is not allowed.
pub fn tier_for(coins: i64) -> Option<&'static Tier> {
    if !(MIN_GIFT_COINS..=MAX_GIFT_COINS).contains(&coins) {
        return None;
    }
    TIERS.iter().rev().find(|tier| coins >= tier.min_coins)
}

/// The table as the API returns it.
pub fn to_proto() -> Vec<GiftTier> {
    TIERS
        .iter()
        .map(|tier| GiftTier {
            level: tier.level,
            min_coins: tier.min_coins,
            color: tier.color.to_owned(),
            background_color: tier.background.to_owned(),
            text_color: tier.text.to_owned(),
            gradient_colors: if tier.color == "rainbow" {
                RAINBOW.iter().map(|c| (*c).to_owned()).collect()
            } else {
                Vec::new()
            },
            max_message_length: tier.max_message_length as i32,
            highlight_seconds: tier.highlight_seconds as i32,
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_twelve_tiers_that_only_grow() {
        assert_eq!(TIERS.len(), 12);
        assert_eq!(TIERS[0].min_coins, MIN_GIFT_COINS);
        for pair in TIERS.windows(2) {
            assert_eq!(pair[1].level, pair[0].level + 1);
            assert!(pair[1].min_coins > pair[0].min_coins);
            assert!(pair[1].max_message_length >= pair[0].max_message_length);
            assert!(pair[1].highlight_seconds >= pair[0].highlight_seconds);
        }
    }

    #[test]
    fn a_gift_belongs_to_the_highest_tier_it_reaches() {
        assert!(tier_for(9).is_none() && tier_for(0).is_none() && tier_for(-5).is_none());
        assert_eq!(tier_for(10).unwrap().level, 1);
        assert_eq!(tier_for(49).unwrap().level, 1);
        assert_eq!(tier_for(50).unwrap().level, 2);
        assert_eq!(tier_for(999).unwrap().level, 5);
        assert_eq!(tier_for(1_000).unwrap().level, 6);
        assert_eq!(tier_for(50_000).unwrap().level, 12);
        assert_eq!(tier_for(MAX_GIFT_COINS).unwrap().level, 12);
        assert!(tier_for(MAX_GIFT_COINS + 1).is_none());
    }

    #[test]
    fn the_api_table_mirrors_the_ladder() {
        let tiers = to_proto();
        assert_eq!(tiers.len(), 12);
        assert_eq!(tiers[0].color, "blue");
        assert_eq!(tiers[0].max_message_length, 0);
        assert!(tiers[11].gradient_colors.len() == 6 && tiers[10].gradient_colors.is_empty());
    }
}
