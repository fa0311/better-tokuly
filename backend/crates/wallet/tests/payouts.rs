//! Creator earnings and payout requests, from request to review to payment.
mod common;

use std::time::Duration;

use common::*;
use tokuly_proto::wallet::*;
use tokuly_testkit::{RpcError, TestApp, TestUser};
use tokuly_wallet::ledger;

const PENDING: Option<PayoutStatus> = Some(PayoutStatus::PAYOUT_STATUS_PENDING);
const APPROVED: Option<PayoutStatus> = Some(PayoutStatus::PAYOUT_STATUS_APPROVED);
const REJECTED: Option<PayoutStatus> = Some(PayoutStatus::PAYOUT_STATUS_REJECTED);
const PAID: Option<PayoutStatus> = Some(PayoutStatus::PAYOUT_STATUS_PAID);
const CANCELLED: Option<PayoutStatus> = Some(PayoutStatus::PAYOUT_STATUS_CANCELLED);

const BANK: &str = "みずほ銀行 渋谷支店 普通 1234567 トクリ タロウ";

fn payout_request(coins: i64) -> RequestPayoutRequest {
    RequestPayoutRequest { coins, method: PayoutMethod::PAYOUT_METHOD_BANK_TRANSFER.into(), destination: BANK.into(), note: "よろしくお願いします!".into(), ..Default::default() }
}

async fn request(app: &TestApp, user: &TestUser, coins: i64) -> Result<RequestPayoutResponse, RpcError> {
    app.call(&format!("{EARNINGS}/RequestPayout"), Some(user), &payout_request(coins)).await
}

async fn open(app: &TestApp, user: &TestUser, coins: i64) -> PayoutRequest {
    request(app, user, coins).await.unwrap().payout.into_option().unwrap()
}

async fn cancel(app: &TestApp, user: &TestUser, payout_id: &str) -> Result<CancelPayoutRequestResponse, RpcError> {
    app.call(&format!("{EARNINGS}/CancelPayoutRequest"), Some(user), &CancelPayoutRequestRequest { payout_id: payout_id.to_owned(), ..Default::default() }).await
}

async fn approve(app: &TestApp, staff: &TestUser, payout_id: &str) -> Result<ApprovePayoutResponse, RpcError> {
    app.call(&format!("{ADMIN}/ApprovePayout"), Some(staff), &ApprovePayoutRequest { payout_id: payout_id.to_owned(), ..Default::default() }).await
}

async fn reject(app: &TestApp, staff: &TestUser, payout_id: &str, note: &str) -> Result<RejectPayoutResponse, RpcError> {
    app.call(&format!("{ADMIN}/RejectPayout"), Some(staff), &RejectPayoutRequest { payout_id: payout_id.to_owned(), review_note: note.to_owned(), ..Default::default() }).await
}

async fn mark_paid(app: &TestApp, staff: &TestUser, payout_id: &str) -> Result<MarkPayoutPaidResponse, RpcError> {
    app.call(&format!("{ADMIN}/MarkPayoutPaid"), Some(staff), &MarkPayoutPaidRequest { payout_id: payout_id.to_owned(), payment_reference: "FB-2026-0042".into(), ..Default::default() }).await
}

async fn summary(app: &TestApp, user: &TestUser, request: &GetEarningsSummaryRequest) -> Result<GetEarningsSummaryResponse, RpcError> {
    app.call(&format!("{EARNINGS}/GetEarningsSummary"), Some(user), request).await
}

/// Net postings on `account` made for one payout request: its hold, the
/// return of that hold and its settlement. (Other tests share the ledger, so
/// the system accounts as a whole say nothing about a single request.)
async fn payout_postings(app: &TestApp, account: &str, payout_id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(sum(p.delta), 0)::bigint
         FROM wallet_postings p
         JOIN wallet_entries e ON e.id = p.entry_id
         LEFT JOIN wallet_entries hold ON hold.id = e.refund_of
         WHERE p.account = $1 AND (e.memo ->> 'payout_id' = $2 OR hold.memo ->> 'payout_id' = $2)",
    )
    .bind(account)
    .bind(payout_id)
    .fetch_one(&app.core.db)
    .await
    .unwrap()
}

