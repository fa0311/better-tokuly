//! `BadgeService` and `QuestAdminService`: the catalogue, a player's
//! badges, the showcase, the title, and awarding by hand.
use std::{collections::HashSet, time::Duration};

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::quest::*;
use tokuly_social::graph;

use super::{
    Standing, award,
    catalog::{self, CATALOG, FAMILIES},
    masked, owned, to_proto,
};
use crate::{
    Quest,
    profile::{MAX_SHOWCASE, title_of},
};

/// Friends shown as owners of a badge.
const FRIEND_OWNERS_SHOWN: usize = 10;

impl BadgeService for Quest {
    async fn list_badge_catalog(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, ListBadgeCatalogRequest>,
    ) -> ServiceResult<ListBadgeCatalogResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            self.limit_lookups(&ctx, viewer).await?;
            let stats = self.badge_stats().await?;
            let (held, standing) = match viewer {
                Some(viewer) => {
                    self.flush_user(viewer).await?;
                    let standing = self.standing(viewer).await?;
                    let mut held = owned(&self.core.db, viewer).await?;
                    // Badges added to the catalogue after the player passed their threshold.
                    if self.catch_up_badges(viewer, &standing, &held).await? {
                        held = owned(&self.core.db, viewer).await?;
                    }
                    (held, standing)
                }
                None => (Default::default(), Standing::default()),
            };
            let families: Vec<BadgeFamily> = FAMILIES
                .iter()
                .map(|family| {
                    let badges: Vec<Badge> = CATALOG
                        .iter()
                        .filter(|badge| badge.family == family.family)
                        .map(|badge| match held.get(badge.key) {
                            Some(mine) => to_proto(badge, Some(mine), None, stats.get(badge.key)),
                            None if badge.secret => masked(badge, stats.get(badge.key)),
                            None => to_proto(badge, None, standing.progress(badge), stats.get(badge.key)),
                        })
                        .collect();
                    BadgeFamily {
                        key: family.key.to_owned(),
                        name: family.name.proto().into(),
                        emoji: family.emoji.to_owned(),
                        owned_count: badges.iter().filter(|badge| badge.owned).count() as i32,
                        badges,
                        ..Default::default()
                    }
                })
                .collect();
            ok(ListBadgeCatalogResponse {
                owned_count: families.iter().map(|family| family.owned_count).sum(),
                total_count: CATALOG.len() as i32,
                families,
                ..Default::default()
            })
        })
        .await
    }

    async fn get_badge(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetBadgeRequest>,
    ) -> ServiceResult<GetBadgeResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            self.limit_lookups(&ctx, viewer).await?;
            // A secret badge is asked for by its stand-in key, unless the caller owns it.
            let (badge, by_stand_in) = match catalog::find(request.key) {
                Some(badge) => (badge, false),
                None => (catalog::find_masked(request.key).or_not_found("BADGE_NOT_FOUND", "no such badge")?, true),
            };
            let stats = self.badge_stats().await?;
            let stat = stats.get(badge.key);
            let held = match viewer {
                Some(viewer) => owned(&self.core.db, viewer).await?,
                None => Default::default(),
            };
            let shown = match held.get(badge.key) {
                Some(mine) => to_proto(badge, Some(mine), None, stat),
                // Guessing the real key of a secret badge finds nothing.
                None if badge.secret && !by_stand_in => bail!(not_found, "BADGE_NOT_FOUND", "no such badge"),
                None if badge.secret => masked(badge, stat),
                None => match viewer {
                    Some(viewer) => to_proto(badge, None, self.standing(viewer).await?.progress(badge), stat),
                    None => to_proto(badge, None, None, stat),
                },
            };
            let Some(viewer) = viewer else {
                return ok(GetBadgeResponse { badge: shown.into(), ..Default::default() });
            };
            let friends = graph::friend_ids(&self.core, viewer).await?;
            let owners: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM quest_badges WHERE badge_key = $1 AND user_id = ANY($2) ORDER BY earned_at DESC, user_id")
                .bind(badge.key)
                .bind(&friends)
                .fetch_all(&self.core.db)
                .await?;
            let preview: Vec<Uuid> = owners.iter().copied().take(FRIEND_OWNERS_SHOWN).collect();
            ok(GetBadgeResponse { badge: shown.into(), friend_owners: tokuly_identity::summaries_ordered(&self.core, &preview).await?, friend_owner_count: owners.len() as i32, ..Default::default() })
        })
        .await
    }

    async fn list_user_badges(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListUserBadgesRequest>,
    ) -> ServiceResult<ListUserBadgesResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let owner = match (parse_opt_id(request.user_id)?, viewer) {
                (Some(owner), _) => owner,
                (None, Some(viewer)) => viewer,
                (None, None) => bail!(unauthenticated, "SIGN_IN_REQUIRED", "sign in required"),
            };
            let own = viewer == Some(owner);
            if !own {
                self.limit_lookups(&ctx, viewer).await?;
                self.require_visible(viewer, owner).await?;
            }
            let profile = self.profile(owner).await?;
            if !profile.show_on_profile && !own {
                return ok(ListUserBadgesResponse { hidden: true, ..Default::default() });
            }
            let showcase = self.badges_by_key(owner, &profile.showcase).await?;
            // The whole list tells when somebody did what (a first video, a
            // night out of town): that is for friends. Everybody else sees
            // what the player put on show.
            let friend = match viewer {
                Some(viewer) if !own => graph::are_friends(&self.core, viewer, owner).await?,
                _ => false,
            };
            let badges = if own || friend {
                let stats = self.badge_stats().await?;
                let mut held: Vec<_> = owned(&self.core.db, owner).await?.into_values().collect();
                held.sort_by(|a, b| b.earned_at.cmp(&a.earned_at).then_with(|| a.badge_key.cmp(&b.badge_key)));
                // A badge somebody earned is shown for what it is, secret or not.
                held.iter()
                    .filter_map(|mine| {
                        catalog::find(&mine.badge_key)
                            .map(|badge| to_proto(badge, Some(mine), None, stats.get(badge.key)))
                    })
                    .collect()
            } else {
                showcase.clone()
            };
            ok(ListUserBadgesResponse { badges, showcase, title: profile.title().into(), ..Default::default() })
        })
        .await
    }

    async fn set_showcase(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetShowcaseRequest>,
    ) -> ServiceResult<SetShowcaseResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.core.cache.rate_limit(&format!("quest:showcase:{user_id}"), 30, Duration::from_secs(60)).await?;
            let keys: Vec<String> = request.badge_keys.iter().map(|key| key.to_string()).collect();
            ensure!(
                keys.len() <= MAX_SHOWCASE,
                invalid,
                "TOO_MANY_BADGES",
                "a showcase holds at most {MAX_SHOWCASE} badges"
            );
            let distinct: HashSet<&String> = keys.iter().collect();
            ensure!(distinct.len() == keys.len(), invalid, "DUPLICATE_BADGE", "a badge can be showcased once");
            ensure!(
                keys.iter().all(|key| catalog::find(key).is_some()),
                precondition,
                "BADGE_NOT_OWNED",
                "you do not own every badge of this showcase"
            );
            self.store_showcase(user_id, &keys).await?;
            ok(SetShowcaseResponse { showcase: self.badges_by_key(user_id, &keys).await?, ..Default::default() })
        })
        .await
    }

    async fn set_title(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetTitleRequest>,
    ) -> ServiceResult<SetTitleResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.core.cache.rate_limit(&format!("quest:title:{user_id}"), 30, Duration::from_secs(60)).await?;
            let key = request.badge_key;
            self.store_title(user_id, key).await?;
            ok(SetTitleResponse { title: title_of(key).into(), ..Default::default() })
        })
        .await
    }
}

