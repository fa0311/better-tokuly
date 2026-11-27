-- When the provider confirmed that the checkout of an unpaid purchase can no
-- longer be paid. A failed, cancelled or expired purchase with a checkout and
-- no such time may still be paid, so it counts against the spending limit.
ALTER TABLE wallet_purchases ADD COLUMN checkout_voided_at timestamptz;
-- The checkouts still to void (retried by the maintenance run).
CREATE INDEX wallet_purchases_unvoided ON wallet_purchases (id)
    WHERE status IN ('failed', 'cancelled', 'expired') AND provider_ref <> '' AND checkout_voided_at IS NULL;