/// A creator with `net` coins of earnings (from one gift with a 30% fee).
async fn creator_with_earnings(app: &TestApp, net: i64) -> TestUser {
    let creator = app.user("creator").await;
    earn(app, &creator, net * 10 / 7, "live.gift").await;
    assert_eq!(balance(app, &creator).await.earnings, net);
    creator
}

#[tokio::test]
async fn earnings_summary_breaks_income_down_by_reason_and_day() {
    let (app, _) = app().await;
    let creator = app.user("creator").await;
    earn(&app, &creator, 300, "live.gift").await;
    earn(&app, &creator, 500, "live.gift").await;
    earn(&app, &creator, 100, "stamp.pack").await;
    let (_, returned) = earn(&app, &creator, 1000, "live.gift").await;
    ledger::refund(&app.core, returned, "live.gift.returned").await.unwrap();

    let report = summary(&app, &creator, &GetEarningsSummaryRequest::default()).await.unwrap();
    assert_eq!((report.available, report.pending_payout, report.lifetime_earned, report.lifetime_paid_out), (630, 0, 630, 0));
    let period = report.period.as_option().unwrap();
    assert_eq!((period.gross, period.fee, period.net, period.count), (900, 270, 630, 3), "the refunded gift does not count");

    assert_eq!(report.by_reason.len(), 2);
    let (gifts, stamps) = (&report.by_reason[0], &report.by_reason[1]);
    assert_eq!((gifts.reason.as_str(), gifts.label.as_str()), ("live.gift", "ライブギフト"));
    let totals = gifts.totals.as_option().unwrap();
    assert_eq!((totals.gross, totals.fee, totals.net, totals.count), (800, 240, 560, 2));
    assert_eq!((stamps.reason.as_str(), stamps.label.as_str()), ("stamp.pack", "スタンプパックの売上"));
    assert_eq!(stamps.totals.as_option().unwrap().net, 70);

    // Thirty days by day: a bucket per day (31 when the period starts mid-day), zeros included, everything today.
    assert!((30..=31).contains(&report.series.len()), "{} buckets", report.series.len());
    let (today, earlier) = report.series.split_last().unwrap();
    assert_eq!(today.totals.as_option().unwrap().net, 630);
    assert!(earlier.iter().all(|point| point.totals.as_option().unwrap().count == 0));
    assert!(report.series.is_sorted_by_key(|point| point.period_start.as_option().unwrap().seconds));

    let terms = report.payout_terms.as_option().unwrap();
    assert_eq!((terms.min_coins, terms.currency.as_str(), terms.minor_per_100_coins, terms.platform_fee_percent), (1000, "JPY", 100, 30));

    // Wider buckets, and a period that ended before anything was earned.
    let monthly = summary(&app, &creator, &GetEarningsSummaryRequest { bucket: EarningsBucket::EARNINGS_BUCKET_MONTH.into(), ..Default::default() }).await.unwrap();
    assert!((1..=2).contains(&monthly.series.len()));
    assert_eq!(monthly.series.iter().map(|point| point.totals.as_option().unwrap().net).sum::<i64>(), 630);
    let last_week = chrono::Utc::now() - chrono::Duration::days(7);
    let past = summary(&app, &creator, &GetEarningsSummaryRequest { to: tokuly_core::time::ts(last_week), ..Default::default() }).await.unwrap();
    assert_eq!(past.period.as_option().unwrap().count, 0);
    assert!(past.by_reason.is_empty());
    assert_eq!(past.lifetime_earned, 630, "lifetime numbers ignore the period");
}

#[tokio::test]
async fn earnings_summary_validates_the_period_and_the_caller() {
    let (app, _) = app().await;
    let creator = app.user("creator").await;
    let now = chrono::Utc::now();
    let backwards = GetEarningsSummaryRequest { from: tokuly_core::time::ts(now), to: tokuly_core::time::ts(now - chrono::Duration::days(1)), ..Default::default() };
    assert_eq!(summary(&app, &creator, &backwards).await.unwrap_err().reason, "PERIOD_INVALID");
    let decade = GetEarningsSummaryRequest { from: tokuly_core::time::ts(now - chrono::Duration::days(3650)), ..Default::default() };
    assert_eq!(summary(&app, &creator, &decade).await.unwrap_err().reason, "PERIOD_TOO_LONG");
    let err = app.call::<_, GetEarningsSummaryResponse>(&format!("{EARNINGS}/GetEarningsSummary"), None, &GetEarningsSummaryRequest::default()).await.unwrap_err();
    assert_eq!(err.reason, "SIGN_IN_REQUIRED");

    let fresh = summary(&app, &creator, &GetEarningsSummaryRequest::default()).await.unwrap();
    assert_eq!((fresh.available, fresh.lifetime_earned), (0, 0));
    assert!(fresh.by_reason.is_empty() && !fresh.series.is_empty());
}

