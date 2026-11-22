//! Who is asking, and what they are to a community.
//!
//! Every RPC starts by loading an [`Access`]: the community, the caller's
//! membership (or ban, or pending request) and whether the caller acts for
//! site moderation. All authorization questions are methods on it, and they
//! all reduce to the table in [`crate::permissions`].
//!
//! Three levels of reading:
//! - **knowing the community exists** — everyone for public and restricted
//!   communities, members only for hidden ones (`COMMUNITY_NOT_FOUND`);
//! - **reading its content** (boards, events) — everyone for public
//!   communities, members only otherwise (`NOT_A_MEMBER`);
//! - **the member directory and anything personal** — members only.
//!
//! Accounts holding the site-wide `moderator` role read everything, so a
//! report about a post in a hidden community can be handled; they hold no
//! community permission through that role except looking up an anonymous
//! author.
use std::collections::{HashMap, HashSet};

use connectrpc::RequestContext;
use tokuly_core::prelude::*;
use tokuly_proto::community::{self as pb, MembershipStatus, Permission};
use tokuly_social::graph;

use crate::{
    Community,
    model::{BanRow, CommunityRow, MemberRow, community_columns, list_from_json, member_columns, state},
    permissions::{self, Role},
};

pub fn community_not_found() -> AppError {
    AppError::not_found("COMMUNITY_NOT_FOUND", "community not found")
}

pub fn not_a_member() -> AppError {
    AppError::denied("NOT_A_MEMBER", "only members of the community can do this")
}

pub fn permission_denied() -> AppError {
    AppError::denied("PERMISSION_DENIED", "your role in the community does not allow this")
}

/// The site-wide role that may read every community.
const SITE_MODERATOR_ROLE: &str = "moderator";

/// Who is asking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Caller {
    /// `None` for anonymous callers.
    pub user_id: Option<Uuid>,
    /// The caller holds the site-wide moderator (or admin) role.
    pub site_moderator: bool,
}

impl Caller {
    /// The caller of a request that also works anonymously.
    pub fn of(ctx: &RequestContext) -> AppResult<Self> {
        Ok(match ctx.maybe_auth()? {
            Some(auth) => Self { user_id: Some(auth.user_id), site_moderator: auth.has_role(SITE_MODERATOR_ROLE) },
            None => Self::default(),
        })
    }

    /// The caller of a request that needs a signed-in user.
    pub fn signed_in(ctx: &RequestContext) -> AppResult<(Self, Uuid)> {
        let auth = ctx.auth()?;
        Ok((Self { user_id: Some(auth.user_id), site_moderator: auth.has_role(SITE_MODERATOR_ROLE) }, auth.user_id))
    }

    /// A member acting without site-wide powers (background work).
    pub fn user(user_id: Uuid) -> Self {
        Self { user_id: Some(user_id), site_moderator: false }
    }

    /// A viewer known only by ID (subject resolvers, the topic guard): the
    /// site-wide role is read from the account.
    pub async fn load(core: &Core, user_id: Option<Uuid>) -> AppResult<Self> {
        let Some(user_id) = user_id else { return Ok(Self::default()) };
        let site_moderator: Option<bool> = sqlx::query_scalar(
            "SELECT roles && ARRAY['admin', 'moderator'] FROM users
             WHERE id = $1 AND deleted_at IS NULL AND banned_at IS NULL",
        )
        .bind(user_id)
        .fetch_optional(&core.db)
        .await?;
        Ok(Self { user_id: Some(user_id), site_moderator: site_moderator.unwrap_or(false) })
    }
}

/// A community as one caller stands to it.
#[derive(Debug, Clone)]
pub struct Access {
    pub community: CommunityRow,
    pub user_id: Option<Uuid>,
    pub site_moderator: bool,
    pub member: Option<MemberRow>,
    /// A ban in force. Banned users are never members.
    pub ban: Option<BanRow>,
    /// The caller asked to join and has no answer (declining is silent).
    pub requested: bool,
}

