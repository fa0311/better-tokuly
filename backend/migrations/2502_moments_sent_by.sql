-- Moments: an admin may send a region's prompt of today at once
-- (MomentAdminService.SendPromptNow); the day remembers who did.
ALTER TABLE moment_days ADD COLUMN sent_by uuid REFERENCES users (id) ON DELETE SET NULL;
