//! Who is making an admin request, and the input rules shared by every
//! moderation RPC.
use connectrpc::RequestContext;
use tokuly_core::prelude::*;

/// Longest reason shown to an affected user.
pub(crate) const REASON_MAX_CHARS: usize = 500;
/// Longest internal note.
pub(crate) const NOTE_MAX_CHARS: usize = 2000;

const MIN_SUSPENSION_SECONDS: i64 = 3600;
const MAX_SUSPENSION_SECONDS: i64 = 365 * 24 * 3600;

/// Roles that make an account "staff" for moderation purposes: moderators
/// may not sanction these accounts, only admins may.
const PROTECTED_ROLES: &[&str] = &["admin", "moderator"];

/// The staff member behind a request, as recorded in the moderation and
/// audit logs.
#[derive(Debug, Clone)]
pub(crate) struct Staff {
    pub id: Uuid,
    pub is_admin: bool,
    /// Client IP, kept with audit entries.
    pub ip: String,
}

impl Staff {
    /// The caller, who must be a moderator (or an admin).
    pub fn moderator(ctx: &RequestContext) -> AppResult<Self> {
        Self::with_role(ctx, "moderator")
    }

    /// The caller, who must be an admin.
    pub fn admin(ctx: &RequestContext) -> AppResult<Self> {
        Self::with_role(ctx, "admin")
    }

    fn with_role(ctx: &RequestContext, role: &str) -> AppResult<Self> {
        let auth = ctx.require_role(role)?;
        Ok(Self { id: auth.user_id, is_admin: auth.has_role("admin"), ip: ctx.client().ip })
    }

    /// Whether this staff member may sanction an account holding `roles`.
    pub fn may_sanction(&self, roles: &[String]) -> bool {
        self.is_admin || !is_staff(roles)
    }
}

/// Whether the roles include one that moderators must not act against.
pub(crate) fn is_staff(roles: &[String]) -> bool {
    roles.iter().any(|r| PROTECTED_ROLES.contains(&r.as_str()))
}

/// The reason shown to the affected user: trimmed, at most 500 characters.
pub(crate) fn reason_text(input: &str, required: bool) -> AppResult<String> {
    let reason = input.trim();
    ensure!(!(required && reason.is_empty()), invalid, "REASON_REQUIRED", "a reason is required for this action");
    ensure!(reason.chars().count() <= REASON_MAX_CHARS, invalid, "REASON_TOO_LONG", "the reason must be at most {REASON_MAX_CHARS} characters");
    Ok(reason.to_owned())
}

/// An internal note: trimmed, at most 2000 characters, may be empty.
pub(crate) fn note_text(input: &str) -> AppResult<String> {
    let note = input.trim();
    ensure!(note.chars().count() <= NOTE_MAX_CHARS, invalid, "NOTE_INVALID", "notes must be at most {NOTE_MAX_CHARS} characters");
    Ok(note.to_owned())
}

/// Length of a timed suspension: one hour to 365 days.
pub(crate) fn suspension_duration(seconds: i64) -> AppResult<chrono::Duration> {
    ensure!(
        (MIN_SUSPENSION_SECONDS..=MAX_SUSPENSION_SECONDS).contains(&seconds),
        invalid,
        "SUSPENSION_DURATION_INVALID",
        "a suspension lasts between 1 hour and 365 days"
    );
    Ok(chrono::Duration::seconds(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staff(is_admin: bool) -> Staff {
        Staff { id: Uuid::now_v7(), is_admin, ip: String::new() }
    }

    #[test]
    fn moderators_cannot_sanction_staff_but_admins_can() {
        let moderator_roles = vec!["moderator".to_owned()];
        let plain_roles = vec!["staff".to_owned()];
        assert!(!staff(false).may_sanction(&moderator_roles));
        assert!(!staff(false).may_sanction(&["admin".to_owned()]));
        assert!(staff(false).may_sanction(&plain_roles), "`staff` alone is not a protected role");
        assert!(staff(false).may_sanction(&[]));
        assert!(staff(true).may_sanction(&moderator_roles));
    }

    #[test]
    fn reasons_are_trimmed_and_bounded() {
        assert_eq!(reason_text("  スパム  ", true).unwrap(), "スパム");
        assert_eq!(reason_text("   ", true).unwrap_err().reason, "REASON_REQUIRED");
        assert_eq!(reason_text("", false).unwrap(), "");
        // The limit counts characters, not bytes.
        assert!(reason_text(&"あ".repeat(REASON_MAX_CHARS), true).is_ok());
        assert_eq!(reason_text(&"あ".repeat(REASON_MAX_CHARS + 1), true).unwrap_err().reason, "REASON_TOO_LONG");
    }

    #[test]
    fn notes_may_be_empty_but_not_huge() {
        assert_eq!(note_text(" ").unwrap(), "");
        assert_eq!(note_text(&"x".repeat(NOTE_MAX_CHARS + 1)).unwrap_err().reason, "NOTE_INVALID");
    }

    #[test]
    fn suspensions_last_an_hour_to_a_year() {
        assert!(suspension_duration(3600).is_ok());
        assert!(suspension_duration(365 * 24 * 3600).is_ok());
        for bad in [0, -5, 3599, 365 * 24 * 3600 + 1] {
            assert_eq!(suspension_duration(bad).unwrap_err().reason, "SUSPENSION_DURATION_INVALID");
        }
    }
}
