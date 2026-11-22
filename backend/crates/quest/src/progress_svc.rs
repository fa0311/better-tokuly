//! `QuestService`: level and XP, the XP history, the rule table for
//! clients, a player's public quest card and the caller's settings.
use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::quest::*;
use tokuly_social::graph;

use crate::{
    Quest, clock, levels,
    metrics::Metric,
    rules::{self, RULES, SOURCES, SourceDef, Xp},
};

/// Largest page of the XP history, in days.
const MAX_HISTORY_DAYS: i64 = 90;

fn source_proto(def: &SourceDef) -> XpSource {
    XpSource {
        key: def.key.to_owned(),
        name: def.name.proto().into(),
        emoji: def.emoji.to_owned(),
        daily_cap: def.daily_cap,
        ..Default::default()
    }
}

impl Quest {
    /// Where the player stands: level, today's XP per source, caps.
    pub(crate) async fn progress(&self, user_id: Uuid, zone: Tz, now: DateTime<Utc>) -> AppResult<Progress> {
        let profile = self.profile(user_id).await?;
        let today = clock::local_date(now, zone);
        let earned: Vec<(String, i32)> =
            sqlx::query_as("SELECT source, xp FROM quest_xp_days WHERE user_id = $1 AND day = $2")
                .bind(user_id)
                .bind(today)
                .fetch_all(&self.core.db)
                .await?;
        let earned: HashMap<String, i32> = earned.into_iter().collect();
        let sources: Vec<SourceProgress> = SOURCES
            .iter()
            .map(|def| {
                let xp_today = earned.get(def.key).copied().unwrap_or(0);
                SourceProgress {
                    source: source_proto(def).into(),
                    xp_today,
                    capped: def.daily_cap > 0 && xp_today >= def.daily_cap,
                    ..Default::default()
                }
            })
            .collect();
        let active_days: Option<i64> =
            sqlx::query_scalar("SELECT value FROM quest_counters WHERE user_id = $1 AND metric = $2")
                .bind(user_id)
                .bind(Metric::ActiveDays.key())
                .fetch_optional(&self.core.db)
                .await?;
        Ok(Progress {
            level: levels::progress(profile.xp).into(),
            xp_today: sources.iter().map(|source| source.xp_today).sum(),
            sources,
            today: clock::date_text(today),
            time_zone: zone.name().to_owned(),
            day_resets_at: ts(clock::end_of_day(today, zone)),
            title: profile.title().into(),
            badge_count: self.badge_count(user_id).await?,
            active_days: active_days.unwrap_or(0) as i32,
            ..Default::default()
        })
    }

