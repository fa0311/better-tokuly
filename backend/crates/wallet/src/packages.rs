//! The coin shop: packages of coins sold for money.
//!
//! Packages live in `wallet_coin_packages` (seeded by the migration) and are
//! edited by admins. A package is never deleted, only deactivated, because
//! purchases keep pointing at it.
use tokuly_core::prelude::*;
use tokuly_proto::wallet::CoinPackage;

use crate::{Wallet, config::is_currency_code, postings::MAX_AMOUNT};

/// Upper bound on a package price: far above any real price, low enough
/// that sums of prices stay well inside `i64`.
const MAX_PRICE_MINOR: i64 = 1_000_000_000_000;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PackageRow {
    pub id: Uuid,
    pub sku: String,
    pub coins: i64,
    pub bonus_coins: i64,
    pub price_minor: i64,
    pub currency: String,
    pub badge: i32,
    pub sort_order: i32,
    pub active: bool,
    pub available_from: Option<Time>,
    pub available_until: Option<Time>,
}

const PACKAGE_COLUMNS: &str = "id, sku, coins, bonus_coins, price_minor, currency, badge, sort_order, active, available_from, available_until";

/// SQL condition for "can be bought right now".
const ON_SALE: &str = "active AND (available_from IS NULL OR available_from <= now()) AND (available_until IS NULL OR available_until > now())";

impl PackageRow {
    pub fn to_proto(&self) -> CoinPackage {
        CoinPackage {
            id: self.id.to_string(),
            sku: self.sku.clone(),
            coins: self.coins,
            bonus_coins: self.bonus_coins,
            price_minor: self.price_minor,
            currency: self.currency.clone(),
            badge: self.badge.into(),
            sort_order: self.sort_order,
            active: self.active,
            available_from: opt_ts(self.available_from),
            available_until: opt_ts(self.available_until),
            ..Default::default()
        }
    }
}

/// A package as an admin submitted it, checked and normalized.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PackageInput {
    pub sku: String,
    pub coins: i64,
    pub bonus_coins: i64,
    pub price_minor: i64,
    pub currency: String,
    pub badge: i32,
    pub sort_order: i32,
    pub active: bool,
    pub available_from: Option<Time>,
    pub available_until: Option<Time>,
}

fn is_valid_sku(sku: &str) -> bool {
    (3..=64).contains(&sku.len()) && sku.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.'))
}

/// Checks every field of a submitted package.
pub(crate) fn validate_package(package: &CoinPackage) -> AppResult<PackageInput> {
    let sku = package.sku.trim();
    ensure!(is_valid_sku(sku), invalid, "SKU_INVALID", "a SKU is 3-64 lowercase letters, digits, `_`, `-` or `.`");
    ensure!((1..=MAX_AMOUNT).contains(&package.coins), invalid, "COINS_INVALID", "coins must be between 1 and 100,000,000");
    // A purchase grants coins + bonus in one ledger entry, so the sum has to fit the ledger's ceiling too.
    ensure!(
        package.bonus_coins >= 0 && package.coins + package.bonus_coins <= MAX_AMOUNT,
        invalid,
        "BONUS_INVALID",
        "bonus coins cannot be negative and coins plus bonus cannot exceed 100,000,000"
    );
    ensure!((1..=MAX_PRICE_MINOR).contains(&package.price_minor), invalid, "PRICE_INVALID", "the price must be positive");
    ensure!(is_currency_code(&package.currency), invalid, "CURRENCY_INVALID", "currency must be a 3-letter ISO 4217 code such as JPY");
    let badge = package.badge.as_known().ok_or_else(|| AppError::invalid("BADGE_INVALID", "unknown badge"))?;
    let available_from = package.available_from.as_option().map(from_ts);
    let available_until = package.available_until.as_option().map(from_ts);
    if let (Some(from), Some(until)) = (available_from, available_until) {
        ensure!(until > from, invalid, "AVAILABILITY_INVALID", "the sale must end after it starts");
    }
    Ok(PackageInput {
        sku: sku.to_owned(),
        coins: package.coins,
        bonus_coins: package.bonus_coins,
        price_minor: package.price_minor,
        currency: package.currency.clone(),
        badge: badge as i32,
        sort_order: package.sort_order,
        active: package.active,
        available_from,
        available_until,
    })
}

