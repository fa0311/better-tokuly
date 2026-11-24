//! Circles: the built-in close-friends list and custom lists of people to
//! share with.
//!
//! Members are drawn from the owner's friends and followers. A circle is an
//! audience for what its owner shares, so only people who already have a tie
//! to the owner belong in one: that keeps circles from becoming lists of
//! strangers, and someone who is in a circle would see what is shared with
//! it anyway through the tie. When the tie ends (unfriended and not a
//! follower, removed as a follower, a block, a deleted account) the member is
//! taken out of all of the owner's circles in the same transaction
//! ([`untie_from_circles`]); the audience check also requires the tie, which
//! covers a member added at the very moment it ended.
//!
//! Members are never told: adding and removing push and notify nothing, and
//! no RPC answers "which circles am I in". Only the owner reads a circle.
use std::{collections::HashSet, time::Duration};

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::{events, prelude::*};
use tokuly_identity::users;
use tokuly_proto::{
    common::PageInfo,
    social::{Circle, CircleCandidate, CircleColor, CircleKind, CircleMember},
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
    Social,
    events::{
        CIRCLE_CREATED, CIRCLE_DELETED, CIRCLE_MEMBERS_CHANGED, CIRCLE_UPDATED, CircleChanged, CircleDeleted,
        CircleMembersChanged,
    },
    graph, idcache,
};

/// Stored `kind` of the close-friends circle (`CircleKind.CLOSE_FRIENDS`).
pub(crate) const CLOSE_FRIENDS: i16 = CircleKind::CIRCLE_KIND_CLOSE_FRIENDS as i16;
const CUSTOM: i16 = CircleKind::CIRCLE_KIND_CUSTOM as i16;
/// Custom circles per user (close friends comes on top).
pub(crate) const MAX_CUSTOM_CIRCLES: i64 = 20;
/// Members per circle.
pub(crate) const MAX_MEMBERS: i64 = 1000;
/// IDs per add / remove call.
pub(crate) const MAX_IDS: usize = 100;
const MAX_NAME_GRAPHEMES: usize = 30;
const MAX_QUERY_CHARS: usize = 64;
const MAX_PAGE: i64 = 100;
const HOUR: Duration = Duration::from_secs(3600);

/// SQL: `{member}` has a tie to `{owner}` that lets them be in the owner's
/// circles (a friend, or a follower).
macro_rules! tied_sql {
    ($owner:literal, $member:literal) => {
        concat!(
            "(EXISTS (SELECT 1 FROM friendships t_ WHERE t_.user_id = ",
            $owner,
            " AND t_.friend_id = ",
            $member,
            ") OR EXISTS (SELECT 1 FROM follows t_ WHERE t_.follower_id = ",
            $member,
            " AND t_.followee_id = ",
            $owner,
            "))"
        )
    };
}

/// Columns of [`CircleRow`] for `circles c`.
macro_rules! circle_columns {
    () => {
        "c.id, c.kind, c.name, c.emoji, c.color, c.position, c.created_at, c.updated_at,
         (SELECT count(*) FROM circle_members m JOIN users u ON u.id = m.member_id
          WHERE m.circle_id = c.id AND u.deleted_at IS NULL) AS member_count"
    };
}

#[derive(Debug, sqlx::FromRow)]
struct CircleRow {
    id: Uuid,
    kind: i16,
    name: String,
    emoji: String,
    color: i16,
    #[allow(dead_code)] // ordering only
    position: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    member_count: i64,
}

impl From<CircleRow> for Circle {
    fn from(row: CircleRow) -> Self {
        Circle {
            id: row.id.to_string(),
            kind: i32::from(row.kind).into(),
            name: row.name,
            emoji: row.emoji,
            color: i32::from(row.color).into(),
            member_count: row.member_count as i32,
            created_at: ts(row.created_at),
            updated_at: ts(row.updated_at),
            ..Default::default()
        }
    }
}