impl Access {
    pub fn status(&self) -> MembershipStatus {
        if self.member.is_some() {
            MembershipStatus::MEMBERSHIP_STATUS_MEMBER
        } else if self.ban.is_some() {
            MembershipStatus::MEMBERSHIP_STATUS_BANNED
        } else if self.requested {
            MembershipStatus::MEMBERSHIP_STATUS_REQUESTED
        } else {
            MembershipStatus::MEMBERSHIP_STATUS_NONE
        }
    }

    pub fn role(&self) -> Option<Role> {
        self.member.as_ref().map(MemberRow::role)
    }

    pub fn is_member(&self) -> bool {
        self.member.is_some()
    }

    pub fn is_staff(&self) -> bool {
        self.role().is_some_and(Role::is_staff)
    }

    /// Whether the caller holds `permission` as a member (the table).
    pub fn has(&self, permission: Permission) -> bool {
        self.role().is_some_and(|role| permissions::allows(role, permission, self.community.grants()))
    }

    pub fn require(&self, permission: Permission) -> AppResult<()> {
        if self.has(permission) { Ok(()) } else { Err(permission_denied()) }
    }

    /// Whether the caller may read boards and events.
    pub fn can_read(&self) -> bool {
        self.community.is_public() || self.is_member() || self.site_moderator
    }

    pub fn require_read(&self) -> AppResult<()> {
        if self.can_read() { Ok(()) } else { Err(not_a_member()) }
    }

    pub fn require_member(&self) -> AppResult<&MemberRow> {
        self.member.as_ref().ok_or_else(not_a_member)
    }

    /// When the caller's timeout ends, while one is running.
    pub fn timeout(&self) -> Option<Time> {
        self.member.as_ref().and_then(|member| member.timeout(now()))
    }

    /// A member who is not timed out: may post, vote and RSVP.
    pub fn require_active(&self) -> AppResult<&MemberRow> {
        let member = self.require_member()?;
        ensure!(member.timeout(now()).is_none(), denied, "TIMED_OUT", "you are timed out in this community");
        Ok(member)
    }

    /// Whether the caller reviews hidden posts.
    pub fn moderates(&self) -> bool {
        self.has(Permission::PERMISSION_MODERATE_CONTENT)
    }

    /// Whether hidden posts of other people are shown to the caller.
    pub fn sees_hidden_posts(&self) -> bool {
        self.moderates() || self.site_moderator
    }

    /// The caller's relation to the community, without the counters that
    /// cost a query (see [`Community::render_community`]).
    pub fn viewer(&self) -> Option<pb::CommunityViewer> {
        self.user_id?;
        let mut viewer = pb::CommunityViewer { status: self.status().into(), ..Default::default() };
        if let Some(member) = &self.member {
            let role = member.role();
            viewer.role = role.to_proto().into();
            viewer.permissions =
                permissions::permissions_of(role, self.community.grants()).into_iter().map(Into::into).collect();
            viewer.notification_level = member.notification_level.into();
            viewer.show_on_profile = member.show_on_profile && self.community.is_public();
            viewer.joined_at = ts(member.joined_at);
            viewer.timeout_until = opt_ts(member.timeout(now()));
        }
        if let Some(ban) = &self.ban {
            viewer.ban_reason = ban.reason.clone();
            viewer.ban_expires_at = opt_ts(ban.expires_at);
        }
        Some(viewer)
    }
}

/// A community reference as clients send it: the UUID or the handle.
enum CommunityRef {
    Id(Uuid),
    Handle(String),
}

fn parse_ref(reference: &str) -> Option<CommunityRef> {
    let reference = reference.trim();
    if let Ok(id) = Uuid::parse_str(reference) {
        return Some(CommunityRef::Id(id));
    }
    let handle = reference.trim_start_matches(['+', '@']).to_ascii_lowercase();
    let plausible = (3..=64).contains(&handle.len())
        && handle.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    plausible.then_some(CommunityRef::Handle(handle))
}

impl Community {
    /// The community named by a UUID or a handle; closed ones do not exist.
    pub(crate) async fn find_community(&self, reference: &str) -> AppResult<Option<CommunityRow>> {
        match parse_ref(reference) {
            Some(CommunityRef::Id(id)) => self.community_by_id(id).await,
            Some(CommunityRef::Handle(handle)) => Ok(sqlx::query_as(concat!(
                "SELECT ",
                community_columns!(),
                " FROM communities c WHERE c.handle = $1::citext AND c.deleted_at IS NULL"
            ))
            .bind(handle)
            .fetch_optional(&self.core.db)
            .await?),
            None => Ok(None),
        }
    }

