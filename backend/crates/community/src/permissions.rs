//! Roles and the permission table: the one place that says who may do what
//! in a community. Pure functions; `access` applies them to a caller.
//!
//! | permission | member | moderator | admin | owner |
//! |---|---|---|---|---|
//! | POST | ✓ | ✓ | ✓ | ✓ |
//! | CREATE_INVITE | if the community lets members | ✓ | ✓ | ✓ |
//! | CREATE_EVENT | if the community lets members | ✓ | ✓ | ✓ |
//! | MODERATE_CONTENT | | ✓ | ✓ | ✓ |
//! | MANAGE_MEMBERS | | ✓ | ✓ | ✓ |
//! | ANNOUNCE | | ✓ | ✓ | ✓ |
//! | REVEAL_AUTHOR | | ✓ | ✓ | ✓ |
//! | VIEW_MODERATION_LOG | | ✓ | ✓ | ✓ |
//! | MANAGE_SAFETY | | ✓ | ✓ | ✓ |
//! | MANAGE_CHANNELS | | | ✓ | ✓ |
//! | MANAGE_COMMUNITY | | | ✓ | ✓ |
//! | MANAGE_ROLES | | | ✓ | ✓ |
//! | TRANSFER_OWNERSHIP | | | | ✓ |
//! | DELETE_COMMUNITY | | | | ✓ |
//!
//! On top of the table, a member only ever acts on members of a lower role
//! ([`Role::outranks`]), and roles are handed out by [`may_assign`].
use tokuly_proto::community::{MemberRole, Permission};

/// A member's role. Ordered: a higher role includes everything a lower one may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Member = 1,
    Moderator = 2,
    Admin = 3,
    Owner = 4,
}

impl Role {
    /// The role stored as `code` (`MemberRole` as its number).
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            1 => Some(Self::Member),
            2 => Some(Self::Moderator),
            3 => Some(Self::Admin),
            4 => Some(Self::Owner),
            _ => None,
        }
    }

    pub fn code(self) -> i32 {
        self as i32
    }

    pub fn to_proto(self) -> MemberRole {
        match self {
            Self::Member => MemberRole::MEMBER_ROLE_MEMBER,
            Self::Moderator => MemberRole::MEMBER_ROLE_MODERATOR,
            Self::Admin => MemberRole::MEMBER_ROLE_ADMIN,
            Self::Owner => MemberRole::MEMBER_ROLE_OWNER,
        }
    }

    /// Moderators and up.
    pub fn is_staff(self) -> bool {
        self >= Self::Moderator
    }

    /// Whether a member of this role may sanction, remove or re-role a
    /// member of role `other`.
    pub fn outranks(self, other: Role) -> bool {
        self > other
    }

    /// Name used in moderation log details.
    pub fn name(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Moderator => "moderator",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }
}

/// The community switches that widen what plain members may do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemberGrants {
    pub invite: bool,
    pub create_events: bool,
}

/// Every permission, in the order of the table.
pub const ALL: [Permission; 14] = [
    Permission::PERMISSION_POST,
    Permission::PERMISSION_CREATE_INVITE,
    Permission::PERMISSION_CREATE_EVENT,
    Permission::PERMISSION_MODERATE_CONTENT,
    Permission::PERMISSION_MANAGE_MEMBERS,
    Permission::PERMISSION_ANNOUNCE,
    Permission::PERMISSION_REVEAL_AUTHOR,
    Permission::PERMISSION_VIEW_MODERATION_LOG,
    Permission::PERMISSION_MANAGE_SAFETY,
    Permission::PERMISSION_MANAGE_CHANNELS,
    Permission::PERMISSION_MANAGE_COMMUNITY,
    Permission::PERMISSION_MANAGE_ROLES,
    Permission::PERMISSION_TRANSFER_OWNERSHIP,
    Permission::PERMISSION_DELETE_COMMUNITY,
];

/// The lowest role holding `permission`, given the community's grants to
/// members. `None` for a permission nobody holds.
fn lowest_role(permission: Permission, grants: MemberGrants) -> Option<Role> {
    use Permission::*;
    Some(match permission {
        PERMISSION_POST => Role::Member,
        PERMISSION_CREATE_INVITE => {
            if grants.invite {
                Role::Member
            } else {
                Role::Moderator
            }
        }
        PERMISSION_CREATE_EVENT => {
            if grants.create_events {
                Role::Member
            } else {
                Role::Moderator
            }
        }
        PERMISSION_MODERATE_CONTENT
        | PERMISSION_MANAGE_MEMBERS
        | PERMISSION_ANNOUNCE
        | PERMISSION_REVEAL_AUTHOR
        | PERMISSION_VIEW_MODERATION_LOG
        | PERMISSION_MANAGE_SAFETY => Role::Moderator,
        PERMISSION_MANAGE_CHANNELS | PERMISSION_MANAGE_COMMUNITY | PERMISSION_MANAGE_ROLES => Role::Admin,
        PERMISSION_TRANSFER_OWNERSHIP | PERMISSION_DELETE_COMMUNITY => Role::Owner,
        PERMISSION_UNSPECIFIED => return None,
    })
}

