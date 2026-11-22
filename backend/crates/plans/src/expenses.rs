//! 割り勘 inside a plan: recording who paid what, where the split stands, and
//! transfers marked as done.
//!
//! This module validates and stores; the arithmetic — dividing an expense,
//! balances, the settlement plan — is in [`crate::split`]. Shares are
//! computed once, when an expense is written, and stored, so the numbers
//! people saw never shift under them. No real money moves and coins are not
//! involved.
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Duration,
};

use buffa::Enumeration as _;
use sqlx::{PgConnection, PgExecutor};
use tokuly_core::{
    outbound::{self, NotificationRequest},
    prelude::*,
};
use tokuly_proto::{
    common::{PageInfo, SubjectType},
    plans::{plans_event::Event as Push, *},
};
use tokuly_social::graph;

use crate::{
    Plans, copy, input, people,
    plan::{Access, load_access, participant_ids, plan_path},
    split::{self, Paid},
};

const MAX_TITLE: usize = 80;
/// Largest single amount, in minor units. Keeps every sum far from overflow.
const MAX_AMOUNT: i64 = 1_000_000_000_000;
const MAX_WEIGHT: i32 = 1000;
const MAX_EXPENSES: i64 = 500;
/// Transfers one account may record or take back in an hour.
const SETTLEMENTS_PER_HOUR: u32 = 60;

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExpenseRow {
    id: Uuid,
    plan_id: Uuid,
    title: String,
    amount: i64,
    currency: String,
    paid_by: Uuid,
    method: i16,
    created_by: Uuid,
    created_at: Time,
    updated_at: Time,
}

