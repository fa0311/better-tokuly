//! 割り勘: expenses, balances, the settlement plan and transfers marked as done.
mod common;

use chrono::{Duration, Utc};
use common::*;
use tokuly_proto::{
    common::PageRequest,
    plans::{plans_event::Event as Push, *},
};
use tokuly_testkit::TestUser;
use uuid::Uuid;

/// A plan hosted by the first user with the others invited and going.
async fn plan_with(ctx: &Ctx, users: &[TestUser]) -> Plan {
    let guests: Vec<&TestUser> = users[1..].iter().collect();
    let plan = ctx.create_plan(&users[0], plan_at("打ち上げ", Utc::now() + Duration::days(1), &guests)).await;
    for guest in guests {
        ctx.rsvp(guest, &plan.id, Rsvp::RSVP_GOING).await;
    }
    plan
}

fn shares_for(users: &[&TestUser]) -> Vec<ExpenseShare> {
    users.iter().map(|user| ExpenseShare { user_id: user.id.to_string(), ..Default::default() }).collect()
}

fn equal(title: &str, amount: i64, users: &[&TestUser]) -> ExpenseInput {
    ExpenseInput {
        title: title.to_owned(),
        amount,
        method: SplitMethod::SPLIT_METHOD_EQUAL.into(),
        shares: shares_for(users),
        ..Default::default()
    }
}

async fn add(ctx: &Ctx, user: &TestUser, plan: &Plan, input: ExpenseInput) -> Expense {
    let request = AddExpenseRequest { plan_id: plan.id.clone(), expense: input.into(), ..Default::default() };
    let response: AddExpenseResponse = ctx.call(PLANS, "AddExpense", user, &request).await.expect("add expense");
    response.expense.as_option().cloned().expect("an expense")
}

async fn split(ctx: &Ctx, user: &TestUser, plan: &Plan) -> GetSplitResponse {
    ctx.call(PLANS, "GetSplit", user, &GetSplitRequest { plan_id: plan.id.clone(), ..Default::default() })
        .await
        .expect("get split")
}

fn net_of(split: &CurrencySplit, user: &TestUser) -> i64 {
    split.balances.iter().find(|balance| balance.user_id == user.id.to_string()).map(|balance| balance.net).unwrap_or(0)
}

/// Transfers as `(from, to, amount)`, in a fixed order.
fn transfers(split: &CurrencySplit) -> Vec<(String, String, i64)> {
    let mut transfers: Vec<_> = split
        .transfers
        .iter()
        .map(|transfer| (transfer.from_user_id.clone(), transfer.to_user_id.clone(), transfer.amount))
        .collect();
    transfers.sort();
    transfers
}

fn share_of(expense: &Expense, user: &TestUser) -> i64 {
    expense.shares.iter().find(|share| share.user_id == user.id.to_string()).map(|share| share.amount).expect("a share")
}

