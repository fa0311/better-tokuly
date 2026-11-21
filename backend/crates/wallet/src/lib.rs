//! Coins: balances, the double-entry ledger, purchases, the daily bonus,
//! creator earnings and payouts.
//!
//! What other domains use:
//! - [`ledger`] — `balance`, `spend`, `transfer`, `refund` (and `grant`): the
//!   functions that move coins. Every call is idempotent on its key.
//! - [`Wallet::transfer_with_fee`] — `ledger::transfer` with the platform fee
//!   from the configuration, for gifts and sales between users.
//! - [`events`] — the payload of `wallet.balance.changed`.
//! - [`providers`] — the interface a payment integration implements.
//!
//! Everything else is the wallet's own product surface: three RPC services
//! (`WalletService`, `EarningsService`, `WalletAdminService`) and the
//! workers behind them.
use std::{ops::Deref, sync::Arc};

use chrono_tz::Tz;
use parking_lot::RwLock;
use tokuly_core::prelude::*;
use tokuly_identity::Identity;
use tokuly_proto::{
    realtime::server_frame::Body,
    wallet::{WalletEvent, wallet_event::Event},
};

mod adjustments;
mod admin_svc;
mod cleanup;
pub mod config;
mod daily_bonus;
mod describe;
mod earnings;
mod earnings_svc;
mod entries;
pub mod events;
mod export;
pub mod ledger;
pub mod ledger_tx;
mod money;
mod packages;
mod payouts;
mod postings;
pub mod providers;
mod purchases;
mod wallet_svc;
mod workers;

pub use config::WalletConfig;
pub use providers::{Checkout, DevelopmentProvider, PaymentProvider, PaymentState, PurchaseOrder};

pub struct WalletInner {
    pub core: Core,
    pub identity: Identity,
    pub cfg: WalletConfig,
    /// The daily bonus ladder parsed from the configuration; empty = disabled.
    daily_cycle: Vec<i64>,
    /// Enabled payment providers. The first one is the default.
    providers: RwLock<Vec<Arc<dyn PaymentProvider>>>,
}

/// The wallet domain. Cheap to clone.
#[derive(Clone)]
pub struct Wallet(Arc<WalletInner>);

impl Deref for Wallet {
    type Target = WalletInner;

    fn deref(&self) -> &WalletInner {
        &self.0
    }
}

/// The two account settings the wallet reads from a user's profile.
pub(crate) struct UserPrefs {
    /// Language for server-written text.
    pub locale: String,
    /// Defines the user's calendar day (daily bonus) and chart buckets.
    pub time_zone: Tz,
}

impl Wallet {
    pub fn new(core: Core, identity: Identity, cfg: WalletConfig) -> anyhow::Result<Self> {
        cfg.validate()?;
        let daily_cycle = cfg.daily_bonus_cycle()?;
        let mut providers: Vec<Arc<dyn PaymentProvider>> = Vec::new();
        if cfg.dev_provider_enabled(core.cfg.is_production()) {
            if core.cfg.is_production() {
                tracing::warn!("the development payment provider is enabled in production: coins can be had for free");
            }
            providers.push(Arc::new(DevelopmentProvider));
        }
        // The exporter lives inside `core`, so it must not hold a `Core` (or
        // this domain) back: that would be a reference cycle.
        core.exporters.register(Arc::new(export::WalletExporter::new(&core)));
        Ok(Self(Arc::new(WalletInner { core, identity, cfg, daily_cycle, providers: RwLock::new(providers) })))
    }

    /// Registers this domain's RPC services on the Connect router.
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router {
        use tokuly_proto::wallet::{EarningsServiceExt, WalletAdminServiceExt, WalletServiceExt};
        let me = Arc::new(self.clone());
        let router = WalletServiceExt::register(me.clone(), router);
        let router = EarningsServiceExt::register(me.clone(), router);
        WalletAdminServiceExt::register(me, router)
    }

    /// Share of every transfer between users the platform keeps, in percent.
    pub fn platform_fee_percent(&self) -> u32 {
        self.cfg.platform_fee_percent
    }

    /// Moves coins from a payer to a recipient's earnings, withholding the
    /// configured platform fee: [`ledger::transfer`] for gifts and sales.
    pub async fn transfer_with_fee(
        &self,
        from_user: Uuid,
        to_user: Uuid,
        amount: i64,
        reason: &str,
        idempotency_key: &str,
        memo: serde_json::Value,
    ) -> AppResult<ledger::TransferReceipt> {
        ledger::transfer(&self.core, from_user, to_user, amount, self.cfg.platform_fee_percent, reason, idempotency_key, memo).await
    }

    /// Enables a payment provider. Call at startup, before serving requests;
    /// a provider registered under an existing name replaces it.
    pub fn add_payment_provider(&self, provider: Arc<dyn PaymentProvider>) {
        let mut providers = self.providers.write();
        providers.retain(|existing| existing.name() != provider.name());
        providers.push(provider);
    }

    /// Names of the enabled payment providers, the default first.
    pub fn payment_providers(&self) -> Vec<&'static str> {
        self.providers.read().iter().map(|provider| provider.name()).collect()
    }

    pub(crate) fn provider(&self, name: &str) -> Option<Arc<dyn PaymentProvider>> {
        self.providers.read().iter().find(|provider| provider.name() == name).cloned()
    }

    /// The provider a purchase request asked for (empty name = the default).
    pub(crate) fn resolve_provider(&self, name: &str) -> AppResult<Arc<dyn PaymentProvider>> {
        if name.is_empty() {
            let default = self.providers.read().first().cloned();
            return default.ok_or_else(|| AppError::unimplemented("PURCHASES_UNAVAILABLE", "coins cannot be bought on this server"));
        }
        ensure!(!self.providers.read().is_empty(), unimplemented, "PURCHASES_UNAVAILABLE", "coins cannot be bought on this server");
        self.provider(name).ok_or_else(|| AppError::invalid("PROVIDER_UNKNOWN", "unknown payment provider"))
    }

    /// Pushes a wallet event to every connected device of `user_id`.
    pub(crate) async fn push(&self, user_id: Uuid, event: Event) {
        let event = WalletEvent { event: Some(event), ..Default::default() };
        self.core.rt.notify_user(user_id, Body::Wallet(Box::new(event))).await;
    }

    /// Locale and time zone of a user, or `USER_NOT_FOUND`.
    pub(crate) async fn user_prefs(&self, user_id: Uuid) -> AppResult<UserPrefs> {
        let row: Option<(String, String)> = sqlx::query_as("SELECT locale, time_zone FROM users WHERE id = $1").bind(user_id).fetch_optional(&self.core.db).await?;
        let (locale, time_zone) = row.or_not_found("USER_NOT_FOUND", "user not found")?;
        // Identity only stores zone names it could parse; should one slip
        // through, the product's home zone beats failing every wallet screen.
        Ok(UserPrefs { locale, time_zone: time_zone.parse().unwrap_or(chrono_tz::Asia::Tokyo) })
    }
}