#[tokio::test]
async fn a_payout_is_requested_approved_and_paid() {
    let (app, _) = app().await;
    let creator = creator_with_earnings(&app, 2100).await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;

    // Requesting locks the coins out of earnings straight away.
    let response = request(&app, &creator, 1500).await.unwrap();
    let payout = response.payout.into_option().unwrap();
    assert_eq!(payout.status.as_known(), PENDING);
    assert_eq!((payout.coins, payout.payout_minor, payout.currency.as_str()), (1500, 1500, "JPY"));
    assert_eq!((payout.method.as_known(), payout.destination.as_str()), (Some(PayoutMethod::PAYOUT_METHOD_BANK_TRANSFER), BANK));
    assert_eq!(response.balance.as_option().unwrap().earnings, 600);
    assert_eq!(payout_postings(&app, "system:payout_hold", &payout.id).await, 1500);
    let entries = history(&app, &creator).await;
    assert_eq!((entries[0].kind.as_known(), entries[0].earnings_delta, entries[0].description.as_str()), (Some(EntryKind::ENTRY_KIND_PAYOUT), -1500, "出金を申請しました"));
    let report = summary(&app, &creator, &GetEarningsSummaryRequest::default()).await.unwrap();
    assert_eq!((report.available, report.pending_payout, report.lifetime_earned), (600, 1500, 2100));

    // The destination is encrypted at rest.
    let stored: Vec<u8> = sqlx::query_scalar("SELECT destination FROM wallet_payout_requests WHERE id = $1::uuid").bind(&payout.id).fetch_one(&app.core.db).await.unwrap();
    assert!(!String::from_utf8_lossy(&stored).contains("1234567"));

    // Approval tells the creator, in the inbox and on their devices.
    let mut frames = app.listen_user(creator.id).await;
    let approved = approve(&app, &moderator, &payout.id).await.unwrap().payout.into_option().unwrap();
    assert_eq!(approved.status.as_known(), APPROVED);
    assert_eq!(approved.user.as_option().unwrap().id, creator.id.to_string());
    assert_eq!(approved.reviewer.as_option().unwrap().id, moderator.id.to_string());
    assert!(approved.reviewed_at.as_option().is_some());
    let frame = frames.next(Duration::from_secs(2)).await.expect("payout pushed");
    let event: WalletEvent = frame.event().expect("not a wallet frame");
    let Some(wallet_event::Event::PayoutUpdated(pushed)) = event.event else { panic!("not a payout update") };
    assert_eq!((pushed.id.as_str(), pushed.status.as_known()), (payout.id.as_str(), APPROVED));
    assert!(pushed.user.as_option().is_none(), "staff-only fields are not pushed to the requester");
    let notices = notifications(&app, &creator, "wallet.payout").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["title"]["ja"], "出金申請が承認されました!");
    assert_eq!(notices[0]["body"]["en"], "We are preparing your payout of ¥1,500 (1,500 coins).");
    assert_eq!(notices[0]["path"], format!("/wallet/payouts/{}", payout.id));

    // Paying settles the hold for good.
    let paid = mark_paid(&app, &moderator, &payout.id).await.unwrap().payout.into_option().unwrap();
    assert_eq!((paid.status.as_known(), paid.payment_reference.as_str()), (PAID, "FB-2026-0042"));
    assert!(paid.paid_at.as_option().is_some());
    assert_eq!(payout_postings(&app, "system:payout_hold", &payout.id).await, 0);
    assert_eq!(payout_postings(&app, "system:paid_out", &payout.id).await, 1500);
    assert_eq!(postings_sum(&app).await, 0);
    let notices = notifications(&app, &creator, "wallet.payout").await;
    assert_eq!(notices.len(), 2);
    assert_eq!(notices[1]["body"]["ja"], "¥1,500を送金したよ!");

    let report = summary(&app, &creator, &GetEarningsSummaryRequest::default()).await.unwrap();
    assert_eq!((report.available, report.pending_payout, report.lifetime_paid_out), (600, 0, 1500));
    assert_eq!(history(&app, &creator).await.len(), 2, "the settlement is bookkeeping, not a line in the creator's history");

    // A paid request is final.
    assert_eq!(reject(&app, &moderator, &payout.id, "too late").await.unwrap_err().reason, "PAYOUT_NOT_OPEN");
    assert_eq!(mark_paid(&app, &moderator, &payout.id).await.unwrap_err().reason, "PAYOUT_NOT_APPROVED");
    assert_eq!(cancel(&app, &creator, &payout.id).await.unwrap_err().reason, "PAYOUT_NOT_PENDING");
}

