-- Unused recovery codes, sealed together (`totp::seal_recovery_codes`):
-- unlike keyed hashes, `tokuly rekey` carries them to a new TOKULY_SECRET.
-- `recovery_code_hashes` keeps the codes made before, which work while the
-- secret they were made under is set.
ALTER TABLE users ADD COLUMN recovery_codes text;