/// Badges given by hand are given by somebody else: a staff member's own
/// account is not theirs to decorate (or to tidy up).
fn ensure_not_self(staff: Uuid, user_id: Uuid) -> AppResult<()> {
    ensure!(staff != user_id, denied, "SELF_AWARD", "staff cannot award or revoke their own badges");
    Ok(())
}

impl QuestAdminService for Quest {
    async fn award_badge(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AwardBadgeRequest>,
    ) -> ServiceResult<AwardBadgeResponse> {
        handle(async {
            let staff = ctx.require_role("admin")?.user_id;
            let user_id = parse_id(request.user_id)?;
            ensure_not_self(staff, user_id)?;
            let badge = catalog::find(request.badge_key).or_not_found("BADGE_NOT_FOUND", "no such badge")?;
            let awarded = award(&self.core, user_id, badge.key).await?;
            if awarded {
                tracing::info!(%staff, %user_id, badge = badge.key, "quest: badge awarded by staff");
            }
            let held = owned(&self.core.db, user_id).await?;
            ok(AwardBadgeResponse {
                awarded,
                badge: to_proto(badge, held.get(badge.key), None, self.badge_stats().await?.get(badge.key)).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn revoke_badge(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeBadgeRequest>,
    ) -> ServiceResult<RevokeBadgeResponse> {
        handle(async {
            let staff = ctx.require_role("admin")?.user_id;
            let user_id = parse_id(request.user_id)?;
            ensure_not_self(staff, user_id)?;
            let badge = catalog::find(request.badge_key).or_not_found("BADGE_NOT_FOUND", "no such badge")?;
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)")
                    .bind(user_id)
                    .fetch_one(&self.core.db)
                    .await?;
            ensure!(exists, not_found, "USER_NOT_FOUND", "user not found");
            let revoked = self.take_badge(user_id, badge).await?;
            if revoked {
                tracing::info!(%staff, %user_id, badge = badge.key, "quest: badge revoked by staff");
            }
            ok(RevokeBadgeResponse { revoked, ..Default::default() })
        })
        .await
    }
}
