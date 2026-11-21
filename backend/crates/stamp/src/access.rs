//! Who may see, send and edit what. Every rule of the domain lives here as a
//! pure function, so handlers only load the facts and ask.
use connectrpc::RequestContext;
use tokuly_core::prelude::*;

use crate::model::status;

/// The caller of an RPC, or the user an internal API call is made for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Viewer {
    /// `None` for anonymous callers.
    pub user_id: Option<Uuid>,
    /// Holds the `moderator` role (admins do too): sees every pack.
    pub moderator: bool,
    /// Holds the `admin` role: manages official packs.
    pub admin: bool,
}

impl Viewer {
    /// A signed-in user without staff roles. Used by the cross-domain API,
    /// which knows users only by ID.
    pub fn user(user_id: Uuid) -> Self {
        Self { user_id: Some(user_id), moderator: false, admin: false }
    }

    fn from_auth(auth: &AuthContext) -> Self {
        Self { user_id: Some(auth.user_id), moderator: auth.has_role("moderator"), admin: auth.has_role("admin") }
    }

    /// The caller of a public RPC: signed in or anonymous.
    pub fn of(ctx: &RequestContext) -> AppResult<Self> {
        Ok(ctx.maybe_auth()?.map(Self::from_auth).unwrap_or_default())
    }

    /// The caller of an RPC that requires sign-in, with their user ID.
    pub fn signed_in(ctx: &RequestContext) -> AppResult<(Self, Uuid)> {
        let auth = ctx.auth()?;
        Ok((Self::from_auth(auth), auth.user_id))
    }
}

/// The properties of a pack that decide what a viewer may do with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackFacts {
    /// `tokuly.stamp.v1.PackStatus`.
    pub status: i32,
    pub price: i32,
    /// `None` for official packs.
    pub creator_id: Option<Uuid>,
    pub official: bool,
}

/// What a viewer holds of a pack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Holding {
    pub installed: bool,
    pub owned: bool,
}

impl Viewer {
    /// May edit the pack: its creator, or an admin for official packs.
    pub fn manages(&self, pack: &PackFacts) -> bool {
        match (self.user_id, pack.creator_id) {
            (Some(user), Some(creator)) => user == creator,
            (Some(_), None) => pack.official && self.admin,
            (None, _) => false,
        }
    }

    /// Sees stamps that have not passed review, and the review state.
    pub fn sees_unapproved(&self, pack: &PackFacts) -> bool {
        self.manages(pack) || self.moderator
    }

    /// May open the pack: published packs are public; anything else is for
    /// the people who manage or hold it, and for moderators. A pack that was
    /// taken down is gone for everyone but its managers and moderators.
    pub fn can_view(&self, pack: &PackFacts, holding: Holding) -> bool {
        if self.sees_unapproved(pack) {
            return true;
        }
        match pack.status {
            status::PUBLISHED => true,
            status::REMOVED => false,
            _ => holding.installed || holding.owned,
        }
    }

    /// May send a stamp of the pack. `approved` is whether that stamp passed
    /// review: only managers may send stamps that did not.
    pub fn can_use(&self, pack: &PackFacts, holding: Holding, approved: bool) -> bool {
        if self.user_id.is_none() || pack.status == status::REMOVED {
            return false;
        }
        if self.manages(pack) {
            return true;
        }
        let free_in_store = pack.status == status::PUBLISHED && pack.price == 0;
        approved && (free_in_store || holding.installed || holding.owned)
    }

