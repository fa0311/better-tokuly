//! Game titles for the "game" field: free text, with suggestions from a
//! table that is seeded with well-known titles and grows with the titles
//! creators actually use on public videos (and live streams, through
//! [`record_use`]).
use tokuly_core::prelude::*;

/// Counts one more public use of a game title, adding it to the suggestions
/// if it is new. Titles are matched case-insensitively.
pub(crate) async fn record_use_in(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, title: &str) -> AppResult<()> {
    let title = title.trim();
    if title.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO video_games (title, use_count) VALUES ($1::citext, 1)
         ON CONFLICT (title) DO UPDATE SET use_count = video_games.use_count + 1",
    )
    .bind(title)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// [`record_use_in`] for other domains (a live stream went on air with this
/// game). The title should already be validated by the caller.
pub async fn record_use(core: &Core, title: &str) -> AppResult<()> {
    let mut tx = core.db.begin().await?;
    record_use_in(&mut tx, title).await?;
    tx.commit().await?;
    Ok(())
}

/// Titles that start with or resemble `query`, most used first; the most
/// used titles overall for an empty query.
pub async fn suggest(core: &Core, query: &str, limit: i64) -> AppResult<Vec<String>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(sqlx::query_scalar("SELECT title::text FROM video_games ORDER BY use_count DESC, title LIMIT $1")
            .bind(limit)
            .fetch_all(&core.db)
            .await?);
    }
    let escaped = query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    Ok(sqlx::query_scalar(
        "SELECT title::text FROM video_games
         WHERE title::text ILIKE $2 OR title::text % $1
         ORDER BY (title::text ILIKE $3) DESC, similarity(title::text, $1) DESC, use_count DESC, title
         LIMIT $4",
    )
    .bind(query)
    .bind(format!("%{escaped}%"))
    .bind(format!("{escaped}%"))
    .bind(limit)
    .fetch_all(&core.db)
    .await?)
}