#[tokio::test]
async fn who_paid_what_and_who_owes_whom() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let plan = plan_with(&ctx, &users).await;

    // Mika paid dinner for all three; Yui paid the taxi she shared with Sota.
    let mut sotas_phone = ctx.app.listen_user(sota.id).await;
    let dinner = add(&ctx, mika, &plan, equal("焼肉", 3000, &[mika, yui, sota])).await;
    assert_eq!(
        (dinner.amount, dinner.currency.as_str(), dinner.paid_by_user_id.clone()),
        (3000, "JPY", mika.id.to_string())
    );
    assert_eq!(dinner.shares.iter().map(|share| share.amount).collect::<Vec<_>>(), [1000, 1000, 1000]);
    assert!(
        matches!(next_push(&mut sotas_phone).await, Some(Push::ExpensesChanged(change)) if change.expense.as_option().is_some_and(|expense| expense.id == dinner.id))
    );
    let taxi = add(&ctx, yui, &plan, equal("タクシー", 1200, &[yui, sota])).await;

    let listed: ListExpensesResponse = ctx
        .call(PLANS, "ListExpenses", sota, &ListExpensesRequest { plan_id: plan.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(
        listed.expenses.iter().map(|expense| expense.id.clone()).collect::<Vec<_>>(),
        [taxi.id.clone(), dinner.id.clone()]
    );
    assert_eq!(listed.expenses[0].shares.len(), 2);
    let first_page: ListExpensesResponse = ctx
        .call(
            PLANS,
            "ListExpenses",
            sota,
            &ListExpensesRequest {
                plan_id: plan.id.clone(),
                page: PageRequest { limit: 1, ..Default::default() }.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(first_page.page.as_option().unwrap().has_more && first_page.expenses.len() == 1);
    assert_eq!(ctx.get_plan(mika, &plan.id).await.expense_count, 2);

    // Mika is owed 2000; Yui owes 400 (paid 1200, owes 1600); Sota owes 1600.
    let before = split(&ctx, sota, &plan).await;
    assert_eq!(before.splits.len(), 1);
    let yen = &before.splits[0];
    assert_eq!((yen.currency.as_str(), yen.total), ("JPY", 4200));
    assert_eq!((net_of(yen, mika), net_of(yen, yui), net_of(yen, sota)), (2000, -400, -1600));
    assert_eq!(yen.balances.iter().map(|balance| balance.net).sum::<i64>(), 0);
    let yuis = yen.balances.iter().find(|balance| balance.user_id == yui.id.to_string()).unwrap();
    assert_eq!((yuis.paid, yuis.owed, yuis.sent, yuis.received), (1200, 1600, 0, 0));
    // Two transfers settle everyone.
    let mut expected =
        vec![(yui.id.to_string(), mika.id.to_string(), 400), (sota.id.to_string(), mika.id.to_string(), 1600)];
    expected.sort();
    assert_eq!(transfers(yen), expected);
    assert!(before.settlements.is_empty());

    // Sota hands Mika the money. Mika, who received it, marks it: saying
    // "I paid" settles nothing, neither for Sota nor for anyone else.
    let settle = |from: &TestUser, to: &TestUser, amount: i64| SettleTransferRequest {
        plan_id: plan.id.clone(),
        from_user_id: from.id.to_string(),
        to_user_id: to.id.to_string(),
        amount,
        ..Default::default()
    };
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", sota, &settle(sota, mika, 1600)).await, "NOT_ALLOWED");
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", yui, &settle(sota, mika, 1600)).await, "NOT_ALLOWED");
    // Nor can more be marked than is owed, or a debt be invented.
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", mika, &settle(sota, mika, 1601)).await, "AMOUNT_EXCEEDS_BALANCE");
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", sota, &settle(mika, sota, 500)).await, "AMOUNT_EXCEEDS_BALANCE");
    let done: SettleTransferResponse =
        ctx.call(PLANS, "SettleTransfer", mika, &settle(sota, mika, 1600)).await.unwrap();
    let done = done.settlement.as_option().cloned().unwrap();
    assert_eq!(
        (done.amount, done.currency.as_str(), done.recorded_by_user_id.clone()),
        (1600, "JPY", mika.id.to_string())
    );
    let told = ctx.notifications_for(sota.id, "plans.settled").await;
    assert_eq!((told.len(), told[0].actor_id), (1, Some(mika.id)));
    assert_eq!(
        told[0].data["body"]["ja"],
        serde_json::json!("Mikaさんが1,600円を受け取ったと記録したよ(「打ち上げ」)")
    );
    assert!(ctx.notifications_for(mika.id, "plans.settled").await.is_empty());
    // Once: the debt is gone.
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", mika, &settle(sota, mika, 1600)).await, "AMOUNT_EXCEEDS_BALANCE");

    let after = split(&ctx, mika, &plan).await;
    let yen = &after.splits[0];
    assert_eq!((net_of(yen, mika), net_of(yen, yui), net_of(yen, sota)), (400, -400, 0));
    assert_eq!(transfers(yen), [(yui.id.to_string(), mika.id.to_string(), 400)]);
    assert_eq!(after.settlements.iter().map(|settlement| settlement.id.clone()).collect::<Vec<_>>(), [done.id.clone()]);
    let sotas = yen.balances.iter().find(|balance| balance.user_id == sota.id.to_string()).unwrap();
    assert_eq!((sotas.sent, sotas.received), (1600, 0));

    // What is not a transfer.
    let outsider = ctx.app.user("Outsider").await;
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", yui, &settle(yui, yui, 400)).await, "SAME_USER");
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", mika, &settle(&outsider, mika, 400)).await, "USER_NOT_IN_PLAN");
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", mika, &settle(yui, mika, 0)).await, "AMOUNT_INVALID");
    assert_eq!(
        ctx.refusal(
            PLANS,
            "SettleTransfer",
            mika,
            &SettleTransferRequest { currency: "円".into(), ..settle(yui, mika, 400) }
        )
        .await,
        "CURRENCY_INVALID"
    );
    assert_eq!(ctx.refusal(PLANS, "SettleTransfer", &outsider, &settle(yui, mika, 400)).await, "PLAN_NOT_FOUND");
    assert_eq!(
        ctx.refusal(PLANS, "GetSplit", &outsider, &GetSplitRequest { plan_id: plan.id.clone(), ..Default::default() })
            .await,
        "PLAN_NOT_FOUND"
    );
    assert_eq!(
        ctx.refusal(
            PLANS,
            "ListExpenses",
            &outsider,
            &ListExpensesRequest { plan_id: plan.id.clone(), ..Default::default() }
        )
        .await,
        "PLAN_NOT_FOUND"
    );

    // Mika can take back what she marked; nobody else can, and it leaves a trace.
    let take_back = RemoveSettlementRequest { settlement_id: done.id.clone(), ..Default::default() };
    assert_eq!(ctx.refusal(PLANS, "RemoveSettlement", yui, &take_back).await, "NOT_ALLOWED");
    assert_eq!(ctx.refusal(PLANS, "RemoveSettlement", sota, &take_back).await, "NOT_ALLOWED");
    assert_eq!(ctx.refusal(PLANS, "RemoveSettlement", &outsider, &take_back).await, "SETTLEMENT_NOT_FOUND");
    assert_eq!(
        ctx.refusal(
            PLANS,
            "RemoveSettlement",
            mika,
            &RemoveSettlementRequest { settlement_id: Uuid::now_v7().to_string(), ..Default::default() }
        )
        .await,
        "SETTLEMENT_NOT_FOUND"
    );
    let _: RemoveSettlementResponse = ctx.call(PLANS, "RemoveSettlement", mika, &take_back).await.unwrap();
    let restored = split(&ctx, mika, &plan).await;
    assert_eq!(net_of(&restored.splits[0], sota), -1600);
    assert!(restored.settlements.is_empty());
    // The record stays for everyone in the plan: who took it back, and when.
    for viewer in [mika, yui, sota] {
        let seen = split(&ctx, viewer, &plan).await;
        assert_eq!(seen.removed_settlements.len(), 1);
        let gone = &seen.removed_settlements[0];
        assert_eq!(
            (gone.id.as_str(), gone.amount, gone.removed_by_user_id.as_str(), gone.removed_at.is_set()),
            (done.id.as_str(), 1600, mika.id.to_string().as_str(), true)
        );
    }
    // Sota, whose debt is open again, is told.
    let told = ctx.notifications_for(sota.id, "plans.settlement_removed").await;
    assert_eq!((told.len(), told[0].actor_id), (1, Some(mika.id)));
    assert_eq!(
        told[0].data["body"]["ja"],
        serde_json::json!("Mikaさんが1,600円の受け取りを取り消したよ(「打ち上げ」)")
    );
    // Taken back once.
    assert_eq!(ctx.refusal(PLANS, "RemoveSettlement", mika, &take_back).await, "SETTLEMENT_NOT_FOUND");
}

#[tokio::test]
async fn an_expense_is_divided_to_the_last_yen() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Sota"]).await;
    let (mika, yui, sota) = (&users[0], &users[1], &users[2]);
    let plan = plan_with(&ctx, &users).await;

    // 1000 yen by three: the yen that cannot be divided goes to the smallest user ID.
    let pizza = add(&ctx, mika, &plan, equal("ピザ", 1000, &[mika, yui, sota])).await;
    let first = [mika, yui, sota].into_iter().min_by_key(|user| user.id).unwrap();
    for user in [mika, yui, sota] {
        assert_eq!(share_of(&pizza, user), if user.id == first.id { 334 } else { 333 });
    }
    assert_eq!(pizza.shares.iter().map(|share| share.amount).sum::<i64>(), 1000);

    // By weight, 2 : 1 — the larger fraction gets the leftover yen.
    let weighted = ExpenseInput {
        method: SplitMethod::SPLIT_METHOD_SHARES.into(),
        shares: vec![
            ExpenseShare { user_id: yui.id.to_string(), weight: 2, ..Default::default() },
            ExpenseShare { user_id: sota.id.to_string(), weight: 1, ..Default::default() },
        ],
        ..equal("ドリンク", 1000, &[])
    };
    let drinks = add(&ctx, mika, &plan, weighted).await;
    assert_eq!((share_of(&drinks, yui), share_of(&drinks, sota)), (667, 333));
    assert_eq!(drinks.method, SplitMethod::SPLIT_METHOD_SHARES);
    assert_eq!(drinks.shares.iter().map(|share| share.weight).sum::<i32>(), 3);

    // Exact amounts, paid by someone other than the person writing it down.
    let exact = ExpenseInput {
        method: SplitMethod::SPLIT_METHOD_EXACT.into(),
        paid_by_user_id: yui.id.to_string(),
        shares: vec![
            ExpenseShare { user_id: mika.id.to_string(), amount: 700, ..Default::default() },
            ExpenseShare { user_id: sota.id.to_string(), amount: 300, ..Default::default() },
        ],
        ..equal("プレゼント", 1000, &[])
    };
    let present = add(&ctx, mika, &plan, exact.clone()).await;
    assert_eq!(
        (present.paid_by_user_id.clone(), present.created_by_user_id.clone()),
        (yui.id.to_string(), mika.id.to_string())
    );
    assert_eq!((share_of(&present, mika), share_of(&present, sota)), (700, 300));

    // Whatever the remainders, the balances add up to zero and the plan clears them.
    let state = split(&ctx, mika, &plan).await;
    let yen = &state.splits[0];
    assert_eq!(yen.total, 3000);
    assert_eq!(yen.balances.iter().map(|balance| balance.net).sum::<i64>(), 0);
    let mut left: std::collections::HashMap<String, i64> =
        yen.balances.iter().map(|balance| (balance.user_id.clone(), balance.net)).collect();
    for transfer in &yen.transfers {
        *left.get_mut(&transfer.from_user_id).unwrap() += transfer.amount;
        *left.get_mut(&transfer.to_user_id).unwrap() -= transfer.amount;
    }
    assert!(left.values().all(|net| *net == 0), "the transfers leave balances open: {left:?}");

    // Changing an expense: whoever wrote it down, the payer, or a host.
    let change = |expense: &Expense, input: ExpenseInput| UpdateExpenseRequest {
        expense_id: expense.id.clone(),
        expense: input.into(),
        ..Default::default()
    };
    let cheaper = ExpenseInput {
        amount: 900,
        shares: vec![
            ExpenseShare { user_id: mika.id.to_string(), amount: 600, ..Default::default() },
            ExpenseShare { user_id: sota.id.to_string(), amount: 300, ..Default::default() },
        ],
        ..exact
    };
    assert_eq!(ctx.refusal(PLANS, "UpdateExpense", sota, &change(&present, cheaper.clone())).await, "NOT_ALLOWED");
    let updated: UpdateExpenseResponse =
        ctx.call(PLANS, "UpdateExpense", yui, &change(&present, cheaper)).await.unwrap();
    let updated = updated.expense.as_option().unwrap();
    assert_eq!((updated.amount, share_of(updated, mika)), (900, 600));
    let outsider = ctx.app.user("Outsider").await;
    assert_eq!(
        ctx.refusal(PLANS, "UpdateExpense", &outsider, &change(&present, equal("x", 1, &[mika]))).await,
        "EXPENSE_NOT_FOUND"
    );
    // An update is validated like a new expense.
    assert_eq!(
        ctx.refusal(PLANS, "UpdateExpense", mika, &change(&pizza, equal("ピザ", 0, &[mika]))).await,
        "AMOUNT_INVALID"
    );

    // Deleting follows the same rule.
    let delete = |expense: &Expense| RemoveExpenseRequest { expense_id: expense.id.clone(), ..Default::default() };
    assert_eq!(ctx.refusal(PLANS, "RemoveExpense", sota, &delete(&pizza)).await, "NOT_ALLOWED");
    assert_eq!(ctx.refusal(PLANS, "RemoveExpense", &outsider, &delete(&pizza)).await, "EXPENSE_NOT_FOUND");
    let mut yuis_phone = ctx.app.listen_user(yui.id).await;
    let _: RemoveExpenseResponse = ctx.call(PLANS, "RemoveExpense", mika, &delete(&pizza)).await.unwrap();
    assert!(
        matches!(next_push(&mut yuis_phone).await, Some(Push::ExpensesChanged(change)) if change.removed_expense_id == pizza.id)
    );
    assert_eq!(ctx.refusal(PLANS, "RemoveExpense", mika, &delete(&pizza)).await, "EXPENSE_NOT_FOUND");
    assert_eq!(split(&ctx, mika, &plan).await.splits[0].total, 1900);
}

#[tokio::test]
async fn expenses_that_do_not_add_up_are_rejected() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui", "Hana"]).await;
    let (mika, yui, hana) = (&users[0], &users[1], &users[2]);
    let outsider = ctx.app.user("Outsider").await;
    // Hana is invited and has not answered.
    let plan = ctx.create_plan(mika, plan_at("たこパ", Utc::now() + Duration::days(1), &[yui, hana])).await;
    ctx.rsvp(yui, &plan.id, Rsvp::RSVP_GOING).await;
    let request = |input: ExpenseInput| AddExpenseRequest {
        plan_id: plan.id.clone(),
        expense: input.into(),
        ..Default::default()
    };
    let share = |user: &TestUser, amount: i64, weight: i32| ExpenseShare {
        user_id: user.id.to_string(),
        amount,
        weight,
        ..Default::default()
    };
    let both = || equal("たこ", 1000, &[mika, yui]);

    let cases = [
        (ExpenseInput { title: " ".into(), ..both() }, "TITLE_REQUIRED"),
        (ExpenseInput { title: "あ".repeat(81), ..both() }, "TITLE_TOO_LONG"),
        (ExpenseInput { amount: 0, ..both() }, "AMOUNT_INVALID"),
        (ExpenseInput { amount: -500, ..both() }, "AMOUNT_INVALID"),
        (ExpenseInput { currency: "jpy".into(), ..both() }, "CURRENCY_INVALID"),
        (ExpenseInput { paid_by_user_id: outsider.id.to_string(), ..both() }, "PAYER_NOT_IN_PLAN"),
        (ExpenseInput { method: SplitMethod::SPLIT_METHOD_UNSPECIFIED.into(), ..both() }, "SPLIT_METHOD_REQUIRED"),
        (ExpenseInput { shares: Vec::new(), ..both() }, "SHARES_REQUIRED"),
        (ExpenseInput { shares: shares_for(&[mika, &outsider]), ..both() }, "SHARE_NOT_IN_PLAN"),
        (ExpenseInput { shares: shares_for(&[mika, mika]), ..both() }, "SHARE_DUPLICATE"),
        (
            ExpenseInput {
                method: SplitMethod::SPLIT_METHOD_EXACT.into(),
                shares: vec![share(mika, 600, 0), share(yui, 300, 0)],
                ..both()
            },
            "SHARES_INVALID",
        ),
        (
            ExpenseInput {
                method: SplitMethod::SPLIT_METHOD_SHARES.into(),
                shares: vec![share(mika, 0, 0), share(yui, 0, 2)],
                ..both()
            },
            "SHARES_INVALID",
        ),
        (ExpenseInput::default(), "TITLE_REQUIRED"),
    ];
    for (input, reason) in cases {
        assert_eq!(ctx.refusal(PLANS, "AddExpense", mika, &request(input)).await, reason);
    }
    // Recording expenses is for the people who are going.
    assert_eq!(ctx.refusal(PLANS, "AddExpense", hana, &request(both())).await, "NOT_GOING");
    assert_eq!(ctx.refusal(PLANS, "AddExpense", &outsider, &request(both())).await, "PLAN_NOT_FOUND");
    assert!(split(&ctx, mika, &plan).await.splits.is_empty());

    // Someone who has not answered is not charged: nobody can put a debt
    // on a person who never said they would come.
    assert_eq!(
        ctx.refusal(PLANS, "AddExpense", mika, &request(equal("たこ", 900, &[mika, yui, hana]))).await,
        "SHARE_NOT_GOING"
    );
    let hana_paid = ExpenseInput { paid_by_user_id: hana.id.to_string(), ..equal("たこ", 900, &[mika, yui]) };
    assert_eq!(ctx.refusal(PLANS, "AddExpense", mika, &request(hana_paid)).await, "PAYER_NOT_GOING");
    // Once she says she is going she can be given a share, and from then on
    // she is part of the split: she cannot leave or be removed.
    ctx.rsvp(hana, &plan.id, Rsvp::RSVP_GOING).await;
    add(&ctx, mika, &plan, equal("たこ", 900, &[mika, yui, hana])).await;
    assert_eq!(
        ctx.refusal(PLANS, "LeavePlan", hana, &LeavePlanRequest { plan_id: plan.id.clone(), ..Default::default() })
            .await,
        "HAS_EXPENSES"
    );
    let remove =
        RemovePlanParticipantRequest { plan_id: plan.id.clone(), user_id: hana.id.to_string(), ..Default::default() };
    assert_eq!(ctx.refusal(PLANS, "RemovePlanParticipant", mika, &remove).await, "HAS_EXPENSES");
    // Declining is still possible.
    assert_eq!(ctx.rsvp(hana, &plan.id, Rsvp::RSVP_DECLINED).await, Rsvp::RSVP_DECLINED);
}

#[tokio::test]
async fn currencies_do_not_mix() {
    let ctx = start().await;
    let users = ctx.group(&["Mika", "Yui"]).await;
    let (mika, yui) = (&users[0], &users[1]);
    let plan = plan_with(&ctx, &users).await;

    add(&ctx, mika, &plan, equal("ランチ", 3000, &[mika, yui])).await;
    add(&ctx, yui, &plan, ExpenseInput { currency: "USD".into(), ..equal("おみやげ", 2501, &[mika, yui]) }).await;

    let state = split(&ctx, mika, &plan).await;
    // One split per currency, each balanced on its own.
    assert_eq!(state.splits.iter().map(|split| split.currency.as_str()).collect::<Vec<_>>(), ["JPY", "USD"]);
    let (yen, dollars) = (&state.splits[0], &state.splits[1]);
    assert_eq!((yen.total, net_of(yen, mika), net_of(yen, yui)), (3000, 1500, -1500));
    assert_eq!(transfers(yen), [(yui.id.to_string(), mika.id.to_string(), 1500)]);
    assert_eq!(dollars.total, 2501);
    assert_eq!(net_of(dollars, mika) + net_of(dollars, yui), 0);
    assert_eq!(dollars.transfers.len(), 1);
    assert_eq!(dollars.transfers[0].from_user_id, mika.id.to_string());

    // A transfer settles the currency it is made in.
    let owed = dollars.transfers[0].amount;
    let request = SettleTransferRequest {
        plan_id: plan.id.clone(),
        from_user_id: mika.id.to_string(),
        to_user_id: yui.id.to_string(),
        amount: owed,
        currency: "USD".into(),
        ..Default::default()
    };
    // The receiver records it.
    let _: SettleTransferResponse = ctx.call(PLANS, "SettleTransfer", yui, &request).await.unwrap();
    let state = split(&ctx, yui, &plan).await;
    assert!(state.splits[1].transfers.is_empty());
    assert_eq!(state.splits[0].transfers.len(), 1);
    let told = ctx.notifications_for(mika.id, "plans.settled").await;
    assert_eq!(told.len(), 1);
    assert!(told[0].data["body"]["en"].as_str().unwrap().contains("USD 12."));
}
