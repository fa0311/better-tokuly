//! Lists (リスト): named sets of accounts with a timeline of their own.
//!
//! - A list is its owner's; members are added without following them.
//! - A **public** list shows on the owner's profile, can be followed by
//!   others, and tells the people added to it (`notes.list_added`), who can
//!   take themselves off it ([`Notes::leave_list`]). A private list is the
//!   owner's alone: nobody else can tell that it exists.
//! - Nobody is added across a block, and a block takes the two off each
//!   other's lists and list follows ([`Notes::sever_list_ties`]).
//! - A locked account goes on a public list only when the owner is a
//!   follower it approved (or a friend); on a private list, anyone.
//! - Limits: 50 lists per owner, 500 members per list, 50 added per call,
//!   200 followed lists per person.
//!
//! What a list shows a viewer is always filtered the same way: the list
//! itself is visible to its owner, or to anyone when it is public and its
//! owner is an account in good standing no block hides from them; members
//! hidden from the viewer by a block, and suspended or deleted accounts,
//! are left out of the member list, the member count and the timeline.
use std::{collections::HashSet, time::Duration};

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use tokuly_core::{
    outbound::{self, Localized, NotificationRequest},
    prelude::*,
};
use tokuly_identity::users::pick;
use tokuly_proto::{
    common::{PageInfo, UserSummary},
    notes::{NoteList, TimelineEntry},
};
use tokuly_social::graph;

use crate::{
    Notes, channels,
    copy::display_name,
    model::list_path,
    text,
    timelines::with_renotes,
    visibility::{Viewer, not_hidden},
};

/// Lists one person may own.
pub const MAX_LISTS: i64 = 50;
/// Members of one list.
pub const MAX_MEMBERS: i64 = 500;
/// Accounts added or removed in one call.
pub const MAX_MEMBERS_PER_CALL: usize = 50;
/// Public lists one person may follow.
pub const MAX_FOLLOWED_LISTS: i64 = 200;
pub const MAX_NAME_CHARS: usize = 50;
pub const MAX_DESCRIPTION_CHARS: usize = 200;

/// Lists created, per person and hour.
const CREATES_PER_HOUR: u32 = 30;
/// Changes to lists (renames, audience changes, member changes), per person and minute.
const CHANGES_PER_MINUTE: u32 = 60;
/// Follows of lists, per person and minute.
const FOLLOWS_PER_MINUTE: u32 = 60;
/// `notes.list_added` notifications one owner may cause per hour; beyond
/// that people are still added, just not told.
const ADDED_NOTICES_PER_HOUR: u32 = 100;
/// One person is told about being on one list at most once in this time.
const ADDED_NOTICE_WINDOW: Duration = Duration::from_secs(24 * 3600);

/// A row of `note_lists`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ListRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub description: String,
    pub public: bool,
    pub follower_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const LIST_COLUMNS: &str =
    "l.id, l.owner_id, l.name, l.description, l.public, l.follower_count, l.created_at, l.updated_at";

/// SQL condition: the list in alias `l` is visible to the viewer (`$1`–`$6`):
/// their own, or public with an owner in good standing that no block hides.
fn sees_list(l: &str) -> String {
    format!(
        "({l}.owner_id = $1 OR ({l}.public AND {l}.owner_id <> ALL($3) AND EXISTS (
             SELECT 1 FROM users lu_ WHERE lu_.id = {l}.owner_id AND lu_.banned_at IS NULL AND lu_.deleted_at IS NULL)))"
    )
}

/// SQL condition: the member in alias `m` counts for the viewer: an account
/// in good standing that no block hides from them.
fn shown_member(m: &str) -> String {
    format!(
        "({hidden} AND EXISTS (SELECT 1 FROM users mu_ WHERE mu_.id = {m}.user_id AND mu_.banned_at IS NULL AND mu_.deleted_at IS NULL))",
        hidden = not_hidden(&format!("{m}.user_id")),
    )
}

/// What a person typed as a list's name and description, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListInput {
    pub name: String,
    pub description: String,
    pub public: bool,
}

