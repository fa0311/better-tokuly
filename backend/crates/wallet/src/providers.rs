//! Payment providers: how money for coins is collected.
//!
//! Tokuly bundles no payment processor. The wallet drives every purchase
//! through the [`PaymentProvider`] trait — create a checkout, then confirm it
//! was paid — and grants the coins itself, exactly once, when a provider says
//! "paid". A real integration (card processor, app-store receipts) implements
//! the trait and is registered with [`crate::Wallet::add_payment_provider`];
//! if it learns about payments through webhooks it reports them with
//! [`crate::Wallet::settle_purchase`].
//!
//! The only built-in provider is [`DevelopmentProvider`], which pretends every
//! checkout was paid immediately.
use async_trait::async_trait;
use tokuly_core::prelude::*;

/// What a provider needs to know to charge for one purchase.
#[derive(Debug, Clone)]
pub struct PurchaseOrder {
    /// The wallet's purchase ID; pass it to the provider as the order
    /// reference so its records can be matched to ours.
    pub purchase_id: Uuid,
    pub user_id: Uuid,
    /// Product identifier of the coin package.
    pub sku: String,
    /// Coins the buyer receives, bonus included.
    pub coins: i64,
    /// Price in the minor unit of `currency`.
    pub price_minor: i64,
    /// ISO 4217 code.
    pub currency: String,
    /// Page of the web app to send the buyer back to after a hosted checkout.
    pub return_url: String,
}

/// Where a payment stands, according to the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentState {
    /// Not paid yet; it still may be.
    Pending,
    /// The money was collected.
    Paid,
    /// The payment was declined or abandoned. `reason` is the provider's
    /// short code, e.g. `card_declined`. The wallet then voids the checkout
    /// ([`PaymentProvider::cancel_checkout`]); should the provider still
    /// report `Paid` later, the purchase completes all the same.
    Failed { reason: String },
}

/// A checkout a provider opened for a purchase.
#[derive(Debug, Clone)]
pub struct Checkout {
    /// The provider's identifier of the checkout (payment intent, order ID).
    /// Must be unique per provider and non-empty.
    pub reference: String,
    /// Hosted page to send the buyer to, when the provider has one.
    pub checkout_url: Option<String>,
    /// State right after creation. Providers that settle instantly return
    /// [`PaymentState::Paid`] and the purchase completes without a second
    /// round trip.
    pub state: PaymentState,
}

#[async_trait]
pub trait PaymentProvider: Send + Sync + 'static {
    /// Stable lowercase name clients select the provider by, e.g. `"stripe"`.
    fn name(&self) -> &'static str;

    /// Opens a checkout for `order`.
    async fn create_checkout(&self, order: &PurchaseOrder) -> AppResult<Checkout>;

    /// Reports whether the checkout `reference` was paid. `client_proof` is
    /// whatever the provider's client SDK handed the app (a receipt, a
    /// session ID); it is empty when the wallet asks on its own, e.g. before
    /// letting an unpaid purchase expire. Must verify with the provider —
    /// never trust `client_proof` by itself.
    async fn confirm(&self, order: &PurchaseOrder, reference: &str, client_proof: &str) -> AppResult<PaymentState>;

    /// Makes the checkout `reference` impossible to pay. The wallet asks when
    /// a purchase is cancelled, fails or expires, and again later while this
    /// returns an error: until it succeeds the purchase counts against the
    /// buyer's spending limit, since it may still be paid. The default does
    /// nothing, which is right only for providers whose checkouts cannot be
    /// paid once they were not paid at once.
    async fn cancel_checkout(&self, _order: &PurchaseOrder, _reference: &str) -> AppResult<()> {
        Ok(())
    }
}

/// Grants coins without taking money: every checkout is paid the moment it
/// is created. For development and tests only — it is registered when
/// `TOKULY_WALLET_DEV_PROVIDER` allows it (see [`crate::WalletConfig`]).
pub struct DevelopmentProvider;

impl DevelopmentProvider {
    pub const NAME: &'static str = "development";
}

#[async_trait]
impl PaymentProvider for DevelopmentProvider {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn create_checkout(&self, order: &PurchaseOrder) -> AppResult<Checkout> {
        Ok(Checkout { reference: format!("dev_{}", order.purchase_id.simple()), checkout_url: None, state: PaymentState::Paid })
    }

    async fn confirm(&self, _order: &PurchaseOrder, _reference: &str, _client_proof: &str) -> AppResult<PaymentState> {
        Ok(PaymentState::Paid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn development_provider_settles_instantly() {
        let order = PurchaseOrder {
            purchase_id: new_id(),
            user_id: new_id(),
            sku: "coins_100".into(),
            coins: 100,
            price_minor: 100,
            currency: "JPY".into(),
            return_url: "http://localhost:8080/wallet".into(),
        };
        let checkout = DevelopmentProvider.create_checkout(&order).await.unwrap();
        assert_eq!(checkout.state, PaymentState::Paid);
        assert!(checkout.reference.starts_with("dev_") && checkout.checkout_url.is_none());
        assert_eq!(DevelopmentProvider.confirm(&order, &checkout.reference, "").await.unwrap(), PaymentState::Paid);
    }
}