#[tokio::test]
async fn rejecting_or_cancelling_returns_the_coins() {
    let (app, _) = app().await;
    let creator = creator_with_earnings(&app, 2100).await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;

    // Cancelled by the creator.
    let first = open(&app, &creator, 2000).await;
    let cancelled = cancel(&app, &creator, &first.id).await.unwrap();
    assert_eq!(cancelled.payout.as_option().unwrap().status.as_known(), CANCELLED);
    assert_eq!(cancelled.balance.as_option().unwrap().earnings, 2100);
    assert_eq!(cancel(&app, &creator, &first.id).await.unwrap_err().reason, "PAYOUT_NOT_PENDING");
    let entries = history(&app, &creator).await;
    assert_eq!((entries[0].kind.as_known(), entries[0].earnings_delta, entries[0].reason.as_str()), (Some(EntryKind::ENTRY_KIND_REFUND), 2000, "payout.cancelled"));
    assert!(entries[1].refunded, "the hold is marked as returned");

    // Rejected by staff: a reason is mandatory and reaches the creator.
    let second = open(&app, &creator, 1000).await;
    assert_eq!(reject(&app, &moderator, &second.id, "  ").await.unwrap_err().reason, "REVIEW_NOTE_REQUIRED");
    assert_eq!(reject(&app, &moderator, &second.id, &"x".repeat(501)).await.unwrap_err().reason, "REVIEW_NOTE_TOO_LONG");
    let rejected = reject(&app, &moderator, &second.id, "口座名義が一致しません").await.unwrap().payout.into_option().unwrap();
    assert_eq!((rejected.status.as_known(), rejected.review_note.as_str()), (REJECTED, "口座名義が一致しません"));
    assert_eq!(balance(&app, &creator).await.earnings, 2100);
    let notices = notifications(&app, &creator, "wallet.payout").await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["body"]["ja"], "1,000コインは売上に戻りました。理由: 口座名義が一致しません");
    assert_eq!(reject(&app, &moderator, &second.id, "again").await.unwrap_err().reason, "PAYOUT_NOT_OPEN");
    assert_eq!(approve(&app, &moderator, &second.id).await.unwrap_err().reason, "PAYOUT_NOT_PENDING");

    // An approved request can still be rejected (the transfer bounced, say).
    let third = open(&app, &creator, 1200).await;
    approve(&app, &moderator, &third.id).await.unwrap();
    assert_eq!(cancel(&app, &creator, &third.id).await.unwrap_err().reason, "PAYOUT_NOT_PENDING", "once approved, only staff can stop it");
    reject(&app, &moderator, &third.id, "振込が戻ってきました").await.unwrap();
    assert_eq!(balance(&app, &creator).await.earnings, 2100);

    for payout in [&first, &second, &third] {
        assert_eq!(payout_postings(&app, "system:payout_hold", &payout.id).await, 0, "nothing is left on hold");
    }
    assert_eq!(postings_sum(&app).await, 0);
    let list: ListPayoutRequestsResponse = app.call(&format!("{EARNINGS}/ListPayoutRequests"), Some(&creator), &ListPayoutRequestsRequest::default()).await.unwrap();
    let statuses: Vec<_> = list.payouts.iter().map(|payout| payout.status.as_known()).collect();
    assert_eq!(statuses, vec![REJECTED, REJECTED, CANCELLED], "newest first");
    assert!(list.payouts.iter().all(|payout| payout.user.as_option().is_none() && payout.destination == BANK));
}