    /// May add the pack to their library without paying.
    pub fn can_install(&self, pack: &PackFacts, holding: Holding) -> InstallCheck {
        if pack.status == status::REMOVED {
            return InstallCheck::Unavailable;
        }
        if self.manages(pack) || holding.owned || holding.installed {
            return InstallCheck::Allowed;
        }
        match (pack.status == status::PUBLISHED, pack.price == 0) {
            (true, true) => InstallCheck::Allowed,
            (true, false) => InstallCheck::PurchaseRequired,
            (false, _) => InstallCheck::Unavailable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallCheck {
    Allowed,
    /// A paid pack the viewer has not bought.
    PurchaseRequired,
    /// Not in the store (or taken down) and not held by the viewer.
    Unavailable,
}

/// Listing and stamp text can change in every status but IN_REVIEW, so that
/// moderators judge exactly what will be published.
pub fn ensure_editable(pack_status: i32) -> AppResult<()> {
    ensure!(
        pack_status != status::IN_REVIEW,
        precondition,
        "PACK_LOCKED",
        "the pack is in review; withdraw it to make changes"
    );
    Ok(())
}

/// New stamps must be reviewed before other people see them. A published
/// pack therefore only accepts stamps from creators whose stamps need no
/// review (`auto_approved`); everyone else unpublishes first.
pub fn ensure_can_add_stamps(pack_status: i32, auto_approved: bool) -> AppResult<()> {
    ensure_editable(pack_status)?;
    ensure!(
        pack_status != status::PUBLISHED || auto_approved,
        precondition,
        "PACK_LOCKED",
        "unpublish the pack to add stamps; they are reviewed when you submit it again"
    );
    Ok(())
}

/// Whether submitting a pack sends it to a moderator instead of publishing it.
///
/// Official packs never need one. A pack that was taken down always does,
/// even for trusted creators. Otherwise a moderator is needed when the
/// creator is not trusted and something changed since the last approval.
pub fn submission_needs_moderator(pack: &PackFacts, needs_review: bool, creator_trusted: bool) -> bool {
    if pack.official {
        return false;
    }
    pack.status == status::REMOVED || (needs_review && !creator_trusted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(pack_status: i32, price: i32, creator: Option<Uuid>) -> PackFacts {
        PackFacts { status: pack_status, price, creator_id: creator, official: creator.is_none() }
    }

    const NOTHING: Holding = Holding { installed: false, owned: false };
    const INSTALLED: Holding = Holding { installed: true, owned: false };
    const OWNED: Holding = Holding { installed: false, owned: true };

    #[test]
    fn creators_manage_their_packs_and_admins_the_official_ones() {
        let creator = Uuid::now_v7();
        let other = Viewer::user(Uuid::now_v7());
        let admin = Viewer { admin: true, moderator: true, ..other };
        let moderator = Viewer { moderator: true, ..other };
        let user_pack = pack(status::DRAFT, 0, Some(creator));
        let official = pack(status::DRAFT, 0, None);

        assert!(Viewer::user(creator).manages(&user_pack));
        assert!(!other.manages(&user_pack));
        assert!(!admin.manages(&user_pack), "admins moderate user packs, they do not edit them");
        assert!(admin.manages(&official));
        assert!(!moderator.manages(&official));
        assert!(!Viewer::default().manages(&official));
        assert!(moderator.sees_unapproved(&user_pack) && !other.sees_unapproved(&user_pack));
    }

    #[test]
    fn unpublished_packs_are_visible_only_to_people_who_hold_them() {
        let creator = Uuid::now_v7();
        let stranger = Viewer::user(Uuid::now_v7());
        let anonymous = Viewer::default();
        let published = pack(status::PUBLISHED, 100, Some(creator));
        let draft = pack(status::DRAFT, 0, Some(creator));
        let removed = pack(status::REMOVED, 0, Some(creator));

        assert!(anonymous.can_view(&published, NOTHING));
        assert!(!stranger.can_view(&draft, NOTHING));
        assert!(stranger.can_view(&draft, INSTALLED) && stranger.can_view(&draft, OWNED));
        assert!(Viewer::user(creator).can_view(&draft, NOTHING));
        assert!(!stranger.can_view(&removed, OWNED), "a takedown hides the pack from its holders too");
        assert!(Viewer::user(creator).can_view(&removed, NOTHING));
        assert!(Viewer { moderator: true, ..stranger }.can_view(&removed, NOTHING));
    }

    #[test]
    fn stamps_are_usable_when_free_installed_owned_or_your_own() {
        let creator = Uuid::now_v7();
        let user = Viewer::user(Uuid::now_v7());
        let free = pack(status::PUBLISHED, 0, Some(creator));
        let paid = pack(status::PUBLISHED, 100, Some(creator));
        let unpublished = pack(status::DRAFT, 0, Some(creator));

        assert!(user.can_use(&free, NOTHING, true), "free published packs need no install");
        assert!(!user.can_use(&paid, NOTHING, true));
        assert!(user.can_use(&paid, OWNED, true));
        assert!(user.can_use(&paid, INSTALLED, true), "installed while it was free");
        assert!(!user.can_use(&unpublished, NOTHING, true));
        assert!(user.can_use(&unpublished, INSTALLED, true), "holders keep an unpublished pack");
        assert!(!Viewer::default().can_use(&free, NOTHING, true), "anonymous callers send nothing");
    }

    #[test]
    fn unreviewed_stamps_are_usable_only_by_managers() {
        let creator = Uuid::now_v7();
        let user = Viewer::user(Uuid::now_v7());
        let free = pack(status::PUBLISHED, 0, Some(creator));
        assert!(!user.can_use(&free, INSTALLED, false));
        assert!(!user.can_use(&free, OWNED, false));
        assert!(Viewer::user(creator).can_use(&pack(status::DRAFT, 0, Some(creator)), NOTHING, false));
    }

    #[test]
    fn a_takedown_makes_stamps_unusable_for_everyone() {
        let creator = Uuid::now_v7();
        let removed = pack(status::REMOVED, 0, Some(creator));
        assert!(!Viewer::user(creator).can_use(&removed, INSTALLED, true));
        assert!(!Viewer::user(Uuid::now_v7()).can_use(&removed, OWNED, true));
    }

    #[test]
    fn installing_needs_a_purchase_for_paid_packs_only() {
        let creator = Uuid::now_v7();
        let user = Viewer::user(Uuid::now_v7());
        assert_eq!(user.can_install(&pack(status::PUBLISHED, 0, Some(creator)), NOTHING), InstallCheck::Allowed);
        assert_eq!(
            user.can_install(&pack(status::PUBLISHED, 50, Some(creator)), NOTHING),
            InstallCheck::PurchaseRequired
        );
        assert_eq!(user.can_install(&pack(status::PUBLISHED, 50, Some(creator)), OWNED), InstallCheck::Allowed);
        assert_eq!(user.can_install(&pack(status::DRAFT, 0, Some(creator)), NOTHING), InstallCheck::Unavailable);
        assert_eq!(
            user.can_install(&pack(status::DRAFT, 50, Some(creator)), OWNED),
            InstallCheck::Allowed,
            "re-install after unpublish"
        );
        assert_eq!(user.can_install(&pack(status::REMOVED, 0, Some(creator)), OWNED), InstallCheck::Unavailable);
        assert_eq!(
            Viewer::user(creator).can_install(&pack(status::DRAFT, 50, Some(creator)), NOTHING),
            InstallCheck::Allowed
        );
    }

    #[test]
    fn packs_in_review_are_locked_and_published_packs_take_only_pre_approved_stamps() {
        assert_eq!(ensure_editable(status::IN_REVIEW).unwrap_err().reason, "PACK_LOCKED");
        for open in [status::DRAFT, status::PUBLISHED, status::REJECTED, status::REMOVED] {
            assert!(ensure_editable(open).is_ok());
        }
        assert_eq!(ensure_can_add_stamps(status::PUBLISHED, false).unwrap_err().reason, "PACK_LOCKED");
        assert!(ensure_can_add_stamps(status::PUBLISHED, true).is_ok());
        assert!(ensure_can_add_stamps(status::DRAFT, false).is_ok());
        assert_eq!(ensure_can_add_stamps(status::IN_REVIEW, true).unwrap_err().reason, "PACK_LOCKED");
    }

    #[test]
    fn review_is_skipped_for_official_trusted_and_unchanged_packs() {
        let creator = Some(Uuid::now_v7());
        let draft = pack(status::DRAFT, 0, creator);
        assert!(submission_needs_moderator(&draft, true, false));
        assert!(!submission_needs_moderator(&draft, true, true), "trusted creator");
        assert!(!submission_needs_moderator(&draft, false, false), "nothing changed since the last approval");
        assert!(!submission_needs_moderator(&pack(status::DRAFT, 0, None), true, false), "official");
        assert!(
            submission_needs_moderator(&pack(status::REMOVED, 0, creator), false, true),
            "a takedown is always reviewed"
        );
    }
}