    pub(crate) async fn community_by_id(&self, id: Uuid) -> AppResult<Option<CommunityRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            community_columns!(),
            " FROM communities c WHERE c.id = $1 AND c.deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// The community with this ID, a closed one included for as long as
    /// its content is retained: what was posted there stays reportable.
    pub(crate) async fn community_retained(&self, id: Uuid) -> AppResult<Option<CommunityRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            community_columns!(),
            " FROM communities c WHERE c.id = $1 AND c.purged_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// [`Self::community_retained`] by UUID. A closed community has given
    /// up its handle, so only its ID still names it.
    pub(crate) async fn find_retained_community(&self, reference: &str) -> AppResult<Option<CommunityRow>> {
        match Uuid::parse_str(reference.trim()) {
            Ok(id) => self.community_retained(id).await,
            Err(_) => Ok(None),
        }
    }

    pub(crate) async fn member(&self, community_id: Uuid, user_id: Uuid) -> AppResult<Option<MemberRow>> {
        Ok(sqlx::query_as(concat!(
            "SELECT ",
            member_columns!(),
            " FROM community_members m WHERE m.community_id = $1 AND m.user_id = $2"
        ))
        .bind(community_id)
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// The ban of `user_id` in force, if any.
    pub(crate) async fn ban(&self, community_id: Uuid, user_id: Uuid) -> AppResult<Option<BanRow>> {
        Ok(sqlx::query_as(
            "SELECT community_id, user_id, reason, banned_by, expires_at, created_at FROM community_bans
             WHERE community_id = $1 AND user_id = $2 AND (expires_at IS NULL OR expires_at > now())",
        )
        .bind(community_id)
        .bind(user_id)
        .fetch_optional(&self.core.db)
        .await?)
    }

    /// What `caller` is to `community`, without judging whether they may
    /// know it exists (invites lead into hidden communities).
    pub(crate) async fn stance(&self, caller: &Caller, community: CommunityRow) -> AppResult<Access> {
        let mut access = Access {
            community,
            user_id: caller.user_id,
            site_moderator: caller.site_moderator,
            member: None,
            ban: None,
            requested: false,
        };
        if let Some(user_id) = caller.user_id {
            access.member = self.member(access.community.id, user_id).await?;
            if access.member.is_none() {
                access.ban = self.ban(access.community.id, user_id).await?;
                access.requested = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM community_join_requests WHERE community_id = $1 AND user_id = $2)",
                )
                .bind(access.community.id)
                .bind(user_id)
                .fetch_one(&self.core.db)
                .await?;
            }
        }
        Ok(access)
    }

    /// [`Self::stance`] for a community the caller may know about: a hidden
    /// community does not exist for outsiders.
    pub(crate) async fn access_to(&self, caller: &Caller, community: CommunityRow) -> AppResult<Access> {
        let access = self.stance(caller, community).await?;
        if access.community.is_hidden() && !access.is_member() && !access.site_moderator {
            return Err(community_not_found());
        }
        Ok(access)
    }

    /// The community named by `reference` as `caller` stands to it.
    pub(crate) async fn access(&self, caller: &Caller, reference: &str) -> AppResult<Access> {
        let community = self.find_community(reference).await?.ok_or_else(community_not_found)?;
        self.access_to(caller, community).await
    }

    pub(crate) async fn access_by_id(&self, caller: &Caller, community_id: Uuid) -> AppResult<Access> {
        let community = self.community_by_id(community_id).await?.ok_or_else(community_not_found)?;
        self.access_to(caller, community).await
    }

    /// Everyone hidden from `user_id` by a block, in either direction.
    pub(crate) async fn blocked(&self, user_id: Option<Uuid>) -> AppResult<Vec<Uuid>> {
        match user_id {
            Some(user_id) => graph::blocked_ids_either(&self.core, user_id).await,
            None => Ok(Vec::new()),
        }
    }

