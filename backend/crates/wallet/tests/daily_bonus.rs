//! The daily login bonus: once per local day, streak-aware.
mod common;

use common::*;
use tokuly_proto::wallet::*;
use tokuly_testkit::{RpcError, TestApp, TestUser};
use tokuly_wallet::WalletConfig;

async fn status(app: &TestApp, user: &TestUser) -> DailyBonus {
    let response: GetDailyBonusResponse = app.call(&format!("{WALLET}/GetDailyBonus"), Some(user), &GetDailyBonusRequest::default()).await.unwrap();
    response.bonus.into_option().unwrap()
}

async fn claim(app: &TestApp, user: &TestUser) -> Result<ClaimDailyBonusResponse, RpcError> {
    app.call(&format!("{WALLET}/ClaimDailyBonus"), Some(user), &ClaimDailyBonusRequest::default()).await
}

/// Stores a streak whose last claim was `days_ago` days before today in `time_zone`.
async fn seed_streak(app: &TestApp, user: &TestUser, time_zone: &str, days_ago: i32, streak: i32) {
    sqlx::query(
        "INSERT INTO wallet_daily_bonuses (user_id, last_claimed_on, streak, longest_streak, total_claims)
         VALUES ($1, (now() AT TIME ZONE $2)::date - $3, $4, $4, $4)",
    )
    .bind(user.id)
    .bind(time_zone)
    .bind(days_ago)
    .bind(streak)
    .execute(&app.core.db)
    .await
    .unwrap();
}

async fn set_time_zone(app: &TestApp, user: &TestUser, time_zone: &str) {
    sqlx::query("UPDATE users SET time_zone = $2 WHERE id = $1").bind(user.id).bind(time_zone).execute(&app.core.db).await.unwrap();
}