#[tokio::test]
async fn payout_requests_are_validated() {
    let (app, _) = app().await;
    let creator = creator_with_earnings(&app, 2100).await;
    let cases: Vec<(&str, Box<dyn Fn(&mut RequestPayoutRequest)>)> = vec![
        ("AMOUNT_INVALID", Box::new(|r| r.coins = 0)),
        ("AMOUNT_INVALID", Box::new(|r| r.coins = -5)),
        ("PAYOUT_BELOW_MINIMUM", Box::new(|r| r.coins = 999)),
        ("PAYOUT_METHOD_INVALID", Box::new(|r| r.method = PayoutMethod::PAYOUT_METHOD_UNSPECIFIED.into())),
        ("PAYOUT_METHOD_INVALID", Box::new(|r| r.method = 42.into())),
        ("PAYOUT_DESTINATION_INVALID", Box::new(|r| r.destination = "  ".into())),
        ("PAYOUT_DESTINATION_INVALID", Box::new(|r| r.destination = "x".repeat(501))),
        ("PAYOUT_NOTE_TOO_LONG", Box::new(|r| r.note = "x".repeat(501))),
        ("IDEMPOTENCY_KEY_INVALID", Box::new(|r| r.idempotency_key = "k".repeat(201))),
        ("INSUFFICIENT_EARNINGS", Box::new(|r| r.coins = 2101)),
    ];
    for (reason, mutate) in cases {
        let mut candidate = payout_request(1000);
        mutate(&mut candidate);
        let err = app.call::<_, RequestPayoutResponse>(&format!("{EARNINGS}/RequestPayout"), Some(&creator), &candidate).await.unwrap_err();
        assert_eq!(err.reason, reason, "{err}");
    }
    assert_eq!(balance(&app, &creator).await.earnings, 2100, "a refused request locks nothing");

    // Spendable coins are not earnings.
    let spender = app.user("spender").await;
    fund(&app, &spender, 5000).await;
    assert_eq!(request(&app, &spender, 1000).await.unwrap_err().reason, "INSUFFICIENT_EARNINGS");

    let err = app.call::<_, RequestPayoutResponse>(&format!("{EARNINGS}/RequestPayout"), None, &payout_request(1000)).await.unwrap_err();
    assert_eq!(err.reason, "SIGN_IN_REQUIRED");
}

#[tokio::test]
async fn one_open_request_at_a_time_and_retries_are_safe() {
    let (app, _) = app().await;
    let creator = creator_with_earnings(&app, 3500).await;
    let retry_key = key("payout");
    let keyed = RequestPayoutRequest { idempotency_key: retry_key.clone(), ..payout_request(1000) };

    let first: RequestPayoutResponse = app.call(&format!("{EARNINGS}/RequestPayout"), Some(&creator), &keyed).await.unwrap();
    let retried: RequestPayoutResponse = app.call(&format!("{EARNINGS}/RequestPayout"), Some(&creator), &keyed).await.unwrap();
    assert_eq!(first.payout.as_option().unwrap().id, retried.payout.as_option().unwrap().id);
    assert_eq!(balance(&app, &creator).await.earnings, 2500, "the retry locked nothing more");
    // The key names that request: asking for something else with it is refused, not answered with the old one.
    for different in [RequestPayoutRequest { coins: 2000, ..keyed.clone() }, RequestPayoutRequest { destination: "PayPal tokuly@example.com".into(), ..keyed.clone() }] {
        let err = app.call::<_, RequestPayoutResponse>(&format!("{EARNINGS}/RequestPayout"), Some(&creator), &different).await.unwrap_err();
        assert_eq!((err.code.as_str(), err.reason.as_str()), ("already_exists", "IDEMPOTENCY_KEY_REUSED"));
    }

    let err = request(&app, &creator, 1000).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("failed_precondition", "PAYOUT_ALREADY_OPEN"));

    // Racing requests: the account lock lets exactly one through.
    let racer = creator_with_earnings(&app, 3500).await;
    let attempts = (0..4).map(|_| request(&app, &racer, 1000));
    let results = futures::future::join_all(attempts).await;
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(results.iter().filter_map(|r| r.as_ref().err()).all(|e| e.reason == "PAYOUT_ALREADY_OPEN"));
    assert_eq!(balance(&app, &racer).await.earnings, 2500);
}