impl Wallet {
    /// Packages that can be bought right now, in shop order.
    pub(crate) async fn packages_on_sale(&self) -> AppResult<Vec<PackageRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {PACKAGE_COLUMNS} FROM wallet_coin_packages WHERE {ON_SALE} ORDER BY sort_order, price_minor, id")))
            .fetch_all(&self.core.db)
            .await?)
    }

    /// Every package, including retired ones, in shop order.
    pub(crate) async fn all_packages(&self) -> AppResult<Vec<PackageRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {PACKAGE_COLUMNS} FROM wallet_coin_packages ORDER BY sort_order, price_minor, id")))
            .fetch_all(&self.core.db)
            .await?)
    }

    /// A package that can be bought right now, or `PACKAGE_NOT_FOUND`.
    pub(crate) async fn package_on_sale(&self, package_id: Uuid) -> AppResult<PackageRow> {
        let row: Option<PackageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {PACKAGE_COLUMNS} FROM wallet_coin_packages WHERE id = $1 AND {ON_SALE}")))
            .bind(package_id)
            .fetch_optional(&self.core.db)
            .await?;
        row.or_not_found("PACKAGE_NOT_FOUND", "this coin package is not on sale")
    }

    /// Creates a package (`id` of `None`) or replaces every field of an existing one.
    pub(crate) async fn save_package(&self, id: Option<Uuid>, input: &PackageInput) -> AppResult<PackageRow> {
        let sql = match id {
            None => format!(
                "INSERT INTO wallet_coin_packages (id, sku, coins, bonus_coins, price_minor, currency, badge, sort_order, active, available_from, available_until)
                 VALUES (COALESCE($1, uuidv7()), $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING {PACKAGE_COLUMNS}"
            ),
            Some(_) => format!(
                "UPDATE wallet_coin_packages SET sku = $2, coins = $3, bonus_coins = $4, price_minor = $5, currency = $6, badge = $7,
                        sort_order = $8, active = $9, available_from = $10, available_until = $11
                 WHERE id = $1 RETURNING {PACKAGE_COLUMNS}"
            ),
        };
        let saved: Result<Option<PackageRow>, sqlx::Error> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .bind(&input.sku)
            .bind(input.coins)
            .bind(input.bonus_coins)
            .bind(input.price_minor)
            .bind(&input.currency)
            .bind(input.badge)
            .bind(input.sort_order)
            .bind(input.active)
            .bind(input.available_from)
            .bind(input.available_until)
            .fetch_optional(&self.core.db)
            .await;
        match saved.map_err(AppError::from) {
            Ok(row) => row.or_not_found("PACKAGE_NOT_FOUND", "coin package not found"),
            // The only unique constraint a save can hit is the SKU.
            Err(e) if e.is_already_exists() => Err(AppError::already_exists("SKU_TAKEN", "another package already uses this SKU")),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use tokuly_proto::wallet::PackageBadge;

    use super::*;

    fn package() -> CoinPackage {
        CoinPackage {
            sku: "coins_250".into(),
            coins: 250,
            bonus_coins: 10,
            price_minor: 250,
            currency: "JPY".into(),
            badge: PackageBadge::PACKAGE_BADGE_POPULAR.into(),
            sort_order: 15,
            active: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_well_formed_package_passes() {
        let input = validate_package(&package()).unwrap();
        assert_eq!((input.sku.as_str(), input.coins, input.bonus_coins, input.badge), ("coins_250", 250, 10, 1));
        assert!(input.available_from.is_none() && input.available_until.is_none());
    }

    #[test]
    fn each_field_is_checked() {
        let cases: Vec<(&str, fn(&mut CoinPackage))> = vec![
            ("SKU_INVALID", |p| p.sku = "No Spaces".into()),
            ("SKU_INVALID", |p| p.sku = "ab".into()),
            ("COINS_INVALID", |p| p.coins = 0),
            ("COINS_INVALID", |p| p.coins = MAX_AMOUNT + 1),
            ("BONUS_INVALID", |p| p.bonus_coins = -1),
            ("BONUS_INVALID", |p| {
                p.coins = MAX_AMOUNT;
                p.bonus_coins = 1;
            }),
            ("PRICE_INVALID", |p| p.price_minor = 0),
            ("CURRENCY_INVALID", |p| p.currency = "yen".into()),
            ("BADGE_INVALID", |p| p.badge = 99.into()),
            ("AVAILABILITY_INVALID", |p| {
                p.available_from = ts(now());
                p.available_until = ts(now() - chrono::Duration::hours(1));
            }),
        ];
        for (reason, mutate) in cases {
            let mut candidate = package();
            mutate(&mut candidate);
            assert_eq!(validate_package(&candidate).unwrap_err().reason, reason);
        }
    }
}
