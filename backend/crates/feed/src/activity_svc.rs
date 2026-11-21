//! `ActivityService`: the caller's own log, profile activity lists, the
//! friends' feed and per-kind visibility preferences.
use std::{collections::BTreeSet, time::Duration};

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use sqlx::{Postgres, QueryBuilder};
use tokuly_core::prelude::*;
use tokuly_proto::{Timestamp, feed::*};
use tokuly_social::graph;

use crate::{
    Feed,
    activity::{
        AUDIENCE_FRIENDS, AUDIENCE_PRIVATE, AUDIENCE_PUBLIC, ActivityRow, activity_columns, kind_shown,
        push_kind_filter, push_not_involving,
    },
    cards::CardQuery,
    kinds,
};

const MAX_PAGE_SIZE: i64 = 100;
/// Kinds (or kind prefixes) one user can hide.
const MAX_HIDDEN_KINDS: usize = 50;

/// An optional `[from, to)` range from a request.
pub(crate) fn time_range(from: Option<&Timestamp>, to: Option<&Timestamp>) -> AppResult<(Option<Time>, Option<Time>)> {
    let (from, to) = (from.map(from_ts), to.map(from_ts));
    if let (Some(from), Some(to)) = (from, to) {
        ensure!(from < to, invalid, "INVALID_RANGE", "the range must end after it starts");
    }
    Ok((from, to))
}

/// The preference list: well-known kinds, every kind in the user's log and
/// every hidden kind, each with its label, visibility and entry count.
pub(crate) fn build_preferences(
    log_counts: &[(String, i64)],
    hidden: &[String],
    locale: &str,
) -> Vec<ActivityKindPreference> {
    let listed: BTreeSet<&str> = kinds::WELL_KNOWN
        .iter()
        .copied()
        .chain(log_counts.iter().map(|(kind, _)| kind.as_str()))
        .chain(hidden.iter().map(String::as_str))
        .collect();
    let mut preferences: Vec<ActivityKindPreference> = listed
        .into_iter()
        .map(|kind| {
            let entries: i64 =
                log_counts.iter().filter(|(logged, _)| kinds::matches(logged, kind)).map(|(_, count)| count).sum();
            ActivityKindPreference {
                kind: kind.to_owned(),
                label: kinds::label(kind).pick(locale).to_owned(),
                visible_to_others: !hidden.iter().any(|prefix| kinds::matches(kind, prefix)),
                entry_count: entries.min(i64::from(i32::MAX)) as i32,
                ..Default::default()
            }
        })
        .collect();
    preferences.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.kind.cmp(&b.kind)));
    preferences
}

impl Feed {
    /// Kinds (or kind prefixes) the user hides from other people.
    pub(crate) async fn hidden_kinds(&self, user_id: Uuid) -> AppResult<Vec<String>> {
        Ok(sqlx::query_scalar("SELECT kind FROM activity_hidden_kinds WHERE user_id = $1 ORDER BY kind")
            .bind(user_id)
            .fetch_all(&self.core.db)
            .await?)
    }

    async fn preferences_of(&self, user_id: Uuid, locale: &str) -> AppResult<Vec<ActivityKindPreference>> {
        let log_counts: Vec<(String, i64)> =
            sqlx::query_as("SELECT kind, count(*) FROM activities WHERE user_id = $1 GROUP BY kind")
                .bind(user_id)
                .fetch_all(&self.core.db)
                .await?;
        let hidden = self.hidden_kinds(user_id).await?;
        Ok(build_preferences(&log_counts, &hidden, locale))
    }

    /// The caller's friends whose content may reach them.
    pub(crate) async fn visible_friends(&self, user_id: Uuid, hidden: &[Uuid]) -> AppResult<Vec<Uuid>> {
        let mut friends = graph::friend_ids(&self.core, user_id).await?;
        friends.retain(|friend| !hidden.contains(friend) && *friend != user_id);
        Ok(friends)
    }
}

impl ActivityService for Feed {
    async fn list_my_activity(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListMyActivityRequest>,
    ) -> ServiceResult<ListMyActivityResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let prefix = kinds::parse_prefix(&request.kind_prefix)?;
            let (from, to) = time_range(request.from.as_option(), request.to.as_option())?;
            let page = Page::from_request(request.page.as_option(), MAX_PAGE_SIZE);
            let after: Option<(Time, Uuid)> = page.decode()?;

