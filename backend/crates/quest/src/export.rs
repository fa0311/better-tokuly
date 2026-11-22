//! A player's quest data: the `quest` section of a personal data export,
//! and erasing it all when the account is deleted.
use async_trait::async_trait;
use sqlx::PgPool;
use tokuly_core::{export::DataExporter, prelude::*};

use crate::Quest;

/// Contributes the `quest` section of a personal data export.
pub(crate) struct QuestExporter {
    db: PgPool,
}

impl QuestExporter {
    /// Holds only the database pool: the exporter is owned by `Core`, so
    /// keeping a `Core` here would be a reference cycle.
    pub(crate) fn new(core: &Core) -> Self {
        Self { db: core.db.clone() }
    }

    /// Runs a query that aggregates its rows into one JSON value.
    async fn json(&self, sql: &'static str, user_id: Uuid) -> AppResult<serde_json::Value> {
        let value: Option<serde_json::Value> = sqlx::query_scalar(sql).bind(user_id).fetch_one(&self.db).await?;
        Ok(value.unwrap_or(serde_json::Value::Null))
    }
}

#[async_trait]
impl DataExporter for QuestExporter {
    fn name(&self) -> &'static str {
        "quest"
    }

    async fn export(&self, user_id: Uuid) -> AppResult<serde_json::Value> {
        Ok(serde_json::json!({
            "profile": self.json(
                "SELECT (SELECT jsonb_build_object('xp', xp, 'level', level, 'title_badge', title_badge, 'showcase', showcase,
                            'last_active_day', last_active_day, 'show_on_leaderboards', show_on_leaderboards,
                            'show_on_profile', show_on_profile, 'streak_reminders', streak_reminders, 'created_at', created_at)
                         FROM quest_profiles WHERE user_id = $1)", user_id).await?,
            "xp_by_day": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('day', day, 'source', source, 'xp', xp) ORDER BY day, source), '[]'::jsonb)
                 FROM quest_xp_days WHERE user_id = $1", user_id).await?,
            "counters": self.json(
                "SELECT COALESCE(jsonb_object_agg(metric, value), '{}'::jsonb) FROM quest_counters WHERE user_id = $1", user_id).await?,
            "badges": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('badge', badge_key, 'earned_at', earned_at, 'times', times, 'data', data) ORDER BY earned_at), '[]'::jsonb)
                 FROM quest_badges WHERE user_id = $1", user_id).await?,
            "missions": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('mission', mission_key, 'period', period, 'period_start', period_start, 'progress', progress,
                            'target', target, 'reward_coins', reward_coins, 'reward_xp', reward_xp, 'reward_badge', reward_badge,
                            'completed_at', completed_at, 'claimed_at', claimed_at) ORDER BY period_start, period, slot), '[]'::jsonb)
                 FROM quest_missions WHERE user_id = $1", user_id).await?,
            "streaks": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('friend_id', CASE WHEN user_lo = $1 THEN user_hi ELSE user_lo END, 'days', count,
                            'best_days', best, 'started_on', started_on, 'lost_days', lost_count, 'lost_at', lost_at) ORDER BY count DESC), '[]'::jsonb)
                 FROM quest_streaks WHERE user_lo = $1 OR user_hi = $1", user_id).await?,
            "streak_restores": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('month', month, 'friend_id', friend_id, 'cost', cost, 'restored_days', restored_count,
                            'created_at', created_at) ORDER BY month), '[]'::jsonb)
                 FROM quest_streak_restores WHERE user_id = $1", user_id).await?,
            "seasons": self.json(
                "SELECT COALESCE(jsonb_agg(jsonb_build_object('month', month, 'xp', xp, 'rank', rank, 'group_size', group_size, 'badge_won', badge_won) ORDER BY month), '[]'::jsonb)
                 FROM quest_season_results WHERE user_id = $1", user_id).await?,
        }))
    }
}

impl Quest {
    /// `identity.user.deleted`: removes everything the quest domain holds
    /// about the player, including the streaks they had with others. Safe
    /// to repeat.
    pub(crate) async fn erase_player(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        for table in [
            "quest_missions",
            "quest_mission_days",
            "quest_badges",
            "quest_counters",
            "quest_metric_days",
            "quest_xp_days",
            "quest_streak_restores",
            "quest_season_results",
            "quest_once",
            "quest_badge_actions",
            "quest_profiles",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE user_id = $1")))
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM quest_streaks WHERE user_lo = $1 OR user_hi = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM quest_direct_chats WHERE user_lo = $1 OR user_hi = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        // What others were credited once for doing with this player, and
        // what this player did as staff, lose the reference.
        sqlx::query("DELETE FROM quest_once WHERE about = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query(
            "UPDATE quest_badge_actions SET staff_id = '00000000-0000-0000-0000-000000000000' WHERE staff_id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        // Restores other players paid for a streak with this one keep their
        // month (it is their allowance), without the name.
        sqlx::query(
            "UPDATE quest_streak_restores SET friend_id = '00000000-0000-0000-0000-000000000000' WHERE friend_id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        // Counters still waiting in Valkey are dropped when their turn comes
        // (the account no longer resolves); nothing is written for it again.
        self.forget_account(user_id).await;
        Ok(())
    }
}