    /// The members holding a staff role (moderators and up).
    pub(crate) async fn staff_ids(&self, community_id: Uuid) -> AppResult<Vec<Uuid>> {
        Ok(sqlx::query_scalar("SELECT user_id FROM community_members WHERE community_id = $1 AND role >= 2")
            .bind(community_id)
            .fetch_all(&self.core.db)
            .await?)
    }

    /// Join requests waiting for a decision.
    pub(crate) async fn pending_request_count(&self, community_id: Uuid) -> AppResult<i32> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM community_join_requests WHERE community_id = $1 AND declined_at IS NULL",
        )
        .bind(community_id)
        .fetch_one(&self.core.db)
        .await?;
        Ok(count as i32)
    }

    /// Hidden posts waiting for review.
    pub(crate) async fn review_queue_count(&self, community_id: Uuid) -> AppResult<i32> {
        let count: i64 = sqlx::query_scalar(
            "SELECT (SELECT count(*) FROM community_threads WHERE community_id = $1 AND state = $2 AND deleted_at IS NULL)
                  + (SELECT count(*) FROM community_replies r JOIN community_threads t ON t.id = r.thread_id
                     WHERE r.community_id = $1 AND r.state = $2 AND r.deleted_at IS NULL AND t.deleted_at IS NULL)",
        )
        .bind(community_id)
        .bind(state::HIDDEN)
        .fetch_one(&self.core.db)
        .await?;
        Ok(count as i32)
    }

    /// The communities among `community_ids` in which `user_id` has unread threads.
    async fn unread_communities(&self, user_id: Uuid, community_ids: &[Uuid]) -> AppResult<HashSet<Uuid>> {
        if community_ids.is_empty() {
            return Ok(HashSet::new());
        }
        let unread: Vec<Uuid> = sqlx::query_scalar(
            "SELECT m.community_id FROM community_members m
             WHERE m.user_id = $1 AND m.community_id = ANY($2) AND EXISTS (
                 SELECT 1 FROM community_threads t
                 LEFT JOIN community_thread_reads tr ON tr.thread_id = t.id AND tr.user_id = m.user_id
                 LEFT JOIN community_channel_reads cr ON cr.channel_id = t.channel_id AND cr.user_id = m.user_id
                 WHERE t.community_id = m.community_id AND t.deleted_at IS NULL AND t.state = 1
                   AND t.last_activity_at > greatest(m.joined_at, coalesce(m.read_all_at, m.joined_at))
                   AND (tr.read_at IS NULL OR t.last_activity_at > tr.read_at)
                   AND (cr.read_at IS NULL OR t.last_activity_at > cr.read_at))",
        )
        .bind(user_id)
        .bind(community_ids)
        .fetch_all(&self.core.db)
        .await?;
        Ok(unread.into_iter().collect())
    }

    fn community_proto(
        &self,
        community: &CommunityRow,
        owner: tokuly_proto::common::UserSummary,
        viewer: Option<pb::CommunityViewer>,
    ) -> pb::Community {
        pb::Community {
            id: community.id.to_string(),
            handle: community.handle.clone(),
            name: community.name.clone(),
            description: community.description.clone(),
            icon: community.icon().into(),
            banner: tokuly_media::assets::image_from_json(community.banner.as_ref()).into(),
            rules: list_from_json(&community.rules),
            tags: community.tags.clone(),
            category: community.category.into(),
            language: community.language.clone(),
            visibility: community.visibility.into(),
            school_name: community.school_name.clone(),
            member_count: community.member_count,
            thread_count: community.thread_count,
            owner: owner.into(),
            join_questions: community.join_questions.clone(),
            settings: pb::CommunitySettings {
                members_can_invite: community.members_can_invite,
                members_can_create_events: community.members_can_create_events,
                ..Default::default()
            }
            .into(),
            created_at: ts(community.created_at),
            last_activity_at: ts(community.last_activity_at),
            max_members: self.cfg.max_members as i32,
            viewer: viewer.into(),
            ..Default::default()
        }
    }

    /// One community with everything its page shows, including the unread
    /// marker and (for the staff) the queues waiting for them.
    pub(crate) async fn render_community(&self, access: &Access) -> AppResult<pb::Community> {
        let community = &access.community;
        let owner = tokuly_identity::summary(&self.core, community.owner_id).await?;
        let mut viewer = access.viewer();
        if let (Some(viewer), Some(member)) = (viewer.as_mut(), &access.member) {
            viewer.has_unread = self.unread_communities(member.user_id, &[community.id]).await?.contains(&community.id);
            if access.has(Permission::PERMISSION_MANAGE_MEMBERS) {
                viewer.pending_request_count = self.pending_request_count(community.id).await?;
            }
            if access.moderates() {
                viewer.review_queue_count = self.review_queue_count(community.id).await?;
            }
        }
        Ok(self.community_proto(community, owner, viewer))
    }

    /// Communities for a list, each with the caller's relation to it. The
    /// rows must be communities the caller may know about.
    pub(crate) async fn render_communities(
        &self,
        caller: &Caller,
        rows: &[CommunityRow],
    ) -> AppResult<Vec<pb::Community>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let core = &self.core;
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let owner_ids: Vec<Uuid> = rows.iter().map(|row| row.owner_id).collect();
        let owners = tokuly_identity::summaries(core, &owner_ids).await?;

        let mut members: HashMap<Uuid, MemberRow> = HashMap::new();
        let mut bans: HashMap<Uuid, BanRow> = HashMap::new();
        let mut requested: HashSet<Uuid> = HashSet::new();
        let mut unread: HashSet<Uuid> = HashSet::new();
        if let Some(user_id) = caller.user_id {
            let rows: Vec<MemberRow> = sqlx::query_as(concat!(
                "SELECT ",
                member_columns!(),
                " FROM community_members m WHERE m.user_id = $1 AND m.community_id = ANY($2)"
            ))
            .bind(user_id)
            .bind(&ids)
            .fetch_all(&core.db)
            .await?;
            members = rows.into_iter().map(|row| (row.community_id, row)).collect();
            let rows: Vec<BanRow> = sqlx::query_as(
                "SELECT community_id, user_id, reason, banned_by, expires_at, created_at FROM community_bans
                 WHERE user_id = $1 AND community_id = ANY($2) AND (expires_at IS NULL OR expires_at > now())",
            )
            .bind(user_id)
            .bind(&ids)
            .fetch_all(&core.db)
            .await?;
            bans = rows.into_iter().map(|row| (row.community_id, row)).collect();
            let rows: Vec<Uuid> = sqlx::query_scalar(
                "SELECT community_id FROM community_join_requests WHERE user_id = $1 AND community_id = ANY($2)",
            )
            .bind(user_id)
            .bind(&ids)
            .fetch_all(&core.db)
            .await?;
            requested = rows.into_iter().collect();
            let member_of: Vec<Uuid> = members.keys().copied().collect();
            unread = self.unread_communities(user_id, &member_of).await?;
        }

        Ok(rows
            .iter()
            .map(|row| {
                let access = Access {
                    community: row.clone(),
                    user_id: caller.user_id,
                    site_moderator: caller.site_moderator,
                    member: members.get(&row.id).cloned(),
                    ban: bans.get(&row.id).cloned(),
                    requested: requested.contains(&row.id),
                };
                let mut viewer = access.viewer();
                if let Some(viewer) = viewer.as_mut() {
                    viewer.has_unread = unread.contains(&row.id);
                }
                self.community_proto(row, tokuly_identity::users::pick(&owners, row.owner_id), viewer)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{level, visibility};

    fn community(visibility: i32) -> CommunityRow {
        CommunityRow {
            id: Uuid::now_v7(),
            handle: "club".into(),
            name: "Club".into(),
            description: String::new(),
            icon: None,
            icon_asset_id: None,
            banner: None,
            banner_asset_id: None,
            rules: serde_json::json!([]),
            tags: Vec::new(),
            category: 2,
            language: "ja".into(),
            visibility,
            school_name: String::new(),
            join_questions: Vec::new(),
            members_can_invite: false,
            members_can_create_events: false,
            muted_words: Vec::new(),
            auto_hide_reports: 0,
            owner_id: Uuid::now_v7(),
            member_count: 1,
            thread_count: 0,
            last_activity_at: now(),
            deleted_at: None,
            created_at: now(),
        }
    }

    fn access(visibility: i32, role: Option<i32>) -> Access {
        let community = community(visibility);
        let user_id = Uuid::now_v7();
        Access {
            member: role.map(|role| MemberRow {
                community_id: community.id,
                user_id,
                role,
                notification_level: level::ALL,
                show_on_profile: true,
                timeout_until: None,
                timeout_reason: String::new(),
                read_all_at: None,
                joined_at: now(),
            }),
            community,
            user_id: Some(user_id),
            site_moderator: false,
            ban: None,
            requested: false,
        }
    }

    #[test]
    fn outsiders_read_public_communities_only() {
        assert!(access(visibility::PUBLIC, None).can_read());
        assert!(!access(visibility::RESTRICTED, None).can_read());
        assert!(!access(visibility::HIDDEN, None).can_read());
        assert!(access(visibility::RESTRICTED, Some(1)).can_read());
        assert_eq!(access(visibility::RESTRICTED, None).require_read().unwrap_err().reason, "NOT_A_MEMBER");
    }

    #[test]
    fn site_moderators_read_everything_but_hold_no_community_permission() {
        let mut outsider = access(visibility::HIDDEN, None);
        outsider.site_moderator = true;
        assert!(outsider.can_read() && outsider.sees_hidden_posts());
        assert!(!outsider.has(Permission::PERMISSION_MODERATE_CONTENT));
        assert!(!outsider.has(Permission::PERMISSION_POST));
        assert_eq!(outsider.require_member().unwrap_err().reason, "NOT_A_MEMBER");
    }

    #[test]
    fn permissions_follow_the_role() {
        let member = access(visibility::PUBLIC, Some(1));
        assert!(member.has(Permission::PERMISSION_POST) && !member.moderates());
        assert_eq!(member.require(Permission::PERMISSION_MANAGE_MEMBERS).unwrap_err().reason, "PERMISSION_DENIED");
        let moderator = access(visibility::PUBLIC, Some(2));
        assert!(moderator.moderates() && moderator.is_staff());
        assert!(moderator.require(Permission::PERMISSION_MANAGE_CHANNELS).is_err());
        assert!(access(visibility::PUBLIC, Some(3)).require(Permission::PERMISSION_MANAGE_CHANNELS).is_ok());
    }

    #[test]
    fn a_timeout_stops_posting_but_not_membership() {
        let mut member = access(visibility::PUBLIC, Some(1));
        assert!(member.require_active().is_ok());
        member.member.as_mut().unwrap().timeout_until = Some(now() + chrono::Duration::hours(1));
        assert_eq!(member.require_active().unwrap_err().reason, "TIMED_OUT");
        assert!(member.require_member().is_ok());
        assert!(member.viewer().unwrap().timeout_until.as_option().is_some());
    }

    #[test]
    fn the_viewer_block_tells_status_and_permissions() {
        let outsider = access(visibility::PUBLIC, None);
        let viewer = outsider.viewer().unwrap();
        assert_eq!(viewer.status, MembershipStatus::MEMBERSHIP_STATUS_NONE);
        assert!(viewer.permissions.is_empty());

        let owner = access(visibility::PUBLIC, Some(4)).viewer().unwrap();
        assert_eq!(owner.status, MembershipStatus::MEMBERSHIP_STATUS_MEMBER);
        assert_eq!(owner.permissions.len(), permissions::ALL.len());
        assert!(owner.show_on_profile);

        // A membership in a community that is not public never shows on a profile.
        assert!(!access(visibility::RESTRICTED, Some(1)).viewer().unwrap().show_on_profile);

        let mut anonymous = access(visibility::PUBLIC, None);
        anonymous.user_id = None;
        assert!(anonymous.viewer().is_none());
    }

    #[test]
    fn references_are_uuids_or_handles() {
        assert!(matches!(parse_ref(&Uuid::now_v7().to_string()), Some(CommunityRef::Id(_))));
        assert!(matches!(parse_ref("+Apex_Club"), Some(CommunityRef::Handle(handle)) if handle == "apex_club"));
        assert!(parse_ref("no spaces").is_none());
        assert!(parse_ref("").is_none());
    }
}