            let mut query: QueryBuilder<Postgres> =
                QueryBuilder::new(concat!("SELECT ", activity_columns!(), " FROM activities a WHERE a.user_id = "));
            query.push_bind(user_id);
            push_kind_filter(&mut query, "a", prefix.as_deref());
            if let Some(from) = from {
                query.push(" AND a.occurred_at >= ").push_bind(from);
            }
            if let Some(to) = to {
                query.push(" AND a.occurred_at < ").push_bind(to);
            }
            if let Some((at, id)) = after {
                query.push(" AND (a.occurred_at, a.id) < (").push_bind(at).push(", ").push_bind(id).push(")");
            }
            query.push(" ORDER BY a.occurred_at DESC, a.id DESC LIMIT ").push_bind(page.fetch());
            let mut rows: Vec<ActivityRow> = query.build_query_as().fetch_all(&self.core.db).await?;
            let info = page.finish(&mut rows, ActivityRow::key);

            let reader = self.reader_for(user_id).await?;
            let hidden_kinds = self.hidden_kinds(user_id).await?;
            ok(ListMyActivityResponse {
                activities: self.to_activities(&reader, &rows, Some(&hidden_kinds)).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn delete_activity(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DeleteActivityRequest>,
    ) -> ServiceResult<DeleteActivityResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let activity_id = parse_id(request.activity_id)?;
            // Someone else's entry is "not found": whether it exists is not the caller's business.
            let deleted = sqlx::query("DELETE FROM activities WHERE id = $1 AND user_id = $2")
                .bind(activity_id)
                .bind(user_id)
                .execute(&self.core.db)
                .await?;
            ensure!(deleted.rows_affected() > 0, not_found, "ACTIVITY_NOT_FOUND", "activity not found");
            self.invalidate_recaps(user_id).await;
            ok(DeleteActivityResponse::default())
        })
        .await
    }

    async fn clear_activity(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ClearActivityRequest>,
    ) -> ServiceResult<ClearActivityResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let request = request.to_owned_message();
            let prefix = kinds::parse_prefix(&request.kind_prefix)?;
            let (from, to) = time_range(request.from.as_option(), request.to.as_option())?;
            ensure!(
                from.is_some() || to.is_some() || request.everything,
                invalid,
                "RANGE_REQUIRED",
                "give a time range, or confirm with `everything` that the whole log is meant"
            );
            self.core.cache.rate_limit(&format!("feed:clear:{user_id}"), 10, Duration::from_secs(60)).await?;

            let mut query: QueryBuilder<Postgres> = QueryBuilder::new("DELETE FROM activities a WHERE a.user_id = ");
            query.push_bind(user_id);
            push_kind_filter(&mut query, "a", prefix.as_deref());
            if let Some(from) = from {
                query.push(" AND a.occurred_at >= ").push_bind(from);
            }
            if let Some(to) = to {
                query.push(" AND a.occurred_at < ").push_bind(to);
            }
            let deleted = query.build().execute(&self.core.db).await?.rows_affected();
            if deleted > 0 {
                self.invalidate_recaps(user_id).await;
            }
            ok(ClearActivityResponse { deleted_count: deleted.min(i32::MAX as u64) as i32, ..Default::default() })
        })
        .await
    }

    async fn list_user_activity(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListUserActivityRequest>,
    ) -> ServiceResult<ListUserActivityResponse> {
        handle(async {
            let reader = self.reader(&ctx).await?;
            let owner_id = parse_id(request.user_id)?;
            let prefix = kinds::parse_prefix(request.kind_prefix)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE_SIZE);
            let after: Option<(Time, Uuid)> = page.decode()?;
            let own = reader.viewer == Some(owner_id);

            if !own {
                // A blocked or blocking owner looks exactly like a missing one.
                let active: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL AND banned_at IS NULL)",
                )
                .bind(owner_id)
                .fetch_one(&self.core.db)
                .await?;
                ensure!(active && !reader.is_hidden(owner_id), not_found, "USER_NOT_FOUND", "user not found");
            }
            let friends = match reader.viewer {
                Some(viewer) if !own => graph::are_friends(&self.core, owner_id, viewer).await?,
                _ => false,
            };
            let least_audience = if own {
                AUDIENCE_PRIVATE
            } else if friends {
                AUDIENCE_FRIENDS
            } else {
                AUDIENCE_PUBLIC
            };

            let mut query: QueryBuilder<Postgres> =
                QueryBuilder::new(concat!("SELECT ", activity_columns!(), " FROM activities a WHERE a.user_id = "));
            query.push_bind(owner_id).push(" AND a.audience >= ").push_bind(least_audience);
            if !own {
                query.push(concat!(" AND ", kind_shown!("a")));
                push_not_involving(&mut query, "a", &reader.hidden);
            }
            push_kind_filter(&mut query, "a", prefix.as_deref());
            if let Some((at, id)) = after {
                query.push(" AND (a.occurred_at, a.id) < (").push_bind(at).push(", ").push_bind(id).push(")");
            }
            query.push(" ORDER BY a.occurred_at DESC, a.id DESC LIMIT ").push_bind(page.fetch());
            let mut rows: Vec<ActivityRow> = query.build_query_as().fetch_all(&self.core.db).await?;
            let info = page.finish(&mut rows, ActivityRow::key);

            let hidden_kinds = if own { Some(self.hidden_kinds(owner_id).await?) } else { None };
            ok(ListUserActivityResponse {
                activities: self.to_activities(&reader, &rows, hidden_kinds.as_deref()).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn list_friends_activity(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListFriendsActivityRequest>,
    ) -> ServiceResult<ListFriendsActivityResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let prefix = kinds::parse_prefix(request.kind_prefix)?;
            let page = Page::from_view(request.page.as_option(), MAX_PAGE_SIZE);
            let after: Option<(Time, Uuid)> = page.decode()?;
            let reader = self.reader_for(user_id).await?;
            let friends = self.visible_friends(user_id, &reader.hidden).await?;

            let mut heads = self
                .card_heads(&CardQuery {
                    authors: &friends,
                    hidden: &reader.hidden,
                    kind_prefix: prefix.as_deref(),
                    after,
                    limit: page.fetch(),
                    home: false,
                })
                .await?;
            let info = page.finish(&mut heads, ActivityRow::key);
            let bursts = self.bursts(heads, &reader.hidden).await?;
            ok(ListFriendsActivityResponse {
                cards: self.to_cards(&reader, &bursts).await?,
                page: info.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn get_activity_preferences(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetActivityPreferencesRequest>,
    ) -> ServiceResult<GetActivityPreferencesResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let reader = self.reader_for(user_id).await?;
            ok(GetActivityPreferencesResponse {
                kinds: self.preferences_of(user_id, &reader.locale).await?,
                ..Default::default()
            })
        })
        .await
    }

    async fn set_activity_kind_visibility(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetActivityKindVisibilityRequest>,
    ) -> ServiceResult<SetActivityKindVisibilityResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let kind = kinds::parse_required(request.kind)?;
            self.core.cache.rate_limit(&format!("feed:kind-visibility:{user_id}"), 60, Duration::from_secs(60)).await?;

            let mut tx = self.core.db.begin().await?;
            // One user's changes are applied one after the other, so the limit holds.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!("feed:hidden-kinds:{user_id}"))
                .execute(&mut *tx)
                .await?;
            let hidden: Vec<String> = sqlx::query_scalar("SELECT kind FROM activity_hidden_kinds WHERE user_id = $1")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
            // Entries below `kind` say nothing new once `kind` itself is decided.
            let narrower: Vec<String> = hidden.iter().filter(|entry| kinds::matches(entry, &kind)).cloned().collect();
            if request.visible_to_others {
                // Showing a kind never widens what a broader hidden family covers.
                if let Some(broader) = hidden.iter().find(|entry| **entry != kind && kinds::matches(&kind, entry)) {
                    bail!(
                        precondition,
                        "KIND_HIDDEN_BY_PREFIX",
                        "{kind} is hidden through {broader}; show {broader} instead"
                    );
                }
                sqlx::query("DELETE FROM activity_hidden_kinds WHERE user_id = $1 AND kind = ANY($2)")
                    .bind(user_id)
                    .bind(&narrower)
                    .execute(&mut *tx)
                    .await?;
            } else if !hidden.iter().any(|entry| kinds::matches(&kind, entry)) {
                let remaining = hidden.len() - narrower.len();
                ensure!(
                    remaining < MAX_HIDDEN_KINDS,
                    invalid,
                    "TOO_MANY_HIDDEN_KINDS",
                    "at most {MAX_HIDDEN_KINDS} kinds can be hidden"
                );
                sqlx::query("DELETE FROM activity_hidden_kinds WHERE user_id = $1 AND kind = ANY($2)")
                    .bind(user_id)
                    .bind(&narrower)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("INSERT INTO activity_hidden_kinds (user_id, kind) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                    .bind(user_id)
                    .bind(&kind)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;

            let reader = self.reader_for(user_id).await?;
            ok(SetActivityKindVisibilityResponse {
                kinds: self.preferences_of(user_id, &reader.locale).await?,
                ..Default::default()
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp(seconds: i64) -> Timestamp {
        Timestamp { seconds, ..Default::default() }
    }

    #[test]
    fn ranges_may_be_open_but_not_backwards() {
        assert_eq!(time_range(None, None).unwrap(), (None, None));
        let (from, to) = time_range(Some(&timestamp(100)), Some(&timestamp(200))).unwrap();
        assert_eq!((from.unwrap().timestamp(), to.unwrap().timestamp()), (100, 200));
        assert!(time_range(Some(&timestamp(100)), None).unwrap().0.is_some());
        assert_eq!(time_range(Some(&timestamp(200)), Some(&timestamp(100))).unwrap_err().reason, "INVALID_RANGE");
        assert_eq!(time_range(Some(&timestamp(100)), Some(&timestamp(100))).unwrap_err().reason, "INVALID_RANGE");
    }

    fn find<'a>(preferences: &'a [ActivityKindPreference], kind: &str) -> &'a ActivityKindPreference {
        preferences.iter().find(|p| p.kind == kind).unwrap_or_else(|| panic!("{kind} is listed"))
    }

    #[test]
    fn preferences_list_well_known_logged_and_hidden_kinds() {
        let log = vec![
            ("social.bumped".to_owned(), 3),
            ("geo.new_region".to_owned(), 2),
            ("geo.visit".to_owned(), 5),
            ("mystery.thing".to_owned(), 1),
        ];
        let hidden = vec!["geo".to_owned(), "wallet.bonus".to_owned()];
        let preferences = build_preferences(&log, &hidden, "ja");

        // Well-known kinds are listed even without entries.
        let notes = find(&preferences, "note");
        assert!(notes.visible_to_others);
        assert_eq!((notes.entry_count, notes.label.as_str()), (0, "ノート"));
        // A family counts everything below it and hides it.
        let geo = find(&preferences, "geo");
        assert!(!geo.visible_to_others);
        assert_eq!(geo.entry_count, 7);
        let region = find(&preferences, "geo.new_region");
        assert!(!region.visible_to_others, "hidden through its family");
        assert_eq!(region.entry_count, 2);
        // Kinds from the log and hidden kinds without entries are listed too.
        assert_eq!(find(&preferences, "social.bumped").entry_count, 3);
        assert!(find(&preferences, "mystery.thing").visible_to_others);
        assert!(!find(&preferences, "wallet.bonus").visible_to_others);

        let labels: Vec<&str> = preferences.iter().map(|p| p.label.as_str()).collect();
        let mut sorted = labels.clone();
        sorted.sort();
        assert_eq!(labels, sorted, "sorted by label");
        assert_eq!(
            preferences.iter().map(|p| p.kind.as_str()).collect::<BTreeSet<_>>().len(),
            preferences.len(),
            "no kind twice"
        );
    }

    #[test]
    fn preference_labels_follow_the_locale() {
        let preferences = build_preferences(&[], &[], "en");
        assert_eq!(find(&preferences, "social.bumped").label, "Bumps");
        assert_eq!(find(&preferences, "account.joined").label, "Joined Tokuly");
    }
}
