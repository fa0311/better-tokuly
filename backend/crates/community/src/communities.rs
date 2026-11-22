//! Communities themselves: creating, editing, closing and handing over.
use std::time::Duration;

use sqlx::{Postgres, Transaction};
use tokuly_core::{
    events as core_events,
    outbound::{self, SearchDoc},
    prelude::*,
};
use tokuly_media::assets;
use tokuly_proto::{
    community::{self as pb, CommunityCategory, ModerationAction, Permission},
    media::AssetPurpose,
};

use crate::{
    Community,
    access::{Access, permission_denied},
    events::{self, CommunityChanged},
    model::{CommunityRow, community_columns, to_json, visibility},
    modlog::{self, LogEntry},
    notices::{self, MembershipNotice},
    permissions::Role,
    validate,
    workers::SweepJob,
};

/// Upload purposes accepted for a community's icon and banner.
const ICON_PURPOSES: [AssetPurpose; 2] = [AssetPurpose::ASSET_PURPOSE_AVATAR, AssetPurpose::ASSET_PURPOSE_CHANNEL_ICON];
const BANNER_PURPOSES: [AssetPurpose; 2] =
    [AssetPurpose::ASSET_PURPOSE_BANNER, AssetPurpose::ASSET_PURPOSE_CHANNEL_BANNER];

pub fn icon_holder(community_id: Uuid) -> String {
    format!("community:icon:{community_id}")
}

pub fn banner_holder(community_id: Uuid) -> String {
    format!("community:banner:{community_id}")
}

/// Serializes changes that count rows before inserting one (the creation
/// limit, the join limit) for the rest of the transaction.
pub(crate) async fn lock(tx: &mut Transaction<'_, Postgres>, key: &str) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))").bind(key).execute(&mut **tx).await?;
    Ok(())
}

/// Turns the unique violation of the handle column into `HANDLE_TAKEN`.
fn handle_taken(error: sqlx::Error) -> AppError {
    match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::already_exists("HANDLE_TAKEN", "this handle is taken")
        }
        _ => error.into(),
    }
}

/// Whether a community's school field applies: school and club communities only.
fn has_school(category: i32) -> bool {
    category == CommunityCategory::COMMUNITY_CATEGORY_SCHOOL as i32
        || category == CommunityCategory::COMMUNITY_CATEGORY_CLUB as i32
}

/// The search document of a community. Hidden and closed communities are
/// not public, which removes them from the index.
pub fn search_doc(community: &CommunityRow) -> SearchDoc {
    SearchDoc {
        kind: "community".into(),
        id: community.id,
        title: community.name.clone(),
        text: community.description.clone(),
        tags: community.tags.clone(),
        owner_id: Some(community.owner_id),
        public: !community.is_hidden() && community.deleted_at.is_none(),
        image_url: community.icon_url(),
        path: community.path(),
        created_at: community.created_at.timestamp(),
        score: f64::from(community.member_count),
        extra: serde_json::json!({
            "handle": community.handle,
            "category": community.category,
            "visibility": community.visibility,
            "member_count": community.member_count,
            "school_name": community.school_name,
        }),
    }
}

fn changed(community: &CommunityRow) -> CommunityChanged {
    CommunityChanged {
        community_id: community.id,
        owner_id: community.owner_id,
        visibility: community.visibility,
        category: community.category,
    }
}

/// What an update changes; `None` leaves a field alone.
#[derive(Debug, Default)]
pub struct CommunityEdit {
    pub name: Option<String>,
    pub handle: Option<String>,
    pub description: Option<String>,
    pub category: Option<i32>,
    pub visibility: Option<i32>,
    pub language: Option<String>,
    pub tags: Option<Vec<String>>,
    pub rules: Option<Vec<pb::Rule>>,
    pub join_questions: Option<Vec<String>>,
    /// `Some(None)` removes the image.
    pub icon_asset_id: Option<Option<Uuid>>,
    pub banner_asset_id: Option<Option<Uuid>>,
    pub school_name: Option<String>,
    pub settings: Option<pb::CommunitySettings>,
}

