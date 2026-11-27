-- The monthly spending limit a user chose for themselves, in yen; NULL: none.
-- The ceiling under 18 is a setting (TOKULY_WALLET_MINOR_MONTHLY_LIMIT_YEN),
-- applied by age on the day of each charge, and is not stored.
ALTER TABLE wallet_accounts ADD COLUMN monthly_limit_yen bigint CHECK (monthly_limit_yen >= 0);