#[tokio::test]
async fn only_staff_review_and_never_their_own_request() {
    let (app, _) = app().await;
    let creator = creator_with_earnings(&app, 2100).await;
    let payout = open(&app, &creator, 1000).await;
    let (moderator, admin, nobody) = (app.user_with_roles("mod", &["moderator"]).await, app.user_with_roles("boss", &["admin"]).await, app.user("nobody").await);

    // Ordinary users (the requester included) and guests are turned away by every staff RPC.
    for user in [Some(&creator), Some(&nobody), None] {
        let expected = if user.is_some() { ("permission_denied", "FORBIDDEN") } else { ("unauthenticated", "SIGN_IN_REQUIRED") };
        let by_id = GetPayoutRequestRequest { payout_id: payout.id.clone(), ..Default::default() };
        let errors = [
            app.call::<_, ListPayoutQueueResponse>(&format!("{ADMIN}/ListPayoutQueue"), user, &ListPayoutQueueRequest::default()).await.unwrap_err(),
            app.call::<_, GetPayoutRequestResponse>(&format!("{ADMIN}/GetPayoutRequest"), user, &by_id).await.unwrap_err(),
            app.call::<_, ApprovePayoutResponse>(&format!("{ADMIN}/ApprovePayout"), user, &ApprovePayoutRequest { payout_id: payout.id.clone(), ..Default::default() }).await.unwrap_err(),
            app.call::<_, RejectPayoutResponse>(&format!("{ADMIN}/RejectPayout"), user, &RejectPayoutRequest { payout_id: payout.id.clone(), review_note: "no".into(), ..Default::default() }).await.unwrap_err(),
            app.call::<_, MarkPayoutPaidResponse>(&format!("{ADMIN}/MarkPayoutPaid"), user, &MarkPayoutPaidRequest { payout_id: payout.id.clone(), ..Default::default() }).await.unwrap_err(),
        ];
        for err in errors {
            assert_eq!((err.code.as_str(), err.reason.as_str()), expected);
        }
    }

    // Other people's requests do not exist as far as a user can tell.
    assert_eq!(cancel(&app, &nobody, &payout.id).await.unwrap_err().reason, "PAYOUT_NOT_FOUND");
    assert_eq!(cancel(&app, &creator, &uuid::Uuid::now_v7().to_string()).await.unwrap_err().reason, "PAYOUT_NOT_FOUND");
    assert_eq!(cancel(&app, &creator, "nope").await.unwrap_err().reason, "INVALID_ID");

    // Staff cannot wave their own money through.
    let earning_mod = app.user_with_roles("earningmod", &["moderator"]).await;
    earn(&app, &earning_mod, 3000, "live.gift").await;
    let own = open(&app, &earning_mod, 1000).await;
    for err in [
        approve(&app, &earning_mod, &own.id).await.unwrap_err(),
        reject(&app, &earning_mod, &own.id, "no").await.unwrap_err(),
    ] {
        assert_eq!((err.code.as_str(), err.reason.as_str()), ("permission_denied", "SELF_REVIEW"));
    }
    approve(&app, &moderator, &own.id).await.unwrap();
    assert_eq!(mark_paid(&app, &earning_mod, &own.id).await.unwrap_err().reason, "SELF_REVIEW");

    // Admins hold the moderator role; unknown requests are not found; the state machine is enforced.
    assert_eq!(mark_paid(&app, &admin, &payout.id).await.unwrap_err().reason, "PAYOUT_NOT_APPROVED", "paying needs an approval first");
    assert_eq!(approve(&app, &admin, &payout.id).await.unwrap().payout.as_option().unwrap().status.as_known(), APPROVED);
    assert_eq!(approve(&app, &admin, &payout.id).await.unwrap_err().reason, "PAYOUT_NOT_PENDING");
    assert_eq!(approve(&app, &admin, &uuid::Uuid::now_v7().to_string()).await.unwrap_err().reason, "PAYOUT_NOT_FOUND");
    let long_reference = MarkPayoutPaidRequest { payout_id: payout.id.clone(), payment_reference: "r".repeat(201), ..Default::default() };
    let err = app.call::<_, MarkPayoutPaidResponse>(&format!("{ADMIN}/MarkPayoutPaid"), Some(&admin), &long_reference).await.unwrap_err();
    assert_eq!(err.reason, "PAYMENT_REFERENCE_TOO_LONG");
    assert_eq!(mark_paid(&app, &admin, &payout.id).await.unwrap().payout.as_option().unwrap().status.as_known(), PAID);
}