/// The ID of `owner`'s close-friends circle, if it was ever made.
pub(crate) async fn close_friends_id(core: &Core, owner: Uuid) -> AppResult<Option<Uuid>> {
    Ok(sqlx::query_scalar("SELECT id FROM circles WHERE owner_id = $1 AND kind = $2")
        .bind(owner)
        .bind(CLOSE_FRIENDS)
        .fetch_optional(&core.db)
        .await?)
}

/// Members taken out of circles, for the cache invalidation after the commit.
#[derive(Debug, Default)]
pub(crate) struct CircleExits {
    circles: HashSet<Uuid>,
    members: HashSet<Uuid>,
}

impl CircleExits {
    fn add(&mut self, circle: Uuid, member: Uuid) {
        self.circles.insert(circle);
        self.members.insert(member);
    }

    pub(crate) fn merge(&mut self, other: CircleExits) {
        self.circles.extend(other.circles);
        self.members.extend(other.members);
    }

    /// Drops the cached lists the exits changed. Call after the commit.
    pub(crate) async fn invalidate(&self, core: &Core) {
        circles_changed(
            core,
            &self.circles.iter().copied().collect::<Vec<_>>(),
            &self.members.iter().copied().collect::<Vec<_>>(),
        )
        .await;
    }
}

/// Drops the cached member lists of `circles` and circle lists of `members`.
pub(crate) async fn circles_changed(core: &Core, circles: &[Uuid], members: &[Uuid]) {
    idcache::CIRCLE_MEMBERS.invalidate(core, circles).await;
    idcache::MEMBER_OF.invalidate(core, members).await;
}

/// Deletes `rows` of (circle, member) already removed, grouped into one
/// `members_changed` event per circle.
async fn announce_exits(
    tx: &mut Transaction<'_, Postgres>,
    owner_of: impl Fn(Uuid) -> Uuid,
    rows: &[(Uuid, Uuid)],
) -> AppResult<CircleExits> {
    let mut exits = CircleExits::default();
    let mut by_circle: std::collections::BTreeMap<Uuid, Vec<Uuid>> = Default::default();
    for (circle, member) in rows {
        exits.add(*circle, *member);
        by_circle.entry(*circle).or_default().push(*member);
    }
    for (circle_id, removed) in by_circle {
        let owner_id = owner_of(circle_id);
        let event = CircleMembersChanged { circle_id, owner_id, added: Vec::new(), removed };
        events::emit(&mut **tx, CIRCLE_MEMBERS_CHANGED, Some(owner_id), &event).await?;
    }
    Ok(exits)
}

/// Inside `tx`: takes `member` out of all of `owner`'s circles unless they
/// still have a tie to the owner (friend or follower). Call after anything
/// that may have ended the tie, under the pair's lock.
pub(crate) async fn untie_from_circles(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    member: Uuid,
) -> AppResult<CircleExits> {
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(concat!(
        "DELETE FROM circle_members WHERE owner_id = $1 AND member_id = $2 AND NOT ",
        tied_sql!("$1", "$2"),
        " RETURNING circle_id, member_id"
    ))
    .bind(owner)
    .bind(member)
    .fetch_all(&mut **tx)
    .await?;
    announce_exits(tx, |_| owner, &rows).await
}

/// [`untie_from_circles`] in both directions.
pub(crate) async fn untie_both(tx: &mut Transaction<'_, Postgres>, a: Uuid, b: Uuid) -> AppResult<CircleExits> {
    let mut exits = untie_from_circles(tx, a, b).await?;
    exits.merge(untie_from_circles(tx, b, a).await?);
    Ok(exits)
}

/// Trims a circle name and drops control characters.
fn normalize_name(input: &str) -> AppResult<String> {
    let name: String = input.chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();
    ensure!(!name.is_empty(), invalid, "NAME_REQUIRED", "a circle needs a name");
    ensure!(
        name.graphemes(true).count() <= MAX_NAME_GRAPHEMES,
        invalid,
        "NAME_TOO_LONG",
        "circle names are at most {MAX_NAME_GRAPHEMES} characters"
    );
    Ok(name.to_owned())
}

