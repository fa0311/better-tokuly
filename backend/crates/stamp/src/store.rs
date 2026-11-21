//! Store queries: shelves, search, tags and a creator's published packs.
//! All of them only ever return published packs.
use serde::{Deserialize, Serialize};
use sqlx::QueryBuilder;
use tokuly_core::{page::encode_cursor, prelude::*};
use tokuly_proto::{common::PageInfo, stamp as pb};

use crate::{
    model::{PackRow, pack_columns},
    packs::push_price_filter,
    validate,
};

/// A shelf of the store and the order it is sorted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    /// Most recently featured first.
    Featured,
    /// Most recently published first.
    New,
    /// Highest popularity score first.
    Popular,
    /// Official packs, most recently published first.
    Official,
}

impl Shelf {
    pub fn from_proto(section: pb::StoreSection) -> Self {
        match section {
            pb::StoreSection::STORE_SECTION_FEATURED => Self::Featured,
            pb::StoreSection::STORE_SECTION_POPULAR => Self::Popular,
            pb::StoreSection::STORE_SECTION_OFFICIAL => Self::Official,
            pb::StoreSection::STORE_SECTION_NEW | pb::StoreSection::STORE_SECTION_UNSPECIFIED => Self::New,
        }
    }
}

/// Where the next page of a shelf starts: the sort key of the last pack
/// returned. Only the field the shelf sorts by is compared.
#[derive(Debug, Serialize, Deserialize)]
struct ShelfCursor {
    /// `featured_at` on the featured shelf, `published_at` elsewhere.
    at: Time,
    /// Bits of the popularity score. Sent as bits because a float that went
    /// through JSON may come back one ULP off, which would skip or repeat a pack.
    score_bits: u64,
    id: Uuid,
}

impl ShelfCursor {
    fn after(shelf: Shelf, pack: &PackRow) -> Self {
        let published_at = pack.published_at.unwrap_or(pack.created_at);
        let at = if shelf == Shelf::Featured { pack.featured_at.unwrap_or(published_at) } else { published_at };
        Self { at, score_bits: pack.popular_score.to_bits(), id: pack.id }
    }
}

/// One page of a shelf, optionally narrowed to a tag and a price class.
pub async fn shelf(
    core: &Core,
    shelf: Shelf,
    tag: Option<&str>,
    price: pb::PriceFilter,
    page: &Page,
) -> AppResult<(Vec<PackRow>, PageInfo)> {
    let cursor: Option<ShelfCursor> = page.decode()?;
    let mut query = QueryBuilder::new(concat!("SELECT ", pack_columns!(), " FROM stamp_packs p WHERE p.status = 3"));
    if let Some(tag) = tag {
        query.push(" AND p.tags @> ARRAY[").push_bind(tag.to_owned()).push("]::text[]");
    }
    push_price_filter(&mut query, price);
    match shelf {
        Shelf::Featured => {
            query.push(" AND p.featured_at IS NOT NULL");
        }
        Shelf::Official => {
            query.push(" AND p.official");
        }
        Shelf::New | Shelf::Popular => {}
    }
    // Keyset pagination: continue strictly after the last pack of the previous page.
    let sort_column = match shelf {
        Shelf::Featured => "p.featured_at",
        Shelf::New | Shelf::Official => "p.published_at",
        Shelf::Popular => "p.popular_score",
    };
    if let Some(cursor) = &cursor {
        query.push(" AND (").push(sort_column).push(", p.id) < (");
        if shelf == Shelf::Popular {
            query.push_bind(f64::from_bits(cursor.score_bits));
        } else {
            query.push_bind(cursor.at);
        }
        query.push(", ").push_bind(cursor.id).push(")");
    }
    query.push(" ORDER BY ").push(sort_column).push(" DESC, p.id DESC LIMIT ").push_bind(page.fetch());

    let mut rows: Vec<PackRow> = query.build_query_as().fetch_all(&core.db).await?;
    let info = page.finish(&mut rows, |pack| ShelfCursor::after(shelf, pack));
    Ok((rows, info))
}