/// Whether a member of `role` holds `permission`.
pub fn allows(role: Role, permission: Permission, grants: MemberGrants) -> bool {
    lowest_role(permission, grants).is_some_and(|needed| role >= needed)
}

/// Everything a member of `role` may do, in the order of the table.
pub fn permissions_of(role: Role, grants: MemberGrants) -> Vec<Permission> {
    ALL.into_iter().filter(|permission| allows(role, *permission, grants)).collect()
}

/// Whether `actor` may change a member from role `current` to role `new`
/// with `SetMemberRole`: admins appoint and dismiss moderators, the owner
/// also admins. Nobody is made owner this way (ownership is transferred),
/// and nobody touches a member they do not outrank.
pub fn may_assign(actor: Role, current: Role, new: Role) -> bool {
    actor >= Role::Admin && actor.outranks(current) && actor.outranks(new) && new != Role::Owner
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: MemberGrants = MemberGrants { invite: false, create_events: false };

    #[test]
    fn the_table_is_cumulative() {
        let roles = [Role::Member, Role::Moderator, Role::Admin, Role::Owner];
        for pair in roles.windows(2) {
            let lower = permissions_of(pair[0], NONE);
            let higher = permissions_of(pair[1], NONE);
            assert!(lower.iter().all(|p| higher.contains(p)), "{:?} loses a permission of {:?}", pair[1], pair[0]);
            assert!(higher.len() > lower.len());
        }
        assert_eq!(permissions_of(Role::Owner, NONE).len(), ALL.len());
    }

    #[test]
    fn members_post_and_nothing_else_by_default() {
        assert_eq!(permissions_of(Role::Member, NONE), vec![Permission::PERMISSION_POST]);
        let open = MemberGrants { invite: true, create_events: true };
        assert_eq!(
            permissions_of(Role::Member, open),
            vec![
                Permission::PERMISSION_POST,
                Permission::PERMISSION_CREATE_INVITE,
                Permission::PERMISSION_CREATE_EVENT
            ]
        );
    }

    #[test]
    fn each_tier_gets_its_own_tools() {
        use Permission::*;
        for permission in [
            PERMISSION_MODERATE_CONTENT,
            PERMISSION_MANAGE_MEMBERS,
            PERMISSION_ANNOUNCE,
            PERMISSION_REVEAL_AUTHOR,
            PERMISSION_VIEW_MODERATION_LOG,
            PERMISSION_MANAGE_SAFETY,
        ] {
            assert!(!allows(Role::Member, permission, NONE));
            assert!(allows(Role::Moderator, permission, NONE));
        }
        for permission in [PERMISSION_MANAGE_CHANNELS, PERMISSION_MANAGE_COMMUNITY, PERMISSION_MANAGE_ROLES] {
            assert!(!allows(Role::Moderator, permission, NONE));
            assert!(allows(Role::Admin, permission, NONE));
        }
        for permission in [PERMISSION_TRANSFER_OWNERSHIP, PERMISSION_DELETE_COMMUNITY] {
            assert!(!allows(Role::Admin, permission, NONE));
            assert!(allows(Role::Owner, permission, NONE));
        }
        assert!(!allows(Role::Owner, PERMISSION_UNSPECIFIED, NONE));
    }

    #[test]
    fn roles_are_assigned_downwards_only() {
        // Admins appoint and dismiss moderators.
        assert!(may_assign(Role::Admin, Role::Member, Role::Moderator));
        assert!(may_assign(Role::Admin, Role::Moderator, Role::Member));
        // …but not admins, and they do not touch other admins.
        assert!(!may_assign(Role::Admin, Role::Member, Role::Admin));
        assert!(!may_assign(Role::Admin, Role::Admin, Role::Member));
        // The owner appoints and dismisses admins.
        assert!(may_assign(Role::Owner, Role::Member, Role::Admin));
        assert!(may_assign(Role::Owner, Role::Admin, Role::Moderator));
        // Nobody becomes owner through a role change; moderators assign nothing.
        assert!(!may_assign(Role::Owner, Role::Admin, Role::Owner));
        assert!(!may_assign(Role::Moderator, Role::Member, Role::Moderator));
    }

    #[test]
    fn codes_round_trip() {
        for role in [Role::Member, Role::Moderator, Role::Admin, Role::Owner] {
            assert_eq!(Role::from_code(role.code()), Some(role));
            assert_eq!(role.to_proto() as i32, role.code());
        }
        assert_eq!(Role::from_code(0), None);
        assert_eq!(Role::from_code(5), None);
    }
}