impl ListInput {
    pub fn parse(name: &str, description: &str, public: bool) -> AppResult<Self> {
        let name = text::clean(name).replace('\n', " ");
        ensure!(
            !name.is_empty() && text::fits(&name, MAX_NAME_CHARS),
            invalid,
            "INVALID_NAME",
            "a list name is 1–{MAX_NAME_CHARS} characters"
        );
        let description = text::clean(description);
        ensure!(
            text::fits(&description, MAX_DESCRIPTION_CHARS),
            invalid,
            "DESCRIPTION_TOO_LONG",
            "a list description is at most {MAX_DESCRIPTION_CHARS} characters"
        );
        Ok(Self { name, description, public })
    }
}

/// Parses the accounts of a member change: at most 50, duplicates dropped.
pub(crate) fn member_ids<S: AsRef<str>>(ids: &[S]) -> AppResult<Vec<Uuid>> {
    ensure!(
        ids.len() <= MAX_MEMBERS_PER_CALL,
        invalid,
        "TOO_MANY_MEMBERS",
        "at most {MAX_MEMBERS_PER_CALL} accounts per call"
    );
    let mut seen = HashSet::new();
    Ok(parse_ids(ids)?.into_iter().filter(|id| seen.insert(*id)).collect())
}

/// Serializes changes to one person's set of lists (their number).
async fn lock_owner_lists(tx: &mut Transaction<'_, Postgres>, owner: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("notes:lists:{owner}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Serializes one person's list follows (their number).
async fn lock_list_follows(tx: &mut Transaction<'_, Postgres>, user_id: Uuid) -> AppResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("notes:list-follows:{user_id}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// `notes.list_added`: the owner of a public list added the recipient.
/// Collapsed per list, so being taken off and put back is one entry.
pub(crate) fn list_added_notice(owner: &UserSummary, list: &ListRow, recipient: Uuid) -> NotificationRequest {
    let name = display_name(owner);
    let title = Localized::new(
        format!("{name}さんがあなたをリスト「{}」に追加したよ!", list.name),
        format!("{name} added you to the list \"{}\"", list.name),
    );
    let body = Localized::new(
        "リストから外れたいときは、リストの画面からいつでも抜けられるよ",
        "You can leave the list from its page at any time",
    );
    let mut request = NotificationRequest::new(recipient, "notes.list_added", title, body, list_path(list.id))
        .actor(list.owner_id)
        .collapse(format!("notes.list_added:{}", list.id))
        .data(serde_json::json!({ "list_id": list.id, "list_name": list.name }));
    if let Some(avatar) = owner.avatar.as_option().filter(|avatar| !avatar.url.is_empty()) {
        request = request.image(avatar.url.clone());
    }
    request
}

impl Notes {
    /// The list, if the viewer may see it.
    pub(crate) async fn find_list(&self, viewer: &Viewer, list_id: Uuid) -> AppResult<Option<ListRow>> {
        let sql = format!("SELECT {LIST_COLUMNS} FROM note_lists l WHERE l.id = $7 AND {}", sees_list("l"));
        Ok(viewer.query_as(sql).bind(list_id).fetch_optional(&self.core.db).await?)
    }

    /// The list, or `LIST_NOT_FOUND` (also when the viewer may not see it).
    pub(crate) async fn require_list(&self, viewer: &Viewer, list_id: Uuid) -> AppResult<ListRow> {
        self.find_list(viewer, list_id).await?.or_not_found("LIST_NOT_FOUND", "list not found")
    }

    /// One of `owner`'s own lists, or `LIST_NOT_FOUND`.
    pub(crate) async fn require_own_list(&self, owner: Uuid, list_id: Uuid) -> AppResult<ListRow> {
        let sql = format!("SELECT {LIST_COLUMNS} FROM note_lists l WHERE l.id = $1 AND l.owner_id = $2");
        let row: Option<ListRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(list_id).bind(owner).fetch_optional(&self.core.db).await?;
        row.or_not_found("LIST_NOT_FOUND", "list not found")
    }

    /// Lists as `viewer` gets them, in the order of `rows`.
    pub(crate) async fn lists_to_proto(&self, viewer: &Viewer, rows: &[ListRow]) -> AppResult<Vec<NoteList>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let mut owners: Vec<Uuid> = rows.iter().map(|row| row.owner_id).collect();
        owners.sort_unstable();
        owners.dedup();
        let sql = format!(
            "SELECT m.list_id, count(*) FROM note_list_members m
             WHERE m.list_id = ANY($7) AND {} GROUP BY m.list_id",
            shown_member("m")
        );
        let counts: Vec<(Uuid, i64)> = viewer.query_as(sql).bind(&ids).fetch_all(&self.core.db).await?;
        let (following, member_of): (HashSet<Uuid>, HashSet<Uuid>) = match viewer.id {
            Some(user_id) => {
                let ties: Vec<(Uuid, bool)> = sqlx::query_as(
                    "SELECT list_id, true FROM note_list_follows WHERE user_id = $1 AND list_id = ANY($2)
                     UNION ALL
                     SELECT list_id, false FROM note_list_members WHERE user_id = $1 AND list_id = ANY($2)",
                )
                .bind(user_id)
                .bind(&ids)
                .fetch_all(&self.core.db)
                .await?;
                (
                    ties.iter().filter(|(_, follow)| *follow).map(|(id, _)| *id).collect(),
                    ties.iter().filter(|(_, follow)| !*follow).map(|(id, _)| *id).collect(),
                )
            }
            None => (HashSet::new(), HashSet::new()),
        };
        let users = tokuly_identity::summaries(&self.core, &owners).await?;
        Ok(rows
            .iter()
            .map(|row| NoteList {
                id: row.id.to_string(),
                owner: pick(&users, row.owner_id).into(),
                name: row.name.clone(),
                description: row.description.clone(),
                public: row.public,
                member_count: counts
                    .iter()
                    .find(|(id, _)| *id == row.id)
                    .map_or(0, |(_, count)| (*count).min(i64::from(i32::MAX)) as i32),
                follower_count: row.follower_count.clamp(0, i64::from(i32::MAX)) as i32,
                following: following.contains(&row.id),
                is_member: member_of.contains(&row.id),
                created_at: ts(row.created_at),
                updated_at: ts(row.updated_at),
                ..Default::default()
            })
            .collect())
    }

    pub(crate) async fn list_to_proto(&self, viewer: &Viewer, row: &ListRow) -> AppResult<NoteList> {
        Ok(self.lists_to_proto(viewer, std::slice::from_ref(row)).await?.pop().expect("one list in, one out"))
    }

    /// Creates a list of `owner`'s (at most [`MAX_LISTS`]).
    pub(crate) async fn create_list(&self, owner: Uuid, input: ListInput) -> AppResult<ListRow> {
        self.core
            .cache
            .rate_limit(&format!("notes:list-create:{owner}"), CREATES_PER_HOUR, Duration::from_secs(3600))
            .await?;
        let mut tx = self.core.db.begin().await?;
        lock_owner_lists(&mut tx, owner).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM note_lists WHERE owner_id = $1")
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(count < MAX_LISTS, precondition, "TOO_MANY_LISTS", "at most {MAX_LISTS} lists");
        let sql = format!(
            "INSERT INTO note_lists AS l (owner_id, name, description, public) VALUES ($1, $2, $3, $4) RETURNING {LIST_COLUMNS}"
        );
        let row: ListRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(owner)
            .bind(&input.name)
            .bind(&input.description)
            .bind(input.public)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(row)
    }

    /// Renames a list or changes who may see it.
    ///
    /// A list made private loses its followers (nobody else may see it any
    /// more). A list made public loses the locked accounts on it that would
    /// not be allowed on a public list, and tells the others they are on it.
    pub(crate) async fn update_list(&self, owner: Uuid, list_id: Uuid, input: ListInput) -> AppResult<ListRow> {
        self.core
            .cache
            .rate_limit(&format!("notes:list-change:{owner}"), CHANGES_PER_MINUTE, Duration::from_secs(60))
            .await?;
        let before = self.require_own_list(owner, list_id).await?;
        // Read before the transaction: the social domain's helpers use the pool.
        let not_allowed = if input.public && !before.public {
            let members: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM note_list_members WHERE list_id = $1")
                .bind(list_id)
                .fetch_all(&self.core.db)
                .await?;
            self.not_allowed_on_public_list(owner, &members).await?
        } else {
            Vec::new()
        };

        let owner_summary = tokuly_identity::summary(&self.core, owner).await?;

        let mut tx = self.core.db.begin().await?;
        let sql = format!(
            "UPDATE note_lists AS l SET name = $3, description = $4, public = $5,
                    follower_count = CASE WHEN $5 THEN l.follower_count ELSE 0 END
             WHERE l.id = $1 AND l.owner_id = $2 RETURNING {LIST_COLUMNS}"
        );
        let row: Option<ListRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(list_id)
            .bind(owner)
            .bind(&input.name)
            .bind(&input.description)
            .bind(input.public)
            .fetch_optional(&mut *tx)
            .await?;
        let row = row.or_not_found("LIST_NOT_FOUND", "list not found")?;
        if !row.public {
            sqlx::query("DELETE FROM note_list_follows WHERE list_id = $1").bind(list_id).execute(&mut *tx).await?;
        }
        let mut told = Vec::new();
        if row.public && !before.public {
            sqlx::query("DELETE FROM note_list_members WHERE list_id = $1 AND user_id = ANY($2)")
                .bind(list_id)
                .bind(&not_allowed)
                .execute(&mut *tx)
                .await?;
            told = sqlx::query_scalar("SELECT user_id FROM note_list_members WHERE list_id = $1 AND user_id <> $2")
                .bind(list_id)
                .bind(owner)
                .fetch_all(&mut *tx)
                .await?;
        }
        self.tell_added(&mut tx, &owner_summary, &row, &told).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// Deletes one of `owner`'s lists, with its members and follows (and
    /// the antennas that collected from it).
    pub(crate) async fn delete_list(&self, owner: Uuid, list_id: Uuid) -> AppResult<()> {
        let deleted = sqlx::query("DELETE FROM note_lists WHERE id = $1 AND owner_id = $2")
            .bind(list_id)
            .bind(owner)
            .execute(&self.core.db)
            .await?
            .rows_affected();
        ensure!(deleted == 1, not_found, "LIST_NOT_FOUND", "list not found");
        // Antennas collecting from the list went with it.
        self.antennas_changed().await;
        Ok(())
    }

    /// The lists of `user_id` the viewer may see (all of them for the
    /// owner, public ones for others), oldest first.
    pub(crate) async fn lists_of(&self, viewer: &Viewer, user_id: Uuid) -> AppResult<Vec<NoteList>> {
        self.require_user(viewer, user_id).await?;
        let sql = format!(
            "SELECT {LIST_COLUMNS} FROM note_lists l WHERE l.owner_id = $7 AND {} ORDER BY l.id",
            sees_list("l")
        );
        let rows: Vec<ListRow> = viewer.query_as(sql).bind(user_id).fetch_all(&self.core.db).await?;
        self.lists_to_proto(viewer, &rows).await
    }

    /// Public lists `user_id` follows, most recently followed first.
    pub(crate) async fn followed_lists(
        &self,
        viewer: &Viewer,
        user_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<NoteList>, PageInfo)> {
        let cursor: Option<Uuid> = page.decode()?;
        let sql = format!(
            "SELECT f.id AS key, {LIST_COLUMNS} FROM note_list_follows f JOIN note_lists l ON l.id = f.list_id
             WHERE f.user_id = $7 AND ($8::uuid IS NULL OR f.id < $8) AND {}
             ORDER BY f.id DESC LIMIT $9",
            sees_list("l")
        );
        self.keyed_lists(viewer, page, sql, user_id, cursor).await
    }

    /// Public lists `user_id` is on, most recently added first.
    pub(crate) async fn lists_containing(
        &self,
        viewer: &Viewer,
        user_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<NoteList>, PageInfo)> {
        let cursor: Option<Uuid> = page.decode()?;
        let sql = format!(
            "SELECT m.id AS key, {LIST_COLUMNS} FROM note_list_members m JOIN note_lists l ON l.id = m.list_id
             WHERE m.user_id = $7 AND l.public AND l.owner_id <> $7 AND ($8::uuid IS NULL OR m.id < $8) AND {}
             ORDER BY m.id DESC LIMIT $9",
            sees_list("l")
        );
        self.keyed_lists(viewer, page, sql, user_id, cursor).await
    }

    async fn keyed_lists(
        &self,
        viewer: &Viewer,
        page: &Page,
        sql: String,
        user_id: Uuid,
        cursor: Option<Uuid>,
    ) -> AppResult<(Vec<NoteList>, PageInfo)> {
        #[derive(sqlx::FromRow)]
        struct Keyed {
            key: Uuid,
            #[sqlx(flatten)]
            list: ListRow,
        }
        let mut rows: Vec<Keyed> =
            viewer.query_as(sql).bind(user_id).bind(cursor).bind(page.fetch()).fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |row| row.key);
        let lists: Vec<ListRow> = rows.into_iter().map(|row| row.list).collect();
        Ok((self.lists_to_proto(viewer, &lists).await?, info))
    }

    /// Of `candidates`, the locked accounts that `owner` may not put on a
    /// public list: those that have not approved them as a follower (and
    /// are not their friend).
    async fn not_allowed_on_public_list(&self, owner: Uuid, candidates: &[Uuid]) -> AppResult<Vec<Uuid>> {
        let others: Vec<Uuid> = candidates.iter().copied().filter(|id| *id != owner).collect();
        let locked = graph::locked_among(&self.core, &others).await?;
        if locked.is_empty() {
            return Ok(Vec::new());
        }
        let (following, friends) =
            tokio::try_join!(graph::following_ids(&self.core, owner), graph::friend_ids(&self.core, owner))?;
        let tied: HashSet<Uuid> = following.into_iter().chain(friends).collect();
        Ok(others.into_iter().filter(|id| locked.contains(id) && !tied.contains(id)).collect())
    }

    /// Adds accounts to one of `owner`'s lists.
    ///
    /// Every account must be in good standing with no block between it and
    /// the owner (`USER_NOT_FOUND` otherwise, so a block cannot be told from
    /// a missing account), and a locked one must allow the owner to follow
    /// it when the list is public (`MEMBER_NOT_ALLOWED`). The list's member
    /// limit holds under concurrent calls: the list row is locked while
    /// members are added and counted.
    pub(crate) async fn add_list_members(&self, owner: Uuid, list_id: Uuid, user_ids: &[Uuid]) -> AppResult<ListRow> {
        self.core
            .cache
            .rate_limit(&format!("notes:list-change:{owner}"), CHANGES_PER_MINUTE, Duration::from_secs(60))
            .await?;
        let list = self.require_own_list(owner, list_id).await?;
        if user_ids.is_empty() {
            return Ok(list);
        }
        // Everything asked of other domains happens before the transaction.
        let active = tokuly_identity::users::filter_active(&self.core, user_ids).await?;
        let hidden: HashSet<Uuid> = graph::blocked_ids_either(&self.core, owner).await?.into_iter().collect();
        ensure!(
            active.len() == user_ids.len() && user_ids.iter().all(|id| !hidden.contains(id)),
            not_found,
            "USER_NOT_FOUND",
            "user not found"
        );
        let not_allowed = self.not_allowed_on_public_list(owner, user_ids).await?;
        let owner_summary = tokuly_identity::summary(&self.core, owner).await?;

        let mut tx = self.core.db.begin().await?;
        let sql = format!("SELECT {LIST_COLUMNS} FROM note_lists l WHERE l.id = $1 AND l.owner_id = $2 FOR UPDATE");
        let locked: Option<ListRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(list_id).bind(owner).fetch_optional(&mut *tx).await?;
        let list = locked.or_not_found("LIST_NOT_FOUND", "list not found")?;
        ensure!(
            !list.public || not_allowed.is_empty(),
            denied,
            "MEMBER_NOT_ALLOWED",
            "a locked account can only be on a public list of someone it lets follow it"
        );
        let added: Vec<Uuid> = sqlx::query_scalar(
            "INSERT INTO note_list_members (list_id, user_id) SELECT $1, u FROM unnest($2::uuid[]) AS u
             ON CONFLICT DO NOTHING RETURNING user_id",
        )
        .bind(list_id)
        .bind(user_ids)
        .fetch_all(&mut *tx)
        .await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM note_list_members WHERE list_id = $1")
            .bind(list_id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(count <= MAX_MEMBERS, precondition, "TOO_MANY_MEMBERS", "a list holds at most {MAX_MEMBERS} accounts");
        if list.public {
            let others: Vec<Uuid> = added.iter().copied().filter(|id| *id != owner).collect();
            self.tell_added(&mut tx, &owner_summary, &list, &others).await?;
        }
        tx.commit().await?;

        // A block that landed between the check and the commit: its event
        // may already have been handled, so undo what it would have undone.
        let hidden_now = graph::blocked_ids_either(&self.core, owner).await?;
        let crossed: Vec<Uuid> = added.iter().copied().filter(|id| hidden_now.contains(id)).collect();
        if !crossed.is_empty() {
            sqlx::query("DELETE FROM note_list_members WHERE list_id = $1 AND user_id = ANY($2)")
                .bind(list_id)
                .bind(&crossed)
                .execute(&self.core.db)
                .await?;
        }
        Ok(list)
    }

    /// Queues `notes.list_added` for the people just put on a public list:
    /// once per person and list a day, and at most
    /// [`ADDED_NOTICES_PER_HOUR`] per owner (beyond that nobody is told).
    async fn tell_added(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        owner: &UserSummary,
        list: &ListRow,
        user_ids: &[Uuid],
    ) -> AppResult<()> {
        if user_ids.is_empty() || !list.public {
            return Ok(());
        }
        // Valkey only, no database connection: fine inside the transaction.
        let mut recipients = Vec::new();
        for user_id in user_ids {
            if !self
                .core
                .cache
                .set_nx(&format!("notes:notified:list:{}:{user_id}", list.id), "1", ADDED_NOTICE_WINDOW)
                .await?
            {
                continue;
            }
            if self
                .core
                .cache
                .rate_limit(
                    &format!("notes:list-notices:{}", list.owner_id),
                    ADDED_NOTICES_PER_HOUR,
                    Duration::from_secs(3600),
                )
                .await
                .is_err()
            {
                break;
            }
            recipients.push(*user_id);
        }
        if recipients.is_empty() {
            return Ok(());
        }
        for recipient in recipients {
            outbound::notify(&mut **tx, &list_added_notice(owner, list, recipient)).await?;
        }
        Ok(())
    }

    /// Takes accounts off one of `owner`'s lists.
    pub(crate) async fn remove_list_members(
        &self,
        owner: Uuid,
        list_id: Uuid,
        user_ids: &[Uuid],
    ) -> AppResult<ListRow> {
        let list = self.require_own_list(owner, list_id).await?;
        sqlx::query("DELETE FROM note_list_members WHERE list_id = $1 AND user_id = ANY($2)")
            .bind(list_id)
            .bind(user_ids)
            .execute(&self.core.db)
            .await?;
        Ok(list)
    }

    /// Members of a list the viewer may see, most recently added first.
    pub(crate) async fn list_members(
        &self,
        viewer: &Viewer,
        list_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<UserSummary>, PageInfo)> {
        self.require_list(viewer, list_id).await?;
        let cursor: Option<Uuid> = page.decode()?;
        let sql = format!(
            "SELECT m.id, m.user_id FROM note_list_members m
             WHERE m.list_id = $7 AND ($8::uuid IS NULL OR m.id < $8) AND {}
             ORDER BY m.id DESC LIMIT $9",
            shown_member("m")
        );
        let mut rows: Vec<(Uuid, Uuid)> =
            viewer.query_as(sql).bind(list_id).bind(cursor).bind(page.fetch()).fetch_all(&self.core.db).await?;
        let info = page.finish(&mut rows, |(key, _)| *key);
        let ids: Vec<Uuid> = rows.iter().map(|(_, user_id)| *user_id).collect();
        Ok((tokuly_identity::summaries_ordered(&self.core, &ids).await?, info))
    }

    /// Takes the caller off a list they can see (someone's public list).
    pub(crate) async fn leave_list(&self, viewer: &Viewer, user_id: Uuid, list_id: Uuid) -> AppResult<()> {
        self.require_list(viewer, list_id).await?;
        sqlx::query("DELETE FROM note_list_members WHERE list_id = $1 AND user_id = $2")
            .bind(list_id)
            .bind(user_id)
            .execute(&self.core.db)
            .await?;
        Ok(())
    }

    /// Follows someone else's public list (at most [`MAX_FOLLOWED_LISTS`]).
    pub(crate) async fn follow_list(&self, viewer: &Viewer, user_id: Uuid, list_id: Uuid) -> AppResult<ListRow> {
        self.core
            .cache
            .rate_limit(&format!("notes:list-follow:{user_id}"), FOLLOWS_PER_MINUTE, Duration::from_secs(60))
            .await?;
        let list = self.require_list(viewer, list_id).await?;
        ensure!(list.owner_id != user_id, invalid, "CANNOT_FOLLOW_OWN_LIST", "that is your own list");

        let mut tx = self.core.db.begin().await?;
        lock_list_follows(&mut tx, user_id).await?;
        let already: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM note_list_follows WHERE list_id = $1 AND user_id = $2)")
                .bind(list_id)
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
        if !already {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM note_list_follows WHERE user_id = $1")
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
            ensure!(
                count < MAX_FOLLOWED_LISTS,
                precondition,
                "TOO_MANY_FOLLOWED_LISTS",
                "at most {MAX_FOLLOWED_LISTS} followed lists"
            );
            // Only a list that is still public is followed; the update takes
            // the row lock an audience change takes too.
            let followed = sqlx::query(
                "WITH l AS (UPDATE note_lists SET follower_count = follower_count + 1 WHERE id = $1 AND public RETURNING id)
                 INSERT INTO note_list_follows (list_id, user_id) SELECT id, $2 FROM l",
            )
            .bind(list_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            ensure!(followed == 1, not_found, "LIST_NOT_FOUND", "list not found");
        }
        tx.commit().await?;
        self.require_list(viewer, list_id).await
    }

    pub(crate) async fn unfollow_list(&self, user_id: Uuid, list_id: Uuid) -> AppResult<()> {
        sqlx::query(
            "WITH gone AS (DELETE FROM note_list_follows WHERE list_id = $1 AND user_id = $2 RETURNING list_id)
             UPDATE note_lists SET follower_count = GREATEST(follower_count - 1, 0) WHERE id IN (SELECT list_id FROM gone)",
        )
        .bind(list_id)
        .bind(user_id)
        .execute(&self.core.db)
        .await?;
        Ok(())
    }

    /// Notes and renotes of a list's members as the viewer may see them,
    /// plus the replies the members wrote to each other. Members hidden
    /// from the viewer by a block and accounts the viewer muted (as a
    /// followee) are left out, as on the following timeline.
    pub(crate) async fn list_timeline(
        &self,
        viewer: &Viewer,
        list_id: Uuid,
        page: &Page,
    ) -> AppResult<(Vec<TimelineEntry>, PageInfo)> {
        self.require_list(viewer, list_id).await?;
        let members: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM note_list_members WHERE list_id = $1")
            .bind(list_id)
            .fetch_all(&self.core.db)
            .await?;
        let muted: HashSet<Uuid> = match viewer.id {
            Some(user_id) => graph::muted_followee_ids(&self.core, user_id).await?.into_iter().collect(),
            None => HashSet::new(),
        };
        let people: Vec<Uuid> = members.into_iter().filter(|id| !viewer.hides(*id) && !muted.contains(id)).collect();
        let filter = format!(
            "n.author_id = ANY($7) AND (n.reply_to_id IS NULL OR n.reply_to_author_id = ANY($7)) AND {}",
            channels::outside_unfollowed_channels("n"),
        );
        let sql = with_renotes(&filter, "= ANY($7)");
        self.entries(viewer, page, sql, people).await
    }

    /// `social.user.blocked`: the two leave each other's lists and stop
    /// following each other's lists; an antenna of one stops naming the other.
    pub(crate) async fn sever_list_ties(&self, a: Uuid, b: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query(
            "DELETE FROM note_list_members m USING note_lists l
             WHERE l.id = m.list_id AND ((l.owner_id = $1 AND m.user_id = $2) OR (l.owner_id = $2 AND m.user_id = $1))",
        )
        .bind(a)
        .bind(b)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "WITH gone AS (
                 DELETE FROM note_list_follows f USING note_lists l
                 WHERE l.id = f.list_id AND ((l.owner_id = $1 AND f.user_id = $2) OR (l.owner_id = $2 AND f.user_id = $1))
                 RETURNING f.list_id)
             UPDATE note_lists SET follower_count = GREATEST(follower_count - 1, 0) WHERE id IN (SELECT list_id FROM gone)",
        )
        .bind(a)
        .bind(b)
        .execute(&mut *tx)
        .await?;
        let changed = sqlx::query(
            "UPDATE antennas SET user_ids = array_remove(user_ids, CASE WHEN owner_id = $1 THEN $2 ELSE $1 END)
             WHERE (owner_id = $1 AND user_ids @> ARRAY[$2]::uuid[]) OR (owner_id = $2 AND user_ids @> ARRAY[$1]::uuid[])",
        )
        .bind(a)
        .bind(b)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        if changed > 0 {
            self.antennas_changed().await;
        }
        Ok(())
    }

    /// `identity.user.deleted`: the account's lists (with their members and
    /// follows) and antennas go, it leaves everyone's lists, its list
    /// follows go (the counts they added go down again), and other people's
    /// antennas stop naming it.
    pub(crate) async fn erase_lists_and_antennas(&self, user_id: Uuid) -> AppResult<()> {
        let mut tx = self.core.db.begin().await?;
        sqlx::query(
            "WITH gone AS (DELETE FROM note_list_follows WHERE user_id = $1 RETURNING list_id)
             UPDATE note_lists SET follower_count = GREATEST(follower_count - 1, 0) WHERE id IN (SELECT list_id FROM gone)",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        for sql in [
            "DELETE FROM note_list_members WHERE user_id = $1",
            "DELETE FROM antennas WHERE owner_id = $1",
            "DELETE FROM note_lists WHERE owner_id = $1",
            "UPDATE antennas SET user_ids = array_remove(user_ids, $1) WHERE user_ids @> ARRAY[$1]::uuid[]",
        ] {
            sqlx::query(sql).bind(user_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        self.antennas_changed().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_descriptions_are_checked() {
        let input = ListInput::parse("  友達\nリスト ", " よろしく ", true).unwrap();
        assert_eq!(input, ListInput { name: "友達 リスト".into(), description: "よろしく".into(), public: true });
        assert_eq!(ListInput::parse("   ", "", false).unwrap_err().reason, "INVALID_NAME");
        assert_eq!(ListInput::parse(&"あ".repeat(51), "", false).unwrap_err().reason, "INVALID_NAME");
        assert!(ListInput::parse(&"あ".repeat(50), &"い".repeat(200), false).is_ok());
        assert_eq!(ListInput::parse("a", &"い".repeat(201), false).unwrap_err().reason, "DESCRIPTION_TOO_LONG");
    }

    #[test]
    fn member_changes_are_bounded_and_deduplicated() {
        let one = Uuid::now_v7().to_string();
        assert_eq!(member_ids(&[one.clone(), one.clone()]).unwrap().len(), 1);
        let many: Vec<String> = (0..51).map(|_| Uuid::now_v7().to_string()).collect();
        assert_eq!(member_ids(&many).unwrap_err().reason, "TOO_MANY_MEMBERS");
    }

    #[test]
    fn list_conditions_only_use_the_viewer_parameters() {
        for sql in [sees_list("l"), shown_member("m")] {
            for parameter in sql.split('$').skip(1) {
                let number: String = parameter.chars().take_while(char::is_ascii_digit).collect();
                assert!(matches!(number.as_str(), "1" | "3"), "unexpected parameter ${number} in {sql}");
            }
            assert_eq!(sql.matches('(').count(), sql.matches(')').count(), "unbalanced: {sql}");
        }
    }
}