/// Published packs matching a search query, best match first.
///
/// A query starting with `#` matches tags only. Anything else matches names
/// and descriptions by substring (which works for Japanese, where trigram
/// similarity does not), tags exactly, and names fuzzily.
pub async fn search(
    core: &Core,
    query: &str,
    price: pb::PriceFilter,
    page: &Page,
) -> AppResult<(Vec<PackRow>, PageInfo)> {
    let offset: i64 = page.decode()?.unwrap_or(0);
    let tags_only = query.starts_with('#');
    // The query as a tag, when it is a single word.
    let tag: Vec<String> = validate::normalize_word(query).into_iter().collect();
    let text = query.trim_start_matches('#');
    let contains = format!("%{}%", validate::like_escape(text));
    let starts_with = format!("{}%", validate::like_escape(text));

    let mut sql = QueryBuilder::new(concat!(
        "SELECT ",
        pack_columns!(),
        " FROM stamp_packs p WHERE p.status = 3 AND (p.tags && "
    ));
    sql.push_bind(tag.clone());
    if !tags_only {
        sql.push(" OR p.name ILIKE ").push_bind(contains.clone());
        sql.push(" OR p.description ILIKE ").push_bind(contains);
        sql.push(" OR p.name % ").push_bind(text.to_owned());
    }
    sql.push(")");
    push_price_filter(&mut sql, price);
    sql.push(" ORDER BY (lower(p.name) = lower(").push_bind(text.to_owned()).push(")) DESC, (p.tags && ");
    sql.push_bind(tag).push(") DESC, (p.name ILIKE ").push_bind(starts_with);
    sql.push(") DESC, similarity(p.name, ").push_bind(text.to_owned());
    sql.push(") DESC, p.popular_score DESC, p.id DESC LIMIT ").push_bind(page.fetch());
    sql.push(" OFFSET ").push_bind(offset);

    let mut rows: Vec<PackRow> = sql.build_query_as().fetch_all(&core.db).await?;
    let has_more = rows.len() as i64 > page.limit;
    rows.truncate(page.limit as usize);
    let next_cursor = if has_more { encode_cursor(&(offset + page.limit)) } else { String::new() };
    Ok((rows, PageInfo { next_cursor, has_more, ..Default::default() }))
}

/// The tags carried by the most published packs.
pub async fn popular_tags(core: &Core, limit: i64) -> AppResult<Vec<pb::TagCount>> {
    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT tag, count(*)::int AS pack_count
         FROM stamp_packs p CROSS JOIN LATERAL unnest(p.tags) AS tag
         WHERE p.status = 3
         GROUP BY tag
         ORDER BY pack_count DESC, tag
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&core.db)
    .await?;
    Ok(rows.into_iter().map(|(tag, pack_count)| pb::TagCount { tag, pack_count, ..Default::default() }).collect())
}

/// One page of a creator's published packs, most recently published first.
pub async fn by_creator(core: &Core, creator_id: Uuid, page: &Page) -> AppResult<(Vec<PackRow>, PageInfo)> {
    let cursor: Option<(Time, Uuid)> = page.decode()?;
    let (after_at, after_id) = cursor.unzip();
    let mut rows: Vec<PackRow> = sqlx::query_as(concat!(
        "SELECT ",
        pack_columns!(),
        " FROM stamp_packs p
          WHERE p.creator_id = $1 AND p.status = 3
            AND ($2::timestamptz IS NULL OR (p.published_at, p.id) < ($2, $3))
          ORDER BY p.published_at DESC, p.id DESC
          LIMIT $4"
    ))
    .bind(creator_id)
    .bind(after_at)
    .bind(after_id)
    .bind(page.fetch())
    .fetch_all(&core.db)
    .await?;
    let info = page.finish(&mut rows, |pack| (pack.published_at.unwrap_or(pack.created_at), pack.id));
    Ok((rows, info))
}