    /// A page of the XP history: whole days, newest first.
    async fn xp_history(&self, user_id: Uuid, page: &Page) -> AppResult<(Vec<XpDay>, tokuly_proto::common::PageInfo)> {
        let before: Option<NaiveDate> = page.decode()?;
        let mut days: Vec<(NaiveDate, i64)> = sqlx::query_as(
            "SELECT day, sum(xp)::bigint FROM quest_xp_days WHERE user_id = $1 AND ($2::date IS NULL OR day < $2)
             GROUP BY day ORDER BY day DESC LIMIT $3",
        )
        .bind(user_id)
        .bind(before)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut days, |(day, _)| *day);
        let dates: Vec<NaiveDate> = days.iter().map(|(day, _)| *day).collect();
        let rows: Vec<(NaiveDate, String, i32)> = sqlx::query_as(
            "SELECT day, source, xp FROM quest_xp_days WHERE user_id = $1 AND day = ANY($2) ORDER BY xp DESC, source",
        )
        .bind(user_id)
        .bind(&dates)
        .fetch_all(&self.core.db)
        .await?;
        let history = days
            .into_iter()
            .map(|(day, xp)| XpDay {
                date: clock::date_text(day),
                xp: xp as i32,
                sources: rows
                    .iter()
                    .filter(|(row_day, _, _)| *row_day == day)
                    .map(|(_, source, xp)| XpDaySource { source: source.clone(), xp: *xp, ..Default::default() })
                    .collect(),
                ..Default::default()
            })
            .collect();
        Ok((history, info))
    }

    /// The rule table as clients show it.
    async fn earning_rules(&self) -> AppResult<Vec<EarningRule>> {
        let live = self.live_rule_keys().await?;
        let mut listed: Vec<EarningRule> = RULES
            .iter()
            .filter(|rule| rule.listed())
            .map(|rule| EarningRule {
                key: rule.key.to_owned(),
                source: rule.source.key().to_owned(),
                text: rule.text.proto().into(),
                xp: rule.xp.per_unit(),
                per_unit: if matches!(rule.xp, Xp::PerUnit(_)) { rule.unit.proto_opt() } else { None }.into(),
                counts_for_streak: rule.streak,
                live: live.contains(rule.key),
                ..Default::default()
            })
            .collect();
        // What the quest domain credits by itself: a completed streak day.
        // It is live as soon as anything done towards a friend is.
        listed.push(EarningRule {
            key: rules::STREAK_DAY_KEY.to_owned(),
            source: rules::Source::Streaks.key().to_owned(),
            text: rules::STREAK_DAY_TEXT.proto().into(),
            xp: rules::STREAK_DAY_XP,
            live: RULES.iter().any(|rule| rule.streak && live.contains(rule.key)),
            ..Default::default()
        });
        Ok(listed)
    }

    /// Limits how often one viewer — or one address, for guests — looks up
    /// other players' quest data and the catalogue.
    pub(crate) async fn limit_lookups(&self, ctx: &RequestContext, viewer: Option<Uuid>) -> AppResult<()> {
        match viewer {
            Some(viewer) => {
                self.core.cache.rate_limit(&format!("quest:lookup:{viewer}"), 300, Duration::from_secs(60)).await
            }
            None => {
                self.core
                    .cache
                    .rate_limit(&format!("quest:lookup:ip:{}", ctx.client().ip), 120, Duration::from_secs(60))
                    .await
            }
        }
    }

    /// Grants the level-up coins the player is still owed, if any. Coins
    /// follow a level-up right away; this is for the case that they did not
    /// (a process stopped in between). A failure waits for the next time.
    pub(crate) async fn catch_up_level_rewards(&self, user_id: Uuid) {
        match self.profile(user_id).await {
            Ok(profile) if profile.rewarded_level < profile.level => {
                self.announce(user_id, crate::earnings::Effects::starting_from(&profile)).await
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(%user_id, error = %e, "quest: level rewards not checked"),
        }
    }

    /// Fails with `USER_NOT_FOUND` unless `owner` is an account `viewer`
    /// may see: it exists, was not deleted, and neither blocked the other.
    pub(crate) async fn require_visible(&self, viewer: Option<Uuid>, owner: Uuid) -> AppResult<()> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL AND banned_at IS NULL)",
        )
        .bind(owner)
        .fetch_one(&self.core.db)
        .await?;
        ensure!(exists, not_found, "USER_NOT_FOUND", "user not found");
        if let Some(viewer) = viewer.filter(|viewer| *viewer != owner) {
            ensure!(
                !graph::is_blocked_either(&self.core, viewer, owner).await?,
                not_found,
                "USER_NOT_FOUND",
                "user not found"
            );
        }
        Ok(())
    }
}

impl QuestService for Quest {
    async fn get_progress(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetProgressRequest>,
    ) -> ServiceResult<GetProgressResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let zone = self.caller_zone(user_id).await?;
            // What the player earned a second ago should already show.
            self.flush_user(user_id).await?;
            // Coins of a level-up that never reached the wallet are granted now.
            self.catch_up_level_rewards(user_id).await;
            ok(GetProgressResponse {
                progress: self.progress(user_id, zone, now()).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_xp_history(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListXpHistoryRequest>,
    ) -> ServiceResult<ListXpHistoryResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let page = Page::from_view(request.page.as_option(), MAX_HISTORY_DAYS);
            self.flush_user(user_id).await?;
            let (days, page) = self.xp_history(user_id, &page).await?;
            ok(ListXpHistoryResponse { days, page: page.into(), ..Default::default() })
        })
        .await
    }

    async fn list_earning_rules(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, ListEarningRulesRequest>,
    ) -> ServiceResult<ListEarningRulesResponse> {
        handle(async {
            ok(ListEarningRulesResponse {
                sources: SOURCES.iter().map(source_proto).collect(),
                rules: self.earning_rules().await?,
                curve: levels::curve(self.cfg.level_up_coins, self.cfg.milestone_level_coins).into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_profile(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetProfileRequest>,
    ) -> ServiceResult<GetProfileResponse> {
        handle(async {
            let viewer = ctx.maybe_user_id()?;
            let owner = parse_id(request.user_id)?;
            self.limit_lookups(&ctx, viewer).await?;
            self.require_visible(viewer, owner).await?;
            ok(GetProfileResponse { profile: self.quest_profile(viewer, owner).await?.into(), ..Default::default() })
        })
        .await
    }

    async fn get_preferences(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetPreferencesRequest>,
    ) -> ServiceResult<GetPreferencesResponse> {
        handle(async {
            ok(GetPreferencesResponse {
                preferences: self.profile(ctx.user_id()?).await?.preferences().into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_preferences(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdatePreferencesRequest>,
    ) -> ServiceResult<UpdatePreferencesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.core.cache.rate_limit(&format!("quest:prefs:{user_id}"), 30, Duration::from_secs(60)).await?;
            let profile = self
                .save_preferences(
                    user_id,
                    request.show_on_leaderboards,
                    request.show_on_profile,
                    request.streak_reminders,
                )
                .await?;
            ok(UpdatePreferencesResponse { preferences: profile.preferences().into(), ..Default::default() })
        })
        .await
    }
}