impl Community {
    /// How many communities `user_id` owns.
    pub(crate) async fn owned_count<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E, user_id: Uuid) -> AppResult<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM communities WHERE owner_id = $1 AND deleted_at IS NULL")
            .bind(user_id)
            .fetch_one(executor)
            .await?)
    }

    /// Whether a handle is well-formed and free: `(available, reason, normalized)`.
    pub(crate) async fn check_handle(&self, input: &str) -> AppResult<(bool, &'static str, String)> {
        let handle = match tokuly_identity::users::normalize_handle(input) {
            Ok(handle) => handle,
            Err("RESERVED") => return Ok((false, "HANDLE_RESERVED", String::new())),
            Err(_) => return Ok((false, "HANDLE_INVALID", String::new())),
        };
        let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM communities WHERE handle = $1::citext)")
            .bind(&handle)
            .fetch_one(&self.core.db)
            .await?;
        Ok(if taken { (false, "HANDLE_TAKEN", handle) } else { (true, "", handle) })
    }

    /// Creates a community owned by `user_id`, with its first channel.
    pub(crate) async fn create_community(
        &self,
        user_id: Uuid,
        req: pb::CreateCommunityRequest,
    ) -> AppResult<CommunityRow> {
        let core = &self.core;
        let name = validate::community_name(&req.name)?;
        let handle = validate::handle(&req.handle)?;
        let description = validate::description(&req.description, validate::MAX_DESCRIPTION_CHARS)?;
        let category = validate::category(req.category)?;
        let visibility = validate::visibility(req.visibility)?;
        let tags = validate::tags(&req.tags, validate::MAX_TAGS)?;
        let rules = validate::rules(req.rules)?;
        let join_questions = validate::join_questions(&req.join_questions)?;
        let (locale, creator_school): (String, String) =
            sqlx::query_as("SELECT locale, school_name FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_one(&core.db)
                .await?;
        let language = if req.language.trim().is_empty() {
            // The creator's language, without the region ("ja-JP" → "ja").
            locale.split('-').next().filter(|l| l.len() >= 2).unwrap_or("ja").to_ascii_lowercase()
        } else {
            validate::language(&req.language)?
        };
        let school_name = if !has_school(category) {
            String::new()
        } else if req.school_name.trim().is_empty() {
            creator_school
        } else {
            validate::school_name(&req.school_name)?
        };
        let settings = req.settings.into_option().unwrap_or_default();

        self.limit("create", user_id, 5, Duration::from_secs(3600)).await?;
        let icon_id = parse_opt_id(&req.icon_asset_id)?;
        let banner_id = parse_opt_id(&req.banner_asset_id)?;
        let icon = match icon_id {
            Some(id) => Some(assets::require_image_json(core, id, user_id, &ICON_PURPOSES).await?),
            None => None,
        };
        let banner = match banner_id {
            Some(id) => Some(assets::require_image_json(core, id, user_id, &BANNER_PURPOSES).await?),
            None => None,
        };

        let mut tx = core.db.begin().await?;
        lock(&mut tx, &format!("community:user:{user_id}")).await?;
        ensure!(
            self.owned_count(&mut *tx, user_id).await? < i64::from(self.cfg.max_owned),
            precondition,
            "COMMUNITY_LIMIT_REACHED",
            "one account owns at most {} communities",
            self.cfg.max_owned
        );
        self.ensure_can_join_more(&mut tx, user_id).await?;
        let community: CommunityRow = sqlx::query_as(concat!(
            "INSERT INTO communities AS c
                 (handle, name, description, icon, icon_asset_id, banner, banner_asset_id, rules, tags, category,
                  language, visibility, school_name, join_questions, members_can_invite, members_can_create_events,
                  owner_id, member_count)
             VALUES ($1::citext, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, 1)
             RETURNING ",
            community_columns!()
        ))
        .bind(&handle)
        .bind(&name)
        .bind(&description)
        .bind(&icon)
        .bind(icon_id)
        .bind(&banner)
        .bind(banner_id)
        .bind(to_json(&rules)?)
        .bind(&tags)
        .bind(category)
        .bind(&language)
        .bind(visibility)
        .bind(&school_name)
        .bind(&join_questions)
        .bind(settings.members_can_invite)
        .bind(settings.members_can_create_events)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(handle_taken)?;
        sqlx::query("INSERT INTO community_members (community_id, user_id, role) VALUES ($1, $2, $3)")
            .bind(community.id)
            .bind(user_id)
            .bind(Role::Owner.code())
            .execute(&mut *tx)
            .await?;
        let (channel_name, channel_description) = if language.starts_with("ja") {
            ("雑談", "なんでも話そう!")
        } else {
            ("general", "Talk about anything")
        };
        sqlx::query(
            "INSERT INTO community_channels (community_id, name, description, emoji) VALUES ($1, $2, $3, '💬')",
        )
        .bind(community.id)
        .bind(channel_name)
        .bind(channel_description)
        .execute(&mut *tx)
        .await?;
        if let Some(id) = icon_id {
            assets::attach(&mut *tx, id, &icon_holder(community.id)).await?;
        }
        if let Some(id) = banner_id {
            assets::attach(&mut *tx, id, &banner_holder(community.id)).await?;
        }
        core_events::emit(&mut *tx, events::COMMUNITY_CREATED, Some(user_id), &changed(&community)).await?;
        core_events::emit(
            &mut *tx,
            events::MEMBER_JOINED,
            Some(user_id),
            &events::MemberChanged {
                community_id: community.id,
                user_id,
                public: community.is_public(),
                cause: String::new(),
            },
        )
        .await?;
        outbound::index(&mut *tx, search_doc(&community)).await?;
        outbound::activity(&mut *tx, &notices::joined_activity(user_id, &community, true, false)).await?;
        tx.commit().await?;
        Ok(community)
    }

    /// Applies an edit to the community of `access`. Needs MANAGE_COMMUNITY.
    pub(crate) async fn update_community(&self, access: &Access, edit: CommunityEdit) -> AppResult<CommunityRow> {
        access.require(Permission::PERMISSION_MANAGE_COMMUNITY)?;
        let core = &self.core;
        let current = &access.community;
        let actor = access.user_id.ok_or_else(permission_denied)?;

        let category = edit.category.unwrap_or(current.category);
        let school_name = match edit.school_name {
            _ if !has_school(category) => String::new(),
            Some(school) => validate::school_name(&school)?,
            None => current.school_name.clone(),
        };
        let icon = match edit.icon_asset_id {
            Some(Some(id)) if Some(id) != current.icon_asset_id => {
                Some((Some(id), Some(assets::require_image_json(core, id, actor, &ICON_PURPOSES).await?)))
            }
            Some(None) => Some((None, None)),
            _ => None,
        };
        let banner = match edit.banner_asset_id {
            Some(Some(id)) if Some(id) != current.banner_asset_id => {
                Some((Some(id), Some(assets::require_image_json(core, id, actor, &BANNER_PURPOSES).await?)))
            }
            Some(None) => Some((None, None)),
            _ => None,
        };
        let settings = edit.settings.as_ref();
        let mut changes: Vec<&str> = Vec::new();
        let mut note = |changed: bool, field: &'static str| {
            if changed {
                changes.push(field);
            }
        };
        note(edit.name.as_ref().is_some_and(|v| *v != current.name), "name");
        note(edit.handle.as_ref().is_some_and(|v| *v != current.handle), "handle");
        note(edit.description.as_ref().is_some_and(|v| *v != current.description), "description");
        note(category != current.category, "category");
        note(edit.visibility.is_some_and(|v| v != current.visibility), "visibility");
        note(edit.language.as_ref().is_some_and(|v| *v != current.language), "language");
        note(edit.tags.as_ref().is_some_and(|v| *v != current.tags), "tags");
        note(edit.rules.is_some(), "rules");
        note(edit.join_questions.as_ref().is_some_and(|v| *v != current.join_questions), "join_questions");
        note(icon.is_some(), "icon");
        note(banner.is_some(), "banner");
        note(school_name != current.school_name, "school_name");
        note(
            settings.is_some_and(|s| {
                s.members_can_invite != current.members_can_invite
                    || s.members_can_create_events != current.members_can_create_events
            }),
            "settings",
        );
        let visibility_changed = edit.visibility.is_some_and(|v| v != current.visibility);
        let rules = edit.rules.as_ref().map(|rules| to_json(rules)).transpose()?;
        self.limit("community-edit", actor, 30, Duration::from_secs(3600)).await?;
        if visibility_changed {
            // Every change walks all the threads of the community (search).
            self.limit("visibility", current.id, 4, Duration::from_secs(24 * 3600)).await?;
        }

        let mut tx = core.db.begin().await?;
        let updated: CommunityRow = sqlx::query_as(concat!(
            "UPDATE communities AS c SET
                 name = coalesce($2, name),
                 handle = coalesce($3::citext, handle),
                 description = coalesce($4, description),
                 category = $5,
                 visibility = coalesce($6, visibility),
                 language = coalesce($7, language),
                 tags = coalesce($8, tags),
                 rules = coalesce($9, rules),
                 join_questions = coalesce($10, join_questions),
                 school_name = $11,
                 members_can_invite = coalesce($12, members_can_invite),
                 members_can_create_events = coalesce($13, members_can_create_events),
                 icon = CASE WHEN $14 THEN $15 ELSE icon END,
                 icon_asset_id = CASE WHEN $14 THEN $16 ELSE icon_asset_id END,
                 banner = CASE WHEN $17 THEN $18 ELSE banner END,
                 banner_asset_id = CASE WHEN $17 THEN $19 ELSE banner_asset_id END
             WHERE c.id = $1 AND c.deleted_at IS NULL
             RETURNING ",
            community_columns!()
        ))
        .bind(current.id)
        .bind(&edit.name)
        .bind(&edit.handle)
        .bind(&edit.description)
        .bind(category)
        .bind(edit.visibility)
        .bind(&edit.language)
        .bind(&edit.tags)
        .bind(&rules)
        .bind(&edit.join_questions)
        .bind(&school_name)
        .bind(settings.map(|s| s.members_can_invite))
        .bind(settings.map(|s| s.members_can_create_events))
        .bind(icon.is_some())
        .bind(icon.as_ref().and_then(|(_, image)| image.clone()))
        .bind(icon.as_ref().and_then(|(id, _)| *id))
        .bind(banner.is_some())
        .bind(banner.as_ref().and_then(|(_, image)| image.clone()))
        .bind(banner.as_ref().and_then(|(id, _)| *id))
        .fetch_optional(&mut *tx)
        .await
        .map_err(handle_taken)?
        .ok_or_else(crate::access::community_not_found)?;
        if let Some((new_id, _)) = &icon {
            if let Some(old) = current.icon_asset_id {
                assets::detach(&mut *tx, old, &icon_holder(current.id)).await?;
            }
            if let Some(new) = new_id {
                assets::attach(&mut *tx, *new, &icon_holder(current.id)).await?;
            }
        }
        if let Some((new_id, _)) = &banner {
            if let Some(old) = current.banner_asset_id {
                assets::detach(&mut *tx, old, &banner_holder(current.id)).await?;
            }
            if let Some(new) = new_id {
                assets::attach(&mut *tx, *new, &banner_holder(current.id)).await?;
            }
        }
        if !changes.is_empty() {
            let mut entry = LogEntry::new(current.id, ModerationAction::MODERATION_ACTION_COMMUNITY_UPDATED)
                .by(actor)
                .detail("changed", changes.join(","));
            // What the community called itself before stays on file: a
            // rename does not take back what was reported.
            for (field, before) in
                [("name", &current.name), ("handle", &current.handle), ("description", &current.description)]
            {
                if changes.contains(&field) {
                    entry = entry.detail(&format!("{field}_before"), before);
                }
            }
            modlog::record(&mut *tx, &entry).await?;
            core_events::emit(&mut *tx, events::COMMUNITY_UPDATED, Some(actor), &changed(&updated)).await?;
            outbound::index(&mut *tx, search_doc(&updated)).await?;
        }
        tx.commit().await?;

        if visibility_changed {
            // Threads are searchable only while the community is public.
            self.queue_sweep(SweepJob { community_id: updated.id, channel_id: None }).await;
        }
        if !changes.is_empty() {
            self.push_community_changed(updated.id).await;
        }
        Ok(updated)
    }

    /// Closes a community inside `tx`: it stops existing for everyone and
    /// its handle becomes free. Returns false when it was closed already.
    /// Call [`Self::after_close`] once the transaction committed.
    pub(crate) async fn close_community(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        actor: Option<Uuid>,
    ) -> AppResult<bool> {
        let closed = sqlx::query(
            "UPDATE communities SET deleted_at = now(), handle = ('deleted-' || replace(id::text, '-', ''))::citext
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(community.id)
        .execute(&mut **tx)
        .await?;
        if closed.rows_affected() == 0 {
            return Ok(false);
        }
        // Icon, banner and everything posted stay held until the retention
        // is over (`purge_removed`): closing is not a way to destroy what
        // was reported.
        outbound::unindex(&mut **tx, "community", community.id).await?;
        core_events::emit(&mut **tx, events::COMMUNITY_DELETED, actor, &changed(community)).await?;
        Ok(true)
    }

    /// What follows the closing of a community once it is committed: its
    /// threads leave the search index (in the background), and everyone
    /// watching is told.
    pub(crate) async fn after_close(&self, community_id: Uuid) {
        self.queue_sweep(SweepJob { community_id, channel_id: None }).await;
        self.push_community_changed(community_id).await;
    }

    /// Closes the community of `access` for good. Owner only.
    pub(crate) async fn delete_community(&self, access: &Access) -> AppResult<()> {
        access.require(Permission::PERMISSION_DELETE_COMMUNITY)?;
        let mut tx = self.core.db.begin().await?;
        let closed = self.close_community(&mut tx, &access.community, access.user_id).await?;
        tx.commit().await?;
        if closed {
            self.after_close(access.community.id).await;
        }
        Ok(())
    }

    /// Makes `new_owner` the owner inside `tx`; the previous owner (when
    /// still a member) stays as an admin.
    pub(crate) async fn set_owner(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: &CommunityRow,
        new_owner: Uuid,
    ) -> AppResult<CommunityRow> {
        sqlx::query("UPDATE community_members SET role = $3 WHERE community_id = $1 AND user_id = $2 AND role = $4")
            .bind(community.id)
            .bind(community.owner_id)
            .bind(Role::Admin.code())
            .bind(Role::Owner.code())
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE community_members SET role = $3 WHERE community_id = $1 AND user_id = $2")
            .bind(community.id)
            .bind(new_owner)
            .bind(Role::Owner.code())
            .execute(&mut **tx)
            .await?;
        let updated: CommunityRow = sqlx::query_as(concat!(
            "UPDATE communities AS c SET owner_id = $2 WHERE c.id = $1 RETURNING ",
            community_columns!()
        ))
        .bind(community.id)
        .bind(new_owner)
        .fetch_one(&mut **tx)
        .await?;
        core_events::emit(&mut **tx, events::COMMUNITY_UPDATED, Some(new_owner), &changed(&updated)).await?;
        if updated.visibility != visibility::HIDDEN {
            outbound::index(&mut **tx, search_doc(&updated)).await?;
        }
        Ok(updated)
    }

    /// Hands the community of `access` to another member. Owner only.
    pub(crate) async fn transfer_ownership(&self, access: &Access, new_owner: Uuid) -> AppResult<CommunityRow> {
        access.require(Permission::PERMISSION_TRANSFER_OWNERSHIP)?;
        let community = &access.community;
        let actor = access.user_id.ok_or_else(permission_denied)?;
        ensure!(new_owner != actor, invalid, "CANNOT_TARGET_SELF", "you already own this community");
        self.limit("transfer", actor, 10, Duration::from_secs(24 * 3600)).await?;

        let mut tx = self.core.db.begin().await?;
        lock(&mut tx, &format!("community:user:{new_owner}")).await?;
        let is_member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM community_members WHERE community_id = $1 AND user_id = $2 FOR UPDATE)",
        )
        .bind(community.id)
        .bind(new_owner)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(is_member, not_found, "MEMBER_NOT_FOUND", "the new owner must be a member of the community");
        ensure!(
            self.owned_count(&mut *tx, new_owner).await? < i64::from(self.cfg.max_owned),
            precondition,
            "COMMUNITY_LIMIT_REACHED",
            "the new owner already owns as many communities as one account may"
        );
        // The owner on file decides, not the one this request started with.
        let still_owner: bool =
            sqlx::query_scalar("SELECT owner_id = $2 FROM communities WHERE id = $1 AND deleted_at IS NULL FOR UPDATE")
                .bind(community.id)
                .bind(actor)
                .fetch_optional(&mut *tx)
                .await?
                .unwrap_or(false);
        ensure!(still_owner, denied, "PERMISSION_DENIED", "only the owner hands a community over");
        let updated = self.set_owner(&mut tx, community, new_owner).await?;
        let entry = LogEntry::new(community.id, ModerationAction::MODERATION_ACTION_OWNERSHIP_TRANSFERRED)
            .by(actor)
            .target(new_owner);
        modlog::record(&mut *tx, &entry).await?;
        outbound::notify(
            &mut *tx,
            &notices::membership(new_owner, &updated, &MembershipNotice::Ownership).actor(actor),
        )
        .await?;
        tx.commit().await?;

        self.push_membership(&updated, new_owner).await;
        self.push_membership(&updated, actor).await;
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_school_and_club_communities_carry_a_school() {
        assert!(has_school(CommunityCategory::COMMUNITY_CATEGORY_SCHOOL as i32));
        assert!(has_school(CommunityCategory::COMMUNITY_CATEGORY_CLUB as i32));
        assert!(!has_school(CommunityCategory::COMMUNITY_CATEGORY_GAME as i32));
    }

    #[test]
    fn holders_name_the_community() {
        let id = Uuid::nil();
        assert_eq!(icon_holder(id), format!("community:icon:{id}"));
        assert_eq!(banner_holder(id), format!("community:banner:{id}"));
    }
}