#[tokio::test]
async fn the_review_queue_is_worked_oldest_first() {
    let (app, _) = app().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let (first_creator, second_creator) = (creator_with_earnings(&app, 2100).await, creator_with_earnings(&app, 2100).await);
    let first = open(&app, &first_creator, 1000).await;
    let second = open(&app, &second_creator, 1100).await;

    let queue = |status: PayoutStatus| {
        let app = &app;
        let moderator = &moderator;
        async move {
            let request = ListPayoutQueueRequest { status: status.into(), page: Some(tokuly_proto::common::PageRequest { limit: 100, ..Default::default() }).into(), ..Default::default() };
            app.call::<_, ListPayoutQueueResponse>(&format!("{ADMIN}/ListPayoutQueue"), Some(moderator), &request).await.unwrap()
        }
    };
    // Other tests share the database, so look only at this test's requests.
    let pending = queue(PayoutStatus::PAYOUT_STATUS_UNSPECIFIED).await;
    assert!(pending.pending_count >= 2);
    let mine: Vec<&PayoutRequest> = pending.payouts.iter().filter(|payout| payout.id == first.id || payout.id == second.id).collect();
    assert_eq!(mine.iter().map(|payout| payout.id.as_str()).collect::<Vec<_>>(), vec![first.id.as_str(), second.id.as_str()], "oldest first");
    assert_eq!(mine[0].user.as_option().unwrap().handle, first_creator.handle);
    assert_eq!(mine[0].destination, BANK, "reviewers see where the money goes");
    assert!(pending.payouts.iter().all(|payout| payout.status.as_known() == PENDING));

    approve(&app, &moderator, &first.id).await.unwrap();
    let pending = queue(PayoutStatus::PAYOUT_STATUS_PENDING).await;
    assert!(pending.payouts.iter().all(|payout| payout.id != first.id));
    let approved = queue(PayoutStatus::PAYOUT_STATUS_APPROVED).await;
    let approved_one = approved.payouts.iter().find(|payout| payout.id == first.id).expect("approved request is listed");
    assert_eq!(approved_one.reviewer.as_option().unwrap().id, moderator.id.to_string());

    let detail: GetPayoutRequestResponse = app.call(&format!("{ADMIN}/GetPayoutRequest"), Some(&moderator), &GetPayoutRequestRequest { payout_id: second.id.clone(), ..Default::default() }).await.unwrap();
    let detail = detail.payout.into_option().unwrap();
    assert_eq!((detail.coins, detail.note.as_str()), (1100, "よろしくお願いします!"));
    assert_eq!(detail.user.as_option().unwrap().id, second_creator.id.to_string());
    let missing = GetPayoutRequestRequest { payout_id: uuid::Uuid::now_v7().to_string(), ..Default::default() };
    assert_eq!(app.call::<_, GetPayoutRequestResponse>(&format!("{ADMIN}/GetPayoutRequest"), Some(&moderator), &missing).await.unwrap_err().reason, "PAYOUT_NOT_FOUND");
}