/// One emoji (a single grapheme that is not a letter, digit or space), or
/// nothing.
fn normalize_emoji(input: &str) -> AppResult<String> {
    let emoji = input.trim();
    if emoji.is_empty() {
        return Ok(String::new());
    }
    let single = emoji.graphemes(true).count() == 1 && emoji.len() <= 32;
    let pictographic =
        !emoji.chars().any(|c| c.is_alphanumeric() || c.is_whitespace() || c.is_control() || c.is_ascii_punctuation());
    ensure!(single && pictographic, invalid, "INVALID_EMOJI", "pick one emoji");
    Ok(emoji.to_owned())
}

fn color_code(color: Option<CircleColor>) -> i16 {
    color.unwrap_or(CircleColor::CIRCLE_COLOR_UNSPECIFIED) as i16
}

/// A `LIKE` pattern matching `query` anywhere, with its wildcards escaped.
fn contains_pattern(query: &str) -> String {
    format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

/// Takes, for the rest of the transaction, the lock that serializes changes
/// to the set (and order) of one owner's circles.
async fn lock_circles_of(tx: &mut Transaction<'_, Postgres>, owner: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("social:circles-of:{}", owner.simple()))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

impl Social {
    /// `owner`'s close-friends circle, made on first use.
    async fn ensure_close_friends(&self, owner: Uuid) -> AppResult<Uuid> {
        if let Some(id) = close_friends_id(&self.core, owner).await? {
            return Ok(id);
        }
        sqlx::query(
            "INSERT INTO circles (owner_id, kind) VALUES ($1, $2) ON CONFLICT (owner_id) WHERE kind = 1 DO NOTHING",
        )
        .bind(owner)
        .bind(CLOSE_FRIENDS)
        .execute(&self.core.db)
        .await?;
        close_friends_id(&self.core, owner)
            .await?
            .ok_or_else(|| AppError::internal(anyhow::anyhow!("close friends circle missing")))
    }

    /// The caller's circles: close friends first, then custom ones in order.
    pub(crate) async fn list_circles(&self, owner: Uuid) -> AppResult<Vec<Circle>> {
        self.ensure_close_friends(owner).await?;
        let rows: Vec<CircleRow> = sqlx::query_as(concat!(
            "SELECT ",
            circle_columns!(),
            " FROM circles c WHERE c.owner_id = $1 ORDER BY c.kind, c.position, c.id"
        ))
        .bind(owner)
        .fetch_all(&self.core.db)
        .await?;
        Ok(rows.into_iter().map(Circle::from).collect())
    }

    async fn circle_row(&self, owner: Uuid, circle_id: Uuid) -> AppResult<CircleRow> {
        let row: Option<CircleRow> = sqlx::query_as(concat!(
            "SELECT ",
            circle_columns!(),
            " FROM circles c WHERE c.id = $1 AND c.owner_id = $2"
        ))
        .bind(circle_id)
        .bind(owner)
        .fetch_optional(&self.core.db)
        .await?;
        row.or_not_found("CIRCLE_NOT_FOUND", "circle not found")
    }

    pub(crate) async fn get_circle(&self, owner: Uuid, circle_id: Uuid) -> AppResult<Circle> {
        Ok(self.circle_row(owner, circle_id).await?.into())
    }

    pub(crate) async fn create_circle(
        &self,
        owner: Uuid,
        name: &str,
        emoji: &str,
        color: Option<CircleColor>,
    ) -> AppResult<Circle> {
        let name = normalize_name(name)?;
        let emoji = normalize_emoji(emoji)?;
        self.core.cache.rate_limit(&format!("circle-create:{owner}"), 30, 24 * HOUR).await?;
        let mut tx = self.core.db.begin().await?;
        lock_circles_of(&mut tx, owner).await?;
        let (count, last): (i64, Option<i32>) =
            sqlx::query_as("SELECT count(*), max(position) FROM circles WHERE owner_id = $1 AND kind = $2")
                .bind(owner)
                .bind(CUSTOM)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            count < MAX_CUSTOM_CIRCLES,
            precondition,
            "TOO_MANY_CIRCLES",
            "you can have at most {MAX_CUSTOM_CIRCLES} circles"
        );
        let id: Uuid = sqlx::query_scalar("INSERT INTO circles (owner_id, kind, name, emoji, color, position) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id")
            .bind(owner)
            .bind(CUSTOM)
            .bind(&name)
            .bind(&emoji)
            .bind(color_code(color))
            .bind(last.unwrap_or(0) + 1)
            .fetch_one(&mut *tx)
            .await?;
        events::emit(
            &mut *tx,
            CIRCLE_CREATED,
            Some(owner),
            &CircleChanged { circle_id: id, owner_id: owner, kind: i32::from(CUSTOM) },
        )
        .await?;
        tx.commit().await?;
        self.get_circle(owner, id).await
    }

    pub(crate) async fn update_circle(
        &self,
        owner: Uuid,
        circle_id: Uuid,
        name: Option<&str>,
        emoji: Option<&str>,
        color: Option<CircleColor>,
    ) -> AppResult<Circle> {
        let current = self.circle_row(owner, circle_id).await?;
        if current.kind == CLOSE_FRIENDS {
            ensure!(
                name.is_none() && emoji.is_none(),
                precondition,
                "CLOSE_FRIENDS_FIXED",
                "close friends cannot be renamed"
            );
        }
        let name = name.map(normalize_name).transpose()?;
        let emoji = emoji.map(normalize_emoji).transpose()?;
        let mut tx = self.core.db.begin().await?;
        let updated = sqlx::query(
            "UPDATE circles SET name = COALESCE($3, name), emoji = COALESCE($4, emoji), color = COALESCE($5, color)
             WHERE id = $1 AND owner_id = $2",
        )
        .bind(circle_id)
        .bind(owner)
        .bind(name)
        .bind(emoji)
        .bind(color.map(|color| color_code(Some(color))))
        .execute(&mut *tx)
        .await?;
        ensure!(updated.rows_affected() > 0, not_found, "CIRCLE_NOT_FOUND", "circle not found");
        events::emit(
            &mut *tx,
            CIRCLE_UPDATED,
            Some(owner),
            &CircleChanged { circle_id, owner_id: owner, kind: i32::from(current.kind) },
        )
        .await?;
        tx.commit().await?;
        self.get_circle(owner, circle_id).await
    }

    pub(crate) async fn delete_circle(&self, owner: Uuid, circle_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        lock_circles_of(&mut tx, owner).await?;
        let kind: Option<i16> =
            sqlx::query_scalar("SELECT kind FROM circles WHERE id = $1 AND owner_id = $2 FOR UPDATE")
                .bind(circle_id)
                .bind(owner)
                .fetch_optional(&mut *tx)
                .await?;
        let kind = kind.or_not_found("CIRCLE_NOT_FOUND", "circle not found")?;
        ensure!(kind != CLOSE_FRIENDS, precondition, "CLOSE_FRIENDS_FIXED", "close friends cannot be deleted");
        let members: Vec<Uuid> =
            sqlx::query_scalar("DELETE FROM circle_members WHERE circle_id = $1 RETURNING member_id")
                .bind(circle_id)
                .fetch_all(&mut *tx)
                .await?;
        sqlx::query("DELETE FROM circles WHERE id = $1").bind(circle_id).execute(&mut *tx).await?;
        events::emit(
            &mut *tx,
            CIRCLE_DELETED,
            Some(owner),
            &CircleDeleted { circle_id, owner_id: owner, member_ids: members.clone() },
        )
        .await?;
        tx.commit().await?;
        circles_changed(&self.core, &[circle_id], &members).await;
        Ok(())
    }

    pub(crate) async fn reorder_circles(&self, owner: Uuid, order: &[Uuid]) -> AppResult<Vec<Circle>> {
        let mut tx = self.core.db.begin().await?;
        lock_circles_of(&mut tx, owner).await?;
        let mut custom: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM circles WHERE owner_id = $1 AND kind = $2")
            .bind(owner)
            .bind(CUSTOM)
            .fetch_all(&mut *tx)
            .await?;
        let mut wanted = order.to_vec();
        custom.sort_unstable();
        wanted.sort_unstable();
        ensure!(custom == wanted, invalid, "INVALID_ORDER", "name every custom circle exactly once");
        sqlx::query("UPDATE circles c SET position = o.n FROM unnest($1::uuid[]) WITH ORDINALITY AS o(id, n) WHERE c.id = o.id AND c.owner_id = $2")
            .bind(order)
            .bind(owner)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.list_circles(owner).await
    }

    pub(crate) async fn add_circle_members(
        &self,
        owner: Uuid,
        circle_id: Uuid,
        user_ids: &[Uuid],
    ) -> AppResult<Circle> {
        ensure!(user_ids.len() <= MAX_IDS, invalid, "TOO_MANY_IDS", "at most {MAX_IDS} people at a time");
        let mut wanted = user_ids.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        ensure!(!wanted.contains(&owner), precondition, "NOT_CONNECTED", "you cannot be in your own circle");
        if wanted.is_empty() {
            return self.get_circle(owner, circle_id).await;
        }
        let core = &self.core;
        core.cache.rate_limit(&format!("circle-members:{owner}"), 600, HOUR).await?;

        let mut tx = core.db.begin().await?;
        // Under the pair locks a tie cannot end between the check and the
        // insert (ending one takes the same lock). Taken in one fixed order.
        let mut keys: Vec<String> = wanted.iter().map(|member| graph::pair_lock_key(owner, *member)).collect();
        keys.sort_unstable();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(k, 0)) FROM (SELECT k FROM unnest($1::text[]) AS k ORDER BY k) AS keys")
            .bind(&keys)
            .execute(&mut *tx)
            .await?;
        let owned: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM circles WHERE id = $1 AND owner_id = $2 FOR UPDATE")
                .bind(circle_id)
                .bind(owner)
                .fetch_optional(&mut *tx)
                .await?;
        owned.or_not_found("CIRCLE_NOT_FOUND", "circle not found")?;
        let tied: i64 = sqlx::query_scalar(concat!(
            "SELECT count(*) FROM unnest($2::uuid[]) AS w(id) JOIN users u ON u.id = w.id
             WHERE u.deleted_at IS NULL AND u.banned_at IS NULL AND ",
            tied_sql!("$1", "w.id")
        ))
        .bind(owner)
        .bind(&wanted)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(tied == wanted.len() as i64, precondition, "NOT_CONNECTED", "circles take friends and followers only");
        let current: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM circle_members WHERE circle_id = $1 AND NOT (member_id = ANY($2))",
        )
        .bind(circle_id)
        .bind(&wanted)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            current + wanted.len() as i64 <= MAX_MEMBERS,
            precondition,
            "TOO_MANY_MEMBERS",
            "a circle has at most {MAX_MEMBERS} people"
        );
        let added: Vec<Uuid> = sqlx::query_scalar(
            "INSERT INTO circle_members (circle_id, owner_id, member_id) SELECT $1, $2, id FROM unnest($3::uuid[]) AS w(id)
             ON CONFLICT DO NOTHING RETURNING member_id",
        )
        .bind(circle_id)
        .bind(owner)
        .bind(&wanted)
        .fetch_all(&mut *tx)
        .await?;
        if !added.is_empty() {
            let event = CircleMembersChanged { circle_id, owner_id: owner, added: added.clone(), removed: Vec::new() };
            events::emit(&mut *tx, CIRCLE_MEMBERS_CHANGED, Some(owner), &event).await?;
            sqlx::query("UPDATE circles SET updated_at = now() WHERE id = $1")
                .bind(circle_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        circles_changed(core, &[circle_id], &added).await;
        self.get_circle(owner, circle_id).await
    }

    pub(crate) async fn remove_circle_members(
        &self,
        owner: Uuid,
        circle_id: Uuid,
        user_ids: &[Uuid],
    ) -> AppResult<Circle> {
        ensure!(user_ids.len() <= MAX_IDS, invalid, "TOO_MANY_IDS", "at most {MAX_IDS} people at a time");
        let mut tx = self.core.db.begin().await?;
        let owned: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM circles WHERE id = $1 AND owner_id = $2 FOR UPDATE")
                .bind(circle_id)
                .bind(owner)
                .fetch_optional(&mut *tx)
                .await?;
        owned.or_not_found("CIRCLE_NOT_FOUND", "circle not found")?;
        let removed: Vec<Uuid> = sqlx::query_scalar(
            "DELETE FROM circle_members WHERE circle_id = $1 AND member_id = ANY($2) RETURNING member_id",
        )
        .bind(circle_id)
        .bind(user_ids)
        .fetch_all(&mut *tx)
        .await?;
        if !removed.is_empty() {
            let event =
                CircleMembersChanged { circle_id, owner_id: owner, added: Vec::new(), removed: removed.clone() };
            events::emit(&mut *tx, CIRCLE_MEMBERS_CHANGED, Some(owner), &event).await?;
            sqlx::query("UPDATE circles SET updated_at = now() WHERE id = $1")
                .bind(circle_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        circles_changed(&self.core, &[circle_id], &removed).await;
        self.get_circle(owner, circle_id).await
    }

    pub(crate) async fn list_circle_members(
        &self,
        owner: Uuid,
        circle_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<CircleMember>, PageInfo, i32)> {
        let circle = self.circle_row(owner, circle_id).await?;
        let cursor: Option<(DateTime<Utc>, Uuid)> = page.decode()?;
        let (after_time, after_id) = cursor.unzip();
        #[derive(sqlx::FromRow)]
        struct Row {
            member_id: Uuid,
            added_at: DateTime<Utc>,
            friend: bool,
            follower: bool,
        }
        let mut rows: Vec<Row> = sqlx::query_as(
            "SELECT m.member_id, m.added_at,
                    EXISTS (SELECT 1 FROM friendships f WHERE f.user_id = $2 AND f.friend_id = m.member_id) AS friend,
                    EXISTS (SELECT 1 FROM follows f WHERE f.follower_id = m.member_id AND f.followee_id = $2) AS follower
             FROM circle_members m JOIN users u ON u.id = m.member_id
             WHERE m.circle_id = $1 AND u.deleted_at IS NULL
               AND ($3::timestamptz IS NULL OR (m.added_at, m.member_id) < ($3, $4))
             ORDER BY m.added_at DESC, m.member_id DESC
             LIMIT $5",
        )
        .bind(circle_id)
        .bind(owner)
        .bind(after_time)
        .bind(after_id)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |r| (r.added_at, r.member_id));
        let ids: Vec<Uuid> = rows.iter().map(|r| r.member_id).collect();
        let summaries = tokuly_identity::summaries(&self.core, &ids).await?;
        let members = rows
            .into_iter()
            .map(|row| CircleMember {
                user: users::pick(&summaries, row.member_id).into(),
                added_at: ts(row.added_at),
                friend: row.friend,
                follower: row.follower,
                ..Default::default()
            })
            .collect();
        Ok((members, info, circle.member_count as i32))
    }

    /// The owner's friends and followers (active, not in a block), by
    /// handle, each marked when already in the circle.
    pub(crate) async fn list_circle_candidates(
        &self,
        owner: Uuid,
        circle_id: Uuid,
        query: &str,
        page: &Page,
    ) -> AppResult<(Vec<CircleCandidate>, PageInfo)> {
        self.circle_row(owner, circle_id).await?;
        let query = query.trim().trim_start_matches('@');
        ensure!(query.chars().count() <= MAX_QUERY_CHARS, invalid, "QUERY_TOO_LONG", "search query is too long");
        let after: Option<String> = page.decode()?;
        #[derive(sqlx::FromRow)]
        struct Row {
            id: Uuid,
            handle: String,
            friend: bool,
            follower: bool,
            member: bool,
        }
        let mut rows: Vec<Row> = sqlx::query_as(
            "WITH tied AS (
                 SELECT friend_id AS id FROM friendships WHERE user_id = $1
                 UNION SELECT follower_id FROM follows WHERE followee_id = $1
             )
             SELECT u.id, u.handle::text AS handle,
                    EXISTS (SELECT 1 FROM friendships f WHERE f.user_id = $1 AND f.friend_id = u.id) AS friend,
                    EXISTS (SELECT 1 FROM follows f WHERE f.follower_id = u.id AND f.followee_id = $1) AS follower,
                    EXISTS (SELECT 1 FROM circle_members m WHERE m.circle_id = $2 AND m.member_id = u.id) AS member
             FROM tied JOIN users u ON u.id = tied.id
             WHERE u.deleted_at IS NULL AND u.banned_at IS NULL
               AND NOT EXISTS (SELECT 1 FROM user_blocks b WHERE (b.blocker_id = $1 AND b.blocked_id = u.id) OR (b.blocker_id = u.id AND b.blocked_id = $1))
               AND ($3 OR u.display_name ILIKE $4 OR u.handle::text ILIKE $4)
               AND ($5::text IS NULL OR u.handle::text > $5)
             ORDER BY u.handle::text
             LIMIT $6",
        )
        .bind(owner)
        .bind(circle_id)
        .bind(query.is_empty())
        .bind(contains_pattern(query))
        .bind(after)
        .bind(page.fetch())
        .fetch_all(&self.core.db)
        .await?;
        let info = page.finish(&mut rows, |r| r.handle.clone());
        let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        let summaries = tokuly_identity::summaries(&self.core, &ids).await?;
        let candidates = rows
            .into_iter()
            .map(|row| CircleCandidate {
                user: users::pick(&summaries, row.id).into(),
                friend: row.friend,
                follower: row.follower,
                member: row.member,
                ..Default::default()
            })
            .collect();
        Ok((candidates, info))
    }

    pub(crate) fn circles_page(page: Option<&tokuly_proto::common::PageRequestView<'_>>) -> Page {
        Page::from_view(page, MAX_PAGE)
    }
}