macro_rules! expense_columns {
    () => {
        "id, plan_id, title, amount, currency, paid_by, method, created_by, created_at, updated_at"
    };
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ShareRow {
    expense_id: Uuid,
    user_id: Uuid,
    amount: i64,
    weight: i32,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct SettlementRow {
    id: Uuid,
    plan_id: Uuid,
    from_user_id: Uuid,
    to_user_id: Uuid,
    amount: i64,
    currency: String,
    recorded_by: Uuid,
    created_at: Time,
    /// Set when the transfer was taken back: the row stays as a record and
    /// no longer counts.
    removed_at: Option<Time>,
    removed_by: Option<Uuid>,
}

macro_rules! settlement_columns {
    () => {
        "id, plan_id, from_user_id, to_user_id, amount, currency, recorded_by, created_at, removed_at, removed_by"
    };
}

impl ExpenseRow {
    fn to_proto(&self, shares: &[ShareRow]) -> Expense {
        Expense {
            id: self.id.to_string(),
            plan_id: self.plan_id.to_string(),
            title: self.title.clone(),
            amount: self.amount,
            currency: self.currency.clone(),
            paid_by_user_id: self.paid_by.to_string(),
            method: SplitMethod::from_i32(self.method as i32).unwrap_or(SplitMethod::SPLIT_METHOD_UNSPECIFIED).into(),
            shares: shares
                .iter()
                .map(|share| ExpenseShare {
                    user_id: share.user_id.to_string(),
                    amount: share.amount,
                    weight: share.weight,
                    ..Default::default()
                })
                .collect(),
            created_by_user_id: self.created_by.to_string(),
            created_at: ts(self.created_at),
            updated_at: ts(self.updated_at),
            ..Default::default()
        }
    }
}

impl SettlementRow {
    fn to_proto(&self) -> Settlement {
        Settlement {
            id: self.id.to_string(),
            plan_id: self.plan_id.to_string(),
            from_user_id: self.from_user_id.to_string(),
            to_user_id: self.to_user_id.to_string(),
            amount: self.amount,
            currency: self.currency.clone(),
            recorded_by_user_id: self.recorded_by.to_string(),
            created_at: ts(self.created_at),
            removed_at: self.removed_at.map(ts).unwrap_or_default(),
            removed_by_user_id: self.removed_by.map(|by| by.to_string()).unwrap_or_default(),
            ..Default::default()
        }
    }
}

/// An expense that passed validation, with its shares worked out.
struct CheckedExpense {
    title: String,
    amount: i64,
    currency: String,
    paid_by: Uuid,
    method: SplitMethod,
    /// `(person, what they owe, weight)`; the amounts add up to `amount`.
    shares: Vec<(Uuid, i64, i32)>,
}

fn amount(value: i64) -> AppResult<i64> {
    ensure!(
        (1..=MAX_AMOUNT).contains(&value),
        invalid,
        "AMOUNT_INVALID",
        "the amount must be positive (in the currency's minor unit)"
    );
    Ok(value)
}

/// Validates an expense as written by a client and divides it.
/// `members` are the people in the plan; `chargeable` those of them an
/// expense may name as payer or give a share: the people who are going.
/// Nobody is charged for a plan they declined or never answered.
fn check_expense(
    input: &ExpenseInput,
    caller: Uuid,
    plan_currency: &str,
    members: &HashSet<Uuid>,
    chargeable: &HashSet<Uuid>,
) -> AppResult<CheckedExpense> {
    let title = input::required_line(&input.title, MAX_TITLE, "TITLE_REQUIRED", "TITLE_TOO_LONG")?;
    let total = amount(input.amount)?;
    let currency = input::currency(&input.currency, plan_currency)?;
    let paid_by = parse_opt_id(&input.paid_by_user_id)?.unwrap_or(caller);
    ensure!(members.contains(&paid_by), invalid, "PAYER_NOT_IN_PLAN", "the payer is not in the plan");
    ensure!(chargeable.contains(&paid_by), invalid, "PAYER_NOT_GOING", "the payer is not going to the plan");
    let method = match input.method.as_known() {
        Some(method) if method != SplitMethod::SPLIT_METHOD_UNSPECIFIED => method,
        _ => bail!(invalid, "SPLIT_METHOD_REQUIRED", "say how the expense is split"),
    };

    ensure!(!input.shares.is_empty(), invalid, "SHARES_REQUIRED", "say who the expense was for");
    let mut seen = HashSet::new();
    let mut given: Vec<(Uuid, i64, i32)> = Vec::with_capacity(input.shares.len());
    for share in &input.shares {
        let person = parse_id(&share.user_id)?;
        ensure!(
            members.contains(&person),
            invalid,
            "SHARE_NOT_IN_PLAN",
            "someone the expense is split with is not in the plan"
        );
        ensure!(
            chargeable.contains(&person),
            invalid,
            "SHARE_NOT_GOING",
            "an expense is only split between people who are going"
        );
        ensure!(seen.insert(person), invalid, "SHARE_DUPLICATE", "everyone can only have one share of an expense");
        given.push((person, share.amount, share.weight));
    }

    let shares = match method {
        SplitMethod::SPLIT_METHOD_EXACT => {
            ensure!(
                given.iter().all(|(_, owed, _)| (0..=MAX_AMOUNT).contains(owed)),
                invalid,
                "SHARES_INVALID",
                "a share cannot be negative"
            );
            ensure!(
                given.iter().map(|(_, owed, _)| *owed).sum::<i64>() == total,
                invalid,
                "SHARES_INVALID",
                "the shares must add up to the amount"
            );
            given.into_iter().map(|(person, owed, _)| (person, owed, 0)).collect()
        }
        SplitMethod::SPLIT_METHOD_SHARES => {
            ensure!(
                given.iter().all(|(_, _, weight)| (1..=MAX_WEIGHT).contains(weight)),
                invalid,
                "SHARES_INVALID",
                "weights are whole numbers from 1 to {MAX_WEIGHT}"
            );
            let weights: Vec<(Uuid, i64)> = given.iter().map(|(person, _, weight)| (*person, *weight as i64)).collect();
            split::split_by_weight(total, &weights)
                .into_iter()
                .zip(&given)
                .map(|((person, owed), (_, _, weight))| (person, owed, *weight))
                .collect()
        }
        _ => {
            let people: Vec<Uuid> = given.iter().map(|(person, _, _)| *person).collect();
            split::split_equal(total, &people).into_iter().map(|(person, owed)| (person, owed, 0)).collect()
        }
    };
    Ok(CheckedExpense { title, amount: total, currency, paid_by, method, shares })
}

async fn insert_shares(
    conn: &mut PgConnection,
    expense_id: Uuid,
    shares: &[(Uuid, i64, i32)],
) -> AppResult<Vec<ShareRow>> {
    let (people, amounts, weights): (Vec<Uuid>, Vec<i64>, Vec<i32>) =
        shares.iter().map(|(person, owed, weight)| (*person, *owed, *weight)).collect();
    Ok(sqlx::query_as(
        "INSERT INTO plan_expense_shares (expense_id, user_id, amount, weight)
         SELECT $1, person, owed, weight FROM unnest($2::uuid[], $3::int8[], $4::int4[]) AS share(person, owed, weight)
         RETURNING expense_id, user_id, amount, weight",
    )
    .bind(expense_id)
    .bind(&people)
    .bind(&amounts)
    .bind(&weights)
    .fetch_all(conn)
    .await?)
}

/// The shares of several expenses, each in ascending order of user ID.
async fn shares_of<'e, E: PgExecutor<'e>>(
    executor: E,
    expense_ids: &[Uuid],
) -> AppResult<HashMap<Uuid, Vec<ShareRow>>> {
    let rows: Vec<ShareRow> = sqlx::query_as("SELECT expense_id, user_id, amount, weight FROM plan_expense_shares WHERE expense_id = ANY($1) ORDER BY expense_id, user_id")
        .bind(expense_ids)
        .fetch_all(executor)
        .await?;
    let mut by_expense: HashMap<Uuid, Vec<ShareRow>> = HashMap::new();
    for row in rows {
        by_expense.entry(row.expense_id).or_default().push(row);
    }
    Ok(by_expense)
}

/// Everything the split of a plan is computed from. Currencies do not mix:
/// each one is a split of its own.
struct Ledger {
    /// The expenses of each currency (a currency that only has recorded
    /// transfers left is listed with none).
    paid: BTreeMap<String, Vec<Paid>>,
    /// The transfers marked as received, by currency.
    made: BTreeMap<String, Vec<split::Transfer>>,
    /// The same transfers as stored, newest first.
    settlements: Vec<SettlementRow>,
    /// Transfers that were taken back, most recently removed first. They
    /// count for nothing; they are the record of what happened.
    removed: Vec<SettlementRow>,
}

impl Ledger {
    /// Reads the expenses and recorded transfers of a plan on one connection
    /// (the caller's transaction, when the result decides a write).
    async fn load(conn: &mut PgConnection, plan_id: Uuid) -> AppResult<Self> {
        let expenses: Vec<ExpenseRow> =
            sqlx::query_as(concat!("SELECT ", expense_columns!(), " FROM plan_expenses WHERE plan_id = $1"))
                .bind(plan_id)
                .fetch_all(&mut *conn)
                .await?;
        let ids: Vec<Uuid> = expenses.iter().map(|row| row.id).collect();
        let mut shares = shares_of(&mut *conn, &ids).await?;
        let recorded: Vec<SettlementRow> = sqlx::query_as(concat!(
            "SELECT ",
            settlement_columns!(),
            " FROM plan_settlements WHERE plan_id = $1 ORDER BY id DESC"
        ))
        .bind(plan_id)
        .fetch_all(&mut *conn)
        .await?;
        let (mut removed, settlements): (Vec<SettlementRow>, Vec<SettlementRow>) =
            recorded.into_iter().partition(|row| row.removed_at.is_some());
        removed.sort_by_key(|row| std::cmp::Reverse(row.removed_at));

        let mut paid: BTreeMap<String, Vec<Paid>> = BTreeMap::new();
        for row in &expenses {
            let owed = shares
                .remove(&row.id)
                .unwrap_or_default()
                .into_iter()
                .map(|share| (share.user_id, share.amount))
                .collect();
            paid.entry(row.currency.clone()).or_default().push(Paid {
                payer: row.paid_by,
                amount: row.amount,
                shares: owed,
            });
        }
        let mut made: BTreeMap<String, Vec<split::Transfer>> = BTreeMap::new();
        for row in &settlements {
            made.entry(row.currency.clone()).or_default().push(split::Transfer {
                from: row.from_user_id,
                to: row.to_user_id,
                amount: row.amount,
            });
            paid.entry(row.currency.clone()).or_default();
        }
        Ok(Self { paid, made, settlements, removed })
    }

    /// Everyone's position in one currency.
    fn positions(&self, currency: &str) -> BTreeMap<Uuid, split::Position> {
        split::positions(
            self.paid.get(currency).map(Vec::as_slice).unwrap_or_default(),
            self.made.get(currency).map(Vec::as_slice).unwrap_or_default(),
        )
    }
}

impl Plans {
    /// An expense together with the caller's access to its plan
    /// (`EXPENSE_NOT_FOUND` also for someone who is not in the plan).
    async fn expense(
        &self,
        conn: &mut PgConnection,
        expense_id: Uuid,
        user_id: Uuid,
    ) -> AppResult<(ExpenseRow, Access)> {
        let not_found = || AppError::not_found("EXPENSE_NOT_FOUND", "expense not found");
        let plan_id: Uuid = sqlx::query_scalar("SELECT plan_id FROM plan_expenses WHERE id = $1")
            .bind(expense_id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(not_found)?;
        let access = load_access(conn, plan_id, user_id, true)
            .await
            .map_err(|e| if e.is_not_found() { not_found() } else { e })?;
        let expense = sqlx::query_as(concat!("SELECT ", expense_columns!(), " FROM plan_expenses WHERE id = $1"))
            .bind(expense_id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(not_found)?;
        Ok((expense, access))
    }

    async fn push_expenses(&self, plan_id: Uuid, recipients: Vec<Uuid>, change: ExpensesChanged) {
        self.push(
            recipients,
            Push::ExpensesChanged(Box::new(ExpensesChanged { plan_id: plan_id.to_string(), ..change })),
        )
        .await;
    }

    /// Records who paid what and how it is split.
    pub(crate) async fn add_expense(&self, user_id: Uuid, plan_id: Uuid, input: &ExpenseInput) -> AppResult<Expense> {
        self.core.cache.rate_limit(&format!("plans:expense:{user_id}"), 300, Duration::from_secs(3600)).await?;
        let mut tx = self.core.db.begin().await?;
        let access = load_access(&mut tx, plan_id, user_id, true).await?;
        access.ensure_going()?;
        let everyone = participant_ids(&mut tx, plan_id).await?;
        let going = going_ids(&mut tx, plan_id).await?;
        let checked =
            check_expense(input, user_id, &access.plan.currency, &everyone.iter().copied().collect(), &going)?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM plan_expenses WHERE plan_id = $1")
            .bind(plan_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            count < MAX_EXPENSES,
            precondition,
            "TOO_MANY_EXPENSES",
            "a plan holds at most {MAX_EXPENSES} expenses"
        );

        let row: ExpenseRow = sqlx::query_as(concat!(
            "INSERT INTO plan_expenses (plan_id, title, amount, currency, paid_by, method, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING ",
            expense_columns!()
        ))
        .bind(plan_id)
        .bind(&checked.title)
        .bind(checked.amount)
        .bind(&checked.currency)
        .bind(checked.paid_by)
        .bind(checked.method as i32 as i16)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        let shares = insert_shares(&mut tx, row.id, &checked.shares).await?;
        tx.commit().await?;

        let expense = row.to_proto(&shares);
        self.push_expenses(
            plan_id,
            everyone,
            ExpensesChanged { expense: expense.clone().into(), ..Default::default() },
        )
        .await;
        Ok(expense)
    }

    /// Replaces an expense (whoever recorded it, the payer, or a host).
    pub(crate) async fn update_expense(
        &self,
        user_id: Uuid,
        expense_id: Uuid,
        input: &ExpenseInput,
    ) -> AppResult<Expense> {
        self.core.cache.rate_limit(&format!("plans:expense:{user_id}"), 300, Duration::from_secs(3600)).await?;
        let mut tx = self.core.db.begin().await?;
        let (old, access) = self.expense(&mut tx, expense_id, user_id).await?;
        ensure!(
            may_change(&old, &access, user_id),
            denied,
            "NOT_ALLOWED",
            "only whoever recorded an expense, its payer or a host can change it"
        );
        let everyone = participant_ids(&mut tx, old.plan_id).await?;
        // Whoever is part of the expense already may stay in it, even if
        // they changed their answer since.
        let mut chargeable = going_ids(&mut tx, old.plan_id).await?;
        chargeable.insert(old.paid_by);
        chargeable.extend(
            shares_of(&mut *tx, &[expense_id])
                .await?
                .remove(&expense_id)
                .unwrap_or_default()
                .iter()
                .map(|share| share.user_id),
        );
        let checked =
            check_expense(input, user_id, &access.plan.currency, &everyone.iter().copied().collect(), &chargeable)?;

        let row: ExpenseRow = sqlx::query_as(concat!(
            "UPDATE plan_expenses SET title = $2, amount = $3, currency = $4, paid_by = $5, method = $6 WHERE id = $1 RETURNING ",
            expense_columns!()
        ))
        .bind(expense_id)
        .bind(&checked.title)
        .bind(checked.amount)
        .bind(&checked.currency)
        .bind(checked.paid_by)
        .bind(checked.method as i32 as i16)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM plan_expense_shares WHERE expense_id = $1").bind(expense_id).execute(&mut *tx).await?;
        let shares = insert_shares(&mut tx, expense_id, &checked.shares).await?;
        tx.commit().await?;

        let expense = row.to_proto(&shares);
        self.push_expenses(
            old.plan_id,
            everyone,
            ExpensesChanged { expense: expense.clone().into(), ..Default::default() },
        )
        .await;
        Ok(expense)
    }

    /// Deletes an expense (same people as [`Self::update_expense`]).
    pub(crate) async fn remove_expense(&self, user_id: Uuid, expense_id: Uuid) -> AppResult<()> {
        self.core.cache.rate_limit(&format!("plans:expense:{user_id}"), 300, Duration::from_secs(3600)).await?;
        let mut tx = self.core.db.begin().await?;
        let (old, access) = self.expense(&mut tx, expense_id, user_id).await?;
        ensure!(
            may_change(&old, &access, user_id),
            denied,
            "NOT_ALLOWED",
            "only whoever recorded an expense, its payer or a host can delete it"
        );
        sqlx::query("DELETE FROM plan_expenses WHERE id = $1").bind(expense_id).execute(&mut *tx).await?;
        let everyone = participant_ids(&mut tx, old.plan_id).await?;
        tx.commit().await?;
        self.push_expenses(
            old.plan_id,
            everyone,
            ExpensesChanged { removed_expense_id: expense_id.to_string(), ..Default::default() },
        )
        .await;
        Ok(())
    }

    /// The expenses of a plan, newest first.
    pub(crate) async fn list_expenses(
        &self,
        user_id: Uuid,
        plan_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<Expense>, PageInfo)> {
        let core = &self.core;
        self.plan_access(plan_id, user_id).await?;
        let before: Option<Uuid> = page.decode()?;
        let mut rows: Vec<ExpenseRow> = sqlx::query_as(concat!(
            "SELECT ",
            expense_columns!(),
            " FROM plan_expenses WHERE plan_id = $1 AND ($2::uuid IS NULL OR id < $2) ORDER BY id DESC LIMIT $3"
        ))
        .bind(plan_id)
        .bind(before)
        .bind(page.fetch())
        .fetch_all(&core.db)
        .await?;
        let info = page.finish(&mut rows, |row| row.id);
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let mut shares = shares_of(&core.db, &ids).await?;
        Ok((rows.iter().map(|row| row.to_proto(&shares.remove(&row.id).unwrap_or_default())).collect(), info))
    }

    /// Where the split stands: balances and the transfers that settle them,
    /// per currency, plus the transfers already marked as done.
    pub(crate) async fn get_split(&self, user_id: Uuid, plan_id: Uuid) -> AppResult<GetSplitResponse> {
        self.plan_access(plan_id, user_id).await?;
        let ledger = {
            let mut conn = self.core.db.acquire().await?;
            Ledger::load(&mut conn, plan_id).await?
        };
        let splits = ledger
            .paid
            .iter()
            .map(|(currency, expenses)| {
                let positions = ledger.positions(currency);
                let nets: Vec<(Uuid, i64)> =
                    positions.iter().map(|(person, position)| (*person, position.net())).collect();
                CurrencySplit {
                    currency: currency.clone(),
                    total: expenses.iter().map(|expense| expense.amount).sum(),
                    balances: positions
                        .iter()
                        .map(|(person, position)| Balance {
                            user_id: person.to_string(),
                            paid: position.paid,
                            owed: position.owed,
                            sent: position.sent,
                            received: position.received,
                            net: position.net(),
                            ..Default::default()
                        })
                        .collect(),
                    transfers: split::settle(&nets)
                        .into_iter()
                        .map(|transfer| Transfer {
                            from_user_id: transfer.from.to_string(),
                            to_user_id: transfer.to.to_string(),
                            amount: transfer.amount,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }
            })
            .collect();
        Ok(GetSplitResponse {
            splits,
            settlements: ledger.settlements.iter().map(SettlementRow::to_proto).collect(),
            removed_settlements: ledger.removed.iter().map(SettlementRow::to_proto).collect(),
            ..Default::default()
        })
    }

    /// Marks a transfer as received: `from` handed `to` this amount outside
    /// the app. Only `to` records it. Someone who says "I paid" proves
    /// nothing; the person who got the money giving up their claim does. So
    /// nobody can declare their own debt paid, and nobody can invent a
    /// payment that leaves someone else owing.
    pub(crate) async fn settle_transfer(&self, user_id: Uuid, req: &SettleTransferRequest) -> AppResult<Settlement> {
        let core = &self.core;
        let plan_id = parse_id(&req.plan_id)?;
        let from = parse_id(&req.from_user_id)?;
        let to = parse_id(&req.to_user_id)?;
        let total = amount(req.amount)?;
        ensure!(from != to, invalid, "SAME_USER", "a transfer goes from one person to another");
        // Each record notifies the other person.
        core.cache
            .rate_limit(&format!("plans:settle:{user_id}"), SETTLEMENTS_PER_HOUR, Duration::from_secs(3600))
            .await?;
        // Asked before the plan is locked: the payer is told — unless the
        // two are in a block with each other.
        let silent = graph::is_blocked_either(core, user_id, from).await?;

        let mut tx = core.db.begin().await?;
        let access = load_access(&mut tx, plan_id, user_id, true).await?;
        let currency = input::currency(&req.currency, &access.plan.currency)?;
        ensure!(user_id == to, denied, "NOT_ALLOWED", "only the person who received the money records a transfer");
        let everyone = participant_ids(&mut tx, plan_id).await?;
        ensure!(
            everyone.contains(&from) && everyone.contains(&to),
            invalid,
            "USER_NOT_IN_PLAN",
            "both people of a transfer must be in the plan"
        );
        // Only what is owed can be settled, which also means that both are
        // part of the split. Checked under the plan's lock, so a transfer
        // recorded twice at once (a double tap) is recorded once: the second
        // one finds nothing left to settle.
        let positions = Ledger::load(&mut tx, plan_id).await?.positions(&currency);
        let owes = positions.get(&from).map(|position| -position.net()).unwrap_or(0);
        let is_owed = positions.get(&to).map(|position| position.net()).unwrap_or(0);
        ensure!(
            total <= owes && total <= is_owed,
            precondition,
            "AMOUNT_EXCEEDS_BALANCE",
            "the transfer is more than what is still owed between the two"
        );

        let row: SettlementRow = sqlx::query_as(concat!(
            "INSERT INTO plan_settlements (plan_id, from_user_id, to_user_id, amount, currency, recorded_by) VALUES ($1, $2, $3, $4, $5, $6) RETURNING ",
            settlement_columns!()
        ))
        .bind(plan_id)
        .bind(from)
        .bind(to)
        .bind(total)
        .bind(&currency)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if !silent {
            let by = people::name_on(&mut tx, user_id).await?;
            let text = copy::settled(&by, &split::format_money(total, &currency), &access.plan.name());
            let request = NotificationRequest::new(from, "plans.settled", text.title, text.body, plan_path(plan_id))
                .actor(user_id)
                .subject(SubjectType::SUBJECT_TYPE_PLAN, plan_id)
                .data(serde_json::json!({ "settlement_id": row.id, "amount": total, "currency": currency }));
            outbound::notify(&mut *tx, &request).await?;
        }
        tx.commit().await?;

        self.push_expenses(plan_id, everyone, ExpensesChanged { settlements_changed: true, ..Default::default() })
            .await;
        Ok(row.to_proto())
    }

    /// Takes back a transfer the caller marked as received. The row stays,
    /// with who took it back and when, and the payer — whose debt is open
    /// again — is told.
    pub(crate) async fn remove_settlement(&self, user_id: Uuid, settlement_id: Uuid) -> AppResult<()> {
        let core = &self.core;
        let not_found = || AppError::not_found("SETTLEMENT_NOT_FOUND", "settlement not found");
        core.cache
            .rate_limit(&format!("plans:settle:{user_id}"), SETTLEMENTS_PER_HOUR, Duration::from_secs(3600))
            .await?;
        // A first look without a lock, for what is asked of the social domain.
        let payer: Option<Uuid> = sqlx::query_scalar("SELECT from_user_id FROM plan_settlements WHERE id = $1")
            .bind(settlement_id)
            .fetch_optional(&core.db)
            .await?;
        let payer = payer.ok_or_else(not_found)?;
        let silent = graph::is_blocked_either(core, user_id, payer).await?;

        let mut tx = core.db.begin().await?;
        let plan_id: Uuid = sqlx::query_scalar("SELECT plan_id FROM plan_settlements WHERE id = $1")
            .bind(settlement_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(not_found)?;
        let access = load_access(&mut tx, plan_id, user_id, true)
            .await
            .map_err(|e| if e.is_not_found() { not_found() } else { e })?;
        let row: SettlementRow = sqlx::query_as(concat!(
            "SELECT ",
            settlement_columns!(),
            " FROM plan_settlements WHERE id = $1 AND removed_at IS NULL"
        ))
        .bind(settlement_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(not_found)?;
        ensure!(
            user_id == row.to_user_id,
            denied,
            "NOT_ALLOWED",
            "only the person who marked a transfer as received can take it back"
        );
        sqlx::query("UPDATE plan_settlements SET removed_at = now(), removed_by = $2 WHERE id = $1")
            .bind(settlement_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        if !silent {
            let by = people::name_on(&mut tx, user_id).await?;
            let text =
                copy::settlement_removed(&by, &split::format_money(row.amount, &row.currency), &access.plan.name());
            let request = NotificationRequest::new(
                row.from_user_id,
                "plans.settlement_removed",
                text.title,
                text.body,
                plan_path(plan_id),
            )
            .actor(user_id)
            .subject(SubjectType::SUBJECT_TYPE_PLAN, plan_id)
            .data(serde_json::json!({ "settlement_id": row.id, "amount": row.amount, "currency": row.currency }));
            outbound::notify(&mut *tx, &request).await?;
        }
        let everyone = participant_ids(&mut tx, plan_id).await?;
        tx.commit().await?;
        self.push_expenses(plan_id, everyone, ExpensesChanged { settlements_changed: true, ..Default::default() })
            .await;
        Ok(())
    }
}

/// The people going to a plan: the ones an expense can be split between.
async fn going_ids(conn: &mut PgConnection, plan_id: Uuid) -> AppResult<HashSet<Uuid>> {
    Ok(sqlx::query_scalar("SELECT user_id FROM plan_participants WHERE plan_id = $1 AND rsvp = $2")
        .bind(plan_id)
        .bind(Rsvp::RSVP_GOING as i32 as i16)
        .fetch_all(conn)
        .await?
        .into_iter()
        .collect())
}

/// Whoever recorded an expense, its payer and the hosts may change it.
fn may_change(expense: &ExpenseRow, access: &Access, user_id: Uuid) -> bool {
    expense.created_by == user_id || expense.paid_by == user_id || access.is_organizer()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn share(person: Uuid, amount: i64, weight: i32) -> ExpenseShare {
        ExpenseShare { user_id: person.to_string(), amount, weight, ..Default::default() }
    }

    fn input(amount: i64, method: SplitMethod, shares: Vec<ExpenseShare>) -> ExpenseInput {
        ExpenseInput { title: "カラオケ代".into(), amount, method: method.into(), shares, ..Default::default() }
    }

    fn check(input: &ExpenseInput) -> AppResult<CheckedExpense> {
        // Person 4 is in the plan and has not said they are going.
        let members: HashSet<Uuid> = [person(1), person(2), person(3), person(4)].into();
        let going: HashSet<Uuid> = [person(1), person(2), person(3)].into();
        check_expense(input, person(1), "JPY", &members, &going)
    }

    #[test]
    fn an_expense_is_divided_by_its_method() {
        let equal = check(&input(
            1000,
            SplitMethod::SPLIT_METHOD_EQUAL,
            vec![share(person(1), 0, 0), share(person(2), 0, 0), share(person(3), 0, 0)],
        ))
        .unwrap();
        assert_eq!(equal.shares, vec![(person(1), 334, 0), (person(2), 333, 0), (person(3), 333, 0)]);
        // Defaults: the caller paid, in the plan's currency.
        assert_eq!((equal.paid_by, equal.currency.as_str()), (person(1), "JPY"));

        let weighted = check(&input(
            1000,
            SplitMethod::SPLIT_METHOD_SHARES,
            vec![share(person(1), 0, 2), share(person(2), 0, 1), share(person(3), 0, 1)],
        ))
        .unwrap();
        assert_eq!(weighted.shares, vec![(person(1), 500, 2), (person(2), 250, 1), (person(3), 250, 1)]);

        let exact = check(&input(
            1000,
            SplitMethod::SPLIT_METHOD_EXACT,
            vec![share(person(2), 700, 0), share(person(3), 300, 0)],
        ))
        .unwrap();
        assert_eq!(exact.shares, vec![(person(2), 700, 0), (person(3), 300, 0)]);
    }

    #[test]
    fn expenses_that_do_not_add_up_are_rejected() {
        let all = || vec![share(person(1), 0, 1), share(person(2), 0, 1)];
        let reason = |input: ExpenseInput| check(&input).err().map(|e| e.reason.to_string());
        assert_eq!(reason(input(0, SplitMethod::SPLIT_METHOD_EQUAL, all())).as_deref(), Some("AMOUNT_INVALID"));
        assert_eq!(
            reason(input(MAX_AMOUNT + 1, SplitMethod::SPLIT_METHOD_EQUAL, all())).as_deref(),
            Some("AMOUNT_INVALID")
        );
        assert_eq!(
            reason(input(1000, SplitMethod::SPLIT_METHOD_UNSPECIFIED, all())).as_deref(),
            Some("SPLIT_METHOD_REQUIRED")
        );
        assert_eq!(reason(input(1000, SplitMethod::SPLIT_METHOD_EQUAL, vec![])).as_deref(), Some("SHARES_REQUIRED"));
        assert_eq!(
            reason(input(1000, SplitMethod::SPLIT_METHOD_EQUAL, vec![share(person(9), 0, 0)])).as_deref(),
            Some("SHARE_NOT_IN_PLAN")
        );
        assert_eq!(
            reason(input(1000, SplitMethod::SPLIT_METHOD_EQUAL, vec![share(person(1), 0, 0), share(person(1), 0, 0)]))
                .as_deref(),
            Some("SHARE_DUPLICATE")
        );
        assert_eq!(
            reason(input(
                1000,
                SplitMethod::SPLIT_METHOD_EXACT,
                vec![share(person(1), 600, 0), share(person(2), 300, 0)]
            ))
            .as_deref(),
            Some("SHARES_INVALID")
        );
        assert_eq!(
            reason(input(
                1000,
                SplitMethod::SPLIT_METHOD_EXACT,
                vec![share(person(1), 1100, 0), share(person(2), -100, 0)]
            ))
            .as_deref(),
            Some("SHARES_INVALID")
        );
        assert_eq!(
            reason(input(1000, SplitMethod::SPLIT_METHOD_SHARES, vec![share(person(1), 0, 0), share(person(2), 0, 1)]))
                .as_deref(),
            Some("SHARES_INVALID")
        );

        let stranger_paid = ExpenseInput {
            paid_by_user_id: person(9).to_string(),
            ..input(1000, SplitMethod::SPLIT_METHOD_EQUAL, all())
        };
        assert_eq!(reason(stranger_paid).as_deref(), Some("PAYER_NOT_IN_PLAN"));
        let absent_paid = ExpenseInput {
            paid_by_user_id: person(4).to_string(),
            ..input(1000, SplitMethod::SPLIT_METHOD_EQUAL, all())
        };
        assert_eq!(reason(absent_paid).as_deref(), Some("PAYER_NOT_GOING"));
        assert_eq!(
            reason(input(1000, SplitMethod::SPLIT_METHOD_EQUAL, vec![share(person(1), 0, 0), share(person(4), 0, 0)]))
                .as_deref(),
            Some("SHARE_NOT_GOING")
        );
        let untitled = ExpenseInput { title: " ".into(), ..input(1000, SplitMethod::SPLIT_METHOD_EQUAL, all()) };
        assert_eq!(reason(untitled).as_deref(), Some("TITLE_REQUIRED"));
        let dollars = ExpenseInput { currency: "usd".into(), ..input(1000, SplitMethod::SPLIT_METHOD_EQUAL, all()) };
        assert_eq!(reason(dollars).as_deref(), Some("CURRENCY_INVALID"));
    }
}