#[tokio::test]
async fn destinations_of_long_closed_requests_are_erased() {
    let (app, wallet) = app().await;
    let creator = creator_with_earnings(&app, 2100).await;
    let old = open(&app, &creator, 1000).await;
    cancel(&app, &creator, &old.id).await.unwrap();
    let recent = open(&app, &creator, 1000).await;
    cancel(&app, &creator, &recent.id).await.unwrap();
    let still_open = open(&app, &creator, 1000).await;
    // `updated_at` is maintained by a trigger; switch it off (inside one
    // transaction) to backdate this test's rows.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "BEGIN;
         ALTER TABLE wallet_payout_requests DISABLE TRIGGER wallet_payout_requests_updated_at;
         UPDATE wallet_payout_requests SET updated_at = now() - interval '91 days' WHERE id IN ('{}', '{}');
         ALTER TABLE wallet_payout_requests ENABLE TRIGGER wallet_payout_requests_updated_at;
         COMMIT;",
        old.id, still_open.id
    )))
    .execute(&app.core.db)
    .await
    .unwrap();

    wallet.run_maintenance(true).await;
    let list: ListPayoutRequestsResponse = app.call(&format!("{EARNINGS}/ListPayoutRequests"), Some(&creator), &ListPayoutRequestsRequest::default()).await.unwrap();
    let destination = |id: &str| list.payouts.iter().find(|payout| payout.id == id).unwrap().destination.clone();
    assert_eq!(destination(&old.id), "", "closed three months ago: the bank details are gone");
    assert_eq!(destination(&recent.id), BANK, "recently closed requests keep them for follow-up questions");
    assert_eq!(destination(&still_open.id), BANK, "open requests always keep them");
}

async fn set_suspended(app: &TestApp, user: &TestUser, suspended: bool) {
    let sql = if suspended { "UPDATE users SET banned_at = now() WHERE id = $1" } else { "UPDATE users SET banned_at = NULL WHERE id = $1" };
    sqlx::query(sql).bind(user.id).execute(&app.core.db).await.unwrap();
}

/// No money goes to a suspended account: its payout request can be
/// neither approved nor paid while the suspension lasts (it can still be
/// rejected), and staff see why in the request itself.
#[tokio::test]
async fn a_suspended_requester_is_neither_approved_nor_paid() {
    let (app, _) = app().await;
    let moderator = app.user_with_roles("mod", &["moderator"]).await;
    let creator = creator_with_earnings(&app, 2100).await;
    let payout = open(&app, &creator, 1000).await;
    let staff_view = |payout_id: String| {
        let (app, moderator) = (&app, &moderator);
        async move {
            let response: GetPayoutRequestResponse = app.call(&format!("{ADMIN}/GetPayoutRequest"), Some(moderator), &GetPayoutRequestRequest { payout_id, ..Default::default() }).await.unwrap();
            response.payout.into_option().unwrap()
        }
    };
    assert!(!staff_view(payout.id.clone()).await.user_suspended);

    set_suspended(&app, &creator, true).await;
    assert!(staff_view(payout.id.clone()).await.user_suspended, "the staff view says why");
    let queue: ListPayoutQueueResponse = app.call(&format!("{ADMIN}/ListPayoutQueue"), Some(&moderator), &ListPayoutQueueRequest::default()).await.unwrap();
    assert!(queue.payouts.iter().find(|p| p.id == payout.id).unwrap().user_suspended);
    let err = approve(&app, &moderator, &payout.id).await.unwrap_err();
    assert_eq!((err.code.as_str(), err.reason.as_str()), ("failed_precondition", "ACCOUNT_SUSPENDED"));

    set_suspended(&app, &creator, false).await;
    approve(&app, &moderator, &payout.id).await.unwrap();
    set_suspended(&app, &creator, true).await;
    assert_eq!(mark_paid(&app, &moderator, &payout.id).await.unwrap_err().reason, "ACCOUNT_SUSPENDED");
    assert_eq!(staff_view(payout.id.clone()).await.status.as_known(), APPROVED, "still waiting, coins still held");
    // Rejecting returns the coins to the (suspended) account's earnings.
    reject(&app, &moderator, &payout.id, "利用停止中のため").await.unwrap();
    assert_eq!(ledger::balance(&app.core, creator.id).await.unwrap().earnings, 2100);
    // The requester's own view does not carry the staff flag.
    set_suspended(&app, &creator, false).await;
    let own: ListPayoutRequestsResponse = app.call(&format!("{EARNINGS}/ListPayoutRequests"), Some(&creator), &ListPayoutRequestsRequest::default()).await.unwrap();
    assert!(own.payouts.iter().all(|p| !p.user_suspended));
}