/// Inside `tx`: drops every membership between `a` and `b` (each in the
/// other's circles), whatever their ties. For blocks.
pub(crate) async fn drop_memberships_between(
    tx: &mut Transaction<'_, Postgres>,
    a: Uuid,
    b: Uuid,
) -> AppResult<CircleExits> {
    let rows: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "DELETE FROM circle_members WHERE (owner_id = $1 AND member_id = $2) OR (owner_id = $2 AND member_id = $1)
         RETURNING circle_id, owner_id, member_id",
    )
    .bind(a)
    .bind(b)
    .fetch_all(&mut **tx)
    .await?;
    let owners: std::collections::HashMap<Uuid, Uuid> =
        rows.iter().map(|(circle, owner, _)| (*circle, *owner)).collect();
    let pairs: Vec<(Uuid, Uuid)> = rows.iter().map(|(circle, _, member)| (*circle, *member)).collect();
    announce_exits(tx, |circle| owners[&circle], &pairs).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_limited() {
        assert_eq!(normalize_name("  部活 \u{0007} ").unwrap(), "部活");
        assert_eq!(normalize_name("   ").unwrap_err().reason, "NAME_REQUIRED");
        assert!(normalize_name(&"あ".repeat(30)).is_ok());
        assert_eq!(normalize_name(&"あ".repeat(31)).unwrap_err().reason, "NAME_TOO_LONG");
    }

    #[test]
    fn an_emoji_is_one_pictograph_or_nothing() {
        assert_eq!(normalize_emoji(" 🏀 ").unwrap(), "🏀");
        assert_eq!(normalize_emoji("👨‍👩‍👧").unwrap(), "👨‍👩‍👧", "one grapheme, several code points");
        assert_eq!(normalize_emoji("").unwrap(), "");
        for bad in ["ab", "🏀🏀", "a", "1", "!"] {
            assert_eq!(normalize_emoji(bad).unwrap_err().reason, "INVALID_EMOJI", "{bad}");
        }
    }
}
