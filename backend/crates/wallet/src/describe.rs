//! Human wording for ledger entries.
//!
//! The ledger stores a `kind` (what sort of movement) and a `reason` (what it
//! was for, named by whichever domain moved the coins). History lists and
//! earnings breakdowns show people a sentence instead; this module is the one
//! place that knows the sentences. Reasons it has never heard of still get a
//! sensible line from the kind and the direction of the movement.
use tokuly_core::outbound::Localized;
use tokuly_proto::wallet::EntryKind;

/// The ledger's `kind` column as the API enum. Kinds that never reach a
/// user's history (settlements, account closures) map to `UNSPECIFIED`.
pub fn entry_kind(kind: &str) -> EntryKind {
    match kind {
        "grant" => EntryKind::ENTRY_KIND_GRANT,
        "spend" => EntryKind::ENTRY_KIND_SPEND,
        "transfer" => EntryKind::ENTRY_KIND_TRANSFER,
        "refund" => EntryKind::ENTRY_KIND_REFUND,
        "payout" => EntryKind::ENTRY_KIND_PAYOUT,
        "adjustment" => EntryKind::ENTRY_KIND_ADJUSTMENT,
        _ => EntryKind::ENTRY_KIND_UNSPECIFIED,
    }
}

/// What a reason is about, as far as wording goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Topic {
    Purchase,
    DailyBonus,
    LiveGift,
    StampPack,
    PayoutRejected,
    PayoutCancelled,
    Other,
}

fn topic(reason: &str) -> Topic {
    // Domains name reasons `<domain>.<thing>[.<detail>]`; the first segment
    // is enough to pick the wording.
    match reason.split('.').next().unwrap_or_default() {
        "purchase" => Topic::Purchase,
        "daily_bonus" => Topic::DailyBonus,
        "live" => Topic::LiveGift,
        "stamp" => Topic::StampPack,
        "payout" if reason == "payout.rejected" => Topic::PayoutRejected,
        "payout" if reason == "payout.cancelled" => Topic::PayoutCancelled,
        _ => Topic::Other,
    }
}

/// One history line for an entry, from the side of the user whose balances
/// changed by `coins_delta` / `earnings_delta`.
pub fn describe(kind: &str, reason: &str, coins_delta: i64, earnings_delta: i64) -> Localized {
    let topic = topic(reason);
    let received = coins_delta > 0 || earnings_delta > 0;
    let (ja, en) = match (kind, topic) {
        ("grant", Topic::Purchase) => ("コインを購入しました", "Bought coins"),
        ("grant", Topic::DailyBonus) => ("ログインボーナス", "Daily login bonus"),
        ("grant", _) => ("コインをもらいました", "Coins received"),

        ("spend", Topic::StampPack) => ("スタンプパックを購入しました", "Bought a stamp pack"),
        ("spend", _) => ("コインを使いました", "Coins spent"),

        ("transfer", Topic::LiveGift) if received => ("ライブギフトを受け取りました", "Received a live gift"),
        ("transfer", Topic::LiveGift) => ("ライブギフトを送りました", "Sent a live gift"),
        ("transfer", Topic::StampPack) if received => ("スタンプパックが売れました", "Sold a stamp pack"),
        ("transfer", Topic::StampPack) => ("スタンプパックを購入しました", "Bought a stamp pack"),
        ("transfer", _) if received => ("コインを受け取りました", "Coins received"),
        ("transfer", _) => ("コインを送りました", "Coins sent"),

        ("refund", Topic::PayoutRejected) => ("出金申請が却下され、売上に戻りました", "Payout request rejected; earnings returned"),
        ("refund", Topic::PayoutCancelled) => ("出金申請を取り消しました", "Payout request cancelled"),
        ("refund", _) if received => ("返金されました", "Refund"),
        ("refund", _) => ("返金のため売上が取り消されました", "Earnings reversed by a refund"),

        ("payout", _) => ("出金を申請しました", "Payout requested"),
        ("adjustment", _) => ("運営による残高の調整", "Balance adjusted by Tokuly staff"),
        ("closure", _) => ("アカウント削除による残高の失効", "Balance forfeited on account deletion"),
        _ => ("残高の変動", "Balance change"),
    };
    Localized::new(ja, en)
}

/// Label of an income source in the earnings breakdown.
pub fn earnings_label(reason: &str) -> Localized {
    match topic(reason) {
        Topic::LiveGift => Localized::new("ライブギフト", "Live gifts"),
        Topic::StampPack => Localized::new("スタンプパックの売上", "Stamp pack sales"),
        _ => Localized::new(format!("その他（{reason}）"), format!("Other ({reason})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfers_read_differently_for_each_side() {
        assert_eq!(describe("transfer", "live.gift", -300, 0).ja, "ライブギフトを送りました");
        assert_eq!(describe("transfer", "live.gift", 0, 210).ja, "ライブギフトを受け取りました");
        assert_eq!(describe("transfer", "stamp.pack", -120, 0).en, "Bought a stamp pack");
        assert_eq!(describe("transfer", "stamp.pack", 0, 84).en, "Sold a stamp pack");
    }

    #[test]
    fn known_reasons_get_specific_wording() {
        assert_eq!(describe("grant", "purchase", 500, 0).en, "Bought coins");
        assert_eq!(describe("grant", "daily_bonus", 10, 0).ja, "ログインボーナス");
        assert_eq!(describe("payout", "payout", 0, -1000).en, "Payout requested");
        assert_eq!(describe("refund", "payout.rejected", 0, 1000).en, "Payout request rejected; earnings returned");
        assert_eq!(describe("refund", "payout.cancelled", 0, 1000).en, "Payout request cancelled");
        assert_eq!(describe("adjustment", "adjustment", -5, 0).en, "Balance adjusted by Tokuly staff");
    }

    #[test]
    fn unknown_reasons_fall_back_to_kind_and_direction() {
        assert_eq!(describe("grant", "social.invite_reward", 50, 0).en, "Coins received");
        assert_eq!(describe("spend", "video.rental", -50, 0).en, "Coins spent");
        assert_eq!(describe("transfer", "tip", -50, 0).en, "Coins sent");
        assert_eq!(describe("transfer", "tip", 0, 35).en, "Coins received");
        assert_eq!(describe("refund", "live.gift.returned", 300, 0).en, "Refund");
        assert_eq!(describe("refund", "live.gift.returned", 0, -210).en, "Earnings reversed by a refund");
        assert_eq!(describe("mystery", "x", 0, 0).en, "Balance change");
    }

    #[test]
    fn kinds_map_to_the_api_enum() {
        assert_eq!(entry_kind("transfer"), EntryKind::ENTRY_KIND_TRANSFER);
        assert_eq!(entry_kind("payout"), EntryKind::ENTRY_KIND_PAYOUT);
        assert_eq!(entry_kind("payout_settlement"), EntryKind::ENTRY_KIND_UNSPECIFIED);
    }

    #[test]
    fn earnings_labels_name_the_source() {
        assert_eq!(earnings_label("live.gift").ja, "ライブギフト");
        assert_eq!(earnings_label("stamp.pack").en, "Stamp pack sales");
        assert_eq!(earnings_label("tip").en, "Other (tip)");
    }
}