#[tokio::test]
async fn the_bonus_is_claimed_once_a_day() {
    let (app, _) = app().await;
    let user = app.user("daily").await;

    let before = status(&app, &user).await;
    assert!(before.claimable);
    assert_eq!((before.streak, before.next_coins, before.next_cycle_day), (0, 10, 1));
    assert_eq!(before.cycle, vec![10, 10, 15, 15, 20, 20, 50]);

    let claimed = claim(&app, &user).await.unwrap();
    assert_eq!(claimed.coins, 10);
    assert_eq!(claimed.balance.as_option().unwrap().coins, 10);
    let after = claimed.bonus.as_option().unwrap();
    assert!(!after.claimable);
    assert_eq!((after.streak, after.next_coins, after.next_cycle_day, after.longest_streak, after.total_claims), (1, 10, 2, 1, 1));
    let next_claim_at = tokuly_core::time::from_ts(after.next_claim_at.as_option().expect("tomorrow is announced"));
    let wait = next_claim_at - chrono::Utc::now();
    assert!(wait > chrono::Duration::zero() && wait <= chrono::Duration::hours(24), "the next bonus unlocks at the user's next midnight");

    let err = claim(&app, &user).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("failed_precondition", "DAILY_BONUS_ALREADY_CLAIMED"));
    assert_eq!(balance(&app, &user).await.coins, 10);
    assert_eq!(status(&app, &user).await.streak, 1);

    let entries = history(&app, &user).await;
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].reason.as_str(), entries[0].description.as_str(), entries[0].coins_delta), ("daily_bonus", "ログインボーナス", 10));
    // Their own action: no "coins received" fuss.
    assert!(notifications(&app, &user, "wallet.received").await.is_empty());

    let err = app.call::<_, ClaimDailyBonusResponse>(&format!("{WALLET}/ClaimDailyBonus"), None, &ClaimDailyBonusRequest::default()).await.unwrap_err();
    assert_eq!(err.reason, "SIGN_IN_REQUIRED");
    let err = app.call::<_, GetDailyBonusResponse>(&format!("{WALLET}/GetDailyBonus"), None, &GetDailyBonusRequest::default()).await.unwrap_err();
    assert_eq!(err.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn consecutive_days_grow_the_streak_and_the_reward() {
    let (app, _) = app().await;
    let user = app.user("streaker").await;
    seed_streak(&app, &user, "Asia/Tokyo", 1, 6).await;

    let before = status(&app, &user).await;
    assert!(before.claimable);
    assert_eq!((before.streak, before.next_coins, before.next_cycle_day), (6, 50, 7), "yesterday's streak is alive; day 7 pays the most");

    let claimed = claim(&app, &user).await.unwrap();
    assert_eq!(claimed.coins, 50);
    let after = claimed.bonus.as_option().unwrap();
    assert_eq!((after.streak, after.longest_streak, after.total_claims), (7, 7, 7));
    assert_eq!((after.next_coins, after.next_cycle_day), (10, 1), "after the last step the ladder starts over");
}

#[tokio::test]
async fn a_missed_day_resets_the_streak() {
    let (app, _) = app().await;
    let user = app.user("lapsed").await;
    seed_streak(&app, &user, "Asia/Tokyo", 3, 5).await;

    let before = status(&app, &user).await;
    assert!(before.claimable);
    assert_eq!((before.streak, before.next_coins, before.longest_streak), (0, 10, 5));

    let claimed = claim(&app, &user).await.unwrap();
    assert_eq!(claimed.coins, 10);
    let after = claimed.bonus.as_option().unwrap();
    assert_eq!((after.streak, after.longest_streak), (1, 5), "the record survives the reset");
}

#[tokio::test]
async fn the_day_is_the_users_local_day() {
    let (app, _) = app().await;
    // Kiritimati (UTC+14) is always a calendar day ahead of UTC-12.
    let ahead = app.user("kiritimati").await;
    set_time_zone(&app, &ahead, "Pacific/Kiritimati").await;
    seed_streak(&app, &ahead, "Etc/GMT+12", 0, 2).await;
    let bonus = status(&app, &ahead).await;
    assert!(bonus.claimable, "it is already tomorrow where this user lives");
    assert!(claim(&app, &ahead).await.unwrap().coins > 0);

    // Moving west after claiming does not reopen a day that was already paid.
    let behind = app.user("baker").await;
    set_time_zone(&app, &behind, "Etc/GMT+12").await;
    seed_streak(&app, &behind, "Pacific/Kiritimati", 0, 2).await;
    let bonus = status(&app, &behind).await;
    assert!(!bonus.claimable);
    assert_eq!(bonus.streak, 2);
    assert_eq!(claim(&app, &behind).await.unwrap_err().reason, "DAILY_BONUS_ALREADY_CLAIMED");
    assert_eq!(balance(&app, &behind).await.coins, 0);
}

#[tokio::test]
async fn racing_claims_pay_once() {
    let (app, _) = app().await;
    let user = app.user("greedy").await;
    let attempts = (0..6).map(|_| claim(&app, &user));
    let results = futures::future::join_all(attempts).await;
    let (paid, refused): (Vec<_>, Vec<_>) = results.into_iter().partition(Result::is_ok);
    assert_eq!(paid.len(), 1);
    assert!(refused.into_iter().all(|r| r.unwrap_err().reason == "DAILY_BONUS_ALREADY_CLAIMED"));
    assert_eq!(balance(&app, &user).await.coins, 10);
    assert_eq!(status(&app, &user).await.total_claims, 1);
}

#[tokio::test]
async fn the_bonus_can_be_switched_off() {
    let (app, _) = app_with(WalletConfig { daily_bonus_coins: String::new(), ..Default::default() }).await;
    let user = app.user("nobonus").await;
    let bonus = status(&app, &user).await;
    assert!(!bonus.claimable && bonus.cycle.is_empty());
    let err = claim(&app, &user).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("failed_precondition", "DAILY_BONUS_DISABLED"));
}

#[tokio::test]
async fn a_custom_ladder_is_followed() {
    let (app, _) = app_with(WalletConfig { daily_bonus_coins: "3, 7".to_owned(), ..Default::default() }).await;
    let user = app.user("ladder").await;
    seed_streak(&app, &user, "Asia/Tokyo", 1, 1).await;
    let claimed = claim(&app, &user).await.unwrap();
    assert_eq!(claimed.coins, 7);
    assert_eq!(claimed.bonus.as_option().unwrap().cycle, vec![3, 7]);
}
